//! Install only a validated payload; keep rollback data until registration succeeds.
use std::{
    cell::Cell,
    collections::HashSet,
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{payload::Payload, platform};

const APP_ID: &str = "RuneTeX";
const MANIFEST: &str = ".runetex-install.json";
const UNINSTALLER: &str = "uninstall.exe";

pub struct InstallOptions {
    pub destination: PathBuf,
    pub start_menu: bool,
    pub desktop_shortcut: bool,
}

pub enum InstallEvent {
    Progress {
        percent: f64,
        phase: usize,
        current_file: String,
    },
    Finished(Result<(), String>),
}

#[derive(Clone)]
struct OwnedFile {
    path: String,
    sha256: String,
}

struct Installed {
    version: String,
    files: Vec<OwnedFile>,
    shortcuts: Vec<PathBuf>,
    start_menu: bool,
    desktop_shortcut: bool,
}

/// Reports progress only. The caller forwards the returned result as Finished.
pub fn install(
    options: InstallOptions,
    cancel: Arc<AtomicBool>,
    report: impl Fn(InstallEvent),
) -> Result<(), String> {
    let _lock = OperationLock::acquire()?;
    let root = destination(&options.destination)?;
    let old = existing_install(&root)?;
    let current_exe =
        env::current_exe().map_err(|e| format!("설치 프로그램 경로 확인 실패: {e}"))?;
    if within(&current_exe, &root) {
        return Err("설치 프로그램을 설치할 폴더 밖으로 옮긴 뒤 다시 실행하세요".into());
    }
    report(progress(0.0, 0, "설치 파일 확인"));
    let mut payload = Payload::open_current()?;
    let version = payload.manifest.version.clone();
    let files = payload.manifest.files.clone();
    let mut names = HashSet::new();
    for file in &files {
        relative_path(&file.path)?;
        valid_hash(&file.sha256)?;
        let key = file.path.to_lowercase();
        if key == MANIFEST || key == UNINSTALLER || !names.insert(key) {
            return Err(format!("중복되었거나 예약된 설치 파일: {}", file.path));
        }
        validate_collision(&root, &file.path, old.as_ref())?;
    }
    if !names.contains("runetex.exe") {
        return Err("설치 파일에 RuneTeX 실행 파일이 없습니다".into());
    }
    validate_collision(&root, UNINSTALLER, old.as_ref())?;
    check_cancel(&cancel)?;

    let copied = Cell::new(0_u64);
    let previous_percent = Cell::new(0_u64);
    let own_size = current_exe.metadata().map_err(|e| e.to_string())?.len();
    let total = files
        .iter()
        .try_fold(own_size, |total, file| total.checked_add(file.size))
        .ok_or("설치 파일 크기가 올바르지 않습니다")?
        .max(1);
    let registration = platform::snapshot_install(&root)?;
    let mut transaction = Transaction::new(&root)?;
    let copying = |bytes: u64, name: &str| {
        copied.set(copied.get().saturating_add(bytes));
        let percent = 5.0 + 85.0 * copied.get().min(total) as f64 / total as f64;
        let integer = percent as u64;
        if integer != previous_percent.get() {
            previous_percent.set(integer);
            report(progress(percent, 1, name));
        }
    };
    let mut registration_changed = false;
    let result = catch_operation(|| {
        report(progress(3.0, 0, "설치 폴더 준비"));
        let mut owned = Vec::with_capacity(files.len() + 1);
        for file in &files {
            check_cancel(&cancel)?;
            let staged = transaction.staged.join(relative_path(&file.path)?);
            fs::create_dir_all(
                staged
                    .parent()
                    .ok_or("설치 파일 경로가 올바르지 않습니다")?,
            )
            .map_err(|e| format!("설치 임시 폴더 생성 실패: {e}"))?;
            payload.copy_file(file, &staged, &cancel, |bytes| copying(bytes, &file.path))?;
            owned.push(OwnedFile {
                path: file.path.clone(),
                sha256: file.sha256.clone(),
            });
        }
        let own_hash = copy_self(
            &current_exe,
            &transaction.staged.join(UNINSTALLER),
            &cancel,
            |bytes| copying(bytes, UNINSTALLER),
        )?;
        owned.push(OwnedFile {
            path: UNINSTALLER.into(),
            sha256: own_hash,
        });
        check_cancel(&cancel)?;
        transaction.prepare_root()?;
        if let Some(old) = &old {
            for file in &old.files {
                check_cancel(&cancel)?;
                let path = root.join(relative_path(&file.path)?);
                reject_reparse(&path)?;
                if path.is_file() && hash_file(&path)? == file.sha256.to_lowercase() {
                    transaction.back_up(&file.path)?;
                }
            }
            transaction.back_up(MANIFEST)?;
        }
        for file in &owned {
            check_cancel(&cancel)?;
            transaction.put(&file.path, &file.sha256)?;
        }
        check_cancel(&cancel)?;
        report(progress(92.0, 2, "바로가기와 앱 정보 등록"));
        registration_changed = true;
        let shortcuts = platform::register_install(
            &root,
            &version,
            options.start_menu,
            options.desktop_shortcut,
        )?;
        if let Some(old) = &old {
            let obsolete: Vec<PathBuf> = old
                .shortcuts
                .iter()
                .filter(|path| !shortcuts.iter().any(|new| same_path(new, path)))
                .cloned()
                .collect();
            platform::remove_shortcuts(&root, &obsolete)?;
        }
        check_cancel(&cancel)?;
        let installed = Installed {
            version,
            files: owned,
            shortcuts,
            start_menu: options.start_menu,
            desktop_shortcut: options.desktop_shortcut,
        };
        let value = installed.to_json(&root);
        let manifest = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
        let manifest_hash = format!("{:x}", Sha256::digest(&manifest));
        fs::write(transaction.staged.join(MANIFEST), manifest)
            .map_err(|e| format!("설치 정보 저장 실패: {e}"))?;
        transaction.put(MANIFEST, &manifest_hash)?;
        Ok(())
    });
    if let Err(error) = result {
        let mut errors = transaction.rollback();
        if registration_changed && let Err(error) = platform::restore_install(&root, &registration)
        {
            errors.push(error);
        }
        if errors.is_empty() {
            transaction.cleanup();
            return Err(error);
        }
        return Err(format!(
            "{error}\n복구를 마치지 못했습니다. 복구 파일: {}\n{}",
            transaction.work.display(),
            errors.join("\n")
        ));
    }
    transaction.cleanup();
    // Completion reporting cannot turn a committed installation into a failed one.
    let _ = catch_unwind(AssertUnwindSafe(|| report(progress(100.0, 3, "설치 완료"))));
    Ok(())
}

/// Call from a temporary copy of uninstall.exe so every installed file can close.
/// Unknown files and user-modified installed files are left untouched.
pub fn uninstall(path: &Path) -> Result<(), String> {
    uninstall_with_progress(path, |_| {})
}

pub fn uninstall_with_progress(path: &Path, report: impl Fn(InstallEvent)) -> Result<(), String> {
    let _lock = OperationLock::acquire()?;
    report(progress(0.0, 0, "설치 정보 확인"));
    let root = destination(path)?;
    let installed = read_install(&root)?.ok_or("RuneTeX 설치 정보를 찾을 수 없습니다")?;
    if env::current_exe().is_ok_and(|exe| within(&exe, &root)) {
        return Err("제거 프로그램은 설치 폴더 밖의 임시 복사본에서 실행해야 합니다".into());
    }
    let registration = platform::snapshot_install(&root)?;
    let mut transaction = Transaction::new(&root)?;
    let mut registration_changed = false;
    let result = catch_operation(|| {
        for (index, file) in installed.files.iter().enumerate() {
            let path = root.join(relative_path(&file.path)?);
            reject_reparse(&path)?;
            if path.is_file() && hash_file(&path)? == file.sha256.to_lowercase() {
                transaction.back_up(&file.path)?;
            }
            report(progress(
                5.0 + 85.0 * (index + 1) as f64 / installed.files.len().max(1) as f64,
                1,
                &file.path,
            ));
        }
        transaction.back_up(MANIFEST)?;
        report(progress(92.0, 2, "바로가기와 앱 정보 제거"));
        registration_changed = true;
        platform::remove_shortcuts(&root, &installed.shortcuts)?;
        platform::unregister_install(&root)?;
        Ok(())
    });
    if let Err(error) = result {
        let mut errors = transaction.rollback();
        if registration_changed && let Err(error) = platform::restore_install(&root, &registration)
        {
            errors.push(error);
        }
        if errors.is_empty() {
            transaction.cleanup();
            return Err(error);
        }
        return Err(format!(
            "{error}\n복구 파일: {}\n{}",
            transaction.work.display(),
            errors.join("\n")
        ));
    }
    transaction.cleanup();
    remove_owned_empty_directories(&root, &installed.files);
    let _ = catch_unwind(AssertUnwindSafe(|| report(progress(100.0, 3, "제거 완료"))));
    Ok(())
}

fn catch_operation(operation: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    catch_unwind(AssertUnwindSafe(operation))
        .unwrap_or_else(|_| Err("설치 작업 중 예기치 않은 오류가 발생했습니다".into()))
}

struct OperationLock {
    _file: File,
}

impl OperationLock {
    fn acquire() -> Result<Self, String> {
        let local = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or("Windows 로컬 앱 데이터 폴더를 확인할 수 없습니다")?;
        let directory = local.join("RuneTeX");
        let path = directory.join("setup.lock");
        reject_reparse(&path)?;
        fs::create_dir_all(&directory).map_err(|e| format!("설치 잠금 폴더 생성 실패: {e}"))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("설치 잠금 파일 열기 실패: {e}"))?;
        reject_reparse(&path)?;
        file.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => {
                "다른 RuneTeX 설치 또는 제거 작업이 진행 중입니다. 해당 작업이 끝난 뒤 다시 시도하세요".into()
            }
            fs::TryLockError::Error(error) => format!("설치 작업 잠금 실패: {error}"),
        })?;
        // Keep the zero-byte file: unlinking it permits another process to lock
        // a different file at the same path while this handle remains locked.
        Ok(Self { _file: file })
    }
}

