use super::*;

fn apply(book: &mut LogBook, opening: bool, replace: bool, lines: &[&str]) {
    book.apply(
        Path::new("stream.log"),
        Batch {
            reset: opening,
            replace_partial: replace,
            records: lines
                .iter()
                .map(|line| Record::parse((*line).into()))
                .collect(),
        },
        opening,
    )
    .expect("apply batch");
}

#[test]
fn sorted_partial_tracks_matching_arrivals_and_only_changes_order_when_needed() {
    let mut book = LogBook::with_sample_count(0);
    apply(
        &mut book,
        true,
        false,
        &["[INFO] existing", "[WARN] another"],
    );
    let first = book.record_at(0).expect("first row").id;
    let second = book.record_at(1).expect("second row").id;
    book.set_severity(Severity::Error, false);
    book.sort_by(Column::Level);
    book.sort_by(Column::Level);
    assert!(!book.follow());
    assert_eq!(book.sort(), Some((Column::Level, false)));
    apply(&mut book, false, false, &["[ER"]);
    let key = book.record_at(2).expect("partial row").id;
    let before = book.order_revision();
    assert_eq!(book.unseen(), 1);

    apply(&mut book, false, true, &["[ERRO"]);
    assert_eq!(book.index_of(key), Some(2));
    assert_eq!(book.order_revision(), before);
    assert_eq!(book.unseen(), 1);

    apply(&mut book, false, true, &["[ERROR]"]);
    assert_eq!(book.index_of(key), None);
    assert_eq!(book.order_revision(), before + 1);
    assert_eq!(book.unseen(), 0);
    assert_eq!(book.index_of(first), Some(1));
    assert_eq!(book.index_of(second), Some(0));

    book.set_severity(Severity::Error, true);
    assert_eq!(book.index_of(key), Some(0));
    assert_eq!(book.unseen(), 1);
    book.resume_follow();
    assert!(book.follow());
    assert_eq!(book.sort(), None);
    assert_eq!(book.index_of(key), Some(2));
    assert_eq!(book.unseen(), 0);
}

#[test]
fn patch_that_moves_sorted_tail_keeps_ties_and_repairs_positions() {
    let mut book = LogBook::with_sample_count(0);
    apply(&mut book, true, false, &["abcde", "abcde", "medium"]);
    let first = book.record_at(0).expect("first row").id;
    let second = book.record_at(1).expect("second row").id;
    let third = book.record_at(2).expect("third row").id;
    book.sort_by(Column::Bytes);
    apply(&mut book, false, false, &["a"]);
    let partial = book.record_at(0).expect("shortest row").id;
    let before = book.order_revision();

    apply(&mut book, false, true, &["abcde"]);
    assert_eq!(book.index_of(first), Some(0));
    assert_eq!(book.index_of(second), Some(1));
    assert_eq!(book.index_of(partial), Some(2));
    assert_eq!(book.index_of(third), Some(3));
    assert_eq!(book.order_revision(), before + 1);
    assert_eq!(book.unseen(), 1);

    apply(&mut book, false, true, &["abcdefgh", "x"]);
    assert_eq!(book.index_of(partial), Some(4));
    assert_eq!(book.index_of(third), Some(3));
    assert_eq!(book.unseen(), 2);
}

#[test]
fn filtered_partial_becomes_a_single_matching_arrival_in_both_orders() {
    for sort in [None, Some(Column::Bytes)] {
        let mut book = LogBook::with_sample_count(0);
        apply(&mut book, true, false, &["has needle"]);
        book.set_query("needle");
        book.pause_follow();
        if let Some(column) = sort {
            book.sort_by(column);
        }
        let first = book.record_at(0).expect("first matching row").id;
        apply(&mut book, false, false, &["ne"]);
        let before = book.order_revision();
        assert_eq!(book.unseen(), 0);
        assert_eq!(book.total(), 2);

        apply(&mut book, false, true, &["needle on tail"]);
        let tail = book.record_at(1).expect("newly matching row");
        assert_eq!(tail.id, first + 1);
        assert_eq!(tail.line, 2);
        assert_eq!(book.order_revision(), before);
        assert_eq!(book.unseen(), 1);
        apply(&mut book, false, true, &["needle on tail continued"]);
        assert_eq!(book.unseen(), 1);
    }
}
