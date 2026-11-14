//! Project state and background persistence/compilation. Views never touch disk.
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use rune::{Pixels, Span, TextEdit, Tx, Waker};

use crate::compiler::{self, CompileRequest, CompileResult, Diagnostic, Engine};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Source,
    Split,
    Preview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    Search,
    NewFile,
    Settings,
}

pub struct ProjectFile {
    pub path: String,
    pub edit: TextEdit,
    pub saved_text: String,
}

impl ProjectFile {
    pub fn dirty(&self) -> bool {
        self.edit.text != self.saved_text
    }
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub file: usize,
    pub line: usize,
    pub level: usize,
    pub number: String,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub file: usize,
    pub line: Option<usize>,
    pub label: String,
    pub excerpt: String,
}

type Snapshot = Vec<(String, String)>;
#[derive(Default)]
struct Disk {
    lock: Mutex<()>,
    closing: AtomicBool,
}
enum Work {
    SaveEditorFont {
        path: PathBuf,
        font: String,
        size: f32,
    },
    SaveUiFont {
        path: PathBuf,
        font: String,
    },
    SavePackagePolicy {
        root: PathBuf,
        enabled: bool,
    },
    Save {
        revision: u64,
        root: PathBuf,
        files: Snapshot,
    },
    Compile {
        revision: u64,
        files: Snapshot,
        request: CompileRequest,
    },
    Open {
        revision: u64,
        old_root: PathBuf,
        files: Snapshot,
        path: PathBuf,
    },
    Export {
        source: PathBuf,
        destination: PathBuf,
    },
}
enum Finished {
    Saved {
        revision: u64,
        files: Snapshot,
    },
    Compiled {
        revision: u64,
        engine: Engine,
        files: Snapshot,
        result: CompileResult,
        pages: Vec<Pixels>,
    },
    Failed {
        compile: bool,
        message: String,
    },
    Opened {
        revision: u64,
        root: PathBuf,
        selected: String,
        files: Vec<ProjectFile>,
        auto_install_packages: bool,
    },
    Exported {
        destination: PathBuf,
    },
}