fn progress(percent: f64, phase: usize, file: &str) -> InstallEvent {
    InstallEvent::Progress {
        percent,
        phase,
        current_file: file.into(),
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("설치를 취소했습니다".into())
    } else {
        Ok(())
    }
}

fn copy_self(
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
    report: impl Fn(u64),
) -> Result<String, String> {
    let mut input = File::open(source).map_err(|e| e.to_string())?;
    let mut output = File::create(destination).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        check_cancel(cancel)?;
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        digest.update(&buffer[..count]);
        report(count as u64);
    }
    output.sync_all().map_err(|e| e.to_string())?;
    Ok(format!("{:x}", digest.finalize()))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|e| format!("파일 확인 실패 ({}): {e}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn valid_hash(hash: &str) -> Result<(), String> {
    if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("설치 파일의 SHA-256 정보가 올바르지 않습니다".into())
    }
}

fn relative_path(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("설치 파일 경로가 올바르지 않습니다: {value}"));
    }
    for part in value.split('/') {
        let name = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if part.is_empty()
            || part.ends_with([' ', '.'])
            || part
                .chars()
                .any(|c| c.is_control() || "<>:\"|?*".contains(c))
            || ["CON", "PRN", "AUX", "NUL"].contains(&name.as_str())
            || (name.len() == 4
                && (name.starts_with("COM") || name.starts_with("LPT"))
                && matches!(name.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(format!("설치 파일 이름이 올바르지 않습니다: {value}"));
        }
    }
    Ok(path.to_path_buf())
}

