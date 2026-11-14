//! Per-user Windows registration. File ownership is checked before changing links.
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::OsString,
        fs, io,
        os::windows::ffi::{OsStrExt, OsStringExt},
        ptr,
    };
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READ,
            },
            UI::Shell::{
                FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Programs, IShellLinkW,
                KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath, SLGP_RAWPATH, ShellLink,
            },
        },
        core::{GUID, Interface, PCWSTR},
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
        System::Registry::*,
    };

    const KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\RuneTeX";
    const VALUES: &[&str] = &[
        "DisplayName",
        "DisplayVersion",
        "InstallLocation",
        "DisplayIcon",
        "UninstallString",
        "NoModify",
        "NoRepair",
    ];

    struct Com;
    impl Com {
        fn start() -> Result<Self, String> {
            // Installer operations run on a dedicated worker thread.
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                .ok()
                .map_err(|error| format!("Windows 바로가기 기능 초기화 실패: {error}"))?;
            Ok(Self)
        }
    }
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    struct RegistryKey(HKEY);
    impl Drop for RegistryKey {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    #[derive(Clone)]
    struct RegistryValue {
        kind: u32,
        bytes: Vec<u8>,
    }
    struct RegistrySnapshot {
        existed: bool,
        values: Vec<Option<RegistryValue>>,
    }
    enum SavedLink {
        Absent,
        Owned(Vec<u8>),
        Foreign,
    }
    /// Opaque, exact registration state for an installer transaction.
    pub struct RegistrationSnapshot {
        root: PathBuf,
        registry: RegistrySnapshot,
        shortcuts: Vec<(PathBuf, SavedLink)>,
        menu_existed: bool,
    }
    struct LinkChange {
        path: PathBuf,
        original: Option<Vec<u8>>,
        create: bool,
    }

    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    fn key_name() -> Vec<u16> {
        wide(std::ffi::OsStr::new(KEY))
    }
    fn registry_error(operation: &str, code: u32) -> String {
        format!("{operation}: {}", io::Error::from_raw_os_error(code as i32))
    }
    fn open_key(access: u32) -> Result<Option<RegistryKey>, String> {
        let mut handle = ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                key_name().as_ptr(),
                0,
                access,
                &mut handle,
            )
        };
        match status {
            ERROR_SUCCESS => Ok(Some(RegistryKey(handle))),
            ERROR_FILE_NOT_FOUND => Ok(None),
            code => Err(registry_error("설치 정보 열기 실패", code)),
        }
    }
    fn query(key: &RegistryKey, name: &str) -> Result<Option<RegistryValue>, String> {
        let name = wide(std::ffi::OsStr::new(name));
        let mut kind = 0;
        let mut size = 0;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                ptr::null(),
                &mut kind,
                ptr::null_mut(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(registry_error("설치 정보 읽기 실패", status));
        }
        if size > 4 * 1024 * 1024 {
            return Err("설치 정보가 너무 큽니다".into());
        }
        let mut bytes = vec![0; size as usize];
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                ptr::null(),
                &mut kind,
                bytes.as_mut_ptr(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("설치 정보 읽기 실패", status));
        }
        bytes.truncate(size as usize);
        Ok(Some(RegistryValue { kind, bytes }))
    }
    fn string_value(value: &str) -> RegistryValue {
        RegistryValue {
            kind: REG_SZ,
            bytes: value
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        }
    }
    fn dword_value(value: u32) -> RegistryValue {
        RegistryValue {
            kind: REG_DWORD,
            bytes: value.to_le_bytes().to_vec(),
        }
    }
    fn set_value(key: &RegistryKey, name: &str, value: &RegistryValue) -> Result<(), String> {
        let name = wide(std::ffi::OsStr::new(name));
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                value.kind,
                value.bytes.as_ptr(),
                value.bytes.len() as u32,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(registry_error("설치 정보 저장 실패", status))
        }
    }
    fn remove_value(key: &RegistryKey, name: &str) -> Result<(), String> {
        let name = wide(std::ffi::OsStr::new(name));
        let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(registry_error("설치 정보 제거 실패", status))
        }
    }
    fn delete_empty_registration_key() -> Result<(), String> {
        let Some(key) = open_key(KEY_QUERY_VALUE)? else {
            return Ok(());
        };
        let mut subkeys = 0;
        let mut values = 0;
        let status = unsafe {
            RegQueryInfoKeyW(
                key.0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
                &mut subkeys,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut values,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("설치 등록 확인 실패", status));
        }
        drop(key);
        if subkeys == 0 && values == 0 {
            let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, key_name().as_ptr()) };
            if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
                return Err(registry_error("설치 등록 제거 실패", status));
            }
        }
        Ok(())
    }
    fn standard_path(path: &Path) -> PathBuf {
        let value = path.to_string_lossy().replace('/', "\\");
        if let Some(unc) = value.strip_prefix("\\\\?\\UNC\\") {
            PathBuf::from(format!("\\\\{unc}"))
        } else {
            PathBuf::from(value.strip_prefix("\\\\?\\").unwrap_or(&value))
        }
    }
    fn same_path(left: &Path, right: &Path) -> bool {
        fn normal(path: &Path) -> String {
            standard_path(path)
                .to_string_lossy()
                .trim_end_matches('\\')
                .to_lowercase()
        }
        normal(left) == normal(right)
    }
    fn registered_root(key: &RegistryKey) -> Result<PathBuf, String> {
        let value = query(key, "InstallLocation")?.ok_or("기존 설치 위치를 확인할 수 없습니다")?;
        if value.kind != REG_SZ || value.bytes.len() % 2 != 0 {
            return Err("기존 설치 위치 정보가 올바르지 않습니다".into());
        }
        let mut words: Vec<u16> = value
            .bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        while words.last() == Some(&0) {
            words.pop();
        }
        Ok(PathBuf::from(OsString::from_wide(&words)))
    }
    impl RegistrySnapshot {
        fn capture(root: &Path) -> Result<Self, String> {
            if let Some(key) = open_key(KEY_QUERY_VALUE)? {
                if !same_path(&registered_root(&key)?, root) {
                    return Err("다른 위치에 RuneTeX가 이미 설치되어 있습니다. 기존 설치를 제거한 뒤 다시 시도하세요".into());
                }
                Ok(Self {
                    existed: true,
                    values: VALUES
                        .iter()
                        .map(|name| query(&key, name))
                        .collect::<Result<_, _>>()?,
                })
            } else {
                Ok(Self {
                    existed: false,
                    values: vec![None; VALUES.len()],
                })
            }
        }
        fn restore(&self, root: &Path) -> Result<(), String> {
            let key = match open_key(KEY_QUERY_VALUE | KEY_SET_VALUE)? {
                Some(key) => {
                    if query(&key, "InstallLocation")?.is_some()
                        && !same_path(&registered_root(&key)?, root)
                    {
                        return Err("다른 설치 위치의 등록 정보는 복원하지 않았습니다".into());
                    }
                    key
                }
                None if self.existed => {
                    let mut handle = ptr::null_mut();
                    let status = unsafe {
                        RegCreateKeyW(HKEY_CURRENT_USER, key_name().as_ptr(), &mut handle)
                    };
                    if status != ERROR_SUCCESS {
                        return Err(registry_error("기존 설치 정보 복원 실패", status));
                    }
                    RegistryKey(handle)
                }
                None => return Ok(()),
            };
            let mut errors = Vec::new();
            // Restore ownership first; remove ownership last on a fresh install.
            let order: Vec<usize> = if self.existed {
                std::iter::once(2)
                    .chain((0..VALUES.len()).filter(|index| *index != 2))
                    .collect()
            } else {
                (0..VALUES.len())
                    .filter(|index| *index != 2)
                    .chain(std::iter::once(2))
                    .collect()
            };
            for index in order {
                let result = match &self.values[index] {
                    Some(value) => set_value(&key, VALUES[index], value),
                    None => remove_value(&key, VALUES[index]),
                };
                if let Err(error) = result {
                    errors.push(error);
                }
            }
            drop(key);
            if !self.existed
                && let Err(error) = delete_empty_registration_key()
            {
                errors.push(error);
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("\n"))
            }
        }
    }
    fn write_registration(root: &Path, version: &str, touched: &mut bool) -> Result<(), String> {
        let mut handle = ptr::null_mut();
        let status = unsafe { RegCreateKeyW(HKEY_CURRENT_USER, key_name().as_ptr(), &mut handle) };
        if status != ERROR_SUCCESS {
            return Err(registry_error("설치 정보 등록 실패", status));
        }
        let key = RegistryKey(handle);
        if let Some(existing) = query(&key, "InstallLocation")?
            && (existing.kind != REG_SZ || !same_path(&registered_root(&key)?, root))
        {
            return Err("다른 설치 정보가 등록되어 있어 변경하지 않았습니다".into());
        }
        *touched = true;
        // Record ownership first, before any other values are written.
        set_value(
            &key,
            "InstallLocation",
            &string_value(&root.to_string_lossy()),
        )?;
        let exe = root.join("runetex.exe");
        let uninstall = root.join("uninstall.exe");
        for (name, value) in [
            ("DisplayName", string_value("RuneTeX")),
            ("DisplayVersion", string_value(version)),
            (
                "DisplayIcon",
                string_value(&format!("\"{}\",0", exe.display())),
            ),
            (
                "UninstallString",
                string_value(&format!("\"{}\" --uninstall", uninstall.display())),
            ),
            ("NoModify", dword_value(1)),
            ("NoRepair", dword_value(1)),
        ] {
            set_value(&key, name, &value)?;
        }
        Ok(())
    }

    fn known_folder(id: &GUID) -> Result<PathBuf, String> {
        let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None) }
            .map_err(|error| format!("Windows 사용자 폴더 확인 실패: {error}"))?;
        let path = unsafe {
            let mut length = 0;
            while *raw.0.add(length) != 0 {
                length += 1;
            }
            PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(
                raw.0, length,
            )))
        };
        unsafe {
            CoTaskMemFree(Some(raw.0.cast()));
        }
        Ok(path)
    }
    fn shortcut_paths() -> Result<[PathBuf; 2], String> {
        Ok([
            known_folder(&FOLDERID_Programs)?
                .join("RuneTeX")
                .join("RuneTeX.lnk"),
            known_folder(&FOLDERID_Desktop)?.join("RuneTeX.lnk"),
        ])
    }
    fn link() -> Result<IShellLinkW, String> {
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| format!("Windows 바로가기 열기 실패: {error}"))
    }
    fn shortcut_owned(path: &Path, root: &Path) -> Result<bool, String> {
        let link = link()?;
        let persist: IPersistFile = link.cast().map_err(|error| error.to_string())?;
        let path_wide = wide(path.as_os_str());
        let mut target = vec![0; 32_768];
        let mut arguments = vec![0; 32_768];
        unsafe {
            persist
                .Load(PCWSTR(path_wide.as_ptr()), STGM_READ)
                .map_err(|error| error.to_string())?;
            link.GetPath(&mut target, ptr::null_mut(), SLGP_RAWPATH.0 as u32)
                .map_err(|error| error.to_string())?;
            link.GetArguments(&mut arguments)
                .map_err(|error| error.to_string())?;
        }
        let end = target
            .iter()
            .position(|word| *word == 0)
            .unwrap_or(target.len());
        let target = PathBuf::from(OsString::from_wide(&target[..end]));
        Ok(arguments[0] == 0 && same_path(&target, &root.join("runetex.exe")))
    }
    fn is_reparse(metadata: &fs::Metadata) -> bool {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    fn regular_shortcut(path: &Path) -> Result<bool, String> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() && !is_reparse(&metadata) => Ok(true),
            Ok(_) => Err(format!(
                "바로가기 경로에 일반 파일이 아닌 항목이 있습니다: {}",
                path.display()
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("바로가기 확인 실패: {error}")),
        }
    }
    fn validate_menu_parent(path: &Path, menu: &Path) -> Result<(), String> {
        if !same_path(path, menu) {
            return Ok(());
        }
        let parent = path.parent().ok_or("바로가기 폴더가 올바르지 않습니다")?;
        match fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.is_dir() && !is_reparse(&metadata) => Ok(()),
            Ok(_) => Err("시작 메뉴 폴더가 일반 폴더가 아니어서 변경하지 않았습니다".into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("시작 메뉴 폴더 확인 실패: {error}")),
        }
    }
    fn create_shortcut(path: &Path, root: &Path) -> Result<(), String> {
        let link = link()?;
        let exe = wide(root.join("runetex.exe").as_os_str());
        let working = wide(known_folder(&FOLDERID_Documents)?.as_os_str());
        let description = wide(std::ffi::OsStr::new("RuneTeX LaTeX 편집기"));
        let empty = [0u16];
        unsafe {
            link.SetPath(PCWSTR(exe.as_ptr()))
                .map_err(|error| error.to_string())?;
            link.SetArguments(PCWSTR(empty.as_ptr()))
                .map_err(|error| error.to_string())?;
            link.SetWorkingDirectory(PCWSTR(working.as_ptr()))
                .map_err(|error| error.to_string())?;
            link.SetIconLocation(PCWSTR(exe.as_ptr()), 0)
                .map_err(|error| error.to_string())?;
            link.SetDescription(PCWSTR(description.as_ptr()))
                .map_err(|error| error.to_string())?;
        }
        let persist: IPersistFile = link.cast().map_err(|error| error.to_string())?;
        let path_wide = wide(path.as_os_str());
        unsafe { persist.Save(PCWSTR(path_wide.as_ptr()), true) }
            .map_err(|error| format!("바로가기 저장 실패: {error}"))
    }
    fn restore_links(changes: &[LinkChange]) -> Result<(), String> {
        let allowed = shortcut_paths()?;
        for change in changes.iter().rev() {
            validate_menu_parent(&change.path, &allowed[0])?;
            if let Some(bytes) = &change.original {
                // A link replaced with a reparse point must never be followed during rollback.
                regular_shortcut(&change.path)?;
                fs::write(&change.path, bytes)
                    .map_err(|error| format!("기존 바로가기 복원 실패: {error}"))?;
            } else if regular_shortcut(&change.path)? {
                fs::remove_file(&change.path)
                    .map_err(|error| format!("바로가기 복원 실패: {error}"))?;
            }
        }
        Ok(())
    }

    /// Capture before changing files or registration. A new root need not exist.
    pub fn snapshot_install(root: &Path) -> Result<RegistrationSnapshot, String> {
        if !root.is_absolute() {
            return Err("설치 폴더의 전체 경로가 필요합니다".into());
        }
        let root = standard_path(&std::path::absolute(root).map_err(|error| error.to_string())?);
        let _com = Com::start()?;
        let registry = RegistrySnapshot::capture(&root)?;
        let paths = shortcut_paths()?;
        let mut shortcuts = Vec::new();
        for path in &paths {
            validate_menu_parent(path, &paths[0])?;
            let state = if !regular_shortcut(path)? {
                SavedLink::Absent
            } else if shortcut_owned(path, &root).unwrap_or(false) {
                let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
                if metadata.len() > 16 * 1024 * 1024 {
                    return Err("기존 바로가기 파일이 너무 큽니다".into());
                }
                SavedLink::Owned(fs::read(path).map_err(|error| error.to_string())?)
            } else {
                SavedLink::Foreign
            };
            shortcuts.push((path.clone(), state));
        }
        let menu_existed = paths[0].parent().is_some_and(Path::is_dir);
        Ok(RegistrationSnapshot {
            root,
            registry,
            shortcuts,
            menu_existed,
        })
    }

    /// Restore raw pre-transaction values and link bytes without touching foreign links.
    pub fn restore_install(root: &Path, snapshot: &RegistrationSnapshot) -> Result<(), String> {
        let root = standard_path(&std::path::absolute(root).map_err(|error| error.to_string())?);
        if !same_path(&root, &snapshot.root) {
            return Err("다른 설치 폴더의 등록 상태는 복원하지 않았습니다".into());
        }
        let _com = Com::start()?;
        let allowed = shortcut_paths()?;
        let mut errors = Vec::new();
        for (path, state) in &snapshot.shortcuts {
            let result: Result<(), String> = (|| {
                if !allowed.iter().any(|known| same_path(known, path)) {
                    return Err("사용자 바로가기 폴더가 변경되어 복원하지 않았습니다".into());
                }
                validate_menu_parent(path, &allowed[0])?;
                let exists = regular_shortcut(path)?;
                match state {
                    SavedLink::Foreign => {}
                    SavedLink::Absent => {
                        if exists && shortcut_owned(path, &root).unwrap_or(false) {
                            fs::remove_file(path)
                                .map_err(|error| format!("바로가기 복원 실패: {error}"))?;
                        }
                    }
                    SavedLink::Owned(bytes) => {
                        if exists && !shortcut_owned(path, &root).unwrap_or(false) {
                            return Err(format!(
                                "다른 대상의 바로가기를 보존했습니다: {}",
                                path.display()
                            ));
                        }
                        let parent = path.parent().ok_or("바로가기 폴더가 올바르지 않습니다")?;
                        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                        validate_menu_parent(path, &allowed[0])?;
                        regular_shortcut(path)?;
                        fs::write(path, bytes)
                            .map_err(|error| format!("기존 바로가기 복원 실패: {error}"))?;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                errors.push(error);
            }
        }
        if let Err(error) = snapshot.registry.restore(&root) {
            errors.push(error);
        }
        if !snapshot.menu_existed
            && let Some(directory) = allowed[0].parent()
        {
            // An empty app-specific folder is the only directory we may remove.
            let _ = fs::remove_dir(directory);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }

    pub fn register_install(
        root: &Path,
        version: &str,
        start_menu: bool,
        desktop: bool,
    ) -> Result<Vec<PathBuf>, String> {
        let root = standard_path(
            &root
                .canonicalize()
                .map_err(|error| format!("설치 경로 확인 실패: {error}"))?,
        );
        if !root.join("runetex.exe").is_file() || !root.join("uninstall.exe").is_file() {
            return Err("설치 파일 복사가 끝나지 않았습니다".into());
        }
        let _com = Com::start()?;
        let snapshot = RegistrySnapshot::capture(&root)?;
        let paths = shortcut_paths()?;
        let selected = [start_menu, desktop];
        let mut changes = Vec::new();
        let mut created_menu = None;
        for (path, selected) in paths.iter().zip(selected) {
            validate_menu_parent(path, &paths[0])?;
            let exists = regular_shortcut(path)?;
            let owned = exists && shortcut_owned(path, &root).unwrap_or(false);
            if exists && selected && !owned {
                return Err(format!(
                    "기존 바로가기를 덮어쓰지 않았습니다: {}",
                    path.display()
                ));
            }
            if selected || owned {
                let original = if exists {
                    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
                    if metadata.len() > 16 * 1024 * 1024 {
                        return Err("기존 바로가기 파일이 너무 큽니다".into());
                    }
                    Some(fs::read(path).map_err(|error| error.to_string())?)
                } else {
                    None
                };
                changes.push(LinkChange {
                    path: path.clone(),
                    original,
                    create: selected,
                });
            }
        }
        let mut registry_touched = false;
        let mut applied = 0;
        let result: Result<(), String> = (|| {
            for change in &changes {
                if change.create {
                    let parent = change
                        .path
                        .parent()
                        .ok_or("바로가기 폴더가 올바르지 않습니다")?;
                    if change.path == paths[0] {
                        if let Ok(metadata) = fs::symlink_metadata(parent) {
                            if !metadata.is_dir() || is_reparse(&metadata) {
                                return Err("시작 메뉴 폴더가 올바르지 않습니다".into());
                            }
                        } else {
                            created_menu = Some(parent.to_path_buf());
                        }
                    }
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("바로가기 폴더 생성 실패: {error}"))?;
                    validate_menu_parent(&change.path, &paths[0])?;
                    regular_shortcut(&change.path)?;
                    applied += 1;
                    create_shortcut(&change.path, &root)?;
                } else {
                    validate_menu_parent(&change.path, &paths[0])?;
                    regular_shortcut(&change.path)?;
                    applied += 1;
                    fs::remove_file(&change.path)
                        .map_err(|error| format!("바로가기 변경 실패: {error}"))?;
                }
            }
            write_registration(&root, version, &mut registry_touched)?;
            Ok(())
        })();
        if let Err(error) = result {
            let mut failures = Vec::new();
            if registry_touched && let Err(rollback) = snapshot.restore(&root) {
                failures.push(rollback);
            }
            if let Err(rollback) = restore_links(&changes[..applied]) {
                failures.push(rollback);
            }
            if let Some(directory) = created_menu {
                let _ = fs::remove_dir(directory);
            }
            return Err(if failures.is_empty() {
                error
            } else {
                format!("{error}\n{}", failures.join("\n"))
            });
        }
        Ok(paths
            .into_iter()
            .zip(selected)
            .filter_map(|(path, enabled)| enabled.then_some(path))
            .collect())
    }

    pub fn unregister_install(root: &Path) -> Result<(), String> {
        let snapshot = RegistrySnapshot::capture(root)?;
        let Some(key) = open_key(KEY_QUERY_VALUE | KEY_SET_VALUE)? else {
            return Ok(());
        };
        if !same_path(&registered_root(&key)?, root) {
            return Err("다른 설치 위치의 등록 정보는 제거하지 않았습니다".into());
        }
        let result = (|| {
            for name in VALUES
                .iter()
                .copied()
                .filter(|name| *name != "InstallLocation")
            {
                remove_value(&key, name)?;
            }
            remove_value(&key, "InstallLocation")?;
            drop(key);
            // Unfamiliar values and child keys are deliberately preserved.
            delete_empty_registration_key()
        })();
        if let Err(error) = result {
            return Err(match snapshot.restore(root) {
                Ok(()) => error,
                Err(rollback) => format!("{error}\n{rollback}"),
            });
        }
        Ok(())
    }

    pub fn remove_shortcuts(root: &Path, paths: &[PathBuf]) -> Result<(), String> {
        let _com = Com::start()?;
        let allowed = shortcut_paths()?;
        for path in paths {
            if !allowed.iter().any(|known| same_path(known, path)) {
                return Err("등록되지 않은 바로가기 경로는 제거하지 않았습니다".into());
            }
            validate_menu_parent(path, &allowed[0])?;
            if !regular_shortcut(path)? {
                continue;
            }
            if shortcut_owned(path, root).unwrap_or(false) {
                fs::remove_file(path).map_err(|error| format!("바로가기 제거 실패: {error}"))?;
            }
        }
        // Only an empty application-specific Start Menu directory can be removed.
        if let Some(directory) = allowed[0].parent() {
            let _ = fs::remove_dir(directory);
        }
        Ok(())
    }
}

#[cfg(windows)]
pub use native::{
    register_install, remove_shortcuts, restore_install, snapshot_install, unregister_install,
};

#[cfg(not(windows))]
pub struct RegistrationSnapshot;
#[cfg(not(windows))]
pub fn snapshot_install(_: &Path) -> Result<RegistrationSnapshot, String> {
    Err("설치 기능은 Windows에서 사용할 수 있습니다".into())
}
#[cfg(not(windows))]
pub fn restore_install(_: &Path, _: &RegistrationSnapshot) -> Result<(), String> {
    Err("설치 기능은 Windows에서 사용할 수 있습니다".into())
}

#[cfg(not(windows))]
pub fn register_install(_: &Path, _: &str, _: bool, _: bool) -> Result<Vec<PathBuf>, String> {
    Err("설치 기능은 Windows에서 사용할 수 있습니다".into())
}
#[cfg(not(windows))]
pub fn unregister_install(_: &Path) -> Result<(), String> {
    Err("제거 기능은 Windows에서 사용할 수 있습니다".into())
}
#[cfg(not(windows))]
pub fn remove_shortcuts(_: &Path, _: &[PathBuf]) -> Result<(), String> {
    Err("제거 기능은 Windows에서 사용할 수 있습니다".into())
}
