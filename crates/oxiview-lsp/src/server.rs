use crate::{
    document::{Change, Position, apply_changes},
    features,
    workspace::{Workspace, file_path},
};
use lsp_server::{ErrorCode, Message, Notification, Request, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
struct DocumentId {
    uri: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocumentParams {
    text_document: DocumentId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorParams {
    text_document: DocumentId,
    position: Position,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceContext {
    include_declaration: bool,
}

#[derive(Deserialize)]
struct References {
    #[serde(flatten)]
    cursor: CursorParams,
    context: ReferenceContext,
}

#[derive(Deserialize)]
struct Revision {
    uri: String,
    version: i64,
}

#[derive(Deserialize)]
struct Opened {
    uri: String,
    version: i64,
    text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Open {
    text_document: Opened,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Changed {
    text_document: Revision,
    content_changes: Vec<Change>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Initialize {
    root_uri: Option<String>,
    workspace_folders: Option<Vec<DocumentId>>,
}

fn path(id: &DocumentId) -> Result<PathBuf, String> {
    file_path(&id.uri).ok_or_else(|| "expected a local file URI".into())
}

fn params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn diagnostics(ws: &Workspace, path: &Path, uri: &str) -> Message {
    let entry = ws.files.get(path);
    let problems: Vec<_> = entry
        .into_iter()
        .flat_map(|e| {
            e.doc.facts.problems.iter().map(|d| json!({
        "range":e.doc.range(&d.range), "severity":1, "source":"oxiview", "message":d.message
    }))
        })
        .collect();
    let mut payload = json!({"uri":uri,"diagnostics":problems});
    if let Some(version) = entry.and_then(|e| e.version) {
        payload["version"] = json!(version);
    }
    Notification::new("textDocument/publishDiagnostics".into(), payload).into()
}

fn notify(ws: &mut Workspace, n: Notification) -> Result<Option<Message>, String> {
    match n.method.as_str() {
        "textDocument/didOpen" => {
            let item = params::<Open>(n.params)?.text_document;
            if let Some(path) = file_path(&item.uri) {
                ws.open(path.clone(), item.text, item.version);
                return Ok(Some(diagnostics(ws, &path, &item.uri)));
            }
        }
        "textDocument/didChange" => {
            let change: Changed = params(n.params)?;
            let revision = change.text_document;
            if let Some(path) = file_path(&revision.uri)
                && let Some(entry) = ws.files.get(&path)
                && entry.version.is_some_and(|v| revision.version > v)
            {
                let text = apply_changes(&entry.doc.text, change.content_changes)
                    .ok_or("invalid UTF-16 edit range")?;
                ws.update(path.clone(), text, revision.version);
                return Ok(Some(diagnostics(ws, &path, &revision.uri)));
            }
        }
        "textDocument/didClose" => {
            let item = params::<DocumentParams>(n.params)?.text_document;
            if let Some(path) = file_path(&item.uri) {
                ws.close(&path);
                return Ok(Some(
                    Notification::new(
                        "textDocument/publishDiagnostics".into(),
                        json!({"uri":item.uri,"diagnostics":[]}),
                    )
                    .into(),
                ));
            }
        }
        "textDocument/didSave" | "workspace/didChangeWatchedFiles" => ws.refresh(true),
        _ => {}
    }
    Ok(None)
}

fn query(ws: &mut Workspace, method: &str, args: Value) -> Result<Value, (ErrorCode, String)> {
    let invalid = |s| (ErrorCode::InvalidParams, s);
    ws.refresh(false);
    match method {
        "textDocument/documentSymbol" => {
            let doc = params::<DocumentParams>(args).map_err(invalid)?;
            Ok(features::symbols(
                ws,
                &path(&doc.text_document).map_err(invalid)?,
            ))
        }
        "workspace/symbol" => {
            let term = args
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("expected a symbol query".into()))?;
            Ok(features::workspace_symbols(ws, term))
        }
        "textDocument/references" => {
            let req = params::<References>(args).map_err(invalid)?;
            let path = path(&req.cursor.text_document).map_err(invalid)?;
            Ok(features::references(
                ws,
                &path,
                req.cursor.position,
                req.context.include_declaration,
            )
            .unwrap_or(Value::Null))
        }
        "textDocument/definition"
        | "textDocument/declaration"
        | "textDocument/implementation"
        | "textDocument/typeDefinition"
        | "textDocument/completion"
        | "textDocument/hover" => {
            let at = params::<CursorParams>(args).map_err(invalid)?;
            let path = path(&at.text_document).map_err(invalid)?;
            Ok(match method {
                "textDocument/definition" | "textDocument/declaration" => {
                    features::navigate(ws, &path, at.position, features::Navigation::Definition)
                }
                "textDocument/implementation" => {
                    features::navigate(ws, &path, at.position, features::Navigation::Implementation)
                }
                "textDocument/typeDefinition" => {
                    features::navigate(ws, &path, at.position, features::Navigation::Type)
                }
                "textDocument/completion" => features::completion(ws, &path, at.position),
                _ => features::hover(ws, &path, at.position),
            }
            .unwrap_or(Value::Null))
        }
        _ => Err((
            ErrorCode::MethodNotFound,
            format!("unsupported method {method}"),
        )),
    }
}

#[derive(PartialEq)]
enum State {
    Starting,
    Serving,
    Stopped,
}

fn request(ws: &mut Workspace, state: &mut State, req: Request) -> Response {
    let result = match (req.method.as_str(), &*state) {
        ("initialize", State::Starting) => params::<Initialize>(req.params)
            .map_err(|s| (ErrorCode::InvalidParams, s))
            .map(|init| {
                for uri in init
                    .root_uri
                    .into_iter()
                    .chain(init.workspace_folders.into_iter().flatten().map(|f| f.uri))
                {
                    if let Some(path) = file_path(&uri) {
                        ws.root(&path);
                    }
                }
                *state = State::Serving;
                json!({"serverInfo":{"name":"oxiview-lsp","version":env!("CARGO_PKG_VERSION")},
                    "capabilities":{"positionEncoding":"utf-16",
                    "textDocumentSync":{"openClose":true,"change":2,"save":true},
                    "completionProvider":{"triggerCharacters":["<"," ",":"]},"hoverProvider":true,
                    "definitionProvider":true,"declarationProvider":true,"implementationProvider":true,
                    "typeDefinitionProvider":true,"referencesProvider":true,
                    "documentSymbolProvider":true,"workspaceSymbolProvider":true}})
            }),
        (_, State::Starting) => Err((
            ErrorCode::ServerNotInitialized,
            "expected initialize".into(),
        )),
        (_, State::Stopped) => Err((ErrorCode::InvalidRequest, "server has shut down".into())),
        ("initialize", _) => Err((
            ErrorCode::InvalidRequest,
            "server already initialized".into(),
        )),
        ("shutdown", _) => {
            *state = State::Stopped;
            Ok(Value::Null)
        }
        (method, State::Serving) => query(ws, method, req.params),
    };
    match result {
        Ok(value) => Response::new_ok(req.id, value),
        Err((code, msg)) => Response::new_err(req.id, code as i32, msg),
    }
}

/// Serves one LSP session on framed JSON-RPC streams until the client exits.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut workspace = Workspace::default();
    let mut state = State::Starting;
    while let Some(message) = Message::read(&mut input)? {
        let reply = match message {
            Message::Request(req) => Some(request(&mut workspace, &mut state, req).into()),
            Message::Notification(n) if n.method == "exit" => {
                return if state == State::Stopped {
                    Ok(())
                } else {
                    Err(io::Error::other("exit before shutdown"))
                };
            }
            Message::Notification(n) if state == State::Serving => {
                match notify(&mut workspace, n) {
                    Ok(message) => message,
                    Err(error) => {
                        eprintln!("oxiview-lsp: {error}");
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some(reply) = reply {
            reply.write(&mut output)?;
            output.flush()?;
        }
    }
    Ok(())
}
