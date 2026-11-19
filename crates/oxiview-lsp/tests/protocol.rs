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
fn node_properties_navigate_to_their_rust_builder_and_find_real_uses() -> Result<(), Box<dyn Error>>
{
    let project = example()?;
    let board = project.join("src/board.oxi");
    let board_text = fs::read_to_string(&board)?;
    let core = project
        .join("../../crates/oxiview-core/src/ui")
        .canonicalize()?;
    let builder = core.join("view.rs");
    let code = fs::read_to_string(&builder)?;
    let app = project.join("../snake/src/application.rs").canonicalize()?;
    let app_text = fs::read_to_string(&app)?;
    let document = json!({"uri":uri(&board)?});
    let property = position(&board_text, "style={overlay_surface()}", 1)?;
    let at = json!({"textDocument":document,"position":property});
    let node = position(&board_text, "<Node style={overlay_surface()}", 2)?;
    let inputs = vec![
        json!({"id":0,"method":"initialize","params":{"rootUri":uri(&project)?}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":uri(&board)?,"version":1,"text":board_text}}}),
        json!({"id":1,"method":"textDocument/definition","params":at}),
        json!({"id":2,"method":"textDocument/implementation","params":at}),
        json!({"id":3,"method":"textDocument/references","params":{"textDocument":document,"position":property,"context":{"includeDeclaration":false}}}),
        json!({"id":4,"method":"textDocument/definition","params":{"textDocument":document,"position":node}}),
        json!({"id":5,"method":"textDocument/declaration","params":at}),
        json!({"id":6,"method":"textDocument/typeDefinition","params":at}),
        json!({"id":7,"method":"textDocument/definition","params":{"textDocument":document,"position":position(&board_text,"<action_button props=",16)?}}),
        json!({"id":8,"method":"textDocument/definition","params":{"textDocument":{"uri":uri(&project.join("src/button_face.oxi"))?},"position":{"line":0,"character":7}}}),
        json!({"id":9,"method":"shutdown"}),
        json!({"method":"exit"}),
    ];
    let replies = session(inputs)?;
    let method = position(&code, "pub fn style(", 7)?;
    for id in [1, 2, 5] {
        let target = &response(&replies, id)?["result"][0];
        assert_eq!(
            target["uri"],
            uri(&builder)?,
            "request {id} for the Node style attribute"
        );
        assert_eq!(target["range"]["start"], method);
    }
    let node_target = &response(&replies, 4)?["result"][0];
    assert_eq!(node_target["uri"], uri(&builder)?);
    assert_eq!(
        node_target["range"]["start"],
        position(&code, "pub fn node(", 7)?
    );
    let references = response(&replies, 3)?["result"]
        .as_array()
        .ok_or("missing builder references")?;
    let includes = |file: &Path, start: Value| -> Result<bool, Box<dyn Error>> {
        let uri = uri(file)?;
        Ok(references
            .iter()
            .any(|r| r["uri"] == uri && r["range"]["start"] == start))
    };
    assert!(includes(
        &board,
        position(&board_text, "style={overlay_surface()}", 0)?
    )?);
    assert!(includes(
        &board,
        position(&board_text, ".style(overlay_position())", 1)?
    )?);
    assert!(includes(
        &app,
        position(&app_text, "<Node style={style}>", 6)?
    )?);
    assert!(!includes(&builder, method)?);
    assert!(!includes(
        &builder,
        position(&code, "self.style = Some(style)", 5)?
    )?);
    assert!(
        references.iter().all(|r| !r["uri"]
            .as_str()
            .is_some_and(|s| s.contains("/oxiview-macros/tests/"))),
        "the macro tests explicitly select a different Builder"
    );
    let style = project
        .join("../../crates/oxiview-style/src/lib.rs")
        .canonicalize()?;
    assert_eq!(response(&replies, 6)?["result"][0]["uri"], uri(&style)?);
    assert_eq!(
        response(&replies, 6)?["result"][0]["range"]["start"],
        position(&fs::read_to_string(style)?, "pub struct Style", 11)?
    );
    let controls = project.join("src/controls.rs");
    assert_eq!(
        response(&replies, 7)?["result"][0]["range"]["start"],
        position(&fs::read_to_string(controls)?, "action_button(props:", 14)?
    );
    assert_eq!(
        response(&replies, 8)?["result"][0]["range"]["start"],
        position(&code, "pub fn bind_style(", 7)?
    );
    Ok(())
}

#[test]
fn a_separate_workspace_resolves_a_renamed_path_builder() -> Result<(), Box<dyn Error>> {
    let project = example()?;
    let root = project
        .join("../../target/xtask")
        .join(format!("oxi-navigation-{}", std::process::id()));
    let client = root.join("client");
    let app = client.join("app");
    let facade = root.join("facade");
    fs::create_dir_all(app.join("src"))?;
    fs::create_dir_all(facade.join("api"))?;
    let sdk = project.join("../../crates/oxiview-core").canonicalize()?;
    fs::write(
        client.join("Cargo.toml"),
        "[workspace]\nmembers = ['app']\n[workspace.dependencies]\ntoolkit = {package = 'custom-facade', path = '../facade'}\n",
    )?;
    fs::write(
        facade.join("Cargo.toml"),
        format!(
            "[package]\nname='custom-facade'\nversion='0.0.0'\n[lib]\npath='api/entry.rs'\n[dependencies]\nengine={{package='oxiview-core',path={sdk:?}}}\n"
        ),
    )?;
    fs::write(
        facade.join("api/entry.rs"),
        "mod exports; pub use exports::node;",
    )?;
    fs::write(facade.join("api/exports.rs"), "pub use engine::node;")?;
    fs::write(
        app.join("Cargo.toml"),
        "[package]\nname='path-client'\nversion='0.0.0'\n[dependencies]\ntoolkit.workspace=true\n",
    )?;
    let fragment = "crate = toolkit; <Node style={Default::default()}/>";
    fs::write(app.join("src/panel.oxi"), fragment)?;
    let file = uri(&app.join("src/panel.oxi").canonicalize()?)?;
    let replies = session(vec![
        json!({"id":0,"method":"initialize","params":{"rootUri":uri(&app.canonicalize()?)?}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":file,"version":1,"text":fragment}}}),
        json!({"id":1,"method":"textDocument/definition","params":{"textDocument":{"uri":file},"position":position(fragment,"style=",1)?}}),
        json!({"id":2,"method":"shutdown"}),
        json!({"method":"exit"}),
    ])?;
    let method = &response(&replies, 1)?["result"][0];
    assert_eq!(method["uri"], uri(&sdk.join("src/ui/view.rs"))?);
    assert_eq!(
        method["range"]["start"],
        position(
            &fs::read_to_string(sdk.join("src/ui/view.rs"))?,
            "pub fn style(",
            7
        )?
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn navigation_distinguishes_an_attribute_its_value_and_a_typed_style_field()
-> Result<(), Box<dyn Error>> {
    let project = example()?;
    let file = uri(&project.join("src/unsaved_navigation.oxi"))?;
    let source = "pub view sample() {\n    let style = shape();\n    <Node style={style} bind_text={|| Ok(\"🐍\".into())}/>\n}\nstyle shape() { opacity: 1.0 }";
    let at = |needle, offset| -> Result<Value, Box<dyn Error>> {
        Ok(json!({"textDocument":{"uri":file},"position":position(source,needle,offset)?}))
    };
    let replies = session(vec![
        json!({"id":0,"method":"initialize","params":{}}),
        json!({"method":"textDocument/didOpen","params":{"textDocument":{"uri":file,"version":1,"text":source}}}),
        json!({"id":1,"method":"textDocument/definition","params":at("{style}",2)?}),
        json!({"id":2,"method":"textDocument/implementation","params":at("bind_text=",1)?}),
        json!({"id":3,"method":"textDocument/definition","params":at("opacity:",1)?}),
        json!({"id":4,"method":"shutdown"}),
        json!({"method":"exit"}),
    ])?;
    let local = &response(&replies, 1)?["result"][0];
    assert_eq!(local["uri"], file);
    assert_eq!(local["range"]["start"], position(source, "let style", 4)?);
    for (id, rel, needle, skip) in [
        (2, "oxiview-core/src/ui/text.rs", "pub fn bind_text(", 7),
        (3, "oxiview-style/src/lib.rs", "pub opacity:", 4),
    ] {
        let path = project.join("../../crates").join(rel).canonicalize()?;
        let target = &response(&replies, id)?["result"][0];
        assert_eq!(target["uri"], uri(&path)?);
        assert_eq!(
            target["range"]["start"],
            position(&fs::read_to_string(path)?, needle, skip)?
        );
    }
    Ok(())
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
