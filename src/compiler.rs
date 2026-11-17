//! TeX compilation and PDF metadata. Call `compile` on a worker thread.
//!
//! Processes receive individual arguments, never shell command text. Each request
//! has an isolated output directory, so failed builds cannot reuse an older PDF.

use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TEX_TIMEOUT: Duration = Duration::from_secs(90);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    Pdf,
    #[default]
    Xe,
    Lua,
}

impl Engine {
    pub const ALL: [Self; 3] = [Self::Pdf, Self::Xe, Self::Lua];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "pdfLaTeX",
            Self::Xe => "XeLaTeX",
            Self::Lua => "LuaLaTeX",
        }
    }

    pub fn executable(self) -> &'static str {
        match self {
            Self::Pdf => "pdflatex",
            Self::Xe => "xelatex",
            Self::Lua => "lualatex",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|engine| {
            engine.executable().eq_ignore_ascii_case(name)
                || engine.label().eq_ignore_ascii_case(name)
        })
    }
}

pub struct CompileRequest {
    pub project_dir: PathBuf,
    pub main_file: String,
    pub engine: Engine,
    pub auto_install_packages: bool,
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub file: Option<String>,
    pub line: Option<usize>,
    pub message: String,
    pub warning: bool,
}

pub struct CompileResult {
    pub generation: u64,
    pub pdf_path: Option<PathBuf>,
    pub preview: Option<crate::preview::Document>,
    pub diagnostics: Vec<Diagnostic>,
    pub log: String,
    pub elapsed: Duration,
}

impl CompileResult {
    fn error(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.log.push_str(&format!("\n{message}\n"));
        self.diagnostics.push(Diagnostic {
            file: None,
            line: None,
            message,
            warning: false,
        });
    }

    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.log.push_str(&format!("\n{message}\n"));
        self.diagnostics.push(Diagnostic {
            file: None,
            line: None,
            message,
            warning: true,
        });
    }
}

/// Build twice to resolve cross references, then read the PDF page metadata.
/// Page images are rendered separately when their viewport needs them.
pub fn compile(request: CompileRequest) -> CompileResult {
    let started = Instant::now();
    let mut result = CompileResult {
        generation: request.generation,
        pdf_path: None,
        preview: None,
        diagnostics: Vec::new(),
        log: format!("{} · {}\n", request.engine.label(), request.main_file),
        elapsed: Duration::ZERO,
    };
    if let Err(error) = compile_inner(&request, &mut result) {
        result.error(error);
    }
    result.elapsed = started.elapsed();
    result
        .log
        .push_str(&format!("\n완료: {:.2}초\n", result.elapsed.as_secs_f64()));
    result
}

