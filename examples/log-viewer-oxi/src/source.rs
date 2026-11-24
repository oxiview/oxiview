//! Background subscriptions for local log files, with one ordered stream per Open request.

mod cursor;
#[cfg(test)]
mod tests;

use self::cursor::{Cursor, ReadOutcome};
use crate::model::{Batch, LogBook};
use oxiview_app::FileWatch;
use oxiview_platform::WindowWaker;
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const PREFIX_AUDIT_INTERVAL: Duration = Duration::from_secs(2);
const READ_RETRY_DELAY: Duration = Duration::from_millis(20);

struct Message {
    token: Arc<()>,
    result: io::Result<Batch>,
}
enum Signal {
    Changed(io::Result<()>),
    Close,
}
struct Subscription {
    path: PathBuf,
    token: Arc<()>,
    cancelled: Arc<AtomicBool>,
    control: mpsc::Sender<Signal>,
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.control.send(Signal::Close);
    }
}

/// Keeps the previous file live while a replacement Open request is in flight.
pub(crate) struct SourceMonitor {
    send: mpsc::Sender<Message>,
    receive: mpsc::Receiver<Message>,
    active: Option<Subscription>,
    pending: Option<Subscription>,
}
impl Default for SourceMonitor {
    fn default() -> Self {
        let (send, receive) = mpsc::channel();
        Self {
            send,
            receive,
            active: None,
            pending: None,
        }
    }
}
impl SourceMonitor {
    pub fn open(&mut self, path: PathBuf, waker: &WindowWaker) -> io::Result<()> {
        self.open_with(path, {
            let waker = waker.clone();
            move || waker.wake().is_ok()
        })
    }
    fn open_with(
        &mut self,
        path: PathBuf,
        wake: impl Fn() -> bool + Send + 'static,
    ) -> io::Result<()> {
        let (control, signals) = mpsc::channel();
        let sender = self.send.clone();
        let token = Arc::new(());
        let cancelled = Arc::new(AtomicBool::new(false));
        let subscription = Subscription {
            path: path.clone(),
            token: token.clone(),
            cancelled: cancelled.clone(),
            control: control.clone(),
        };
        thread::Builder::new()
            .name("oxiview-log-reader".into())
            .spawn(move || {
                let publish = |result| {
                    !cancelled.load(Ordering::Acquire)
                        && sender
                            .send(Message {
                                token: token.clone(),
                                result,
                            })
                            .is_ok()
                        && wake()
                };
                let watch = match FileWatch::start(&path, move |event| {
                    let _ = control.send(Signal::Changed(event));
                }) {
                    Ok(watch) => watch,
                    Err(e) => {
                        publish(Err(e));
                        return;
                    }
                };
                let mut cursor = Cursor::default();
                let mut audit_at = None;
                loop {
                    if cancelled.load(Ordering::Acquire) {
                        return;
                    }
                    let audit = audit_at.is_some_and(|deadline| Instant::now() >= deadline);
                    let outcome = cursor.read(watch.path(), &cancelled, audit);
                    let retry = matches!(outcome, Ok(ReadOutcome::Retry));
                    let succeeded = outcome.is_ok();
                    match outcome {
                        Ok(ReadOutcome::Batch(batch)) => {
                            if !publish(Ok(batch)) {
                                return;
                            }
                        }
                        Ok(ReadOutcome::Unchanged | ReadOutcome::Retry) => {}
                        Err(e) => {
                            if !publish(Err(e)) || !cursor.has_read() {
                                return;
                            }
                            audit_at = None;
                        }
                    }
                    if !cursor.needs_audit() {
                        audit_at = None;
                    } else if audit && retry {
                        // Give the append path another turn before retrying an interrupted scan.
                        audit_at = Some(Instant::now() + PREFIX_AUDIT_INTERVAL);
                    } else if succeeded && !retry {
                        // A burst of growth shares a prefix audit even if notifications keep coming.
                        audit_at.get_or_insert_with(|| Instant::now() + PREFIX_AUDIT_INTERVAL);
                    }
                    let timeout = if retry {
                        Some(READ_RETRY_DELAY)
                    } else {
                        audit_at.map(|at| at.saturating_duration_since(Instant::now()))
                    };
                    let first = match timeout {
                        Some(delay) => match signals.recv_timeout(delay) {
                            Ok(signal) => signal,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        },
                        None => match signals.recv() {
                            Ok(signal) => signal,
                            Err(_) => return,
                        },
                    };
                    for signal in std::iter::once(first).chain(signals.try_iter()) {
                        match signal {
                            Signal::Close => return,
                            Signal::Changed(Err(e)) => {
                                if !publish(Err(e)) {
                                    return;
                                }
                            }
                            Signal::Changed(Ok(())) => {}
                        }
                    }
                }
            })?;
        self.pending = Some(subscription);
        Ok(())
    }
    pub fn drain(&mut self, book: &mut LogBook) -> Vec<String> {
        let mut notices = Vec::new();
        for msg in self.receive.try_iter() {
            let pending = self
                .pending
                .as_ref()
                .is_some_and(|sub| Arc::ptr_eq(&sub.token, &msg.token));
            let Some(sub) = (if pending {
                self.pending.as_ref()
            } else {
                self.active
                    .as_ref()
                    .filter(|sub| Arc::ptr_eq(&sub.token, &msg.token))
            }) else {
                continue;
            };
            let result = msg
                .result
                .and_then(|batch| book.apply(&sub.path, batch, pending));
            match result {
                Err(e) => {
                    notices.push(format!("{}: {e}", sub.path.display()));
                    if pending {
                        self.pending = None;
                    }
                }
                Ok(()) if pending => {
                    notices.push(format!("Opened {}", sub.path.display()));
                    self.active = self.pending.take();
                }
                Ok(()) => {}
            }
        }
        notices
    }
}