pub struct State {
    pub language: crate::language::Language,
    pub files: Vec<ProjectFile>,
    pub active: usize,
    pub tabs: Vec<usize>,
    pub project_dir: PathBuf,
    pub main_file: String,
    pub sidebar: bool,
    pub mode: ViewMode,
    pub split_ratio: f32,
    pub dialog: Option<DialogKind>,
    pub search: TextEdit,
    pub new_name: TextEdit,
    pub message: String,
    pub font_size: f32,
    pub ui_font: String,
    pub font_picker: crate::font_picker::FontPicker,
    pub editor_font: String,
    pub editor_font_picker: crate::font_picker::FontPicker,
    pub auto_compile: bool,
    pub auto_install_packages: bool,
    pub engine: Engine,
    pub compiling: bool,
    pub saving: bool,
    pub opening: bool,
    pub preview_pages: Vec<Pixels>,
    pub diagnostics: Vec<Diagnostic>,
    pub log: String,
    pub panel_open: bool,
    pub panel_log: bool,
    pub zoom: f32,
    pub preview_page: usize,
    pub latest_pdf: Option<PathBuf>,
    pub revision: u64,
    pub compiled_revision: Option<u64>,
    compiled_engine: Option<Engine>,
    pub last_compile_seconds: Option<f64>,
    work: mpsc::Sender<Work>,
    finished: mpsc::Receiver<Finished>,
    waker: Arc<Mutex<Option<Waker>>>,
    last_edit: Option<Instant>,
    pending_save: bool,
    attempted: Option<(u64, Engine)>,
    disk: Arc<Disk>,
    package_policy_dirty: bool,
    ui_font_path: PathBuf,
    ui_font_dirty: bool,
    ui_font_applied: bool,
    editor_font_path: PathBuf,
    editor_font_dirty: bool,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    pub fn new() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let project = args
            .windows(2)
            .find(|pair| pair[0] == "--project")
            .map(|pair| PathBuf::from(&pair[1]));
        let dir = project.unwrap_or_else(crate::paths::default_project_dir);
        let mut state = Self::from_project(dir);
        state.ui_font_path = crate::preferences::global_font_path();
        state.ui_font = crate::preferences::read_ui_font(&state.ui_font_path);
        state.font_picker.selected = state.ui_font.clone();
        state.editor_font_path = state.ui_font_path.with_file_name("editor-font.json");
        (state.editor_font, state.font_size) =
            crate::preferences::read_editor_font(&state.editor_font_path);
        state.editor_font_picker.selected = state.editor_font.clone();
        if let Some(engine) = args
            .windows(2)
            .find(|pair| pair[0] == "--engine")
            .and_then(|pair| Engine::from_name(&pair[1]))
        {
            state.engine = engine;
        }
        state
    }

    pub fn from_project(mut project_dir: PathBuf) -> Self {
        let selected = if project_dir.is_file() {
            let name = project_dir
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            project_dir = project_dir.parent().unwrap_or(Path::new(".")).to_path_buf();
            name
        } else {
            None
        };
        let mut initial_message = String::new();
        if !project_dir.exists()
            && let Err(error) = seed_project(&project_dir)
        {
            initial_message = format!("프로젝트를 만들지 못했습니다: {error}");
        }
        let mut loaded = Vec::new();
        read_project(&project_dir, &project_dir, &mut loaded, 0);
        loaded.sort_by(|a, b| (a.0 != "main.tex", &a.0).cmp(&(b.0 != "main.tex", &b.0)));
        let mut files: Vec<ProjectFile> = loaded
            .into_iter()
            .map(|(path, text)| {
                let mut edit = TextEdit::new(text.clone());
                edit.set_caret(0);
                ProjectFile {
                    path,
                    edit,
                    saved_text: text,
                }
            })
            .collect();
        if files.is_empty() {
            // An existing empty directory is a valid new project; seed it without overwriting.
            if let Err(error) = seed_project(&project_dir) {
                initial_message = format!("프로젝트를 읽지 못했습니다: {error}");
            }
            let text = include_str!("../assets/main.tex").to_string();
            let mut edit = TextEdit::new(text.clone());
            edit.set_caret(0);
            files.push(ProjectFile {
                path: "main.tex".into(),
                edit,
                saved_text: text,
            });
            let mut seeded = Vec::new();
            read_project(&project_dir, &project_dir, &mut seeded, 0);
            for (path, text) in seeded {
                if path != "main.tex" {
                    let mut edit = TextEdit::new(text.clone());
                    edit.set_caret(0);
                    files.push(ProjectFile {
                        path,
                        edit,
                        saved_text: text,
                    });
                }
            }
        }
        let main_file = selected
            .clone()
            .filter(|path| path.ends_with(".tex"))
            .or_else(|| {
                files
                    .iter()
                    .find(|file| file.path == "main.tex")
                    .map(|file| file.path.clone())
            })
            .or_else(|| {
                files
                    .iter()
                    .find(|file| file.edit.text.contains("\\documentclass"))
                    .map(|file| file.path.clone())
            })
            .unwrap_or_else(|| files[0].path.clone());
        let active = selected
            .and_then(|path| files.iter().position(|file| file.path == path))
            .unwrap_or(0);
        let (work_tx, work_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let waker: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
        let thread_waker = Arc::clone(&waker);
        let disk = Arc::new(Disk::default());
        let worker_disk = Arc::clone(&disk);
        std::thread::spawn(move || worker(work_rx, finished_tx, thread_waker, worker_disk));
        let (cached_pdf, cached_pages) = load_cached_preview(&project_dir, &main_file);
        let ui_font_path = project_dir.join(".runetex/ui-font.json");
        let ui_font = crate::preferences::read_ui_font(&ui_font_path);
        let editor_font_path = project_dir.join(".runetex/editor-font.json");
        let (editor_font, font_size) = crate::preferences::read_editor_font(&editor_font_path);
        Self {
            editor_font_picker: crate::font_picker::FontPicker::new(editor_font.clone()),
            editor_font,
            editor_font_path,
            editor_font_dirty: false,
            font_picker: crate::font_picker::FontPicker::new(ui_font.clone()),
            ui_font,
            ui_font_path,
            ui_font_dirty: false,
            ui_font_applied: false,
            language: crate::language::Language::default(),
            files,
            active,
            tabs: vec![active],
            auto_install_packages: read_package_policy(&project_dir),
            project_dir,
            main_file,
            sidebar: true,
            mode: ViewMode::Split,
            split_ratio: 0.52,
            dialog: None,
            search: TextEdit::default(),
            new_name: TextEdit::default(),
            message: if initial_message.is_empty() {
                "로컬 편집 준비됨".into()
            } else {
                initial_message
            },
            font_size,
            auto_compile: false,
            engine: Engine::default(),
            compiling: false,
            saving: false,
            opening: false,
            preview_pages: cached_pages,
            diagnostics: Vec::new(),
            log: "컴파일하면 이곳에 LaTeX 출력을 표시합니다.".into(),
            panel_open: false,
            panel_log: false,
            zoom: 1.0,
            preview_page: 0,
            latest_pdf: cached_pdf,
            revision: 0,
            compiled_revision: None,
            compiled_engine: None,
            last_compile_seconds: None,
            work: work_tx,
            finished: finished_rx,
            waker,
            last_edit: None,
            pending_save: false,
            attempted: None,
            disk,
            package_policy_dirty: false,
        }
    }

    pub fn setup(&mut self, waker: Waker) {
        if let Ok(mut slot) = self.waker.lock() {
            *slot = Some(waker);
        }
        self.restart_lsp();
        self.start_compile();
    }

    pub fn restart_lsp(&mut self) {
        let wake = self.waker.lock().ok().and_then(|slot| slot.clone());
        if let Some(wake) = wake {
            self.language = crate::language::Language::connected(crate::lsp::Client::start(
                self.project_dir.clone(),
                self.snapshot(),
                wake,
            ));
        }
    }

    pub fn active_file(&self) -> &ProjectFile {
        &self.files[self.active]
    }
    pub fn active_edit(&self) -> &TextEdit {
        &self.active_file().edit
    }
    pub fn active_edit_mut(&mut self) -> &mut TextEdit {
        &mut self.files[self.active].edit
    }
    pub fn dirty(&self) -> bool {
        self.files.iter().any(ProjectFile::dirty)
    }
    pub fn preview_stale(&self) -> bool {
        self.compiled_revision != Some(self.revision) || self.compiled_engine != Some(self.engine)
    }
    pub fn project_name(&self) -> String {
        self.project_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    pub fn on_edit(&mut self) {
        self.language.changed(&self.active_file().path.clone());
        self.revision = self.revision.wrapping_add(1);
        self.last_edit = Some(Instant::now());
        self.pending_save = true;
        self.message = "변경 사항 있음".into();
    }

    fn snapshot(&self) -> Snapshot {
        self.files
            .iter()
            .map(|file| (file.path.clone(), file.edit.text.clone()))
            .collect()
    }

    pub fn save_all(&mut self) {
        self.pending_save = false;
        self.saving = true;
        self.message = "저장 중…".into();
        if self
            .work
            .send(Work::Save {
                revision: self.revision,
                root: self.project_dir.clone(),
                files: self.snapshot(),
            })
            .is_err()
        {
            self.saving = false;
            self.message = "저장 작업을 시작하지 못했습니다.".into();
        }
    }

    pub fn start_compile(&mut self) {
        if self.compiling || self.opening {
            return;
        }
        self.pending_save = false;
        self.compiling = true;
        self.attempted = Some((self.revision, self.engine));
        self.saving = true;
        self.message = format!("{} 컴파일 중…", self.engine.label());
        let request = CompileRequest {
            project_dir: self.project_dir.clone(),
            main_file: self.main_file.clone(),
            engine: self.engine,
            auto_install_packages: self.auto_install_packages,
            generation: self.revision,
        };
        if self
            .work
            .send(Work::Compile {
                revision: self.revision,
                files: self.snapshot(),
                request,
            })
            .is_err()
        {
            self.compiling = false;
            self.saving = false;
            self.message = "컴파일 작업을 시작하지 못했습니다.".into();
        }
    }

    pub fn toggle_package_installer(&mut self) {
        self.auto_install_packages = !self.auto_install_packages;
        self.package_policy_dirty = true;
        // Allow retrying a missing-package failure after changing the policy.
        self.attempted = None;
        if self
            .work
            .send(Work::SavePackagePolicy {
                root: self.project_dir.clone(),
                enabled: self.auto_install_packages,
            })
            .is_err()
        {
            self.message = "패키지 설치 설정을 저장하지 못했습니다.".into();
        }
    }

    pub fn set_ui_font(&mut self, font: &str, tx: &mut Tx) {
        let font = font.trim();
        if !crate::preferences::valid_font(font) {
            self.message = "올바른 UI 글꼴 이름을 입력하세요.".into();
            return;
        }
        self.ui_font = font.to_string();
        self.font_picker.selected = font.to_string();
        self.font_picker.open = false;
        self.ui_font_dirty = true;
        self.ui_font_applied = true;
        tx.set_font_families(
            crate::preferences::ui_families(font),
            crate::preferences::editor_families(&self.editor_font),
        );
        if self
            .work
            .send(Work::SaveUiFont {
                path: self.ui_font_path.clone(),
                font: font.into(),
            })
            .is_err()
        {
            self.message = "UI 글꼴 설정을 저장하지 못했습니다.".into();
        }
    }

    pub fn apply_ui_font_selection(&mut self, tx: &mut Tx) {
        let font = self.font_picker.selected.clone();
        self.set_ui_font(&font, tx);
    }

    pub fn apply_editor_font_selection(&mut self, tx: &mut Tx) {
        let font = self.editor_font_picker.selected.trim().to_string();
        if !crate::preferences::valid_font(&font) {
            return;
        }
        self.editor_font = font;
        self.editor_font_picker.open = false;
        tx.set_font_families(
            crate::preferences::ui_families(&self.ui_font),
            crate::preferences::editor_families(&self.editor_font),
        );
        self.save_editor_font();
    }

    pub fn change_editor_font_size(&mut self, delta: f32) {
        let size = (self.font_size + delta).clamp(
            crate::preferences::MIN_EDITOR_SIZE,
            crate::preferences::MAX_EDITOR_SIZE,
        );
        if self.font_size != size {
            self.font_size = size;
            self.save_editor_font();
        }
    }

    fn save_editor_font(&mut self) {
        self.editor_font_dirty = true;
        if self
            .work
            .send(Work::SaveEditorFont {
                path: self.editor_font_path.clone(),
                font: self.editor_font.clone(),
                size: self.font_size,
            })
            .is_err()
        {
            self.message = "에디터 글꼴 설정을 저장하지 못했습니다.".into();
        }
    }

    /// Saves the current snapshot before loading another project in the worker.
    pub fn open_project(&mut self, path: PathBuf) {
        if self.opening {
            return;
        }
        self.opening = true;
        self.pending_save = false;
        self.message = "프로젝트를 여는 중…".into();
        if self
            .work
            .send(Work::Open {
                revision: self.revision,
                old_root: self.project_dir.clone(),
                files: self.snapshot(),
                path,
            })
            .is_err()
        {
            self.opening = false;
            self.message = "프로젝트를 열지 못했습니다.".into();
        }
    }

    pub fn export_pdf(&mut self, destination: PathBuf) {
        let Some(source) = self.latest_pdf.clone() else {
            self.message = "먼저 문서를 컴파일하세요.".into();
            return;
        };
        if self
            .work
            .send(Work::Export {
                source,
                destination,
            })
            .is_err()
        {
            self.message = "PDF 내보내기를 시작하지 못했습니다.".into();
        }
    }

    /// Rune invokes this after the window hides and synchronously before OS quit.
    pub fn flush_on_close(&mut self, _tx: &mut Tx) {
        self.flush();
    }

    pub fn flush(&mut self) {
        if self.disk.closing.swap(true, Ordering::SeqCst) {
            return;
        }
        let files = self.snapshot();
        let disk = Arc::clone(&self.disk);
        let _guard = disk
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.ui_font_dirty
            && let Err(error) = crate::preferences::write_ui_font(&self.ui_font_path, &self.ui_font)
        {
            eprintln!("UI 글꼴 설정 저장 실패: {error}");
        }
        if self.editor_font_dirty
            && let Err(error) = crate::preferences::write_editor_font(
                &self.editor_font_path,
                &self.editor_font,
                self.font_size,
            )
        {
            eprintln!("에디터 글꼴 설정 저장 실패: {error}");
        }
        if self.package_policy_dirty
            && let Err(error) = write_package_policy(&self.project_dir, self.auto_install_packages)
        {
            eprintln!("패키지 설치 설정 저장 실패: {error}");
        }
        if self.dirty() || self.pending_save || self.saving {
            match write_snapshot(&self.project_dir, &files) {
                Ok(()) => {
                    self.mark_saved(files);
                    self.pending_save = false;
                    self.message = "모든 변경 사항 저장됨".into();
                }
                Err(error) => {
                    self.message = format!("종료 전 저장 실패: {error}");
                    eprintln!("{}", self.message);
                }
            }
        }
    }

    fn mark_saved(&mut self, snapshot: Snapshot) {
        for (path, text) in snapshot {
            if let Some(file) = self.files.iter_mut().find(|file| file.path == path) {
                file.saved_text = text;
            }
        }
        self.saving = false;
    }

    pub fn poll(&mut self, _time: f64, tx: &mut Tx) -> bool {
        let mut changed = self.poll_language(tx);
        if !self.ui_font_applied {
            tx.set_font_families(
                crate::preferences::ui_families(&self.ui_font),
                crate::preferences::editor_families(&self.editor_font),
            );
            self.ui_font_applied = true;
            changed = true;
        }
        while let Ok(event) = self.finished.try_recv() {
            changed = true;
            match event {
                Finished::Saved { revision, files } => {
                    self.mark_saved(files);
                    for file in &self.files {
                        self.language.client.sync(&file.path, &file.edit.text);
                        self.language.client.saved(&file.path);
                    }
                    self.message = if revision == self.revision {
                        "모든 변경 사항 저장됨"
                    } else {
                        "변경 사항 있음"
                    }
                    .into();
                }
                Finished::Compiled {
                    revision,
                    engine,
                    files,
                    result,
                    pages,
                } => {
                    debug_assert_eq!(
                        result.generation, revision,
                        "compile result must belong to the requested revision"
                    );
                    self.mark_saved(files);
                    self.compiling = false;
                    self.last_compile_seconds = Some(result.elapsed.as_secs_f64());
                    self.log = result.log;
                    self.diagnostics = result.diagnostics;
                    if let Some(path) = result.pdf_path {
                        self.latest_pdf = Some(path);
                        self.preview_pages = pages;
                        self.preview_page = self
                            .preview_page
                            .min(self.preview_pages.len().saturating_sub(1));
                        self.compiled_revision = Some(revision);
                        self.compiled_engine = Some(engine);
                        self.message = format!(
                            "컴파일 완료 · {:.2}초{}",
                            result.elapsed.as_secs_f64(),
                            if revision == self.revision {
                                ""
                            } else {
                                " · 새 변경 사항 있음"
                            }
                        );
                        if self.preview_pages.is_empty() {
                            self.message.push_str(" · PDF 렌더러를 확인하세요");
                            self.panel_open = true;
                            self.panel_log = true;
                        }
                    } else {
                        self.message = "컴파일 실패 · 문제와 출력을 확인하세요".into();
                        self.panel_open = true;
                        self.panel_log = self.diagnostics.is_empty();
                    }
                }
                Finished::Failed { compile, message } => {
                    self.saving = false;
                    self.opening = false;
                    if compile {
                        self.compiling = false;
                    }
                    self.message = message.clone();
                    self.log = message;
                    self.panel_open = true;
                    self.panel_log = true;
                }
                Finished::Opened {
                    revision,
                    root,
                    selected,
                    files,
                    auto_install_packages,
                } => {
                    self.opening = false;
                    if revision != self.revision {
                        self.message = "새 편집 내용이 있어 프로젝트 전환을 취소했습니다. 저장 후 다시 여세요.".into();
                        self.pending_save = true;
                        continue;
                    }
                    self.files = files;
                    self.active = self
                        .files
                        .iter()
                        .position(|file| file.path == selected)
                        .unwrap_or(0);
                    self.tabs = vec![self.active];
                    self.main_file = selected;
                    self.project_dir = root;
                    self.auto_install_packages = auto_install_packages;
                    self.package_policy_dirty = false;
                    self.revision = 0;
                    self.compiled_revision = None;
                    self.compiled_engine = None;
                    self.attempted = None;
                    self.last_edit = None;
                    self.pending_save = false;
                    self.preview_pages.clear();
                    self.latest_pdf = None;
                    self.diagnostics.clear();
                    self.dialog = None;
                    self.preview_page = 0;
                    self.restart_lsp();
                    self.start_compile();
                }
                Finished::Exported { destination } => {
                    self.message = format!(
                        "PDF 저장됨 · {}",
                        destination
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    );
                }
            }
        }
        if let Some(last_edit) = self.last_edit {
            let elapsed = last_edit.elapsed();
            if self.auto_compile
                && !self.compiling
                && self.preview_stale()
                && self.attempted != Some((self.revision, self.engine))
                && elapsed >= Duration::from_millis(1200)
            {
                self.start_compile();
                changed = true;
            } else if self.pending_save && elapsed >= Duration::from_millis(800) {
                self.save_all();
                changed = true;
            }
            if self.pending_save
                || (self.auto_compile
                    && self.preview_stale()
                    && !self.compiling
                    && self.attempted != Some((self.revision, self.engine)))
            {
                tx.wake_after(Duration::from_millis(150));
            }
        }
        changed
    }

    pub fn switch_file(&mut self, index: usize) {
        if index >= self.files.len() {
            return;
        }
        self.active = index;
        self.language.completions.clear();
        self.language.information = None;
        self.language
            .client
            .sync(&self.active_file().path, &self.active_edit().text);
        if !self.tabs.contains(&index) {
            self.tabs.push(index);
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if self.tabs.len() <= 1 {
            return;
        }
        if let Some(position) = self.tabs.iter().position(|tab| *tab == index) {
            self.tabs.remove(position);
            if self.active == index {
                self.active = self.tabs[position.min(self.tabs.len() - 1)];
            }
        }
    }

    pub fn add_file(&mut self) -> bool {
        let path = match validate_file_name(&self.new_name.text) {
            Ok(path) => path,
            Err(message) => {
                self.message = message.into();
                return false;
            }
        };
        if self
            .files
            .iter()
            .any(|file| file.path.eq_ignore_ascii_case(&path))
        {
            self.message = "같은 이름의 파일이 이미 있습니다.".into();
            return false;
        }
        self.files.push(ProjectFile {
            path,
            edit: TextEdit::default(),
            saved_text: String::new(),
        });
        self.switch_file(self.files.len() - 1);
        self.new_name = TextEdit::default();
        self.dialog = None;
        self.on_edit();
        self.save_all();
        true
    }

    pub fn outline(&self) -> Vec<OutlineItem> {
        let main = self
            .files
            .iter()
            .position(|file| file.path == self.main_file)
            .or_else(|| {
                self.files
                    .get(self.active)
                    .filter(|file| file.path.ends_with(".tex"))
                    .map(|_| self.active)
            });
        let Some(main) = main else {
            return Vec::new();
        };
        let mut headings = Vec::new();
        collect_outline(
            &self.files,
            main,
            &mut vec![false; self.files.len()],
            &mut headings,
        );
        let chapters = headings.iter().any(|(_, rank, _)| *rank == 0);
        let mut counters = [0usize; 4];
        headings
            .into_iter()
            .map(|(mut item, rank, starred)| {
                item.level = rank.saturating_sub(usize::from(!chapters));
                if !starred {
                    counters[rank] += 1;
                    counters[rank + 1..].fill(0);
                    let first = usize::from(!chapters);
                    item.number = counters[first..=rank]
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(".");
                }
                item
            })
            .collect()
    }

    pub fn jump_to(&mut self, file: usize, line: usize, tx: &mut Tx) {
        self.switch_file(file);
        let text = &self.files[self.active].edit.text;
        let byte = text
            .split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::len)
            .sum::<usize>()
            .min(text.len());
        self.files[self.active].edit.set_caret(byte);
        self.dialog = None;
        if self.mode == ViewMode::Preview {
            self.mode = ViewMode::Split;
        }
        tx.focus("source-editor", false);
        tx.scroll_to_y(
            "source-scroll",
            (line.saturating_sub(3) as f32) * self.font_size * 1.5,
        );
    }

    pub fn search_results(&self) -> Vec<SearchResult> {
        let query = self.search.text.trim().to_lowercase();
        let mut results = Vec::new();
        for (index, file) in self.files.iter().enumerate() {
            if query.is_empty() || file.path.to_lowercase().contains(&query) {
                results.push(SearchResult {
                    file: index,
                    line: None,
                    label: file.path.clone(),
                    excerpt: String::new(),
                });
            }
            if query.is_empty() {
                continue;
            }
            for (line, text) in file.edit.text.lines().enumerate() {
                if text.to_lowercase().contains(&query) {
                    results.push(SearchResult {
                        file: index,
                        line: Some(line + 1),
                        label: format!("{}:{}", file.path, line + 1),
                        excerpt: text.trim().chars().take(110).collect(),
                    });
                    if results.len() >= 80 {
                        return results;
                    }
                }
            }
        }
        results
    }

    pub fn cursor_position(&self) -> (usize, usize) {
        let edit = self.active_edit();
        let focus = edit.focus.min(edit.text.len());
        let prefix = edit.text.get(..focus).unwrap_or("");
        (
            prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
            prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
        )
    }
}

impl Drop for State {
    fn drop(&mut self) {
        self.flush();
    }
}

fn worker(
    receiver: mpsc::Receiver<Work>,
    sender: mpsc::Sender<Finished>,
    waker: Arc<Mutex<Option<Waker>>>,
    disk: Arc<Disk>,
) {
    for work in receiver {
        if disk.closing.load(Ordering::SeqCst) {
            break;
        }
        let event = match work {
            Work::SaveEditorFont { path, font, size } => {
                let _guard = disk
                    .lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if disk.closing.load(Ordering::SeqCst) {
                    break;
                }
                match crate::preferences::write_editor_font(&path, &font, size) {
                    Ok(()) => continue,
                    Err(error) => Finished::Failed {
                        compile: false,
                        message: format!("에디터 글꼴 설정 저장 실패: {error}"),
                    },
                }
            }
            Work::SaveUiFont { path, font } => {
                let _guard = disk
                    .lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if disk.closing.load(Ordering::SeqCst) {
                    break;
                }
                match crate::preferences::write_ui_font(&path, &font) {
                    Ok(()) => continue,
                    Err(error) => Finished::Failed {
                        compile: false,
                        message: format!("UI 글꼴 설정 저장 실패: {error}"),
                    },
                }
            }
            Work::SavePackagePolicy { root, enabled } => {
                let _guard = disk
                    .lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if disk.closing.load(Ordering::SeqCst) {
                    break;
                }
                match write_package_policy(&root, enabled) {
                    Ok(()) => continue,
                    Err(error) => Finished::Failed {
                        compile: false,
                        message: format!("패키지 설치 설정 저장 실패: {error}"),
                    },
                }
            }
            Work::Save {
                revision,
                root,
                files,
            } => match worker_write(&disk, &root, &files) {
                Ok(()) => Finished::Saved { revision, files },
                Err(error) => Finished::Failed {
                    compile: false,
                    message: format!("파일 저장 실패: {error}"),
                },
            },
            Work::Compile {
                revision,
                files,
                request,
            } => match worker_write(&disk, &request.project_dir, &files) {
                Ok(()) => {
                    if disk.closing.load(Ordering::SeqCst) {
                        break;
                    }
                    let engine = request.engine;
                    let result = compiler::compile(request);
                    let pages = result
                        .pages
                        .iter()
                        .filter_map(|page| {
                            decode_page(&page.png)
                                .filter(|pixels| pixels.size() == (page.width, page.height))
                        })
                        .collect();
                    Finished::Compiled {
                        revision,
                        engine,
                        files,
                        result,
                        pages,
                    }
                }
                Err(error) => Finished::Failed {
                    compile: true,
                    message: format!("파일 저장 실패: {error}"),
                },
            },
            Work::Open {
                revision,
                old_root,
                files,
                path,
            } => match worker_write(&disk, &old_root, &files).and_then(|()| load_project(&path)) {
                Ok((root, selected, files)) => Finished::Opened {
                    auto_install_packages: read_package_policy(&root),
                    revision,
                    root,
                    selected,
                    files,
                },
                Err(error) => Finished::Failed {
                    compile: false,
                    message: format!("프로젝트 열기 실패: {error}"),
                },
            },
            Work::Export {
                source,
                destination,
            } => {
                let result = if source == destination {
                    Ok(0)
                } else {
                    fs::copy(&source, &destination)
                };
                match result {
                    Ok(_) => Finished::Exported { destination },
                    Err(error) => Finished::Failed {
                        compile: false,
                        message: format!("PDF 저장 실패: {error}"),
                    },
                }
            }
        };
        if sender.send(event).is_err() {
            break;
        }
        if let Ok(slot) = waker.lock()
            && let Some(wake) = slot.as_ref()
        {
            wake();
        }
    }
}

fn read_package_policy(root: &Path) -> bool {
    fs::read_to_string(root.join(".runetex/settings.conf"))
        .ok()
        .and_then(|settings| {
            settings.lines().find_map(|line| match line.trim() {
                "auto_install_packages=false" => Some(false),
                "auto_install_packages=true" => Some(true),
                _ => None,
            })
        })
        .unwrap_or(true)
}

fn write_package_policy(root: &Path, enabled: bool) -> std::io::Result<()> {
    fs::create_dir_all(root.join(".runetex"))?;
    let temporary = root.join(".runetex/settings.conf.tmp");
    fs::write(&temporary, format!("auto_install_packages={enabled}\n"))?;
    fs::rename(temporary, root.join(".runetex/settings.conf"))
}

fn worker_write(disk: &Disk, root: &Path, files: &Snapshot) -> std::io::Result<()> {
    let _guard = disk
        .lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if disk.closing.load(Ordering::SeqCst) {
        return Ok(());
    }
    write_snapshot(root, files)
}

fn decode_page(png: &[u8]) -> Option<Pixels> {
    let image = image::load_from_memory(png).ok()?.to_rgba8();
    let pixels = Pixels::new();
    pixels.set(image.width(), image.height(), image.into_raw());
    Some(pixels)
}

fn load_project(path: &Path) -> std::io::Result<(PathBuf, String, Vec<ProjectFile>)> {
    let root = if path.is_file() {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        path.to_path_buf()
    };
    let selected = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut loaded = Vec::new();
    read_project(&root, &root, &mut loaded, 0);
    if loaded.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "TeX 소스 파일이 없습니다.",
        ));
    }
    loaded.sort_by(|a, b| (a.0 != "main.tex", &a.0).cmp(&(b.0 != "main.tex", &b.0)));
    let files: Vec<ProjectFile> = loaded
        .into_iter()
        .map(|(path, text)| {
            let mut edit = TextEdit::new(text.clone());
            edit.set_caret(0);
            ProjectFile {
                path,
                edit,
                saved_text: text,
            }
        })
        .collect();
    let main = files
        .iter()
        .find(|file| file.path == selected && file.path.ends_with(".tex"))
        .or_else(|| files.iter().find(|file| file.path == "main.tex"))
        .or_else(|| {
            files
                .iter()
                .find(|file| file.edit.text.contains("\\documentclass"))
        })
        .or_else(|| files.iter().find(|file| file.path.ends_with(".tex")))
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "TeX 소스 파일이 없습니다.")
        })?
        .path
        .clone();
    Ok((root, main, files))
}

