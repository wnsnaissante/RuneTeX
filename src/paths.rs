//! Runtime paths follow the installed executable and the user's profile.
use std::{
    env,
    path::{Path, PathBuf},
};

pub fn absolute_file(path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = std::path::absolute(path).ok()?;
    path.is_file().then_some(path)
}

pub fn program_override(variable: &str) -> Option<PathBuf> {
    env::var_os(variable).and_then(absolute_file)
}

/// Bundle layout: tools/texlab and tools/poppler beside runetex.exe.
/// This deliberately never searches the working directory or a build-machine path.
pub fn bundled_program(name: &str) -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let root = executable.parent()?;
    let folders: &[&str] = match name {
        "texlab" => &["tools/texlab", "tools"],
        "pdftoppm" | "pdfinfo" => &[
            "tools/poppler/Library/bin",
            "tools/poppler/bin",
            "tools/poppler",
        ],
        _ => return None,
    };
    let filename = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    folders
        .iter()
        .find_map(|folder| absolute_file(root.join(folder).join(&filename)))
}

pub fn default_project_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(documents) = known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_Documents) {
        return documents.join("RuneTeX/research-paper");
    }
    for variable in ["USERPROFILE", "HOME"] {
        if let Some(home) = profile_path(variable) {
            return home.join("Documents/RuneTeX/research-paper");
        }
    }
    config_dir().join("projects/research-paper")
}

pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(root) = profile_path("APPDATA") {
            return root.join("RuneTeX");
        }
        if let Some(root) = known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_RoamingAppData) {
            return root.join("RuneTeX");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(root) = profile_path("XDG_CONFIG_HOME") {
            return root.join("runetex");
        }
    }
    for variable in ["HOME", "USERPROFILE"] {
        if let Some(home) = profile_path(variable) {
            return home.join(".config/runetex");
        }
    }
    env::temp_dir().join("RuneTeX")
}

fn profile_path(variable: &str) -> Option<PathBuf> {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(windows)]
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, ptr};
    use windows_sys::Win32::{System::Com::CoTaskMemFree, UI::Shell::SHGetKnownFolderPath};
    let mut raw = ptr::null_mut();
    // Shell owns the allocation contract: the result is a NUL-terminated UTF-16
    // path freed with CoTaskMemFree, including any allocation on failure.
    let result = unsafe { SHGetKnownFolderPath(id, 0, ptr::null_mut(), &mut raw) };
    let path = if result >= 0 && !raw.is_null() {
        let mut length = 0;
        unsafe {
            while *raw.add(length) != 0 {
                length += 1;
            }
            Some(PathBuf::from(OsString::from_wide(
                std::slice::from_raw_parts(raw, length),
            )))
        }
    } else {
        None
    };
    unsafe {
        CoTaskMemFree(raw.cast());
    }
    path
}
