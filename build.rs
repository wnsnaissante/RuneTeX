use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon.ico");
    println!("cargo:rerun-if-changed=Cargo.toml");
    for name in [
        "RC",
        "PATH",
        "WindowsSdkDir",
        "WindowsSDKVersion",
        "ProgramFiles(x86)",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ENV").as_deref(),
        Ok("msvc"),
        "RuneTeX requires the MSVC Windows target (x86_64-pc-windows-msvc)."
    );

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR"));
    let icon = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo supplies the project directory"),
    )
    .join("assets/app-icon.ico");
    assert!(
        icon.is_file(),
        "Missing application icon: {}",
        icon.display()
    );
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo supplies the package version");
    let numbers: Vec<u16> = version
        .split(['-', '+'])
        .next()
        .unwrap()
        .split('.')
        .map(|part| {
            part.parse()
                .expect("Windows version component must fit u16")
        })
        .collect();
    assert_eq!(numbers.len(), 3, "Expected a three-part package version");
    let numeric_version = format!("{},{},{},0", numbers[0], numbers[1], numbers[2]);
    let icon_path = icon.to_string_lossy().replace('\\', "/");
    for binary in ["runetex", "runetex-setup"] {
        let description = if binary == "runetex" {
            "RuneTeX LaTeX Editor"
        } else {
            "RuneTeX Setup Preview"
        };
        let source = output.join(format!("{binary}.rc"));
        let resource = output.join(format!("{binary}.res"));
        fs::write(
            &source,
            format!(
                r#"1 ICON "{icon_path}"
1 VERSIONINFO
FILEVERSION {numeric_version}
PRODUCTVERSION {numeric_version}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "{description}\0"
      VALUE "FileVersion", "{version}\0"
      VALUE "InternalName", "{binary}\0"
      VALUE "OriginalFilename", "{binary}.exe\0"
      VALUE "ProductName", "RuneTeX\0"
      VALUE "ProductVersion", "{version}\0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
            ),
        )
        .expect("Could not write the Windows icon resource script");
        let compiler = find_rc();
        let result = Command::new(&compiler)
            .arg("/nologo")
            .arg("/fo")
            .arg(&resource)
            .arg(&source)
            .output()
            .unwrap_or_else(|error| panic!("Could not run {}: {error}", compiler.display()));
        assert!(
            result.status.success(),
            "Windows icon compilation failed ({}):\n{}{}",
            compiler.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        println!("cargo:rustc-link-arg-bin={binary}={}", resource.display());
    }
}

fn executable(name: &Path) -> Option<PathBuf> {
    if name.is_file() {
        return Some(name.to_path_buf());
    }
    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

fn find_rc() -> PathBuf {
    if let Some(name) = env::var_os("RC") {
        return executable(Path::new(&name)).unwrap_or_else(|| {
            panic!(
                "RC does not name an existing resource compiler: {}",
                Path::new(&name).display()
            )
        });
    }
    if let Some(path) = executable(Path::new("rc.exe")) {
        return path;
    }
    let host = env::var("HOST").expect("Cargo supplies HOST");
    let architecture = if host.starts_with("aarch64-") {
        "arm64"
    } else if host.starts_with("i686-") {
        "x86"
    } else {
        "x64"
    };
    let mut roots = Vec::new();
    if let Some(root) = env::var_os("WindowsSdkDir") {
        roots.push(PathBuf::from(root));
    }
    if let Some(root) = env::var_os("ProgramFiles(x86)") {
        roots.push(PathBuf::from(root).join("Windows Kits/10"));
    }
    for root in roots {
        if let Ok(version) = env::var("WindowsSDKVersion") {
            let candidate = root
                .join("bin")
                .join(version.trim_matches(['/', '\\']))
                .join(architecture)
                .join("rc.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
        let mut versions: Vec<_> = fs::read_dir(root.join("bin"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let version = entry
                    .file_name()
                    .to_str()?
                    .split('.')
                    .map(str::parse::<u32>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                Some((version, entry.path()))
            })
            .collect();
        versions.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, directory) in versions {
            let candidate = directory.join(architecture).join("rc.exe");
            if candidate.is_file() {
                return candidate;
            }
        }
        let candidate = root.join("bin").join(architecture).join("rc.exe");
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "Windows resource compiler rc.exe was not found. Install the Windows SDK with Visual Studio C++ Build Tools or set RC to its full path."
    );
}