fn destination(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err("설치할 폴더의 전체 경로를 입력하세요".into());
    }
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
    {
        return Err("로컬 드라이브의 설치 폴더를 선택하세요".into());
    }
    for component in path.components() {
        if let Component::Normal(name) = component {
            relative_path(&name.to_string_lossy())?;
        }
    }
    let root = std::path::absolute(path).map_err(|e| e.to_string())?;
    if root.parent().is_none() || root.file_name().is_none() {
        return Err("드라이브 루트에는 설치할 수 없습니다".into());
    }
    for variable in [
        "USERPROFILE",
        "HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "ProgramData",
        "WINDIR",
        "ProgramFiles",
        "ProgramFiles(x86)",
    ] {
        if let Some(protected) = env::var_os(variable)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            && (same_path(&root, &protected)
                || within(&protected, &root)
                || (["WINDIR", "ProgramFiles", "ProgramFiles(x86)"].contains(&variable)
                    && within(&root, &protected)))
        {
            return Err("사용자 폴더나 시스템 폴더 자체에는 설치할 수 없습니다. RuneTeX 전용 폴더를 선택하세요".into());
        }
    }
    reject_reparse(&root)?;
    Ok(root)
}

fn same_path(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

fn within(path: &Path, root: &Path) -> bool {
    let path = path_key(path);
    let root = path_key(root);
    path == root || path.starts_with(&(root + "/"))
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

fn reject_reparse(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let reparse = metadata.file_type().is_symlink();
                if reparse {
                    return Err(format!(
                        "링크 또는 리디렉션된 폴더는 사용할 수 없습니다: {}",
                        ancestor.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("폴더 확인 실패 ({}): {error}", ancestor.display())),
        }
    }
    Ok(())
}

fn existing_install(root: &Path) -> Result<Option<Installed>, String> {
    if !root.exists() {
        return Ok(None);
    }
    if !root.is_dir() {
        return Err("설치 위치가 폴더가 아닙니다".into());
    }
    let installed = read_install(root)?;
    if installed.is_none()
        && fs::read_dir(root)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        return Err(
            "선택한 폴더에 다른 파일이 있습니다. 비어 있는 RuneTeX 전용 폴더를 선택하세요".into(),
        );
    }
    Ok(installed)
}

fn validate_collision(root: &Path, relative: &str, old: Option<&Installed>) -> Result<(), String> {
    let path = root.join(relative_path(relative)?);
    reject_reparse(&path)?;
    if !path.exists() {
        return Ok(());
    }
    if !path.is_file() {
        return Err(format!("설치할 파일 위치에 폴더가 있습니다: {relative}"));
    }
    if let Some(file) = old.and_then(|old| {
        old.files
            .iter()
            .find(|file| file.path.eq_ignore_ascii_case(relative))
    }) && hash_file(&path)? == file.sha256.to_lowercase()
    {
        return Ok(());
    }
    Err(format!(
        "기존 파일을 덮어쓸 수 없습니다: {relative}. 변경된 파일을 다른 곳에 보관한 뒤 다시 시도하세요"
    ))
}

fn read_install(root: &Path) -> Result<Option<Installed>, String> {
    let path = root.join(MANIFEST);
    reject_reparse(&path)?;
    if !path.exists() {
        return Ok(None);
    }
    let metadata = path.metadata().map_err(|e| e.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("설치 정보 파일이 너무 큽니다".into());
    }
    let value: Value = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("설치 정보를 읽을 수 없습니다: {e}"))?;
    if value["app_id"].as_str() != Some(APP_ID)
        || !value["install_dir"]
            .as_str()
            .is_some_and(|path| same_path(Path::new(path), root))
    {
        return Err("이 폴더의 RuneTeX 설치 정보를 확인할 수 없습니다".into());
    }
    let version = value["version"]
        .as_str()
        .ok_or("설치 버전 정보가 없습니다")?
        .to_string();
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for entry in value["files"]
        .as_array()
        .ok_or("설치 파일 목록이 없습니다")?
    {
        let path = entry["path"]
            .as_str()
            .ok_or("설치 파일 경로가 없습니다")?
            .to_string();
        relative_path(&path)?;
        if path.eq_ignore_ascii_case(MANIFEST) || !seen.insert(path.to_lowercase()) {
            return Err("설치 파일 목록이 올바르지 않습니다".into());
        }
        let sha256 = entry["sha256"]
            .as_str()
            .ok_or("설치 파일 확인 정보가 없습니다")?
            .to_string();
        valid_hash(&sha256)?;
        files.push(OwnedFile { path, sha256 });
    }
    let shortcuts = value["shortcuts"]
        .as_array()
        .ok_or("바로가기 정보가 없습니다")?
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(PathBuf::from)
                .ok_or_else(|| "바로가기 경로가 올바르지 않습니다".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(Installed {
        version,
        files,
        shortcuts,
        start_menu: value["start_menu"].as_bool().unwrap_or(false),
        desktop_shortcut: value["desktop_shortcut"].as_bool().unwrap_or(false),
    }))
}

