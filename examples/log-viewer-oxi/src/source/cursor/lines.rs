//! Stages newly read lines while leaving the previous raw partial available for retry.

use crate::model::Record;

#[derive(Default)]
pub(super) struct Lines {
    continuation: Vec<u8>,
    first_complete: bool,
    records: Vec<Record>,
    tail: Vec<u8>,
}

impl Lines {
    pub(super) fn extend(&mut self, mut bytes: &[u8]) {
        if !self.first_complete {
            let Some(end) = bytes.iter().position(|&byte| byte == b'\n') else {
                self.continuation.extend_from_slice(bytes);
                return;
            };
            self.continuation.extend_from_slice(&bytes[..end]);
            self.first_complete = true;
            bytes = &bytes[end + 1..];
        }
        let mut chunks = bytes.split(|&byte| byte == b'\n');
        if let Some(tail) = chunks.next_back() {
            for complete in chunks {
                self.tail.extend_from_slice(complete);
                self.records.push(Record::parse(line(&self.tail, false)));
                self.tail.clear();
            }
            self.tail.extend_from_slice(tail);
        }
    }

    pub(super) fn finish(self, partial: &mut Vec<u8>) -> (Vec<Record>, bool) {
        if partial.is_empty() {
            *partial = self.continuation;
        } else {
            partial.extend(self.continuation);
        }
        let mut records = self.records;
        if self.first_complete {
            records.insert(0, Record::parse(line(partial, false)));
            *partial = self.tail;
        }
        let visible = line(partial, true);
        let published_partial = !visible.is_empty() || partial.starts_with(b"\r");
        if published_partial {
            records.push(Record::parse(visible));
        }
        (records, published_partial)
    }
}

fn line(bytes: &[u8], provisional: bool) -> String {
    let mut bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    if provisional {
        // Only the last three bytes can start an incomplete UTF-8 scalar at EOF.
        let suffix = &bytes[bytes.len().saturating_sub(3)..];
        let mut rest = suffix;
        while let Err(error) = std::str::from_utf8(rest) {
            let valid = error.valid_up_to();
            let Some(invalid) = error.error_len() else {
                bytes = &bytes[..bytes.len() - (rest.len() - valid)];
                break;
            };
            rest = &rest[valid + invalid..];
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}
