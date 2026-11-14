//! Headless setup and the temporary-copy lifecycle of the uninstaller.
use std::{
    env, fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, OnceLock, atomic::AtomicBool},
};

use super::{
    installer::{self, InstallOptions},
    payload::Payload,
};

static UNINSTALL_ROOT: OnceLock<PathBuf> = OnceLock::new();

#[derive(Default)]
struct Arguments {
    package_info: bool,
    silent: bool,
    uninstall: bool,
    root: Option<PathBuf>,
    destination: Option<PathBuf>,
    no_shortcuts: bool,
    desktop: bool,
    preview: bool,
    help: bool,
}

/// Returns true when a command completed, or was handed off to a temporary copy.
/// A silent installed uninstaller also returns immediately after that handoff:
/// keeping its executable loaded would prevent its removal by the child process.
pub fn run() -> Result<bool, String> {
    let executable = env::current_exe().map_err(|e| format!("실행 파일 경로 확인 실패: {e}"))?;
    arm_temporary_cleanup(&executable)?;
    let arguments = parse()?;
    if arguments.help {
        println!(
            "RuneTeX setup\n  --package-info\n  --silent --destination <absolute path> [--no-shortcuts | --desktop]\n  --uninstall [absolute path] [--silent]\n  --preview [--screen configure|progress|complete]"
        );
        return Ok(true);
    }
    if arguments.package_info {
        let mut payload = Payload::open_current()?;
        payload.verify()?;
        let bytes = payload
            .manifest
            .files
            .iter()
            .try_fold(0_u64, |total, file| total.checked_add(file.size))
            .ok_or("설치 파일 크기가 올바르지 않습니다")?;
        println!(
            "{}",
            serde_json::json!({
                "version": payload.manifest.version,
                "files": payload.manifest.files.len(),
                "bytes": bytes,
            })
        );
        return Ok(true);
    }
    if arguments.uninstall {
        let root = match arguments.root {
            Some(root) => root,
            None => default_uninstall_root(&executable)?,
        };
        let root = validated_install_root(&root)?;
        let _ = UNINSTALL_ROOT.set(root.clone());
        let executable = executable.canonicalize().map_err(|e| e.to_string())?;
        if within(&executable, &root) {
            bootstrap_uninstall(&executable, &root, arguments.silent)?;
            return Ok(true);
        }
        if arguments.silent {
            installer::uninstall(&root)?;
            println!("RuneTeX 제거 완료");
            return Ok(true);
        }
        return Ok(false);
    }
    if arguments.silent {
        let destination = arguments
            .destination
            .ok_or("--silent 설치에는 --destination 전체 경로가 필요합니다")?;
        if !destination.is_absolute() {
            return Err("--destination에는 폴더의 전체 경로를 입력하세요".into());
        }
        installer::install(
            InstallOptions {
                destination,
                start_menu: !arguments.no_shortcuts,
                desktop_shortcut: arguments.desktop,
            },
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )?;
        println!("RuneTeX 설치 완료");
        return Ok(true);
    }
    Ok(false)
}

/// The normalized destination of the UI uninstaller, resolved before UI startup.
pub fn uninstall_root() -> Option<PathBuf> {
    UNINSTALL_ROOT.get().cloned()
}