fn load_cached_preview(root: &Path, main: &str) -> (Option<PathBuf>, Vec<Pixels>) {
    let mut builds: Vec<_> = fs::read_dir(root.join(".runetex/build"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .collect();
    builds.sort_by_key(|entry| {
        std::cmp::Reverse(entry.metadata().and_then(|meta| meta.modified()).ok())
    });
    let job = Path::new(main)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    for build in builds {
        let pdf = build.path().join(format!("{job}.pdf"));
        if !pdf.is_file() {
            continue;
        }
        let mut pngs: Vec<_> = fs::read_dir(build.path())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let page = path
                    .file_stem()?
                    .to_str()?
                    .strip_prefix("preview-")?
                    .parse::<usize>()
                    .ok()?;
                (path.extension()? == "png").then_some((page, path))
            })
            .collect();
        pngs.sort_by_key(|(page, _)| *page);
        let pages: Vec<Pixels> = pngs
            .into_iter()
            .take(30)
            .filter_map(|(_, path)| fs::read(path).ok().and_then(|png| decode_page(&png)))
            .collect();
        // A failed LaTeX pass can leave a truncated PDF. Only a completed raster
        // proves this cached build produced a usable document.
        if !pages.is_empty() {
            return (Some(pdf), pages);
        }
    }
    (None, Vec::new())
}