impl Installed {
    fn to_json(&self, root: &Path) -> Value {
        json!({"app_id":APP_ID,"install_dir":root,"version":self.version,
            "files":self.files.iter().map(|file| json!({"path":file.path,"sha256":file.sha256})).collect::<Vec<_>>(),
            "shortcuts":self.shortcuts,"start_menu":self.start_menu,"desktop_shortcut":self.desktop_shortcut})
    }
}

struct Transaction {
    root: PathBuf,
    work: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    moved: Vec<(PathBuf, PathBuf)>,
    added: Vec<(PathBuf, String)>,
    directories: Vec<PathBuf>,
}

impl Transaction {
    fn new(root: &Path) -> Result<Self, String> {
        let parent = root.parent().ok_or("설치 폴더가 올바르지 않습니다")?;
        reject_reparse(parent)?;
        fs::create_dir_all(parent).map_err(|e| format!("설치 폴더 생성 실패: {e}"))?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let work = parent.join(format!(".runetex-install-{}-{nonce}", std::process::id()));
        fs::create_dir(&work).map_err(|e| format!("설치 임시 폴더 생성 실패: {e}"))?;
        let staged = work.join("files");
        let backup = work.join("backup");
        if let Err(error) = fs::create_dir(&staged) {
            let _ = fs::remove_dir(&work);
            return Err(error.to_string());
        }
        if let Err(error) = fs::create_dir(&backup) {
            let _ = fs::remove_dir(&staged);
            let _ = fs::remove_dir(&work);
            return Err(error.to_string());
        }
        Ok(Self {
            root: root.to_path_buf(),
            work,
            staged,
            backup,
            moved: Vec::new(),
            added: Vec::new(),
            directories: Vec::new(),
        })
    }

