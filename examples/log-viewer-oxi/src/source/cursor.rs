//! Reads a stable snapshot of the watched path and advances its byte cursor transactionally.

mod lines;

use self::lines::Lines;
use crate::model::Batch;
use std::{
    fs::{self, File, Metadata},
    hash::{DefaultHasher, Hasher},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

const HEAD_SIZE: usize = 4096;
const TAIL_SIZE: usize = 128;
const HASH_BLOCK: usize = 1024;

#[cfg(unix)]
type FileIdentity = (u64, u64);
#[cfg(not(unix))]
type FileIdentity = Option<SystemTime>;

#[derive(Clone, Copy, Eq, PartialEq)]
struct Stamp {
    identity: FileIdentity,
    length: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    changed: (i64, i64),
}

impl Stamp {
    fn of(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        Self {
            #[cfg(unix)]
            identity: (metadata.dev(), metadata.ino()),
            #[cfg(not(unix))]
            identity: metadata.created().ok(),
            length: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

#[derive(Clone, Default)]
struct Fingerprint {
    hash: DefaultHasher,
    pending: Vec<u8>,
}

impl Fingerprint {
    fn extend(&mut self, mut bytes: &[u8]) {
        // Fixed write boundaries make the digest independent of read and append boundaries.
        if !self.pending.is_empty() {
            let needed = (HASH_BLOCK - self.pending.len()).min(bytes.len());
            let (fill, rest) = bytes.split_at(needed);
            self.pending.extend_from_slice(fill);
            if self.pending.len() < HASH_BLOCK {
                return;
            }
            self.hash.write(&self.pending);
            self.pending.clear();
            bytes = rest;
        }
        let blocks = bytes.chunks_exact(HASH_BLOCK);
        self.pending.extend_from_slice(blocks.remainder());
        for block in blocks {
            self.hash.write(block);
        }
    }

    fn digest(&self) -> u64 {
        let mut hash = self.hash.clone();
        hash.write(&self.pending);
        hash.finish()
    }
}

#[derive(Clone, Default)]
struct Checkpoint {
    head: Vec<u8>,
    tail: Vec<u8>,
    fingerprint: Fingerprint,
}

impl Checkpoint {
    fn extend(&mut self, bytes: &[u8]) {
        let head_bytes = (HEAD_SIZE - self.head.len()).min(bytes.len());
        self.head.extend_from_slice(&bytes[..head_bytes]);
        if bytes.len() >= TAIL_SIZE {
            self.tail.clear();
            self.tail
                .extend_from_slice(&bytes[bytes.len() - TAIL_SIZE..]);
        } else {
            let discard = (self.tail.len() + bytes.len()).saturating_sub(TAIL_SIZE);
            self.tail.drain(..discard);
            self.tail.extend_from_slice(bytes);
        }
        self.fingerprint.extend(bytes);
    }

    fn guards_match(&self, file: &mut File, length: u64) -> io::Result<bool> {
        file.seek(SeekFrom::Start(0))?;
        let mut head = [0; HEAD_SIZE];
        file.read_exact(&mut head[..self.head.len()])?;
        if head[..self.head.len()] != self.head {
            return Ok(false);
        }
        if length <= self.head.len() as u64 {
            return Ok(true);
        }
        file.seek(SeekFrom::Start(length - self.tail.len() as u64))?;
        let mut tail = [0; TAIL_SIZE];
        file.read_exact(&mut tail[..self.tail.len()])?;
        Ok(tail[..self.tail.len()] == self.tail)
    }

    fn prefix_matches(
        &self,
        file: &mut File,
        length: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<bool> {
        let mut fingerprint = Fingerprint::default();
        read_range(file, 0, length, cancelled, |bytes| {
            fingerprint.extend(bytes)
        })?;
        Ok(fingerprint.digest() == self.fingerprint.digest())
    }
}

#[derive(Default)]
pub(super) struct Cursor {
    stamp: Option<Stamp>,
    checkpoint: Checkpoint,
    partial: Vec<u8>,
    published_partial: bool,
    needs_audit: bool,
}

pub(super) enum ReadOutcome {
    Unchanged,
    Batch(Batch),
    Retry,
}

struct Prepared {
    reset: bool,
    checkpoint: Checkpoint,
    lines: Lines,
}

impl Cursor {
    pub(super) fn has_read(&self) -> bool {
        self.stamp.is_some()
    }

    pub(super) fn needs_audit(&self) -> bool {
        self.needs_audit
    }

    pub(super) fn read(
        &mut self,
        path: &Path,
        cancelled: &AtomicBool,
        audit: bool,
    ) -> io::Result<ReadOutcome> {
        check_cancelled(cancelled)?;
        let mut file = File::open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a regular log file",
            ));
        }
        let stamp = Stamp::of(&metadata);
        if self.stamp == Some(stamp) && stamp.modified.is_some() && !audit {
            return Ok(ReadOutcome::Unchanged);
        }
        let prepared = self.prepare(&mut file, stamp, cancelled, audit);
        self.finish(path, stamp, prepared, cancelled, audit)
    }

    fn prepare(
        &self,
        file: &mut File,
        stamp: Stamp,
        cancelled: &AtomicBool,
        audit: bool,
    ) -> io::Result<Option<Prepared>> {
        let offset = self.stamp.map_or(0, |s| s.length);
        let mut reset = self
            .stamp
            .is_none_or(|old| old.identity != stamp.identity || stamp.length < offset);
        if !reset {
            // Growth normally needs only the head and old EOF guards. Same-length changes
            // and deferred audits verify the entire consumed prefix against its digest.
            reset = if stamp.length == offset || audit {
                !self.checkpoint.prefix_matches(file, offset, cancelled)?
            } else {
                !self.checkpoint.guards_match(file, offset)?
            };
        }
        if !reset && stamp.length == offset {
            return Ok(None);
        }
        let start = if reset { 0 } else { offset };
        let mut checkpoint = if reset {
            Checkpoint::default()
        } else {
            self.checkpoint.clone()
        };
        let mut lines = Lines::default();
        read_range(file, start, stamp.length - start, cancelled, |bytes| {
            checkpoint.extend(bytes);
            lines.extend(bytes);
        })?;
        Ok(Some(Prepared {
            reset,
            checkpoint,
            lines,
        }))
    }

    fn finish(
        &mut self,
        path: &Path,
        stamp: Stamp,
        prepared: io::Result<Option<Prepared>>,
        cancelled: &AtomicBool,
        audit: bool,
    ) -> io::Result<ReadOutcome> {
        check_cancelled(cancelled)?;
        // Recheck the pathname, so a rotation during the read cannot publish the old handle.
        match fs::metadata(path) {
            Ok(current) if Stamp::of(&current) == stamp => {}
            Ok(_) => return Ok(ReadOutcome::Retry),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(ReadOutcome::Retry),
            Err(error) => return Err(error),
        }
        let prepared = prepared?;
        let guarded_prefix = self
            .stamp
            .is_some_and(|old| old.length <= (HEAD_SIZE + TAIL_SIZE) as u64);
        self.stamp = Some(stamp);
        let Some(Prepared {
            reset,
            checkpoint,
            lines,
        }) = prepared
        else {
            self.needs_audit = false;
            return Ok(ReadOutcome::Unchanged);
        };
        self.checkpoint = checkpoint;
        self.needs_audit = !reset && !audit && !guarded_prefix;
        let replace_partial = !reset && self.published_partial;
        if reset {
            self.partial.clear();
        }
        let (records, published_partial) = lines.finish(&mut self.partial);
        self.published_partial = published_partial;
        Ok(ReadOutcome::Batch(Batch {
            reset,
            replace_partial,
            records,
        }))
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "source switched",
        ));
    }
    Ok(())
}

fn read_range(
    file: &mut File,
    start: u64,
    length: u64,
    cancelled: &AtomicBool,
    mut consume: impl FnMut(&[u8]),
) -> io::Result<()> {
    file.seek(SeekFrom::Start(start))?;
    let mut limited = file.take(length);
    let mut bytes = [0; 65536];
    while limited.limit() > 0 {
        check_cancelled(cancelled)?;
        let got = limited.read(&mut bytes)?;
        if got == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "log changed during read",
            ));
        }
        consume(&bytes[..got]);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