fn parse() -> Result<Arguments, String> {
    let mut arguments = Arguments::default();
    let raw: Vec<_> = env::args_os().skip(1).collect();
    if raw.iter().any(|value| value == "--help" || value == "-h") {
        arguments.help = true;
        return Ok(arguments);
    }
    // Rune owns rendering flags. Setup owns --desktop (a boolean), which Rune
    // also uses for a backdrop path, so headless setup bypasses Rune's parser.
    let setup_command = raw.iter().any(|value| {
        [
            "--silent",
            "--uninstall",
            "--package-info",
            "--destination",
            "--no-shortcuts",
            "--desktop",
        ]
        .iter()
        .any(|flag| value == *flag)
    });
    let input = if setup_command {
        raw
    } else {
        rune::extra_args()
            .iter()
            .map(std::ffi::OsString::from)
            .collect()
    };
    let mut input = input.into_iter().peekable();
    while let Some(argument) = input.next() {
        match argument.to_str() {
            Some("--package-info") => arguments.package_info = true,
            Some("--silent") => arguments.silent = true,
            Some("--no-shortcuts") => arguments.no_shortcuts = true,
            Some("--desktop") => arguments.desktop = true,
            Some("--help" | "-h") => arguments.help = true,
            Some("--destination") => {
                if arguments.destination.is_some() {
                    return Err("--destination은 한 번만 지정하세요".into());
                }
                arguments.destination = Some(PathBuf::from(
                    input
                        .next()
                        .ok_or("--destination 뒤에 전체 경로를 입력하세요")?,
                ));
            }
            Some("--uninstall") => {
                if arguments.uninstall {
                    return Err("--uninstall은 한 번만 지정하세요".into());
                }
                arguments.uninstall = true;
                if input
                    .peek()
                    .is_some_and(|value| !value.to_string_lossy().starts_with('-'))
                {
                    arguments.root = input.next().map(PathBuf::from);
                }
            }
            Some("--preview") => arguments.preview = true,
            Some("--screen") => {
                let screen = input.next().ok_or("--screen 뒤에 화면 이름을 입력하세요")?;
                if !matches!(screen.to_str(), Some("configure" | "progress" | "complete")) {
                    return Err(
                        "--screen: configure, progress, complete 중 하나를 입력하세요".into(),
                    );
                }
                arguments.preview = true;
            }
            Some(value) if value.starts_with("--screen=") => {
                if !matches!(
                    value.trim_start_matches("--screen="),
                    "configure" | "progress" | "complete"
                ) {
                    return Err(
                        "--screen: configure, progress, complete 중 하나를 입력하세요".into(),
                    );
                }
                arguments.preview = true;
            }
            _ => return Err(format!("알 수 없는 옵션: {}", argument.to_string_lossy())),
        }
    }
    let install_options =
        arguments.destination.is_some() || arguments.no_shortcuts || arguments.desktop;
    if arguments.package_info
        && (arguments.silent || arguments.uninstall || install_options || arguments.preview)
        || arguments.uninstall && (install_options || arguments.preview)
        || arguments.preview && (arguments.silent || install_options)
        || install_options && !arguments.silent
        || arguments.no_shortcuts && arguments.desktop
    {
        return Err("함께 사용할 수 없는 설치 옵션입니다. --help에서 사용법을 확인하세요".into());
    }
    Ok(arguments)
}

fn default_uninstall_root(executable: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = executable.parent()
        && parent.join(".runetex-install.json").exists()
    {
        return Ok(parent.to_path_buf());
    }
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("Programs/RuneTeX"))
        .ok_or_else(|| "제거할 RuneTeX 설치 폴더의 전체 경로를 입력하세요".to_string())
}

fn validated_install_root(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
        || path
            .as_os_str()
            .to_string_lossy()
            .chars()
            .any(char::is_control)
    {
        return Err("제거할 RuneTeX 폴더의 전체 경로를 입력하세요".into());
    }
    reject_reparse(path)?;
    let root = path
        .canonicalize()
        .map_err(|e| format!("설치 폴더 확인 실패: {e}"))?;
    if !root.is_dir() || root.file_name().is_none() {
        return Err("RuneTeX 전용 설치 폴더를 선택하세요".into());
    }
    let marker = root.join(".runetex-install.json");
    reject_reparse(&marker)?;
    let metadata = marker
        .metadata()
        .map_err(|e| format!("설치 정보 확인 실패: {e}"))?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err("RuneTeX 설치 정보가 올바르지 않습니다".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(&marker).map_err(|e| e.to_string())?)
            .map_err(|e| format!("설치 정보 읽기 실패: {e}"))?;
    if value["app_id"].as_str() != Some("RuneTeX")
        || !value["install_dir"]
            .as_str()
            .is_some_and(|value| same_path(Path::new(value), &root))
    {
        return Err("이 폴더의 RuneTeX 설치 소유권을 확인할 수 없습니다".into());
    }
    Ok(root)
}

