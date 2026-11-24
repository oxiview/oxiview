//! The log source and its ordered, filtered projection. Row identity survives a viewport change.

mod sample;
#[cfg(test)]
mod tests;

use std::{borrow::Cow, cmp::Ordering, io, path::Path};

/// Filterable log level, ordered from least to most severe.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    /// Detailed execution tracing.
    Trace,
    /// Diagnostic output.
    Debug,
    /// Routine activity, including unstructured text.
    Info,
    /// Recoverable condition.
    Warn,
    /// Failed operation.
    Error,
}

impl Severity {
    /// Levels in ascending severity.
    pub const ALL: [Self; 5] = [
        Self::Trace,
        Self::Debug,
        Self::Info,
        Self::Warn,
        Self::Error,
    ];
    /// Compact display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
    fn parse(token: &str) -> Option<Self> {
        let word = token.trim_matches(['[', ']', ':']);
        Self::ALL
            .into_iter()
            .find(|level| level.label().eq_ignore_ascii_case(word))
            .or_else(|| word.eq_ignore_ascii_case("warning").then_some(Self::Warn))
    }
}

/// Stable table field keys in the initial display order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Column {
    /// Original line number and row header.
    Number,
    /// Source timestamp.
    Time,
    /// Parsed log severity.
    Level,
    /// Emitting component.
    Source,
    /// Log message.
    Message,
    /// Originating thread when available.
    Thread,
    /// UTF-8 byte length of the original line.
    Bytes,
}
impl Column {
    /// Original table order.
    pub const ALL: [Self; 7] = [
        Self::Number,
        Self::Time,
        Self::Level,
        Self::Source,
        Self::Message,
        Self::Thread,
        Self::Bytes,
    ];
    /// Table header label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Number => "Line",
            Self::Time => "Time",
            Self::Level => "Level",
            Self::Source => "Source",
            Self::Message => "Message",
            Self::Thread => "Thread",
            Self::Bytes => "Bytes",
        }
    }
}

/// A parsed record, including its original text for whole-row Copy.
#[derive(Debug)]
pub struct Record {
    /// Identity unique within this viewer, including after reopening a file.
    pub id: u64,
    /// Original line in the current source, before filtering and sorting.
    pub line: usize,
    /// Complete original record without the line ending.
    pub raw: String,
    /// Message displayed in the list and detail pane.
    pub message: String,
    /// Parsed level, with Info as the default for plain text.
    pub severity: Severity,
    /// Original timestamp if supplied.
    pub time: String,
    /// Component or facility if supplied.
    pub source: String,
    /// Thread name if supplied.
    pub thread: String,
    folded: String,
}

impl Record {
    pub(crate) fn parse(raw: String) -> Self {
        let mut rest = raw.as_str().trim_start();
        let mut time = "";
        let (first, following) = token(rest);
        if first.as_bytes().first().is_some_and(u8::is_ascii_digit) && first.contains(':') {
            time = first;
            rest = following;
        }
        let (level, following) = token(rest);
        let parsed = Severity::parse(level);
        let mut source = "";
        let mut thread = "";
        let message = if parsed.is_some() {
            rest = following;
            let (component, following) = token(rest);
            if let Some(name) = component.strip_suffix(':') {
                (source, thread) = name.split_once('/').unwrap_or((name, ""));
                rest = following;
            }
            rest.to_owned()
        } else {
            raw.clone()
        };
        Self {
            id: 0,
            line: 0,
            severity: parsed.unwrap_or(Severity::Info),
            time: time.into(),
            source: source.into(),
            thread: thread.into(),
            message,
            folded: raw.to_lowercase(),
            raw,
        }
    }
    fn compare(&self, other: &Self, col: Column) -> Ordering {
        match col {
            Column::Number => self.line.cmp(&other.line),
            Column::Time => self.time.cmp(&other.time),
            Column::Level => self.severity.cmp(&other.severity),
            Column::Source => self.source.cmp(&other.source),
            Column::Message => self.message.cmp(&other.message),
            Column::Thread => self.thread.cmp(&other.thread),
            Column::Bytes => self.raw.len().cmp(&other.raw.len()),
        }
    }
    fn cell(&self, col: Column) -> Cow<'_, str> {
        match col {
            Column::Number => self.line.to_string().into(),
            Column::Time => self.time.as_str().into(),
            Column::Level => self.severity.label().into(),
            Column::Source => self.source.as_str().into(),
            Column::Message => self.message.as_str().into(),
            Column::Thread => self.thread.as_str().into(),
            Column::Bytes => self.raw.len().to_string().into(),
        }
    }
}
fn token(text: &str) -> (&str, &str) {
    let split = text.find(char::is_whitespace).unwrap_or(text.len());
    (&text[..split], text[split..].trim_start())
}

