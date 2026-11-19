use crate::{
    document::{Document, Position, identifier},
    workspace::{Workspace, file_uri},
};
use oxiview_syntax::{Declaration, Kind};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const NODE: &[(&str, &str)] = &[
    ("style", "Set the node's typed Style."),
    (
        "bind_style",
        "Recompute the Style when a Signal read by the closure changes.",
    ),
    (
        "on_mount",
        "Receive the retained NodeId once after construction.",
    ),
    (
        "on_pointer",
        "Handle pointer dispatch with the event, tree and queued commands.",
    ),
    (
        "bind_text",
        "Bind a text node to a fallible reactive string computation.",
    ),
    ("content", "Set node-local Scene primitives."),
    (
        "bind_content",
        "Update node-local Scene primitives from a reactive computation.",
    ),
    ("child", "Append one child View."),
    ("children", "Append an ordered sequence of child Views."),
    ("measure", "Install intrinsic layout measurement."),
    ("semantics", "Set the node's semantic metadata."),
];
const STYLE: &[(&str, &str)] = &[
    ("layout", "Typed LayoutStyle for the node."),
    ("background", "Background Color."),
    ("border_color", "Color of the border."),
    ("border_radii", "CornerRadii of the rounded border."),
    ("opacity", "Opacity of the node and its contents."),
    (
        "z_index",
        "Optional stacking order within the stacking context.",
    ),
    ("overflow", "Overflow and clipping behavior."),
    (
        "hit_test",
        "Whether the node participates in pointer hit testing.",
    ),
    (
        "stationary_hover_updates",
        "Update hover when geometry moves beneath a stationary pointer.",
    ),
];

enum Cursor<'a> {
    Tag,
    Attribute(&'a str),
    Code,
}

// Scan to the cursor so incomplete start tags still offer attributes. Delimiters
// inside Rust attribute expressions, strings and comments do not end the tag.
fn cursor(prefix: &str) -> Cursor<'_> {
    let bytes = prefix.as_bytes();
    let mut i = 0;
    let mut tag = None;
    let mut braces = 0_u32;
    while i < bytes.len() {
        let rest = &bytes[i..];
        if rest.starts_with(b"//") {
            i += rest.iter().position(|b| *b == b'\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with(b"/*") {
            i += crate::document::block_comment_len(rest);
            continue;
        }
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if tag.is_some() {
            match bytes[i] {
                b'{' => braces += 1,
                b'}' => braces = braces.saturating_sub(1),
                b'>' if braces == 0 => tag = None,
                _ => {}
            }
        }
        if bytes[i] == b'<' && braces == 0 {
            let start = i + 1 + usize::from(bytes.get(i + 1) == Some(&b'/'));
            if let Some(tail) = prefix.get(start..) {
                let end = tail
                    .find(|c: char| !identifier(c) && c != ':')
                    .unwrap_or(tail.len());
                if end == tail.len() {
                    return Cursor::Tag;
                }
                if end > 0 {
                    tag = tail.get(..end);
                }
            }
        }
        i += 1;
    }
    match tag {
        Some(name) if braces == 0 => Cursor::Attribute(name),
        _ => Cursor::Code,
    }
}

fn symbol_kind(kind: Kind) -> u32 {
    match kind {
        Kind::View | Kind::Function => 12,
        Kind::Style | Kind::Type => 23,
        Kind::Constant => 14,
        Kind::Module => 2,
        Kind::Local => 13,
    }
}

fn symbol(doc: &Document, d: &Declaration) -> Value {
    json!({"name":d.name, "detail":d.detail, "kind":symbol_kind(d.kind),
        "range":doc.range(&d.extent), "selectionRange":doc.range(&d.selection)})
}

pub fn symbols(ws: &Workspace, path: &Path) -> Value {
    json!(
        ws.files
            .get(path)
            .into_iter()
            .flat_map(|e| e
                .doc
                .facts
                .declarations
                .iter()
                .filter(|d| d.kind != Kind::Local)
                .map(|d| symbol(&e.doc, d)))
            .collect::<Vec<_>>()
    )
}

pub fn workspace_symbols(ws: &Workspace, query: &str) -> Value {
    let query = query.to_lowercase();
    let mut result = Vec::new();
    for (path, entry) in &ws.files {
        for d in &entry.doc.facts.declarations {
            if d.kind != Kind::Local && d.name.to_lowercase().contains(&query) {
                result.push(json!({"name":d.name,"kind":symbol_kind(d.kind),
                    "location":{"uri":file_uri(path),"range":entry.doc.range(&d.selection)}}));
                if result.len() == 256 {
                    return json!(result);
                }
            }
        }
    }
    json!(result)
}

pub fn definition(ws: &Workspace, path: &Path, position: Position) -> Option<Value> {
    let doc = &ws.files.get(path)?.doc;
    let pos = doc.byte(position)?;
    if let Some(target) = ws.included_file(path, pos) {
        let start = Position {
            line: 0,
            character: 0,
        };
        return Some(json!([{"uri":file_uri(&target),"range":{"start":start,"end":start}}]));
    }
    let word = doc.name_at(pos)?;
    let targets = ws.resolve(path, pos, &doc.path_at(&word));
    (!targets.is_empty()).then(|| {
        json!(
            targets
                .iter()
                .map(|(path, doc, d)| json!({"uri":file_uri(path),"range":doc.range(&d.selection)}))
                .collect::<Vec<_>>()
        )
    })
}

