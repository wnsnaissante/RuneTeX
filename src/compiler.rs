//! TeX compilation and PDF rasterization. Call `compile` on a worker thread.
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
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_PREVIEW_PAGES: usize = 30;
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PAGE_BYTES: u64 = 16 * 1024 * 1024;

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

#[derive(Debug)]
pub struct PreviewPage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct CompileResult {
    pub generation: u64,
    pub pdf_path: Option<PathBuf>,
    pub pages: Vec<PreviewPage>,
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

/// Build twice to resolve cross references, then rasterize the actual PDF.
/// A missing rasterizer preserves the successful PDF and reports a warning.
pub fn compile(request: CompileRequest) -> CompileResult {
    let started = Instant::now();
    let mut result = CompileResult {
        generation: request.generation,
        pdf_path: None,
        pages: Vec::new(),
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
    render_pdf(&pdf, &build_dir, result);
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

fn render_pdf(pdf: &Path, build_dir: &Path, result: &mut CompileResult) {
    let renderer = crate::paths::program_override("RUNETEX_PDFTOPPM")
        .or_else(|| crate::paths::bundled_program("pdftoppm"))
        .or_else(|| find_program("pdftoppm"));
    let Some(renderer) = renderer else {
        result.warn("PDF가 생성되었습니다. 미리보기를 사용하려면 Poppler의 pdftoppm을 PATH에 추가하거나 RUNETEX_PDFTOPPM을 지정하세요.");
        return;
    };
    let mut command = Command::new(renderer);
    command
        .current_dir(build_dir)
        .arg("-png")
        .arg("-scale-to")
        .arg("1400")
        .arg("-f")
        .arg("1")
        .arg("-l")
        .arg(MAX_PREVIEW_PAGES.to_string())
        .arg(pdf)
        .arg(build_dir.join("preview"));
    let output = match run_process(command, &build_dir.join("preview.txt"), PREVIEW_TIMEOUT) {
        Ok(output) => output,
        Err(error) => {
            result.warn(format!(
                "PDF가 생성되었지만 미리보기 실행에 실패했습니다: {error}"
            ));
            return;
        }
    };
    result.log.push_str("\n── PDF 미리보기 ──\n");
    result.log.push_str(&output.log);
    if output.timed_out || !output.status.success() {
        result.warn(if output.timed_out {
            "PDF 미리보기 변환이 60초를 초과해 중단되었습니다. PDF 파일은 사용할 수 있습니다."
        } else {
            "PDF가 생성되었지만 미리보기 변환에 실패했습니다. 출력 패널을 확인하세요."
        });
        return;
    }
    let mut files: Vec<(usize, PathBuf)> = fs::read_dir(build_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?;
            let page = stem.strip_prefix("preview-")?.parse::<usize>().ok()?;
            (path.extension()? == "png").then_some((page, path))
        })
        .collect();
    files.sort_by_key(|(page, _)| *page);
    for (_, path) in files.into_iter().take(MAX_PREVIEW_PAGES) {
        match load_preview_page(&path) {
            Ok(page) => result.pages.push(page),
            Err(error) => {
                result.warn(format!("PDF 페이지를 읽을 수 없습니다: {error}"));
                break;
            }
        }
    }
    if result.pages.is_empty() {
        result.warn("PDF 미리보기 페이지가 생성되지 않았습니다. PDF 파일을 직접 열어 확인하세요.");
    } else {
        result
            .log
            .push_str(&format!("{}페이지 미리보기 생성\n", result.pages.len()));
        if result.pages.len() == MAX_PREVIEW_PAGES {
            result.warn(
                "미리보기에는 처음 30페이지까지 표시됩니다. 전체 문서는 PDF 파일에서 확인하세요.",
            );
        }
    }
}

fn load_preview_page(path: &Path) -> io::Result<PreviewPage> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_PAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "미리보기 이미지가 너무 큽니다.",
        ));
    }
    let mut png = Vec::new();
    file.take(MAX_PAGE_BYTES).read_to_end(&mut png)?;
    let (width, height) = png_dimensions(&png)?;
    Ok(PreviewPage { png, width, height })
}

fn png_dimensions(bytes: &[u8]) -> io::Result<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PNG 헤더가 올바르지 않습니다.",
        ));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PNG 크기가 올바르지 않습니다.",
        ));
    }
    Ok((width, height))
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
