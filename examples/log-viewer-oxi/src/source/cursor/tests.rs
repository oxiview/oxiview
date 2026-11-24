use super::*;
use crate::source::tests::Fixture;

#[test]
fn rotating_between_prepare_and_finish_discards_the_entire_staged_batch() {
    let tmp = Fixture::new();
    let path = tmp.file();
    let mut cursor = Cursor::default();
    let stop = AtomicBool::new(false);
    fs::write(&path, b"first\nsecond \xe4").expect("write log");
    assert!(matches!(
        cursor.read(&path, &stop, false),
        Ok(ReadOutcome::Batch(_))
    ));
    let before = cursor.stamp;
    let before_digest = cursor.checkpoint.fingerprint.digest();

    fs::write(&path, b"first\nsecond \xe4\xb8\xad\nthird\n").expect("stage growth");
    let mut file = File::open(&path).expect("open file");
    let stamp = Stamp::of(&file.metadata().expect("read descriptor metadata"));
    let prepared = cursor.prepare(&mut file, stamp, &stop, false);
    assert!(matches!(&prepared, Ok(Some(update)) if !update.reset));
    let replacement = path.with_extension("next");
    fs::write(&replacement, b"actual replacement\n").expect("write replacement");
    fs::rename(replacement, &path).expect("rotate before commit");
    assert!(matches!(
        cursor.finish(&path, stamp, prepared, &stop, false),
        Ok(ReadOutcome::Retry)
    ));
    assert!(cursor.stamp == before);
    assert_eq!(cursor.partial, b"second \xe4");
    assert_eq!(cursor.checkpoint.fingerprint.digest(), before_digest);
    assert!(cursor.published_partial);

    let ReadOutcome::Batch(batch) = cursor.read(&path, &stop, false).expect("retry") else {
        panic!("expected a replacement batch");
    };
    assert!(batch.reset && !batch.replace_partial);
    assert_eq!(batch.records.len(), 1);
    assert_eq!(batch.records[0].message, "actual replacement");
}
