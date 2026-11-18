//! One outstanding drawable acquisition per surface. The worker sleeps between requests.
use crate::Error;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    task::Poll,
    thread::{self, JoinHandle},
};

struct Request<T> {
    acquire: Box<dyn FnOnce() -> Result<T, Error> + Send>,
    notify: Box<dyn FnOnce() + Send>,
}

pub(super) struct Acquisition<T> {
    requests: Option<SyncSender<Request<T>>>,
    results: Option<Receiver<Result<T, Error>>>,
    worker: Option<JoinHandle<()>>,
    pending: bool,
}

impl<T: Send + 'static> Acquisition<T> {
    pub fn new() -> Result<Self, Error> {
        let (requests, jobs) = mpsc::sync_channel::<Request<T>>(1);
        let (done, results) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("oxiview-drawable".into())
            .spawn(move || {
                while let Ok(request) = jobs.recv() {
                    let acquired = catch_unwind(AssertUnwindSafe(request.acquire));
                    let stop = acquired.is_err();
                    let result = acquired.unwrap_or_else(|_| {
                        Err(Error::Surface(
                            "drawable acquisition worker panicked".into(),
                        ))
                    });
                    if done.send(result).is_err() {
                        break;
                    }
                    (request.notify)();
                    if stop {
                        break;
                    }
                }
            })
            .map_err(|e| Error::Surface(e.to_string()))?;
        Ok(Self {
            requests: Some(requests),
            results: Some(results),
            worker: Some(worker),
            pending: false,
        })
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    pub fn start(
        &mut self,
        acquire: impl FnOnce() -> Result<T, Error> + Send + 'static,
        notify: impl FnOnce() + Send + 'static,
    ) -> Result<(), Error> {
        if self.pending {
            return Err(Error::Surface(
                "a drawable is already being acquired".into(),
            ));
        }
        let requests = self.requests.as_ref().ok_or_else(stopped)?;
        requests
            .try_send(Request {
                acquire: Box::new(acquire),
                notify: Box::new(notify),
            })
            .map_err(|_| stopped())?;
        self.pending = true;
        Ok(())
    }
    pub fn poll(&mut self) -> Result<Poll<T>, Error> {
        match self.results.as_ref().ok_or_else(stopped)?.try_recv() {
            Ok(result) => {
                self.pending = false;
                result.map(Poll::Ready)
            }
            Err(TryRecvError::Empty) if self.pending => Ok(Poll::Pending),
            Err(_) => Err(stopped()),
        }
    }
    /// A synchronous render can consume an acquisition already in progress.
    pub fn take(&mut self) -> Result<Option<T>, Error> {
        if !self.pending {
            return Ok(None);
        }
        self.pending = false;
        self.results
            .as_ref()
            .ok_or_else(stopped)?
            .recv()
            .map_err(|_| stopped())?
            .map(Some)
    }
}

fn stopped() -> Error {
    Error::Surface("drawable acquisition worker disconnected".into())
}

impl<T> Acquisition<T> {
    /// Stop accepting jobs. Closing the receiver immediately discards any
    /// published texture; a still-blocked acquisition releases its own result.
    pub fn stop(&mut self) {
        self.requests.take();
        self.results.take();
        self.pending = false;
    }
}

impl<T> Drop for Acquisition<T> {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            tracing::error!("drawable acquisition worker panicked at shutdown");
        }
        // No worker or acquired texture can outlive window endpoint teardown.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    #[test]
    fn publishes_one_result_before_notifying_and_retires_held_textures_on_drop() -> Result<(), Error>
    {
        struct Texture(Arc<AtomicUsize>);
        impl Drop for Texture {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        let (release, blocked) = mpsc::channel();
        let (sent, received) = mpsc::channel();
        let mut worker = Acquisition::new()?;
        let texture_count = dropped.clone();
        worker.start(
            move || {
                blocked
                    .recv_timeout(Duration::from_secs(3))
                    .map_err(|_| stopped())?;
                Ok(Texture(texture_count))
            },
            move || {
                let _ = sent.send(());
            },
        )?;
        assert!(worker.poll()?.is_pending());
        assert!(worker.start(|| Err(stopped()), || {}).is_err());
        release.send(()).map_err(|_| stopped())?;
        received
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| stopped())?;
        let Poll::Ready(texture) = worker.poll()? else {
            return Err(stopped());
        };
        assert!(!worker.pending());
        drop(texture);
        let next = dropped.clone();
        worker.start(move || Ok(Texture(next)), || {})?;
        drop(worker.take()?);
        assert_eq!(dropped.load(Ordering::SeqCst), 2);
        let last = dropped.clone();
        worker.start(move || Ok(Texture(last)), || {})?;
        drop(worker);
        assert_eq!(dropped.load(Ordering::SeqCst), 3);
        Ok(())
    }

    #[test]
    fn suspension_abandons_a_blocked_acquisition_without_waiting_or_waking() -> Result<(), Error> {
        let (grant, pool) = mpsc::channel();
        let (notify, notified) = mpsc::channel();
        let mut worker = Acquisition::new()?;
        worker.start(
            move || {
                pool.recv_timeout(Duration::from_secs(3))
                    .map_err(|_| stopped())
            },
            move || {
                let _ = notify.send(());
            },
        )?;
        worker.stop();
        assert!(!worker.pending());
        grant.send(()).map_err(|_| stopped())?;
        drop(worker);
        assert!(notified.recv().is_err());
        Ok(())
    }
}
