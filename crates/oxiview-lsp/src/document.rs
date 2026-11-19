use oxiview_syntax::{Analysis, Source, analyze, release_span_cache};
use serde::{Deserialize, Serialize};
use std::{ops::Range, path::Path};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct Position {
    pub line: usize,
    pub character: usize,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct TextRange {
    pub start: Position,
    pub end: Position,
}

#[derive(Deserialize)]
pub(crate) struct Change {
    range: Option<TextRange>,
    text: String,
}

pub(crate) struct Document {
    pub text: String,
    pub facts: Analysis,
    lines: Vec<usize>,
}

fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

fn byte_at(text: &str, lines: &[usize], pos: Position) -> Option<usize> {
    let start = *lines.get(pos.line)?;
    let end = lines.get(pos.line + 1).map_or(text.len(), |n| n - 1);
    let line = text.get(start..end)?;
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut units = 0;
    for (i, ch) in line.char_indices() {
        if pos.character == units {
            return Some(start + i);
        }
        units += ch.len_utf16();
        if pos.character < units {
            return None;
        }
    }
    Some(start + line.len())
}

pub(crate) fn apply_changes(original: &str, changes: Vec<Change>) -> Option<String> {
    let mut text = original.to_owned();
    for change in changes {
        if let Some(range) = change.range {
            let lines = line_starts(&text);
            let start = byte_at(&text, &lines, range.start)?;
            let end = byte_at(&text, &lines, range.end)?;
            if start > end {
                return None;
            }
            text.replace_range(start..end, &change.text);
        } else {
            text = change.text;
        }
    }
    Some(text)
}

pub(crate) fn block_comment_len(bytes: &[u8]) -> usize {
    let mut depth = 1;
    let mut i = 2;
    while i < bytes.len() && depth > 0 {
        if bytes[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if bytes[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    i
}

fn inferred_mode(mut text: &str) -> Source {
    text = text.trim_start_matches('\u{feff}');
    loop {
        text = text.trim_start();
        if let Some(comment) = text.strip_prefix("//") {
            text = comment.split_once('\n').map_or("", |(_, rest)| rest);
        } else if text.starts_with("/*") {
            text = text
                .get(block_comment_len(text.as_bytes())..)
                .unwrap_or_default();
        } else {
            break;
        }
    }
    if text.starts_with(['<', '{'])
        || text
            .strip_prefix("crate")
            .is_some_and(|s| s.trim_start().starts_with('='))
    {
        Source::Fragment
    } else {
        Source::Module
    }
}

impl Document {
    pub fn new(path: &Path, text: String, mode: Option<Source>) -> Self {
        let grammar = path
            .extension()
            .is_some_and(|ext| ext == "oxi")
            .then(|| mode.unwrap_or_else(|| inferred_mode(&text)));
        let facts = analyze(&text, grammar);
        release_span_cache();
        Self {
            lines: line_starts(&text),
            text,
            facts,
        }
    }

    pub fn byte(&self, pos: Position) -> Option<usize> {
        byte_at(&self.text, &self.lines, pos)
    }

    pub fn position(&self, offset: usize) -> Position {
        let offset = self.text.floor_char_boundary(offset.min(self.text.len()));
        let line = self
            .lines
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        let start = self.lines.get(line).copied().unwrap_or(0);
        let character = self
            .text
            .get(start..offset)
            .unwrap_or_default()
            .trim_end_matches('\r')
            .encode_utf16()
            .count();
        Position { line, character }
    }

    pub fn range(&self, bytes: &Range<usize>) -> TextRange {
        TextRange {
            start: self.position(bytes.start),
            end: self.position(bytes.end),
        }
    }

    pub fn word(&self, offset: usize) -> Option<Range<usize>> {
        let before = self.text.get(..offset)?;
        let after = self.text.get(offset..)?;
        let start = before
            .char_indices()
            .rev()
            .take_while(|(_, ch)| identifier(*ch))
            .last()
            .map_or(offset, |(i, _)| i);
        let end = after
            .char_indices()
            .take_while(|(_, ch)| identifier(*ch))
            .last()
            .map_or(offset, |(i, ch)| offset + i + ch.len_utf8());
        (start < end).then_some(start..end)
    }

    pub fn name_at(&self, offset: usize) -> Option<Range<usize>> {
        let word = self.word(offset)?;
        let name = self.text.get(word.clone())?;
        self.facts
            .identifiers
            .get(name)?
            .iter()
            .find(|r| r.start <= word.start && word.end <= r.end)?;
        // Rust member selection requires type inference. A free name in the
        // enclosing module can have the same spelling as a field or method.
        let before = self.text.get(..word.start)?.trim_end();
        (!before.ends_with('.') || before.ends_with("..")).then_some(word)
    }

    pub fn path_at(&self, word: &Range<usize>) -> Vec<String> {
        let mut segments = vec![self.text.get(word.clone()).unwrap_or_default().to_owned()];
        let mut before = self.text.get(..word.start).unwrap_or_default();
        while let Some(prefix) = before.trim_end().strip_suffix("::") {
            let prefix = prefix.trim_end();
            let Some(previous) = self.word(prefix.len()) else {
                break;
            };
            if previous.end > prefix.len() {
                break;
            }
            segments.push(
                self.text
                    .get(previous.clone())
                    .unwrap_or_default()
                    .to_owned(),
            );
            before = self.text.get(..previous.start).unwrap_or_default();
        }
        segments.reverse();
        segments
    }
}

pub(crate) fn identifier(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_fragment_after_nested_rust_comments() {
        assert_eq!(
            inferred_mode("/* 🐍 /* nested */ */\n<Node/>"),
            Source::Fragment
        );
    }

    #[test]
    fn ordered_utf16_edits_use_the_previous_result_and_reject_half_surrogates() {
        let edit = |start, end, text: &str| Change {
            range: Some(TextRange {
                start: Position {
                    line: 1,
                    character: start,
                },
                end: Position {
                    line: 1,
                    character: end,
                },
            }),
            text: text.into(),
        };
        let source = "first\r\n🐍abc\r\n";
        assert_eq!(
            apply_changes(source, vec![edit(2, 3, "xy"), edit(3, 5, "z")]).as_deref(),
            Some("first\r\n🐍xzc\r\n")
        );
        assert!(apply_changes(source, vec![edit(2, 3, "xy"), edit(1, 2, "z")]).is_none());
    }
}