fn path_key(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let path = path
        .strip_prefix("//?/")
        .unwrap_or(&path)
        .trim_end_matches('/');
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_string()
    }
}
fn same_path(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}
fn within(path: &Path, root: &Path) -> bool {
    let path = path_key(path);
    let root = path_key(root);
    path == root || path.starts_with(&(root + "/"))
}
fn reject_reparse(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        let metadata =
            fs::symlink_metadata(ancestor).map_err(|e| format!("경로 확인 실패: {e}"))?;
        #[cfg(windows)]
        let reparse = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let reparse = metadata.file_type().is_symlink();
        if reparse {
            return Err("링크 또는 리디렉션된 설치 경로는 제거할 수 없습니다".into());
        }
    }
    Ok(())
}

#[cfg(windows)]
const CLEANUP_SCRIPT: &str = r#"param([Parameter(Mandatory=$true)][int]$WaitProcessId)
$ErrorActionPreference = 'Stop'
$folder = [IO.Path]::GetFullPath($PSScriptRoot)
if ([IO.Path]::GetFileName($folder) -notmatch '^RuneTeX-uninstall-[0-9]+-[0-9a-f]{32}$') { exit 2 }
function Get-ComparisonPath([string]$value) {
    if ($value.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
        $value = '\\' + $value.Substring(8)
    } elseif ($value.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) {
        $value = $value.Substring(4)
    }
    return [IO.Path]::GetFullPath($value).TrimEnd([char[]]@('\', '/'))
}
$tempRoot = Get-ComparisonPath ([IO.Path]::GetTempPath())
function Test-CleanupLocation {
    $parent = [IO.Directory]::GetParent($folder)
    if ($null -eq $parent -or ![String]::Equals((Get-ComparisonPath $parent.FullName), $tempRoot, [StringComparison]::OrdinalIgnoreCase)) { return $false }
    $ancestor = Get-Item -LiteralPath $folder -Force -ErrorAction SilentlyContinue
    if ($null -eq $ancestor -or !$ancestor.PSIsContainer) { return $false }
    while ($null -ne $ancestor) {
        if (($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { return $false }
        $ancestor = $ancestor.Parent
    }
    return $true
}
if (!(Test-CleanupLocation)) { exit 3 }
$process = Get-Process -Id $WaitProcessId -ErrorAction SilentlyContinue
if ($null -ne $process) { $process.WaitForExit() }
if (!(Test-CleanupLocation)) { exit 3 }
foreach ($name in @('uninstall.exe', 'cleanup.ps1')) {
    $target = [IO.Path]::Combine($folder, $name)
    $item = Get-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue
    if ($null -ne $item -and !$item.PSIsContainer -and (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0)) {
        Remove-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue
    }
}
try { [IO.Directory]::Delete($folder, $false) } catch { }
"#;

#[cfg(windows)]
fn temporary_parent(executable: &Path) -> Result<Option<PathBuf>, String> {
    let Some(parent) = executable.parent() else {
        return Ok(None);
    };
    if executable
        .file_name()
        .is_none_or(|name| name != "uninstall.exe")
    {
        return Ok(None);
    }
    let name = parent
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let Some(suffix) = name.strip_prefix("RuneTeX-uninstall-") else {
        return Ok(None);
    };
    let Some((pid, nonce)) = suffix.split_once('-') else {
        return Ok(None);
    };
    if pid.is_empty()
        || !pid.bytes().all(|b| b.is_ascii_digit())
        || nonce.len() != 32
        || !nonce.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Ok(None);
    }
    let temp = env::temp_dir().canonicalize().map_err(|e| e.to_string())?;
    let parent = parent.canonicalize().map_err(|e| e.to_string())?;
    if !parent.parent().is_some_and(|base| same_path(base, &temp)) {
        return Ok(None);
    }
    reject_reparse(&parent)?;
    Ok(Some(parent))
}

#[cfg(windows)]
fn arm_temporary_cleanup(executable: &Path) -> Result<(), String> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let Some(directory) = temporary_parent(executable)? else {
        return Ok(());
    };
    let script = directory.join("cleanup.ps1");
    reject_reparse(&script)?;
    if fs::read(&script).map_err(|e| format!("임시 파일 정리 스크립트 확인 실패: {e}"))?
        != CLEANUP_SCRIPT.as_bytes()
    {
        return Err("임시 파일 정리 스크립트가 변경되어 실행하지 않았습니다".into());
    }
    let powershell = env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .ok_or("Windows 폴더를 확인할 수 없습니다")?
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("-WaitProcessId")
        .arg(std::process::id().to_string())
        .current_dir(
            directory
                .parent()
                .ok_or("임시 폴더 경로가 올바르지 않습니다")?,
        )
        .creation_flags(0x0800_0000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("임시 제거 프로그램 정리 예약 실패: {e}"))?;
    Ok(())
}

#[cfg(not(windows))]
fn arm_temporary_cleanup(_: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn bootstrap_uninstall(executable: &Path, root: &Path, silent: bool) -> Result<(), String> {
    use std::{
        fs::OpenOptions,
        io::Write,
        os::windows::process::CommandExt,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };
    let temp = env::temp_dir()
        .canonicalize()
        .map_err(|e| format!("임시 폴더 확인 실패: {e}"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let mut directory = None;
    for attempt in 0..32_u128 {
        let candidate = temp.join(format!(
            "RuneTeX-uninstall-{}-{:032x}",
            std::process::id(),
            nonce.wrapping_add(attempt)
        ));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                directory = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("제거 프로그램 임시 폴더 생성 실패: {error}")),
        }
    }
    let directory = directory.ok_or("고유한 제거 프로그램 임시 폴더를 만들지 못했습니다")?;
    let copy = directory.join("uninstall.exe");
    let script = directory.join("cleanup.ps1");
    let result = (|| {
        let mut source = fs::File::open(executable).map_err(|e| e.to_string())?;
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&copy)
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut source, &mut target)
            .map_err(|e| format!("제거 프로그램 복사 실패: {e}"))?;
        target.sync_all().map_err(|e| e.to_string())?;
        drop(target);
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script)
            .map_err(|e| e.to_string())?;
        target
            .write_all(CLEANUP_SCRIPT.as_bytes())
            .map_err(|e| e.to_string())?;
        target.sync_all().map_err(|e| e.to_string())?;
        drop(target);
        let mut command = Command::new(&copy);
        command.arg("--uninstall").arg(root).current_dir(&directory);
        if silent {
            command.arg("--silent").creation_flags(0x0800_0000);
        }
        let child = command
            .spawn()
            .map_err(|e| format!("임시 제거 프로그램 실행 실패: {e}"))?;
        if silent {
            println!("RuneTeX 제거 프로그램 시작: PID {}", child.id());
        }
        // Do not wait: this installed executable must unload before removal.
        Ok(())
    })();
    if result.is_err() {
        for path in [&copy, &script] {
            if reject_reparse(path).is_ok() {
                let _ = fs::remove_file(path);
            }
        }
        let _ = fs::remove_dir(&directory);
    }
    result
}

#[cfg(not(windows))]
fn bootstrap_uninstall(_: &Path, _: &Path, _: bool) -> Result<(), String> {
    Err("제거 프로그램은 Windows에서 사용할 수 있습니다".into())
}
