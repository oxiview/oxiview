//! Drive the production framing, workspace and edit lifecycle with real .oxi sources.

use lsp_server::Message;
use serde_json::{Value, json};
use std::{
    error::Error,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
use url::Url;

fn session(messages: Vec<Value>) -> Result<Vec<Value>, Box<dyn Error>> {
    let mut input = Vec::new();
    for mut msg in messages {
        msg["jsonrpc"] = json!("2.0");
        serde_json::from_value::<Message>(msg)?.write(&mut input)?;
    }
    let mut output = Vec::new();
    oxiview_lsp::serve(Cursor::new(input), &mut output)?;
    let mut reader = Cursor::new(output);
    let mut replies = Vec::new();
    while let Some(msg) = Message::read(&mut reader)? {
        replies.push(serde_json::to_value(msg)?);
    }
    Ok(replies)
}

fn uri(path: &Path) -> Result<String, Box<dyn Error>> {
    Ok(Url::from_file_path(path)
        .map_err(|()| "expected an absolute path")?
        .into())
}

fn response(replies: &[Value], id: i32) -> Result<&Value, Box<dyn Error>> {
    Ok(replies
        .iter()
        .find(|r| r["id"] == id)
        .ok_or("missing response")?)
}

fn position(text: &str, needle: &str, skip: usize) -> Result<Value, Box<dyn Error>> {
    let offset = text.find(needle).ok_or("missing source needle")? + skip;
    let before = text.get(..offset).ok_or("invalid byte offset")?;
    Ok(
        json!({"line":before.bytes().filter(|&b| b == b'\n').count(),
        "character":before.rsplit('\n').next().unwrap_or_default().encode_utf16().count()}),
    )
}

fn example() -> Result<PathBuf, Box<dyn Error>> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/snake-oxi")
        .canonicalize()?)
}

#[test]
fn navigates_the_mixed_snake_example_and_resolves_fragment_captures() -> Result<(), Box<dyn Error>>
{
    let project = example()?;
    let src = project.join("src");
    let screen = fs::read_to_string(src.join("screen.oxi"))?;
    let fragment = fs::read_to_string(src.join("button_face.oxi"))?;
    let screen_id = uri(&src.join("screen.oxi"))?;
    let fragment_id = uri(&src.join("button_face.oxi"))?;
    let at = |uri: &str, pos: Value| json!({"textDocument":{"uri":uri},"position":pos});
    let tag = position(&screen, "<game_panel", 2)?;
    let msgs = vec![
        json!({"id":0,"method":"initialize","params":{"workspaceFolders":[{"uri":uri(&project)?,"name":"Snake"}]}}),
        json!({"method":"initialized","params":{}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":screen_id,"version":1,"text":screen}}}),
        json!({"id":1,"method":"textDocument/definition","params":at(&screen_id,tag.clone())}),
        json!({"id":2,"method":"textDocument/hover","params":at(&screen_id,tag)}),
        json!({"id":3,"method":"textDocument/completion","params":at(&screen_id,position(&screen,"<Node style=",6)?)}),
        json!({"id":4,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":screen_id}}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":fragment_id,"version":1,"text":fragment}}}),
        json!({"id":5,"method":"textDocument/definition","params":at(&fragment_id,position(&fragment,"{title}",2)?)}),
        json!({"id":6,"method":"textDocument/definition","params":at(&screen_id,position(&screen,"label(&ui",1)?)}),
        json!({"id":7,"method":"shutdown","params":null}),
        json!({"method":"exit"}),
    ];
    let replies = session(msgs)?;
    assert_eq!(
        response(&replies, 0)?["result"]["capabilities"]["positionEncoding"],
        "utf-16"
    );
    for (id, file, name) in [
        (1, src.join("board.oxi"), "game_panel"),
        (5, src.join("controls.rs"), "title"),
        (
            6,
            project
                .join("../snake/src/presentation.rs")
                .canonicalize()?,
            "label",
        ),
    ] {
        let target = &response(&replies, id)?["result"][0];
        assert_eq!(target["uri"], uri(&file)?);
        let start = &target["range"]["start"];
        let text = fs::read_to_string(&file)?;
        let line = text
            .lines()
            .nth(start["line"].as_u64().ok_or("missing line")? as usize)
            .ok_or("invalid line")?;
        assert!(
            line.get(start["character"].as_u64().ok_or("missing column")? as usize..)
                .is_some_and(|s| s.starts_with(name))
        );
    }
    assert!(
        response(&replies, 2)?["result"]["contents"]["value"]
            .as_str()
            .is_some_and(|s| s.contains("view game_panel(ui: Ui)"))
    );
    assert!(
        response(&replies, 3)?["result"]["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|i| i["label"] == "on_pointer"))
    );
    assert!(
        response(&replies, 4)?["result"]
            .as_array()
            .is_some_and(|items| items.iter().any(|i| i["name"] == "phase_badge"))
    );
    Ok(())
}

#[test]
fn edits_use_utf16_ranges_versions_and_clear_diagnostics() -> Result<(), Box<dyn Error>> {
    let path = example()?.join("src/unsaved 编辑.oxi");
    let uri = uri(&path)?;
    let valid = "<Node>{\"🐍\"}</Node>";
    let start = position(valid, "</Node>", 2)?;
    let end = position(valid, "</Node>", 6)?;
    let change = |version, text: &str| {
        json!({"method":"textDocument/didChange","params":{
        "textDocument":{"uri":uri,"version":version},"contentChanges":[{"range":{"start":start,"end":end},"text":text}]}})
    };
    let replies = session(vec![
        json!({"id":1,"method":"initialize","params":{}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"version":1,"text":valid}}}),
        change(2, "Oops"),
        change(1, "Nope"),
        change(3, "Node"),
        json!({"method":"textDocument/didClose","params":{"textDocument":{"uri":uri}}}),
        json!({"id":2,"method":"unsupported","params":{}}),
        json!({"id":3,"method":"shutdown"}),
        json!({"method":"exit"}),
    ])?;
    let diagnostics: Vec<_> = replies
        .iter()
        .filter(|r| r["method"] == "textDocument/publishDiagnostics")
        .map(|r| &r["params"])
        .collect();
    assert_eq!(diagnostics.len(), 4);
    let bad = diagnostics.get(1).ok_or("missing change diagnostics")?;
    assert_eq!(bad["version"], 2);
    assert_eq!(
        bad["diagnostics"][0]["range"],
        json!({"start":start,"end":end})
    );
    assert!(
        bad["diagnostics"][0]["message"]
            .as_str()
            .is_some_and(|m| m.contains("closing tag"))
    );
    assert_eq!(
        diagnostics.get(2).ok_or("missing fix")?["diagnostics"],
        json!([])
    );
    assert_eq!(
        diagnostics.get(3).ok_or("missing close")?["diagnostics"],
        json!([])
    );
    assert_eq!(response(&replies, 2)?["error"]["code"], -32601);
    Ok(())
}