/// An ordered batch from the file worker.
pub(crate) struct Batch {
    pub reset: bool,
    pub replace_partial: bool,
    pub records: Vec<Record>,
}

/// Source rows, table order and Follow state owned by one viewer window.
pub struct LogBook {
    records: Vec<Record>,
    order: Vec<usize>,
    positions: Vec<Option<usize>>,
    next: u64,
    sort: Option<(Column, bool)>,
    levels: [bool; 5],
    query: String,
    needle: String,
    label: String,
    follow: bool,
    pause_watermark: u64,
    unseen: usize,
    revision: u64,
    order_revision: u64,
}
impl LogBook {
    /// Matching row count.
    pub fn len(&self) -> usize {
        self.order.len()
    }
    /// True when no records match.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    /// Unfiltered source count.
    pub fn total(&self) -> usize {
        self.records.len()
    }
    /// Current source display name.
    pub fn label(&self) -> &str {
        &self.label
    }
    /// Data or ordering revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Revision for changes that can affect the identity of a selected range.
    pub fn order_revision(&self) -> u64 {
        self.order_revision
    }
    /// Gets a record in displayed order.
    pub fn record_at(&self, row: usize) -> Option<&Record> {
        self.records.get(*self.order.get(row)?)
    }
    /// Looks up the visible index of a stable record key.
    pub fn index_of(&self, key: u64) -> Option<usize> {
        let ix = usize::try_from(key.checked_sub(self.records.first()?.id)?).ok()?;
        self.positions.get(ix).copied().flatten()
    }
    /// Returns the displayed field from the data model, including unmounted cells.
    pub fn cell(&self, row: usize, col: Column) -> Cow<'_, str> {
        self.record_at(row)
            .map_or(Cow::Borrowed(""), |r| r.cell(col))
    }
    /// Committed search query.
    pub fn query(&self) -> &str {
        &self.query
    }
    /// Replaces the committed filter text.
    pub fn set_query(&mut self, value: &str) -> bool {
        if self.query == value {
            return false;
        }
        self.query = value.into();
        self.needle = value.to_lowercase();
        self.reorder();
        true
    }
    /// Whether a level is included.
    pub fn severity_enabled(&self, value: Severity) -> bool {
        self.levels[value as usize]
    }
    /// Updates one level filter.
    pub fn set_severity(&mut self, value: Severity, enabled: bool) -> bool {
        if self.severity_enabled(value) == enabled {
            return false;
        }
        self.levels[value as usize] = enabled;
        self.reorder();
        true
    }
    /// Current explicit sort and whether its direction is ascending.
    pub fn sort(&self) -> Option<(Column, bool)> {
        self.sort
    }
    /// Changes the sort column, toggling direction on a repeated header activation.
    pub fn sort_by(&mut self, column: Column) {
        self.pause_follow();
        self.sort = Some((
            column,
            !matches!(self.sort, Some((prev,true)) if prev==column),
        ));
        self.reorder();
    }
    /// Whether arrivals should reveal the tail.
    pub fn follow(&self) -> bool {
        self.follow
    }
    /// Number of newly arrived matching records since Follow paused.
    pub fn unseen(&self) -> usize {
        self.unseen
    }
    /// Pauses following while preserving the arrival watermark on repeated gestures.
    pub fn pause_follow(&mut self) {
        if self.follow {
            self.follow = false;
            self.pause_watermark = self.next.saturating_sub(1);
            self.unseen = 0;
            self.revision += 1;
        }
    }
    /// Restores arrival order, clears the notice and follows future arrivals.
    pub fn resume_follow(&mut self) {
        self.follow = true;
        self.unseen = 0;
        self.pause_watermark = self.next.saturating_sub(1);
        if self.sort.take().is_some() {
            self.reorder()
        } else {
            self.revision += 1;
        }
    }
    fn reorder(&mut self) {
        self.rebuild();
        self.order_revision += 1;
        self.revision += 1;
    }
    fn matches(&self, record: &Record) -> bool {
        self.levels[record.severity as usize] && record.folded.contains(&self.needle)
    }

    fn rebuild(&mut self) {
        self.order.clear();
        for (index, record) in self.records.iter().enumerate() {
            if self.matches(record) {
                self.order.push(index);
            }
        }
        if let Some(sort) = self.sort {
            let records = &self.records;
            self.order
                .sort_unstable_by(|&a, &b| compare(records, a, b, sort));
        }
        self.positions.clear();
        self.positions.resize(self.records.len(), None);
        self.unseen = 0;
        for (pos, &ix) in self.order.iter().enumerate() {
            self.positions[ix] = Some(pos);
            if !self.follow && self.records[ix].id > self.pause_watermark {
                self.unseen += 1;
            }
        }
    }

    fn patch_sorted_tail(&mut self, mut replacement: Record, sort: (Column, bool)) {
        let index = self.records.len() - 1;
        let previous = self.positions[index];
        let visible = self.matches(&replacement);
        let old = &mut self.records[index];
        replacement.id = old.id;
        replacement.line = old.line;
        if !self.follow && old.id > self.pause_watermark {
            self.unseen = self.unseen + usize::from(visible) - usize::from(previous.is_some());
        }
        *old = replacement;

        let old_len = self.order.len();
        if let Some(position) = previous {
            let ordered = |a, b| compare(&self.records, a, b, sort).is_lt();
            let after_previous = position == 0 || ordered(self.order[position - 1], index);
            let before_next = position + 1 == old_len || ordered(index, self.order[position + 1]);
            if visible && after_previous && before_next {
                return;
            }
            self.order.remove(position);
            self.positions[index] = None;
        }
        let mut dirty_from = previous.unwrap_or(old_len);
        if visible {
            let position = self
                .order
                .partition_point(|&other| compare(&self.records, other, index, sort).is_lt());
            self.order.insert(position, index);
            dirty_from = dirty_from.min(position);
        }
        for (position, &index) in self.order.iter().enumerate().skip(dirty_from) {
            self.positions[index] = Some(position);
        }
        if dirty_from < old_len {
            self.order_revision += 1;
        }
    }

    pub(crate) fn apply(&mut self, path: &Path, mut batch: Batch, opening: bool) -> io::Result<()> {
        let replace = batch.replace_partial && !batch.reset && !self.records.is_empty();
        if replace && batch.records.is_empty() {
            return Err(io::Error::other("missing partial record replacement"));
        }
        let added = batch.records.len() - usize::from(replace);
        self.next
            .checked_add(added as u64)
            .ok_or_else(|| io::Error::other("log record identities exhausted"))?;
        if replace
            && added == 0
            && let Some(sort) = self.sort
        {
            let replacement = batch
                .records
                .pop()
                .ok_or_else(|| io::Error::other("missing partial record replacement"))?;
            self.patch_sorted_tail(replacement, sort);
            self.revision += 1;
            return Ok(());
        }
        let previous = if !batch.reset && self.sort.is_some() {
            Some(self.order.clone())
        } else {
            None
        };
        let old_tail_visible = replace && self.positions.last().copied().flatten().is_some();
        if batch.reset {
            self.records.clear();
            self.order.clear();
            self.positions.clear();
            if opening {
                self.follow = true;
                self.sort = None;
            }
            self.pause_watermark = self.next.saturating_add(added as u64).saturating_sub(1);
            self.unseen = 0;
            self.label = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned();
        }
        let first = self.records.len().saturating_sub(usize::from(replace));
        if old_tail_visible && self.sort.is_none() {
            self.order.pop();
            if !self.follow && self.records[first].id > self.pause_watermark {
                self.unseen = self.unseen.saturating_sub(1);
            }
            self.positions[first] = None;
        }
        let mut items = batch.records.drain(..);
        if replace && let (Some(old), Some(mut new)) = (self.records.last_mut(), items.next()) {
            new.id = old.id;
            new.line = old.line;
            *old = new;
        }
        for mut item in items {
            item.id = self.next;
            item.line = self.records.len() + 1;
            self.next += 1;
            self.records.push(item);
        }
        if self.sort.is_some() {
            self.rebuild();
            if previous.is_some_and(|old| !self.order.starts_with(&old)) {
                self.order_revision += 1;
            }
        } else {
            self.positions.resize(self.records.len(), None);
            for i in first..self.records.len() {
                let r = &self.records[i];
                if self.matches(r) {
                    self.positions[i] = Some(self.order.len());
                    self.order.push(i);
                    if !self.follow && r.id > self.pause_watermark {
                        self.unseen += 1;
                    }
                }
            }
            if old_tail_visible && self.positions[first].is_none() {
                self.order_revision += 1;
            }
        }
        self.revision += 1;
        if batch.reset {
            self.order_revision += 1;
        }
        Ok(())
    }
}

fn compare(
    records: &[Record],
    a: usize,
    b: usize,
    (column, ascending): (Column, bool),
) -> Ordering {
    let order = records[a].compare(&records[b], column);
    (if ascending { order } else { order.reverse() }).then(a.cmp(&b))
}