fn compile_inner(request: &CompileRequest, result: &mut CompileResult) -> Result<(), String> {
    let project = request
        .project_dir
        .canonicalize()
        .map_err(|e| format!("프로젝트 폴더를 열 수 없습니다: {e}"))?;
    let relative = Path::new(&request.main_file);
    if relative
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
        || relative
            .extension()
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("tex"))
    {
        return Err("프로젝트 안의 .tex 파일을 컴파일 대상으로 선택하세요.".into());
    }
    let source = project.join(relative).canonicalize().map_err(|e| {
        format!(
            "컴파일할 파일을 찾을 수 없습니다: {} ({e})",
            request.main_file
        )
    })?;
    if !source.starts_with(&project) || !source.is_file() {
        return Err("컴파일 대상은 프로젝트 안의 파일이어야 합니다.".into());
    }
    let compiler = find_program(request.engine.executable()).ok_or_else(|| {
        format!(
            "{} 실행 파일을 찾을 수 없습니다. MiKTeX 또는 TeX Live를 설치하고 PATH에 추가하세요.",
            request.engine.label()
        )
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let build_dir = project
        .join(".runetex")
        .join("build")
        .join(format!("{}-{nonce}", request.generation));
    // MiKTeX writes verbatim Windows paths, but its auxiliary search does not
    // reliably read them back. Use the normal spelling for compiler arguments.
    #[cfg(windows)]
    let build_dir = {
        let name = build_dir.to_string_lossy();
        if let Some(unc) = name.strip_prefix("\\\\?\\UNC\\") {
            PathBuf::from(format!("\\\\{unc}"))
        } else {
            PathBuf::from(name.strip_prefix("\\\\?\\").unwrap_or(&name))
        }
    };
    fs::create_dir_all(&build_dir).map_err(|e| format!("빌드 폴더 생성 실패: {e}"))?;
    let jobname = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| "올바르지 않은 TeX 파일 이름입니다.".to_string())?;
    let pdf = build_dir.join(format!("{jobname}.pdf"));

    let installer = installer_option(&compiler, request.auto_install_packages);
    let timeout = if installer == Some("--enable-installer") {
        INSTALL_TIMEOUT
    } else {
        TEX_TIMEOUT
    };
    if let Some(option) = installer {
        result.log.push_str(&format!("MiKTeX: {option}\n"));
    }
    for pass in 1..=2 {
        result.log.push_str(&format!(
            "\n── {} 컴파일 {pass}/2 ──\n",
            request.engine.label()
        ));
        let mut command = Command::new(&compiler);
        command
            .current_dir(&project)
            .env(
                "TEXINPUTS",
                format!(
                    "{}{}{}",
                    build_dir.display(),
                    if cfg!(windows) { ';' } else { ':' },
                    env::var("TEXINPUTS").unwrap_or_default()
                ),
            )
            .env("max_print_line", "10000")
            .arg("-interaction=nonstopmode")
            .arg("-halt-on-error")
            .arg("-file-line-error")
            .arg("-no-shell-escape")
            .arg("-synctex=1")
            .arg(format!("-output-directory={}", build_dir.display()));
        if let Some(option) = installer {
            command
                .arg(option)
                .arg(format!("--aux-directory={}", build_dir.display()));
        }
        // A relative input also keeps TeX's file:line diagnostics readable: a
        // canonical Windows path can wrap in the middle of a filename. The
        // explicit ./ prevents a filename beginning with '-' being an option.
        command.arg(format!("./{}", request.main_file.replace('\\', "/")));
        let output = run_process(
            command,
            &build_dir.join(format!("pass-{pass}.txt")),
            timeout,
        )
        .map_err(|e| format!("{} 실행 실패: {e}", request.engine.label()))?;
        result.log.push_str(&output.log);
        // The last pass owns diagnostics: first-pass undefined references can
        // disappear after the second pass has read the generated .aux file.
        result.diagnostics = parse_diagnostics(&output.log, &project, &request.main_file);
        if output.timed_out {
            return Err(format!(
                "{} 컴파일이 {}초를 초과해 중단되었습니다.",
                request.engine.label(),
                timeout.as_secs()
            ));
        }
        if !output.status.success() {
            if result.diagnostics.iter().all(|d| d.warning) {
                return Err(format!(
                    "{} 컴파일 실패 (종료 코드 {:?}). 출력 패널을 확인하세요.",
                    request.engine.label(),
                    output.status.code()
                ));
            }
            return Ok(());
        }
    }
    if !pdf.is_file() {
        return Err("컴파일러가 PDF 파일을 생성하지 않았습니다. 출력 패널을 확인하세요.".into());
    }
    result.pdf_path = Some(pdf.clone());
    result.log.push_str(&format!("\nPDF: {}\n", pdf.display()));
    match crate::preview::open(&pdf) {
        Ok(document) => {
            result.log.push_str(&format!(
                "{}페이지 · 보이는 페이지부터 미리보기 생성\n",
                document.pages.len()
            ));
            result.preview = Some(document);
        }
        Err(error) => result.warn(format!(
            "PDF가 생성되었지만 미리보기를 준비하지 못했습니다: {error}"
        )),
    }
    Ok(())
}

fn installer_option(compiler: &Path, enabled: bool) -> Option<&'static str> {
    compiler
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains("miktex")
        .then_some(if enabled {
            "--enable-installer"
        } else {
            "--disable-installer"
        })
}

struct ProcessOutput {
    status: ExitStatus,
    log: String,
    timed_out: bool,
}

fn run_process(
    mut command: Command,
    capture: &Path,
    timeout: Duration,
) -> io::Result<ProcessOutput> {
    let log_file = File::create(capture)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file.try_clone()?))
        .stderr(Stdio::from(log_file));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn()?;
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill();
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(30));
    };
    let mut bytes = Vec::new();
    File::open(capture)?
        .take(MAX_LOG_BYTES)
        .read_to_end(&mut bytes)?;
    let mut log = String::from_utf8_lossy(&bytes).into_owned();
    if fs::metadata(capture)?.len() > MAX_LOG_BYTES {
        log.push_str("\n[출력이 4 MiB를 초과하여 표시를 줄였습니다.]\n");
    }
    Ok(ProcessOutput {
        status,
        log,
        timed_out,
    })
}

