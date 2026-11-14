use std::{
    fs,
    path::{Path, PathBuf},
};

pub const DEFAULT_UI_FONT: &str = "Segoe UI";
pub const DEFAULT_UI_FAMILIES: &str = "Segoe UI, Malgun Gothic, Inter";
pub const CODE_FAMILIES: &str = "Cascadia Code, Consolas, Malgun Gothic, Noto Sans Mono CJK KR";
pub const DEFAULT_EDITOR_FONT: &str = "Cascadia Code";
pub const DEFAULT_EDITOR_SIZE: f32 = 16.0;
pub const MIN_EDITOR_SIZE: f32 = 8.0;
pub const MAX_EDITOR_SIZE: f32 = 48.0;

pub fn editor_families(font: &str) -> String {
    if font == DEFAULT_EDITOR_FONT {
        CODE_FAMILIES.into()
    } else {
        format!("{font}, {CODE_FAMILIES}")
    }
}

pub fn read_editor_font(path: &Path) -> (String, f32) {
    let value = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .unwrap_or_default();
    let family = value["family"]
        .as_str()
        .filter(|font| valid_font(font))
        .unwrap_or(DEFAULT_EDITOR_FONT)
        .to_string();
    let size = value["size"]
        .as_f64()
        .filter(|size| size.is_finite())
        .map(|size| size.clamp(MIN_EDITOR_SIZE as f64, MAX_EDITOR_SIZE as f64) as f32)
        .unwrap_or(DEFAULT_EDITOR_SIZE);
    (family, size)
}

pub fn write_editor_font(path: &Path, font: &str, size: f32) -> std::io::Result<()> {
    if !valid_font(font)
        || !size.is_finite()
        || !(MIN_EDITOR_SIZE..=MAX_EDITOR_SIZE).contains(&size)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "올바른 에디터 글꼴과 크기를 선택하세요.",
        ));
    }
    write_font_json(path, serde_json::json!({"family":font, "size":size}))
}
pub fn installed_ui_fonts() -> &'static [String] {
    static FONTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let mut collection = fontique::Collection::new(fontique::CollectionOptions {
            system_fonts: true,
            ..Default::default()
        });
        let mut names: Vec<String> = collection.family_names().map(str::to_string).collect();
        names.push("Inter".into()); // Bundled by Rune.
        names.sort_by_key(|name| name.to_lowercase());
        names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        names
    })
}

pub fn font_display_name(family: &str) -> String {
    if family.eq_ignore_ascii_case("Malgun Gothic") {
        "맑은 고딕 · Malgun Gothic".into()
    } else {
        family.to_string()
    }
}

pub fn ui_families(font: &str) -> String {
    if font == DEFAULT_UI_FONT {
        return DEFAULT_UI_FAMILIES.to_string();
    }
    format!("{font}, Segoe UI, Malgun Gothic, Noto Sans CJK KR, Inter, sans-serif")
}

pub fn valid_font(font: &str) -> bool {
    !font.trim().is_empty() && font.len() <= 240 && !font.chars().any(char::is_control)
}

pub fn global_font_path() -> PathBuf {
    crate::paths::config_dir().join("ui-font.json")
}

pub fn read_ui_font(path: &Path) -> String {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| {
            value["family"]
                .as_str()
                .filter(|font| valid_font(font))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| DEFAULT_UI_FONT.to_string())
}

pub fn write_ui_font(path: &Path, font: &str) -> std::io::Result<()> {
    if !valid_font(font) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "올바른 글꼴 이름을 입력하세요.",
        ));
    }
    write_font_json(path, serde_json::json!({"family":font}))
}

fn write_font_json(path: &Path, value: serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, serde_json::to_vec_pretty(&value)?)?;
    fs::rename(temporary, path)
}
