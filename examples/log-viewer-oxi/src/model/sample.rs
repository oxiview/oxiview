//! Reproducible log records for the demo and headless replay.

use super::{LogBook, Record, Severity};

impl LogBook {
    /// Builds the reproducible 100,000-record acceptance source.
    pub fn sample() -> Self {
        Self::with_sample_count(100_000)
    }
    pub(crate) fn with_sample_count(count: usize) -> Self {
        const COMPONENTS: [&str; 8] = [
            "gateway",
            "scheduler",
            "storage",
            "renderer",
            "transport",
            "indexer",
            "worker",
            "runtime",
        ];
        const MESSAGES: [&str; 12] = [
            "Request completed successfully",
            "Loaded segment from the local cache",
            "Connection pool entered the ready state",
            "Published the latest index revision",
            "Scheduled background checkpoint",
            "Waiting for an available connection",
            "Worker acknowledged the batch",
            "Flushed buffered records to disk",
            "Retrying upstream request after timeout",
            "Client disconnected before the response completed",
            "Updated route configuration",
            "Reconciled outstanding acknowledgements",
        ];
        let mut book = Self {
            records: Vec::with_capacity(count),
            order: Vec::with_capacity(count),
            positions: Vec::with_capacity(count),
            next: 1,
            sort: None,
            levels: [true; 5],
            query: String::new(),
            needle: String::new(),
            label: "Sample stream".into(),
            follow: false,
            pause_watermark: 0,
            unseen: 0,
            revision: 1,
            order_revision: 1,
        };
        for n in 0..count {
            let severity = match n % 23 {
                4 | 11 | 18 => Severity::Warn,
                8 => Severity::Error,
                1 | 6 | 15 => Severity::Debug,
                3 => Severity::Trace,
                _ => Severity::Info,
            };
            let millis = 8 * 3_600_000 + 41 * 60_000 + n * 137;
            let time = format!(
                "{:02}:{:02}:{:02}.{:03}",
                millis / 3_600_000 % 24,
                millis / 60_000 % 60,
                millis / 1000 % 60,
                millis % 1000
            );
            let component = COMPONENTS[n % COMPONENTS.len()];
            let mut message = format!(
                "{} · request {:06x}",
                MESSAGES[(n + n / 8) % MESSAGES.len()],
                n + 1
            );
            if n % 13 == 5 {
                message.push_str(" · The pending operation retained its original sequence number while waiting for the next available connection. The worker will resume from the last acknowledged checkpoint and preserve delivery order.");
            }
            let thread = format!("pool-{:02}", n % 16 + 1);
            let raw = format!(
                "{time} [{}] {component}/{thread}: {message}",
                severity.label()
            );
            book.records.push(Record {
                id: book.next,
                line: n + 1,
                folded: raw.to_lowercase(),
                raw,
                message,
                severity,
                time,
                source: component.into(),
                thread,
            });
            book.next += 1;
            book.order.push(n);
            book.positions.push(Some(n));
        }
        book.pause_watermark = book.next.saturating_sub(1);
        book
    }
}