    fn prepare_root(&mut self) -> Result<(), String> {
        self.ensure_directory(self.root.clone())
    }

    fn ensure_directory(&mut self, path: PathBuf) -> Result<(), String> {
        if !within(&path, &self.root) {
            return Err("설치 경로가 대상 폴더를 벗어났습니다".into());
        }
        reject_reparse(&path)?;
        if path.is_dir() {
            return Ok(());
        }
        if path.exists() {
            return Err(format!("폴더 위치에 파일이 있습니다: {}", path.display()));
        }
        if !same_path(&path, &self.root) {
            self.ensure_directory(
                path.parent()
                    .ok_or("설치 경로가 올바르지 않습니다")?
                    .to_path_buf(),
            )?;
        }
        fs::create_dir(&path).map_err(|e| format!("폴더 생성 실패 ({}): {e}", path.display()))?;
        self.directories.push(path);
        Ok(())
    }

    fn back_up(&mut self, relative: &str) -> Result<(), String> {
        let relative = relative_path(relative)?;
        let original = self.root.join(&relative);
        reject_reparse(&original)?;
        if !original.exists() {
            return Ok(());
        }
        let backup = self.backup.join(relative);
        fs::create_dir_all(backup.parent().ok_or("복구 경로가 올바르지 않습니다")?)
            .map_err(|e| e.to_string())?;
        fs::rename(&original, &backup).map_err(|e| {
            format!(
                "파일을 교체할 수 없습니다 ({}). RuneTeX를 닫고 다시 시도하세요: {e}",
                original.display()
            )
        })?;
        self.moved.push((original, backup));
        Ok(())
    }

