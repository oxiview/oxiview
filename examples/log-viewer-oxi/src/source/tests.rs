use super::*;
use crate::model::{Column, Severity};
use std::{
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::atomic::AtomicU64,
    time::{Duration, Instant, UNIX_EPOCH},
};

pub(super) struct Fixture(PathBuf);
impl Fixture {
    pub(super) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/xtask/log-viewer-files")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&dir).expect("create isolated log directory");
        Self(dir)
    }
    pub(super) fn file(&self) -> PathBuf {
        self.0.join("stream.log")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn append(path: &Path, bytes: &[u8]) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open append")
        .write_all(bytes)
        .expect("append bytes");
}
fn batch(cursor: &mut Cursor, path: &Path, stop: &AtomicBool) -> Batch {
    let ReadOutcome::Batch(batch) = cursor.read(path, stop, false).expect("read log") else {
        panic!("expected a published batch");
    };
    batch
}

#[test]
fn incremental_file_preserves_partial_identity_split_unicode_and_follow_watermark() {
    let tmp = Fixture::new();
    let path = tmp.file();
    let mut cursor = Cursor::default();
    let mut book = LogBook::with_sample_count(4);
    let stop = AtomicBool::new(false);
    fs::write(&path, b"[INFO] api: first\r\n[WARN] api: hel").expect("write log");
    book.apply(&path, batch(&mut cursor, &path, &stop), true)
        .expect("apply");
    let key = book.record_at(1).expect("partial").id;
    assert_eq!(book.len(), 2);
    assert!(book.follow());
    assert!(key > 4);
    book.pause_follow();
    append(&path, b"lo \xe4");
    book.apply(&path, batch(&mut cursor, &path, &stop), false)
        .expect("apply");
    assert_eq!(book.record_at(1).expect("same row").message, "hello ");
    assert_eq!(book.unseen(), 0);
    append(&path, b"\xb8\xad\r\n[ERROR] io: retry\n");
    book.apply(&path, batch(&mut cursor, &path, &stop), false)
        .expect("apply");
    assert_eq!(book.index_of(key), Some(1));
    assert_eq!(book.cell(1, Column::Message), "hello 中");
    assert_eq!(book.unseen(), 1);
    assert!(matches!(
        cursor.read(&path, &stop, false),
        Ok(ReadOutcome::Unchanged)
    ));
    book.set_severity(Severity::Error, false);
    assert_eq!(book.unseen(), 0);
    book.set_severity(Severity::Error, true);
    book.sort_by(Column::Level);
    book.resume_follow();
    assert_eq!(book.sort(), None);
    assert_eq!(book.unseen(), 0);
    fs::write(&path, b"replacement\n").expect("truncate");
    book.apply(&path, batch(&mut cursor, &path, &stop), false)
        .expect("apply");
    assert_eq!(book.index_of(key), None);
    assert_eq!(book.total(), 1);
    assert_eq!(book.cell(0, Column::Message), "replacement");
    let old = book.record_at(0).expect("truncated row").id;
    fs::write(tmp.0.join("next"), b"replacement\n").expect("new inode");
    fs::rename(tmp.0.join("next"), &path).expect("rotate");
    book.apply(&path, batch(&mut cursor, &path, &stop), false)
        .expect("apply");
    assert_eq!(book.index_of(old), None);
}
fn wait_for(
    rx: &mpsc::Receiver<()>,
    monitor: &mut SourceMonitor,
    book: &mut LogBook,
    until: impl Fn(&LogBook, &[String]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut notices = Vec::new();
    loop {
        notices.extend(monitor.drain(book));
        if until(book, &notices) {
            break;
        }
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("file worker notification");
    }
}

#[test]
fn watcher_preserves_prior_stream_on_failed_open_then_switches_atomically() {
    let tmp = Fixture::new();
    let path = tmp.file();
    let mut monitor = SourceMonitor::default();
    let mut book = LogBook::with_sample_count(3);
    let (tx, rx) = mpsc::channel();
    let wake = move || tx.send(()).is_ok();
    fs::write(&path, "[INFO] live: first\n").expect("create log");
    monitor
        .open_with(path.clone(), wake.clone())
        .expect("monitor");
    wait_for(&rx, &mut monitor, &mut book, |b, _| b.len() == 1);
    let first = book.record_at(0).expect("active row").id;
    monitor
        .open_with(tmp.0.join("missing"), wake.clone())
        .expect("begin failing open");
    wait_for(&rx, &mut monitor, &mut book, |_, e| !e.is_empty());
    assert_eq!(book.index_of(first), Some(0));
    append(&path, b"second\n");
    wait_for(&rx, &mut monitor, &mut book, |b, _| b.len() == 2);
    let other = tmp.0.join("other.log");
    fs::write(&other, "new source").expect("create other");
    monitor.open_with(other, wake).expect("switch");
    wait_for(&rx, &mut monitor, &mut book, |b, _| {
        b.label() == "other.log"
    });
    assert_eq!(book.index_of(first), None);
    assert_eq!(book.cell(0, Column::Message), "new source");
}

#[test]
fn same_length_overwrite_resets_but_touch_and_duplicate_notifications_keep_keys() {
    let tmp = Fixture::new();
    let path = tmp.file();
    let mut cursor = Cursor::default();
    let stop = AtomicBool::new(false);
    let mut book = LogBook::with_sample_count(0);
    let context = "unchanged\n".repeat(500);
    let original = format!("{context}old record\n{context}");
    fs::write(&path, &original).expect("write log");
    book.apply(&path, batch(&mut cursor, &path, &stop), true)
        .expect("apply initial batch");
    book.pause_follow();
    let earlier = book.record_at(500).expect("original record").id;
    let initial_revision = (book.revision(), book.order_revision());

    let mut file = OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open log");
    file.set_modified(UNIX_EPOCH + Duration::from_secs(1))
        .expect("touch log");
    for _ in 0..2 {
        assert!(matches!(
            cursor.read(&path, &stop, false),
            Ok(ReadOutcome::Unchanged)
        ));
    }
    assert_eq!((book.revision(), book.order_revision()), initial_revision);
    assert_eq!(book.index_of(earlier), Some(500));
    assert!(!book.follow());

    file.seek(SeekFrom::Start(context.len() as u64))
        .expect("seek earlier line");
    file.write_all(b"new record\n")
        .expect("overwrite earlier line");
    assert_eq!(
        file.metadata().expect("metadata").len(),
        original.len() as u64
    );
    let rewrite = batch(&mut cursor, &path, &stop);
    assert!(rewrite.reset);
    book.apply(&path, rewrite, false).expect("apply overwrite");
    assert_eq!(book.cell(500, Column::Message), "new record");
    assert_eq!(book.index_of(earlier), None);
    assert_eq!(book.unseen(), 0);
    assert!(!book.follow());
}

#[test]
fn coalesced_truncate_and_regrow_with_old_eof_bytes_resets_at_the_head() {
    let tmp = Fixture::new();
    let path = tmp.file();
    let mut cursor = Cursor::default();
    let stop = AtomicBool::new(false);
    let context = "unchanged\n".repeat(500);
    fs::write(&path, format!("old record\n{context}")).expect("write log");
    assert!(batch(&mut cursor, &path, &stop).reset);
    fs::write(&path, format!("new record\n{context}appended\n")).expect("truncate and regrow");
    let rewrite = batch(&mut cursor, &path, &stop);
    assert!(rewrite.reset);
    assert_eq!(rewrite.records.len(), 502);
    assert_eq!(rewrite.records[0].message, "new record");
    assert_eq!(rewrite.records[501].message, "appended");
    assert!(!cursor.needs_audit());
}

#[test]
fn scheduled_audit_repairs_a_regrowth_between_the_guards() {
    let tmp = Fixture::new();
    let path = tmp.file();
    let mut monitor = SourceMonitor::default();
    let mut book = LogBook::with_sample_count(0);
    let (tx, rx) = mpsc::channel();
    let context = "unchanged\n".repeat(500);
    fs::write(&path, format!("{context}old record\n{context}")).expect("write log");
    monitor
        .open_with(path.clone(), move || tx.send(()).is_ok())
        .expect("watch log");
    wait_for(&rx, &mut monitor, &mut book, |b, _| b.len() == 1001);
    let old = book.record_at(500).expect("earlier record").id;
    let mut file = OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open writer");
    file.seek(SeekFrom::Start(context.len() as u64))
        .expect("seek earlier line");
    file.write_all(format!("new record\n{context}appended\n").as_bytes())
        .expect("rewrite and grow together");
    drop(file);
    wait_for(&rx, &mut monitor, &mut book, |b, _| {
        b.cell(500, Column::Message) == "new record"
    });
    assert_eq!(book.total(), 1002);
    assert_eq!(book.index_of(old), None);
    assert!(book.follow());
}

#[test]
fn long_partial_appends_keep_raw_bytes_and_verify_across_read_sizes() {
    let tmp = Fixture::new();
    let path = tmp.file();
    let mut cursor = Cursor::default();
    let stop = AtomicBool::new(false);
    let initial = "finished\n".repeat(8000);
    let raw = "a".repeat(72_000);
    fs::write(&path, format!("{initial}{raw}")).expect("write long stream");
    let mut book = LogBook::with_sample_count(0);
    book.apply(&path, batch(&mut cursor, &path, &stop), true)
        .expect("open stream");
    let last = 8000;
    let key = book.record_at(last).expect("long partial").id;
    for (suffix, visible) in [
        (&b"\xff\xf0"[..], "�"),
        (&b"\x9f\x92"[..], "�"),
        (&b"\xa1\r"[..], "�💡"),
        (&b"\n"[..], "�💡"),
    ] {
        append(&path, suffix);
        let update = batch(&mut cursor, &path, &stop);
        assert!(!update.reset && update.replace_partial);
        assert_eq!(update.records.len(), 1);
        assert!(cursor.needs_audit());
        book.apply(&path, update, false)
            .expect("extend long partial");
        assert_eq!(book.index_of(key), Some(last));
        assert_eq!(
            &book.record_at(last).expect("partial").raw[raw.len()..],
            visible
        );
    }
    assert!(matches!(
        cursor.read(&path, &stop, true),
        Ok(ReadOutcome::Unchanged)
    ));
    assert!(!cursor.needs_audit());
    assert_eq!(book.total(), last + 1);
}