/// Discover standard TeX installations as well as ordinary PATH entries.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let mut directories: Vec<PathBuf> = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).collect())
        .unwrap_or_default();
    #[cfg(windows)]
    {
        if let Some(local) = env::var_os("LOCALAPPDATA") {
            directories.push(PathBuf::from(local).join("Programs/MiKTeX/miktex/bin/x64"));
        }
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = env::var_os(variable) {
                let base = PathBuf::from(base);
                directories.push(base.join("MiKTeX/miktex/bin/x64"));
                directories.push(base.join("poppler/Library/bin"));
            }
        }
    }
    for directory in directories {
        #[cfg(windows)]
        let candidates = [directory.join(format!("{name}.exe")), directory.join(name)];
        #[cfg(not(windows))]
        let candidates = [directory.join(name)];
        if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
            return crate::paths::absolute_file(path);
        }
    }
    None
}

fn parse_diagnostics(log: &str, project: &Path, main_file: &str) -> Vec<Diagnostic> {
    let lines: Vec<&str> = log.lines().collect();
    let mut diagnostics = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let line = line.trim();
        let diagnostic = if let Some((file, line_number, message)) = file_line_diagnostic(line) {
            Some(Diagnostic {
                file: Some(diagnostic_file(file, project)),
                line: Some(line_number),
                warning: message.contains("Warning")
                    || message.starts_with("Overfull")
                    || message.starts_with("Underfull"),
                message: message.to_string(),
            })
        } else if let Some(message) = line.strip_prefix("! ") {
            let line_number = lines.iter().skip(index + 1).take(8).find_map(|next| {
                next.trim_start()
                    .strip_prefix("l.")?
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            });
            Some(Diagnostic {
                file: Some(main_file.into()),
                line: line_number,
                message: message.into(),
                warning: false,
            })
        } else if line.contains("LaTeX Warning:")
            || (line.starts_with("Package ") && line.contains(" Warning:"))
            || line.starts_with("Overfull ")
            || line.starts_with("Underfull ")
        {
            let line_number = line
                .split("input line ")
                .nth(1)
                .and_then(|tail| tail.trim_end_matches('.').parse().ok());
            Some(Diagnostic {
                file: Some(main_file.into()),
                line: line_number,
                message: line.into(),
                warning: true,
            })
        } else {
            None
        };
        if let Some(diagnostic) = diagnostic {
            let duplicate = diagnostics.iter().any(|old: &Diagnostic| {
                old.file == diagnostic.file
                    && old.line == diagnostic.line
                    && old.message == diagnostic.message
            });
            if !duplicate {
                diagnostics.push(diagnostic);
            }
            if diagnostics.len() >= 100 {
                break;
            }
        }
    }
    diagnostics
}

fn diagnostic_file(file: &str, project: &Path) -> String {
    // TeX commonly reports C:/... while canonicalize() yields \\?\C:\...
    // on Windows. Normalize both so clicking a diagnostic opens the project tab.
    let normalize = |value: &str| {
        value
            .replace('\\', "/")
            .trim_start_matches("//?/")
            .trim_start_matches("./")
            .to_string()
    };
    let file = normalize(file);
    let root = normalize(&project.to_string_lossy());
    let root = root.trim_end_matches('/');
    let prefix = format!("{root}/");
    let matches = if cfg!(windows) {
        file.get(..prefix.len())
            .is_some_and(|part| part.eq_ignore_ascii_case(&prefix))
    } else {
        file.starts_with(&prefix)
    };
    if matches {
        file[prefix.len()..].to_string()
    } else {
        file
    }
}

fn file_line_diagnostic(line: &str) -> Option<(&str, usize, &str)> {
    for (position, _) in line.match_indices(':') {
        let tail = &line[position + 1..];
        let Some((number, message)) = tail.split_once(':') else {
            continue;
        };
        let Ok(number) = number.parse::<usize>() else {
            continue;
        };
        if number > 0 && !message.trim().is_empty() {
            return Some((&line[..position], number, message.trim()));
        }
    }
    None
}