#[test]
fn one_fragment_can_capture_different_bindings_from_the_same_host() -> Result<(), Box<dyn Error>> {
    let root = example()?;
    let host = uri(&root.join("src/unsaved_host.rs"))?;
    let fragment = uri(&root.join("src/unsaved_fragment.oxi"))?;
    let rust = r#"fn first(value: u32) -> View { oxi_view!("src/unsaved_fragment.oxi") }
fn second(value: u32) -> View { oxi_view!("src/unsaved_fragment.oxi") }
impl Host { fn third(&self, value: u32) -> View { oxi_view!("src/unsaved_fragment.oxi") } }
fn value() {}"#;
    let at = json!({"textDocument":{"uri":fragment},"position":{"line":0,"character":2}});
    let replies = session(vec![
        json!({"id":0,"method":"initialize","params":{"rootUri":uri(&root)?}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":host,"version":1,"text":rust}}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":fragment,"version":1,"text":"{value}"}}}),
        json!({"id":1,"method":"textDocument/definition","params":at}),
        json!({"id":2,"method":"shutdown"}),
        json!({"method":"exit"}),
    ])?;
    let definitions = response(&replies, 1)?["result"]
        .as_array()
        .ok_or("missing definitions")?;
    assert_eq!(definitions.len(), 3);
    for (line, result) in definitions.iter().enumerate() {
        assert_eq!(result["uri"], host);
        assert_eq!(result["range"]["start"]["line"], line);
    }
    Ok(())
}

#[test]
fn incomplete_tags_keep_import_completions_and_references_follow_aliases()
-> Result<(), Box<dyn Error>> {
    let root = example()?;
    let uri = uri(&root.join("src/unsaved_panel.oxi"))?;
    let header = "use crate::board::game_panel as board_view;\n";
    let good = format!("{header}pub view wrapper(ui: Ui) {{\n    <board_view props={{ui}}/>\n}}");
    let broken = format!("{header}pub view wrapper(ui: Ui) {{\n    <boa");
    let cursor = json!({"line":2,"character":8});
    let replies = session(vec![
        json!({"id":0,"method":"initialize","params":{}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"version":1,"text":good}}}),
        json!({"id":1,"method":"textDocument/references","params":{"textDocument":{"uri":uri},"position":cursor,"context":{"includeDeclaration":true}}}),
        json!({"method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":broken}]}}),
        json!({"id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":uri},"position":cursor}}),
        json!({"id":3,"method":"shutdown"}),
        json!({"method":"exit"}),
    ])?;
    let references = response(&replies, 1)?["result"]
        .as_array()
        .ok_or("missing references")?;
    assert!(references.iter().any(|r| {
        r["uri"]
            .as_str()
            .is_some_and(|p| p.ends_with("/screen.oxi"))
    }));
    assert!(
        references
            .iter()
            .any(|r| r["uri"] == uri && r["range"]["start"]["line"] == 2)
    );
    assert!(
        response(&replies, 2)?["result"]["items"]
            .as_array()
            .is_some_and(|v| v.iter().any(|i| i["label"] == "board_view"))
    );
    Ok(())
}