fn write_snapshot(root: &Path, files: &Snapshot) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    let canonical_root = fs::canonicalize(root)?;
    for (relative, text) in files {
        // Relative names are also checked here: the worker is a second boundary.
        if Path::new(relative).is_absolute()
            || Path::new(relative)
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "프로젝트 밖에는 저장할 수 없습니다.",
            ));
        }
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            let mut checked = root.to_path_buf();
            if let Ok(parts) = parent.strip_prefix(root) {
                for component in parts.components() {
                    checked.push(component.as_os_str());
                    if fs::symlink_metadata(&checked)
                        .is_ok_and(|meta| meta.file_type().is_symlink())
                    {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "연결된 폴더에는 저장할 수 없습니다.",
                        ));
                    }
                }
            }
            fs::create_dir_all(parent)?;
            if !fs::canonicalize(parent)?.starts_with(&canonical_root) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "프로젝트 밖에는 저장할 수 없습니다.",
                ));
            }
        }
        // TeX and Texlab may hold read handles. Do not replace unchanged sources.
        if fs::read(&path).is_ok_and(|current| current == text.as_bytes()) {
            continue;
        }
        let temporary = path.with_extension(format!(
            "{}.runetex-tmp",
            path.extension().unwrap_or_default().to_string_lossy()
        ));
        fs::write(&temporary, text)?;
        for attempt in 0..=10 {
            match fs::rename(&temporary, &path) {
                Ok(()) => break,
                Err(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied && attempt < 10 =>
                {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

fn seed_project(root: &Path) -> std::io::Result<()> {
    fs::create_dir_all(root)?;
    for (path, text) in [
        ("main.tex", include_str!("../assets/main.tex")),
        ("references.bib", include_str!("../assets/references.bib")),
    ] {
        let destination = root.join(path);
        if !destination.exists() {
            fs::write(destination, text)?;
        }
    }
    Ok(())
}

fn read_project(root: &Path, dir: &Path, result: &mut Vec<(String, String)>, depth: usize) {
    if depth > 12 || result.len() >= 500 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if result.len() >= 500 {
            break;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if !entry.file_name().to_string_lossy().starts_with('.') {
                read_project(root, &path, result, depth + 1);
            }
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|ext| matches!(ext.to_str(), Some("tex" | "bib" | "sty" | "cls")))
            && entry
                .metadata()
                .is_ok_and(|meta| meta.len() <= 8 * 1024 * 1024)
            && let Ok(text) = fs::read_to_string(&path)
        {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            result.push((relative, text));
        }
    }
}

pub fn validate_file_name(name: &str) -> Result<String, &'static str> {
    let normalized = name.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err("파일 이름을 입력하세요.");
    }
    if normalized.len() > 220
        || normalized
            .chars()
            .any(|c| c.is_control() || "<>:\"|?*".contains(c))
    {
        return Err("사용할 수 없는 파일 이름입니다.");
    }
    let path = Path::new(&normalized);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("프로젝트 안의 상대 경로를 입력하세요.");
    }
    if normalized.split('/').any(|part| {
        let stem = part.split('.').next().unwrap_or("").to_uppercase();
        part.is_empty()
            || part.starts_with('.')
            || part.ends_with(['.', ' '])
            || [
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9",
            ]
            .contains(&stem.as_str())
    }) {
        return Err("사용할 수 없는 파일 이름입니다.");
    }
    if !matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("tex" | "bib" | "sty")
    ) {
        return Err(".tex, .bib, .sty 확장자를 사용하세요.");
    }
    Ok(normalized)
}

