//! Editing actions and stale-response fencing for the Texlab connection.
use crate::{
    compiler::Diagnostic,
    lsp::{self, Client, Completion, Context, Event, Query},
    state::State,
};
use rune::{Key, KeyEvent, Span, Tx};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Language {
    pub client: Client,
    pub completions: Vec<Completion>,
    pub selected: usize,
    pub information: Option<String>,
    context: Option<Context>,
    edited: Option<Instant>,
}

impl Language {
    pub fn connected(client: Client) -> Self {
        Self {
            client,
            ..Default::default()
        }
    }
    pub fn changed(&mut self, path: &str) {
        self.completions.clear();
        self.information = None;
        self.context = None;
        self.client.diagnostics.remove(path);
        self.edited = Some(Instant::now());
    }
}

impl State {
    pub fn language_request(&mut self, kind: Query) {
        let context = Context {
            path: self.active_file().path.clone(),
            text: self.active_edit().text.clone(),
            cursor: self.active_edit().focus,
            revision: self.revision,
        };
        self.language.client.query(kind, context);
    }
    pub fn language_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        if key.is_command(' ') {
            self.language_request(Query::Completion);
            return true;
        }
        if key.key == Key::Function(12) {
            self.language_request(Query::Definition);
            return true;
        }
        if key.key == Key::Function(1) {
            self.language_request(Query::Hover);
            return true;
        }
        if key.key == Key::Escape
            && (!self.language.completions.is_empty() || self.language.information.is_some())
        {
            self.language.completions.clear();
            self.language.information = None;
            return true;
        }
        if !self.language.completions.is_empty() && !key.mods.cmd {
            match key.key {
                Key::Down => {
                    self.language.selected =
                        (self.language.selected + 1) % self.language.completions.len();
                    return true;
                }
                Key::Up => {
                    self.language.selected =
                        (self.language.selected + self.language.completions.len() - 1)
                            % self.language.completions.len();
                    return true;
                }
                Key::Tab | Key::Enter if !key.mods.shift => {
                    self.accept_completion(self.language.selected, tx);
                    return true;
                }
                _ => {}
            }
        }
        false
    }
    pub fn accept_completion(&mut self, index: usize, tx: &mut Tx) {
        let Some(context) = self.language.context.as_ref() else {
            return;
        };
        if !self.matches_context(context) {
            self.language.completions.clear();
            return;
        }
        let Some(item) = self.language.completions.get(index).cloned() else {
            return;
        };
        let text = &self.active_edit().text;
        let mut edits = item.edits;
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        if edits.iter().any(|(range, _)| {
            range.start > range.end
                || range.end > text.len()
                || !text.is_char_boundary(range.start)
                || !text.is_char_boundary(range.end)
        }) || edits.windows(2).any(|pair| pair[1].0.end > pair[0].0.start)
        {
            self.language.completions.clear();
            return;
        }
        let edit = self.active_edit_mut();
        edit.undo_group(|edit| {
            for (range, text) in edits {
                edit.set_selection(range.start, range.end);
                edit.insert(&text);
            }
        });
        edit.set_caret(item.caret.min(edit.text.len()));
        self.on_edit();
        tx.focus("source-editor", false);
    }
    fn matches_context(&self, context: &Context) -> bool {
        self.active_file().path == context.path
            && self.revision == context.revision
            && self.active_edit().focus == context.cursor
    }
    pub fn poll_language(&mut self, tx: &mut Tx) -> bool {
        let mut changed = false;
        if self
            .language
            .context
            .as_ref()
            .is_some_and(|context| !self.matches_context(context))
        {
            self.language.completions.clear();
            self.language.context = None;
            changed = true;
        }
        for event in self.language.client.drain() {
            changed = true;
            match event {
                Event::Status(status) => self.language.client.status = status,
                Event::Diagnostics(path, diagnostics) => {
                    self.language.client.diagnostics.insert(path, diagnostics);
                }
                Event::Response(kind, context, result) if self.matches_context(&context) => {
                    match kind {
                        Query::Completion => {
                            self.language.completions = lsp::completions(&result, &context);
                            self.language.selected = 0;
                            self.language.context = Some(context);
                        }
                        Query::Hover => {
                            let text = lsp::hover_text(&result["contents"]);
                            self.language.information = Some(if text.is_empty() {
                                "설명을 찾지 못했습니다.".into()
                            } else {
                                text.chars().take(12_000).collect()
                            });
                        }
                        Query::Definition => {
                            let target = result
                                .as_array()
                                .and_then(|items| items.first())
                                .unwrap_or(&result);
                            let uri = target["uri"]
                                .as_str()
                                .or_else(|| target["targetUri"].as_str());
                            let path =
                                uri.and_then(|uri| lsp::relative_path(&self.project_dir, uri));
                            let file = path.and_then(|path| {
                                self.files.iter().position(|file| file.path == path)
                            });
                            if let Some(file) = file {
                                let range = target
                                    .get("targetSelectionRange")
                                    .or_else(|| target.get("range"))
                                    .unwrap_or(&serde_json::Value::Null);
                                let byte =
                                    lsp::byte_at(&self.files[file].edit.text, &range["start"]);
                                let line =
                                    range["start"]["line"].as_u64().unwrap_or(0) as usize + 1;
                                self.jump_to(file, line, tx);
                                self.active_edit_mut().set_caret(byte);
                            } else {
                                self.message = "프로젝트 안에서 정의를 찾지 못했습니다.".into();
                            }
                        }
                    }
                }
                Event::Response(..) => {} // A response to text/caret that has already changed.
            }
        }
        if let Some(edited) = self.language.edited {
            if edited.elapsed() >= Duration::from_millis(250) {
                self.language.edited = None;
                self.language
                    .client
                    .sync(&self.active_file().path, &self.active_edit().text);
                let before = &self.active_edit().text[..self.active_edit().focus];
                let token = before
                    .rsplit(|c: char| c.is_whitespace() || c == '}' || c == '$')
                    .next()
                    .unwrap_or("");
                if self.active_edit().focused && self.dialog.is_none() && token.contains('\\') {
                    self.language_request(Query::Completion);
                }
            } else {
                tx.wake_after(Duration::from_millis(80));
            }
        }
        changed
    }
    pub fn all_diagnostics(&self) -> Vec<Diagnostic> {
        let mut diagnostics = self.diagnostics.clone();
        for (path, items) in &self.language.client.diagnostics {
            diagnostics.extend(items.iter().map(|item| Diagnostic {
                file: Some(path.clone()),
                line: Some(item.range["start"]["line"].as_u64().unwrap_or(0) as usize + 1),
                message: format!("[texlab] {}", item.message),
                warning: item.warning,
            }));
        }
        diagnostics
    }
    pub fn editor_spans(&self) -> Vec<Span> {
        let text = &self.active_edit().text;
        let mut spans = crate::state::syntax_spans(text);
        if let Some(items) = self
            .language
            .client
            .diagnostics
            .get(&self.active_file().path)
        {
            for item in items {
                let start = lsp::byte_at(text, &item.range["start"]);
                let mut end = lsp::byte_at(text, &item.range["end"]);
                if end <= start {
                    end = start + text[start..].chars().next().map_or(0, char::len_utf8);
                }
                if start < end {
                    spans.push(Span::new(start, end).underline(
                        rune::color::hex(if item.warning { "#e0ba80" } else { "#ef9393" }).unwrap(),
                    ));
                }
            }
        }
        spans
    }
}