pub fn hover(ws: &Workspace, path: &Path, position: Position) -> Option<Value> {
    let doc = &ws.files.get(path)?.doc;
    let pos = doc.byte(position)?;
    let word = doc.name_at(pos)?;
    let name = doc.text.get(word.clone())?;
    let builtin = match cursor(doc.text.get(..word.start)?) {
        Cursor::Attribute("Node" | "node") => {
            NODE.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
        }
        Cursor::Attribute(_) if name == "props" => {
            Some("Pass a typed props value to the function component.")
        }
        Cursor::Tag if matches!(name, "Node" | "node") => {
            Some("Construct a retained node using the native Rust Builder.")
        }
        _ => None,
    };
    let locations = ws.resolve(path, pos, &doc.path_at(&word));
    let value = if let Some(text) = builtin {
        format!("`{name}`\n\n{text}")
    } else if let Some((_, _, d)) = locations.first() {
        format!("```rust\n{}\n```", d.detail)
    } else if doc
        .facts
        .declarations
        .iter()
        .any(|d| d.kind == Kind::Style && d.extent.contains(&pos))
    {
        let (_, description) = STYLE.iter().find(|(n, _)| *n == name)?;
        format!("`{name}`\n\n{description}")
    } else {
        return None;
    };
    Some(json!({"contents":{"kind":"markdown","value":value},"range":doc.range(&word)}))
}

pub fn completion(ws: &Workspace, path: &Path, position: Position) -> Option<Value> {
    let entry = ws.files.get(path)?;
    let doc = &entry.doc;
    let pos = doc.byte(position)?;
    let prefix = doc.text.get(..pos)?;
    let mut items = BTreeMap::new();
    let mut add = |name: &str, kind: u32, detail: &str| {
        items
            .entry(name.to_owned())
            .or_insert_with(|| json!({"label":name,"kind":kind,"detail":detail}));
    };
    let context = cursor(prefix);
    match context {
        Cursor::Attribute("Node" | "node") => {
            for (name, detail) in NODE {
                add(name, 10, detail);
            }
        }
        Cursor::Attribute(_) => add("props", 10, "Typed component props"),
        Cursor::Tag | Cursor::Code => {
            if matches!(context, Cursor::Tag) {
                add("Node", 7, "Native node Builder");
            } else {
                for (name, detail) in [
                    ("view", "Declare a named View"),
                    ("style", "Declare a typed Style"),
                    ("fn", "Declare a Rust function"),
                    ("use", "Import a Rust path"),
                ] {
                    add(name, 14, detail);
                }
                if doc
                    .facts
                    .declarations
                    .iter()
                    .any(|d| d.kind == Kind::Style && d.extent.contains(&pos))
                    || prefix
                        .rsplit_once("style ")
                        .is_some_and(|(_, tail)| tail.contains('{') && !tail.contains('}'))
                {
                    for (name, detail) in STYLE {
                        add(name, 5, detail);
                    }
                }
            }
            for name in ws.visible_names(path, pos) {
                let targets = ws.resolve(path, pos, std::slice::from_ref(&name));
                if let Some((_, _, d)) = targets.first() {
                    if matches!(context, Cursor::Tag)
                        && !(d.kind == Kind::View
                            || (d.kind == Kind::Function
                                && d.detail
                                    .rsplit_once("->")
                                    .is_some_and(|(_, ty)| ty.contains("View"))))
                    {
                        continue;
                    }
                    let kind = match d.kind {
                        Kind::View | Kind::Style | Kind::Function => 3,
                        Kind::Constant => 21,
                        Kind::Local => 6,
                        Kind::Module => 9,
                        Kind::Type => 22,
                    };
                    add(&name, kind, &d.detail);
                } else {
                    add(&name, 6, "In scope");
                }
            }
        }
    }
    Some(json!({"isIncomplete":false,"items":items.into_values().collect::<Vec<_>>() }))
}

pub fn references(
    ws: &Workspace,
    path: &Path,
    position: Position,
    declarations: bool,
) -> Option<Value> {
    let doc = &ws.files.get(path)?.doc;
    let at = doc.byte(position)?;
    let word = doc.name_at(at)?;
    let targets = ws.resolve(path, at, &doc.path_at(&word));
    if targets.is_empty() {
        return None;
    }
    let definitions: BTreeSet<_> = targets
        .iter()
        .map(|(p, _, d)| (*p, d.selection.start))
        .collect();
    let mut names: BTreeSet<_> = targets.iter().map(|(_, _, d)| d.name.as_str()).collect();
    loop {
        let count = names.len();
        for entry in ws.files.values() {
            for import in &entry.doc.facts.imports {
                if import
                    .path
                    .last()
                    .is_some_and(|n| names.contains(n.as_str()))
                    && import.name != "*"
                {
                    names.insert(&import.name);
                }
            }
        }
        if names.len() == count {
            break;
        }
    }
    let mut locations = Vec::new();
    for (file, entry) in &ws.files {
        for name in &names {
            for bytes in entry.doc.facts.identifiers.get(*name).into_iter().flatten() {
                let offset = bytes.end.saturating_sub(1);
                let offset = entry.doc.text.floor_char_boundary(offset);
                let Some(word) = entry.doc.name_at(offset) else {
                    continue;
                };
                let is_definition = definitions.contains(&(file, bytes.start));
                if is_definition && !declarations {
                    continue;
                }
                if is_definition
                    || ws
                        .resolve(file, offset, &entry.doc.path_at(&word))
                        .iter()
                        .any(|(p, _, d)| definitions.contains(&(*p, d.selection.start)))
                {
                    locations.push(json!({"uri":file_uri(file),"range":entry.doc.range(bytes)}));
                }
            }
        }
    }
    Some(json!(locations))
}