fn uncomment(text: &str) -> &str {
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'%' {
            let slashes = bytes[..index]
                .iter()
                .rev()
                .take_while(|byte| **byte == b'\\')
                .count();
            if slashes % 2 == 0 {
                return &text[..index];
            }
        }
    }
    text
}

fn collect_outline(
    files: &[ProjectFile],
    index: usize,
    active: &mut [bool],
    result: &mut Vec<(OutlineItem, usize, bool)>,
) {
    if active[index] || result.len() >= 10_000 {
        return;
    }
    active[index] = true;
    let parent = Path::new(&files[index].path)
        .parent()
        .unwrap_or(Path::new(""));
    for (line_index, line) in files[index].edit.text.lines().enumerate() {
        let line = uncomment(line).trim();
        for (command, rank) in [
            ("\\chapter", 0),
            ("\\section", 1),
            ("\\subsection", 2),
            ("\\subsubsection", 3),
        ] {
            if let Some(rest) = command_suffix(line, command) {
                let rest = rest.trim_start();
                let starred = rest.starts_with('*');
                let rest = rest.strip_prefix('*').unwrap_or(rest).trim_start();
                let rest = if rest.starts_with('[') {
                    rest.find(']')
                        .map(|end| rest[end + 1..].trim_start())
                        .unwrap_or("")
                } else {
                    rest
                };
                if let Some(title) = rest.strip_prefix('{').and_then(|_| braced_text(rest)) {
                    result.push((
                        OutlineItem {
                            file: index,
                            line: line_index + 1,
                            level: 0,
                            number: String::new(),
                            title: title.into(),
                        },
                        rank,
                        starred,
                    ));
                }
                break;
            }
        }
        for command in ["\\input", "\\include"] {
            if let Some(rest) = command_suffix(line, command)
                && let Some(relative) = braced_text(rest)
            {
                let relative = if relative.ends_with(".tex") {
                    relative.to_string()
                } else {
                    format!("{relative}.tex")
                };
                let nested = parent.join(&relative).to_string_lossy().replace('\\', "/");
                let target = files
                    .iter()
                    .position(|file| file.path == relative)
                    .or_else(|| files.iter().position(|file| file.path == nested));
                if let Some(target) = target {
                    collect_outline(files, target, active, result);
                }
            }
        }
    }
    active[index] = false;
}

