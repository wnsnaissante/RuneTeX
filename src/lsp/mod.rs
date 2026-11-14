//! Texlab client: the editor owns text; the server supplies language features.
mod transport;

use rune::Waker;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::mpsc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    Completion,
    Hover,
    Definition,
}

#[derive(Clone, Debug)]
pub struct Context {
    pub path: String,
    pub revision: u64,
    pub cursor: usize,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Completion {
    pub label: String,
    pub detail: String,
    pub edits: Vec<(std::ops::Range<usize>, String)>,
    pub caret: usize,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub range: Value,
    pub message: String,
    pub warning: bool,
}

pub enum Event {
    Status(String),
    Diagnostics(String, Vec<Diagnostic>),
    Response(Query, Context, Value),
}

pub(super) enum Command {
    Sync(String, String),
    Saved(String),
    Query(Query, Context),
    Shutdown,
}

pub struct Client {
    pub status: String,
    pub diagnostics: BTreeMap<String, Vec<Diagnostic>>,
    commands: Option<mpsc::Sender<Command>>,
    events: mpsc::Receiver<Event>,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            status: "texlab 대기".into(),
            diagnostics: BTreeMap::new(),
            commands: None,
            events: mpsc::channel().1,
        }
    }
}

impl Client {
    pub fn start(root: PathBuf, documents: Vec<(String, String)>, wake: Waker) -> Self {
        let (send, commands) = mpsc::channel();
        let (events, receive) = mpsc::channel();
        std::thread::spawn(move || transport::run(root, documents, commands, events, wake));
        Self {
            status: "texlab 연결 중".into(),
            diagnostics: BTreeMap::new(),
            commands: Some(send),
            events: receive,
        }
    }
    pub fn sync(&self, path: &str, text: &str) {
        if let Some(send) = &self.commands {
            let _ = send.send(Command::Sync(path.into(), text.into()));
        }
    }
    pub fn saved(&self, path: &str) {
        if let Some(send) = &self.commands {
            let _ = send.send(Command::Saved(path.into()));
        }
    }
    pub fn query(&self, kind: Query, context: Context) {
        if let Some(send) = &self.commands {
            let _ = send.send(Command::Query(kind, context));
        }
    }
    pub fn drain(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(send) = self.commands.take() {
            let _ = send.send(Command::Shutdown);
        }
    }
}

pub fn server_path() -> Option<PathBuf> {
    crate::paths::program_override("RUNETEX_TEXLAB")
        .or_else(|| crate::paths::bundled_program("texlab"))
        .or_else(|| crate::compiler::find_program("texlab"))
}

pub fn uri(root: &Path, relative: &str) -> Result<String, String> {
    let path = std::path::absolute(root.join(relative)).map_err(|e| e.to_string())?;
    url::Url::from_file_path(path)
        .map(String::from)
        .map_err(|_| "파일 URI 변환 실패".into())
}

pub fn relative_path(root: &Path, uri: &str) -> Option<String> {
    let path = url::Url::parse(uri).ok()?.to_file_path().ok()?;
    let root = std::path::absolute(root).ok()?;
    Some(
        path.strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

/// LSP uses UTF-16 columns; Rune's TextEdit uses UTF-8 byte offsets.
pub fn position(text: &str, byte: usize) -> Value {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    let before = &text[..byte];
    json!({"line": before.bytes().filter(|b| *b == b'\n').count(),
        "character": before.rsplit('\n').next().unwrap_or("").encode_utf16().count()})
}

pub fn byte_at(text: &str, position: &Value) -> usize {
    let line = position["line"].as_u64().unwrap_or(0) as usize;
    let column = position["character"].as_u64().unwrap_or(0) as usize;
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    let current = text
        .get(start..)
        .unwrap_or("")
        .split('\n')
        .next()
        .unwrap_or("");
    let mut units = 0;
    for (offset, character) in current.char_indices() {
        if units + character.len_utf16() > column {
            return start + offset;
        }
        units += character.len_utf16();
        if units == column {
            return start + offset + character.len_utf8();
        }
    }
    start + current.len()
}

pub fn completions(result: &Value, context: &Context) -> Vec<Completion> {
    let Some(items) = result.as_array().or_else(|| result["items"].as_array()) else {
        return Vec::new();
    };
    let mut items: Vec<_> = items.iter().collect();
    items.sort_by_key(|item| {
        item["sortText"]
            .as_str()
            .unwrap_or_else(|| item["label"].as_str().unwrap_or(""))
    });
    items
        .into_iter()
        .take(80)
        .filter_map(|item| {
            let label = item["label"].as_str()?.to_string();
            let edit = &item["textEdit"];
            let range = edit.get("range").or_else(|| edit.get("replace"));
            let range = range
                .map(|r| byte_at(&context.text, &r["start"])..byte_at(&context.text, &r["end"]))
                .unwrap_or(context.cursor..context.cursor);
            let inserted = edit["newText"]
                .as_str()
                .or_else(|| item["insertText"].as_str())
                .unwrap_or(&label)
                .to_string();
            // snippetSupport=false is advertised; refuse snippets a server sends anyway.
            if item["insertTextFormat"].as_i64() == Some(2) {
                return None;
            }
            let mut caret = range.start + inserted.len();
            let mut edits = vec![(range.clone(), inserted)];
            if let Some(additional) = item["additionalTextEdits"].as_array() {
                for extra in additional {
                    let r = byte_at(&context.text, &extra["range"]["start"])
                        ..byte_at(&context.text, &extra["range"]["end"]);
                    let value = extra["newText"].as_str().unwrap_or("").to_string();
                    if r.end <= range.start {
                        caret = caret.saturating_add(value.len()).saturating_sub(r.len());
                    }
                    edits.push((r, value));
                }
            }
            Some(Completion {
                label,
                detail: item["detail"].as_str().unwrap_or("").into(),
                edits,
                caret,
            })
        })
        .collect()
}

pub fn hover_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(hover_text)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(_) => value["value"].as_str().unwrap_or("").to_string(),
        _ => String::new(),
    }
}