    fn put(&mut self, relative: &str, expected_hash: &str) -> Result<(), String> {
        let relative = relative_path(relative)?;
        let target = self.root.join(&relative);
        self.ensure_directory(
            target
                .parent()
                .ok_or("설치 경로가 올바르지 않습니다")?
                .to_path_buf(),
        )?;
        reject_reparse(&target)?;
        if target.exists() {
            return Err(format!(
                "설치 위치에 기존 파일이 있습니다: {}",
                target.display()
            ));
        }
        fs::rename(self.staged.join(relative), &target)
            .map_err(|e| format!("설치 파일 저장 실패 ({}): {e}", target.display()))?;
        self.added.push((target, expected_hash.to_lowercase()));
        Ok(())
    }

    fn rollback(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        for (path, expected_hash) in self.added.iter().rev() {
            let result = (|| {
                reject_reparse(path)?;
                if !path.exists() {
                    return Ok(());
                }
                if !path.is_file() || hash_file(path)? != *expected_hash {
                    return Err("작업 중 변경된 파일을 보존했습니다".to_string());
                }
                fs::remove_file(path).map_err(|e| e.to_string())
            })();
            if let Err(error) = result {
                errors.push(format!("{}: {error}", path.display()));
            }
        }
        for (original, backup) in self.moved.iter().rev() {
            let result = (|| {
                reject_reparse(original)?;
                reject_reparse(backup)?;
                if original.exists() {
                    return Err("작업 중 생성되거나 변경된 파일을 덮어쓰지 않았습니다".to_string());
                }
                fs::rename(backup, original).map_err(|e| e.to_string())
            })();
            if let Err(error) = result {
                errors.push(format!("{}: {error}", original.display()));
            }
        }
        for directory in self.directories.iter().rev() {
            let _ = fs::remove_dir(directory);
        }
        errors
    }

    fn cleanup(&self) {
        if self.work.parent().is_some_and(|parent| {
            self.root
                .parent()
                .is_some_and(|root| same_path(parent, root))
        }) && self
            .work
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".runetex-install-"))
            && safe_tree(&self.work).is_ok()
        {
            let _ = fs::remove_dir_all(&self.work);
        }
    }
}

fn safe_tree(path: &Path) -> Result<(), String> {
    reject_reparse(path)?;
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        reject_reparse(&path)?;
        if path.is_dir() {
            safe_tree(&path)?;
        }
    }
    Ok(())
}

fn remove_owned_empty_directories(root: &Path, files: &[OwnedFile]) {
    let mut directories = HashSet::new();
    for file in files {
        if let Ok(relative) = relative_path(&file.path) {
            let full = root.join(relative);
            for parent in full.ancestors().skip(1) {
                if !within(parent, root) {
                    break;
                }
                directories.insert(parent.to_path_buf());
            }
        }
    }
    directories.insert(root.to_path_buf());
    let mut directories: Vec<_> = directories.into_iter().collect();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        if reject_reparse(&directory).is_ok() {
            let _ = fs::remove_dir(directory);
        }
    }
}
