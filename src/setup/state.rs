use super::{
    installer::{self, InstallEvent, InstallOptions},
    payload::Payload,
};
use rune::{FileDialogOptions, TextEdit, Tx, Waker};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

const DEFAULT_FAMILIES: &str = "Segoe UI, Malgun Gothic, Inter";
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Configure,
    Progress,
    Complete,
}
impl Stage {
    pub fn index(self) -> usize {
        match self {
            Self::Configure => 0,
            Self::Progress => 1,
            Self::Complete => 2,
        }
    }
}

pub struct SetupState {
    pub stage: Stage,
    pub destination: TextEdit,
    pub desktop_shortcut: bool,
    pub start_menu: bool,
    pub error: Option<String>,
    pub families: String,
    pub fonts_applied: bool,
    pub preview: bool,
    pub removing: bool,
    pub running: bool,
    pub cancel_requested: bool,
    pub percent: f64,
    pub phase: usize,
    pub current_file: String,
    pub status: String,
    pub payload_ready: bool,
    receive: Option<mpsc::Receiver<InstallEvent>>,
    cancel: Arc<AtomicBool>,
    wake: Option<Waker>,
}
impl SetupState {
    pub fn new() -> Self {
        let arguments: Vec<String> = env::args().collect();
        let screen = arguments
            .windows(2)
            .find(|pair| pair[0] == "--screen")
            .map(|pair| pair[1].as_str())
            .or_else(|| {
                arguments
                    .iter()
                    .find_map(|arg| arg.strip_prefix("--screen="))
            });
        let preview = arguments.iter().any(|arg| arg == "--preview") || screen.is_some();
        let uninstall = super::cli::uninstall_root();
        let removing = uninstall.is_some();
        let destination = uninstall.unwrap_or_else(|| {
            profile_path("LOCALAPPDATA")
                .or_else(|| profile_path("USERPROFILE").map(|home| home.join("AppData/Local")))
                .unwrap_or_else(env::temp_dir)
                .join("Programs")
                .join("RuneTeX")
        });
        let package = if !preview && !removing {
            Payload::open_current().map(|_| ())
        } else {
            Ok(())
        };
        Self {
            stage: match screen {
                Some("progress") => Stage::Progress,
                Some("complete") => Stage::Complete,
                _ => Stage::Configure,
            },
            destination: TextEdit::new(destination.to_string_lossy().into_owned()),
            desktop_shortcut: false,
            start_menu: true,
            error: package.as_ref().err().cloned(),
            families: saved_families(),
            fonts_applied: false,
            preview,
            removing,
            running: false,
            cancel_requested: false,
            percent: if preview { 62.0 } else { 0.0 },
            phase: if preview { 1 } else { 0 },
            current_file: if preview {
                "tools/texlab/texlab.exe".into()
            } else {
                String::new()
            },
            status: String::new(),
            payload_ready: package.is_ok(),
            receive: None,
            cancel: Arc::new(AtomicBool::new(false)),
            wake: None,
        }
    }
    pub fn setup(&mut self, wake: Waker) {
        self.wake = Some(wake);
    }
    pub fn poll(&mut self, _time: f64, tx: &mut Tx) -> bool {
        let mut changed = false;
        if !self.fonts_applied {
            tx.set_font_families(
                self.families.clone(),
                "Cascadia Code, Consolas, Malgun Gothic",
            );
            self.fonts_applied = true;
            changed = true;
        }
        let events: Vec<_> = self
            .receive
            .as_ref()
            .map(|r| r.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            changed = true;
            self.accept(event);
        }
        changed
    }
    fn accept(&mut self, event: InstallEvent) {
        match event {
            InstallEvent::Progress {
                percent,
                phase,
                current_file,
            } => {
                self.percent = percent;
                self.phase = phase;
                self.current_file = current_file;
            }
            InstallEvent::Finished(result) => {
                self.running = false;
                self.receive = None;
                match result {
                    Ok(()) => {
                        self.percent = 100.0;
                        self.phase = 3;
                        self.stage = Stage::Complete;
                        self.error = None;
                    }
                    Err(error) => {
                        self.stage = Stage::Configure;
                        self.error = Some(error);
                    }
                }
            }
        }
    }
    pub fn select_stage(&mut self, stage: Stage) {
        if self.preview {
            self.stage = stage;
            self.error = None;
        }
    }
    pub fn advance(&mut self, tx: &mut Tx) {
        if self.running {
            return;
        }
        if self.stage == Stage::Complete {
            tx.quit();
            return;
        }
        if self.preview {
            self.stage = match self.stage {
                Stage::Configure => Stage::Progress,
                _ => Stage::Complete,
            };
            return;
        }
        let destination = self.destination.text.trim();
        if destination.is_empty()
            || !Path::new(destination).is_absolute()
            || destination.chars().any(char::is_control)
        {
            self.error = Some("설치할 폴더의 전체 경로를 입력하세요".into());
            tx.focus("setup-destination", true);
            return;
        }
        if !self.payload_ready {
            return;
        }
        let options = InstallOptions {
            destination: PathBuf::from(destination),
            start_menu: self.start_menu,
            desktop_shortcut: self.desktop_shortcut,
        };
        let (send, receive) = mpsc::channel();
        self.receive = Some(receive);
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&self.cancel);
        let wake = self.wake.clone();
        let removing = self.removing;
        self.running = true;
        self.cancel_requested = false;
        self.error = None;
        self.percent = 0.0;
        self.phase = 0;
        self.current_file.clear();
        self.stage = Stage::Progress;
        std::thread::spawn(move || {
            let report = |event| {
                let _ = send.send(event);
                if let Some(wake) = &wake {
                    wake();
                }
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if removing {
                    installer::uninstall_with_progress(&options.destination, report)
                } else {
                    installer::install(options, cancel, report)
                }
            }))
            .unwrap_or_else(|_| {
                Err("설치 작업을 완료하지 못했습니다. 설치 폴더를 확인하세요".into())
            });
            let _ = send.send(InstallEvent::Finished(result));
            if let Some(wake) = wake {
                wake();
            }
        });
    }
    pub fn cancel_or_close(&mut self, tx: &mut Tx) {
        if self.running {
            if self.removing {
                return;
            }
            self.cancel_requested = true;
            self.status = "설치를 취소하고 변경을 되돌리는 중입니다".into();
            self.cancel.store(true, Ordering::SeqCst);
        } else {
            tx.quit();
        }
    }
    pub fn close(&mut self, _tx: &mut Tx) {
        if !self.running {
            return;
        }
        self.cancel.store(true, Ordering::SeqCst);
        // The window is already hidden. Wait for rollback before the process can exit.
        while self.running {
            match self
                .receive
                .as_ref()
                .and_then(|receive| receive.recv().ok())
            {
                Some(event) => self.accept(event),
                None => break,
            }
        }
    }
    pub fn browse(&mut self, tx: &mut Tx) {
        if self.running || self.removing {
            return;
        }
        self.error = None;
        let path = PathBuf::from(self.destination.text.trim());
        tx.open_folder(
            FileDialogOptions {
                title: "RuneTeX 설치 위치".into(),
                directory: path.is_absolute().then_some(path),
                ..Default::default()
            },
            |state: &mut Self, result, _| match result {
                Ok(Some(path)) => {
                    state.destination = TextEdit::new(path.to_string_lossy().into_owned());
                    state.error = None;
                }
                Ok(None) => {}
                Err(error) => state.error = Some(format!("폴더를 선택하지 못했습니다: {error}")),
            },
        );
    }
}
fn profile_path(variable: &str) -> Option<PathBuf> {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}
fn saved_families() -> String {
    let family = profile_path("APPDATA")
        .and_then(|root| fs::read(root.join("RuneTeX/ui-font.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value["family"].as_str().map(str::to_owned))
        .filter(|family| {
            !family.trim().is_empty()
                && family.len() <= 240
                && !family.chars().any(char::is_control)
        });
    family
        .map(|family| format!("{family}, Segoe UI, Malgun Gothic, Inter, sans-serif"))
        .unwrap_or_else(|| DEFAULT_FAMILIES.into())
}