fn command_suffix<'a>(line: &'a str, command: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(command)?;
    (!rest.starts_with(|character: char| character.is_ascii_alphabetic() || character == '@'))
        .then_some(rest)
}

fn braced_text(text: &str) -> Option<&str> {
    let start = text.find('{')? + 1;
    let mut depth = 1usize;
    for (offset, character) in text[start..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn syntax_spans(source: &str) -> Vec<Span> {
    let color = |value| rune::color::hex(value).unwrap_or_default();
    let command = color("#a0bdc0");
    let comment = color("#858585");
    let brace = color("#999999");
    let option = color("#bfc2ae");
    let math = color("#cbb694");
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            b'%' => {
                i = source[i..]
                    .find('\n')
                    .map_or(bytes.len(), |offset| i + offset);
                spans.push(Span::new(start, i).color(comment));
            }
            b'\\' => {
                i += 1;
                if i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'@') {
                    while i < bytes.len() && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'@') {
                        i += 1;
                    }
                    spans.push(Span::new(start, i).color(command));
                } else if i < bytes.len() {
                    i += source[i..].chars().next().map_or(1, char::len_utf8);
                    spans.push(Span::new(start, i).color(command));
                }
            }
            b'{' | b'}' => {
                i += 1;
                spans.push(Span::new(start, i).color(brace));
            }
            b'[' => {
                i = source[i..].find(']').map_or(i + 1, |offset| i + offset + 1);
                spans.push(Span::new(start, i).color(option));
            }
            b'$' => {
                let double = bytes.get(i + 1) == Some(&b'$');
                i += if double { 2 } else { 1 };
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 1 + source[i + 1..].chars().next().map_or(1, char::len_utf8);
                    } else if bytes[i] == b'$' {
                        i += 1;
                        if double && bytes.get(i) == Some(&b'$') {
                            i += 1;
                        }
                        break;
                    } else {
                        i += source[i..].chars().next().map_or(1, char::len_utf8);
                    }
                }
                spans.push(Span::new(start, i).color(math));
            }
            _ => i += source[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    spans
}
