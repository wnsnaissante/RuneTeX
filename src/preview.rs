//! PDF metadata and individual page images. All Poppler work runs on workers.
//!
//! Each compiler build owns an immutable PDF and an independent disk cache.
//! Opening a document never rasterizes its pages; callers request only the
//! pages around the current viewport and keep a bounded number of Pixels.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rune::Pixels;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const RASTER_SIZE: u32 = 1400;
const TOOL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_TOOL_BYTES: u64 = 16 * 1024 * 1024;
const METADATA_FILE: &str = "preview-document.json";
const CACHE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

impl PageSize {
    fn valid(self) -> bool {
        self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
            && (self.height / self.width).is_finite()
    }
}

#[derive(Clone, Debug)]
pub struct Document {
    pub pdf_path: PathBuf,
    pub pages: Vec<PageSize>,
    renderer: PathBuf,
    cache_dir: PathBuf,
}

#[derive(Deserialize, Serialize)]
struct Metadata {
    version: u32,
    pdf_name: String,
    identity: String,
    pages: Vec<PageSize>,
}

/// Read every page's geometry without generating any page image. Only called
/// for a completed TeX build (or for an existing PDF explicitly being opened).
pub fn open(pdf: &Path) -> Result<Document, String> {
    let pdf = std::path::absolute(pdf).map_err(|error| error.to_string())?;
    let renderer = renderer().ok_or_else(|| {
        "Poppler의 pdftoppm을 찾을 수 없습니다. RUNETEX_PDFTOPPM 또는 PATH를 확인하세요."
            .to_string()
    })?;
    if let Some(document) = read_cached_document(&pdf, renderer.clone()) {
        return Ok(document);
    }
    let info = crate::paths::program_override("RUNETEX_PDFINFO")
        .or_else(|| sibling_program(&renderer, "pdfinfo"))
        .or_else(|| crate::paths::bundled_program("pdfinfo"))
        .or_else(|| crate::compiler::find_program("pdfinfo"))
        .ok_or_else(|| {
            "Poppler의 pdfinfo를 찾을 수 없습니다. Poppler 설치를 확인하세요.".to_string()
        })?;
    let parent = pdf.parent().ok_or("PDF 폴더를 찾을 수 없습니다.")?;
    let mut command = Command::new(info);
    // Poppler clamps -l to the actual page count. Supplying its full integer
    // range emits per-page sizes/rotations in one metadata-only invocation.
    command
        .arg("-f")
        .arg("1")
        .arg("-l")
        .arg(i32::MAX.to_string())
        .arg(&pdf);
    let info_log = parent.join(format!("preview-info-{}.txt", temporary_tag()));
    let output = run_tool(command, &info_log);
    let _ = fs::remove_file(&info_log);
    let output = output?;
    let pages = parse_page_sizes(&output)?;
    let identity =
        identity(&pdf).map_err(|error| format!("PDF 정보를 읽지 못했습니다: {error}"))?;
    let metadata = Metadata {
        version: CACHE_VERSION,
        pdf_name: pdf
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        identity: identity.clone(),
        pages: pages.clone(),
    };
    // This marker exists only after both TeX passes and metadata validation.
    // A failed build's partially written PDF is never used on the next launch.
    let bytes = serde_json::to_vec(&metadata).map_err(|error| error.to_string())?;
    let marker = parent.join(METADATA_FILE);
    let temporary = parent.join(format!("preview-document-{}.pending", temporary_tag()));
    fs::write(&temporary, bytes).map_err(|error| format!("PDF 캐시 저장 실패: {error}"))?;
    fs::rename(&temporary, &marker).map_err(|error| format!("PDF 캐시 저장 실패: {error}"))?;
    let cache_dir = parent.join(format!("preview-cache-{identity}"));
    Ok(Document {
        pdf_path: pdf,
        pages,
        renderer,
        cache_dir,
    })
}

/// Reopen the newest successfully prepared PDF using its small metadata file.
/// Does not spawn Poppler or decode cached images on the UI startup path.
pub fn load_cached(root: &Path, main: &str) -> Option<Document> {
    let renderer = renderer()?;
    let mut builds: Vec<_> = fs::read_dir(root.join(".runetex/build"))
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .collect();
    builds.sort_by_key(|entry| {
        std::cmp::Reverse(
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok(),
        )
    });
    let job = Path::new(main).file_stem()?;
    let filename = Path::new(job).with_extension("pdf");
    builds
        .into_iter()
        .find_map(|build| read_cached_document(&build.path().join(&filename), renderer.clone()))
}

/// Rasterize one zero-based page, or decode its existing PNG from disk.
/// The caller owns prioritization, cancellation between pages and memory LRU.
pub fn render_page(document: &Document, index: usize) -> Result<Pixels, String> {
    if index >= document.pages.len() {
        return Err("PDF 페이지 번호가 범위를 벗어났습니다.".into());
    }
    fs::create_dir_all(&document.cache_dir)
        .map_err(|error| format!("페이지 캐시 폴더 생성 실패: {error}"))?;
    let page = index + 1;
    let cached = document.cache_dir.join(format!("page-{page}.png"));
    if let Ok(pixels) = decode_png(&cached) {
        return Ok(pixels);
    }
    // Keep the final cache entry absent until the subprocess has completed and
    // the image has been validated. A corrupt/interrupted entry is regenerated.
    let prefix = document
        .cache_dir
        .join(format!("page-{page}-{}-pending", temporary_tag()));
    let temporary = prefix.with_extension("png");
    let log = prefix.with_extension("txt");
    let mut command = Command::new(&document.renderer);
    command
        .arg("-png")
        .arg("-scale-to")
        .arg(RASTER_SIZE.to_string())
        .arg("-f")
        .arg(page.to_string())
        .arg("-l")
        .arg(page.to_string())
        .arg("-singlefile")
        .arg(&document.pdf_path)
        .arg(&prefix);
    let rendered = run_tool(command, &log);
    let _ = fs::remove_file(&log);
    if let Err(error) = rendered {
        let _ = fs::remove_file(&temporary);
        return Err(format!("{page}페이지 미리보기 생성 실패: {error}"));
    }
    let pixels = match decode_png(&temporary) {
        Ok(pixels) => pixels,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(format!("{page}페이지 미리보기 읽기 실패: {error}"));
        }
    };
    // Rename replaces the exact cache entry atomically. Different app instances
    // can finish this page together, each with an independent temporary image.
    fs::rename(&temporary, &cached).map_err(|error| format!("페이지 캐시 저장 실패: {error}"))?;
    Ok(pixels)
}

fn temporary_tag() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}-{:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn renderer() -> Option<PathBuf> {
    crate::paths::program_override("RUNETEX_PDFTOPPM")
        .or_else(|| crate::paths::bundled_program("pdftoppm"))
        .or_else(|| crate::compiler::find_program("pdftoppm"))
}

fn sibling_program(program: &Path, name: &str) -> Option<PathBuf> {
    let filename = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    };
    crate::paths::absolute_file(program.parent()?.join(filename))
}

fn identity(pdf: &Path) -> std::io::Result<String> {
    let metadata = fs::metadata(pdf)?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "PDF 파일이 비어 있습니다.",
        ));
    }
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut hash = Sha256::new();
    hash.update(
        pdf.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_bytes(),
    );
    hash.update(metadata.len().to_le_bytes());
    hash.update(modified.to_le_bytes());
    Ok(format!("{:x}", hash.finalize()))
}

fn read_cached_document(pdf: &Path, renderer: PathBuf) -> Option<Document> {
    let parent = pdf.parent()?;
    let file = File::open(parent.join(METADATA_FILE)).ok()?;
    if file.metadata().ok()?.len() > MAX_TOOL_BYTES {
        return None;
    }
    let metadata: Metadata = serde_json::from_reader(file).ok()?;
    let identity = identity(pdf).ok()?;
    if metadata.version != CACHE_VERSION
        || metadata.pdf_name != pdf.file_name()?.to_string_lossy()
        || metadata.identity != identity
        || metadata.pages.is_empty()
        || metadata.pages.iter().any(|page| !page.valid())
    {
        return None;
    }
    Some(Document {
        pdf_path: pdf.to_path_buf(),
        pages: metadata.pages,
        renderer,
        cache_dir: parent.join(format!("preview-cache-{identity}")),
    })
}

fn parse_page_sizes(output: &str) -> Result<Vec<PageSize>, String> {
    let mut count = None;
    let mut sizes = BTreeMap::new();
    let mut rotations = BTreeMap::new();
    for line in output.lines() {
        if let Some(value) = line.strip_prefix("Pages:") {
            count = value
                .trim()
                .parse::<usize>()
                .ok()
                .filter(|count| *count > 0);
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.first() != Some(&"Page") {
            continue;
        }
        let Some(page) = fields.get(1).and_then(|value| value.parse::<usize>().ok()) else {
            continue;
        };
        if fields.get(2) == Some(&"size:") && fields.get(4) == Some(&"x") {
            if let (Some(width), Some(height)) = (
                fields.get(3).and_then(|value| value.parse::<f32>().ok()),
                fields.get(5).and_then(|value| value.parse::<f32>().ok()),
            ) {
                sizes.insert(page, PageSize { width, height });
            }
        } else if fields.get(2) == Some(&"rot:") {
            if let Some(rotation) = fields.get(3).and_then(|value| value.parse::<i32>().ok()) {
                rotations.insert(page, rotation.rem_euclid(360));
            }
        }
    }
    let count = count.ok_or("PDF 페이지 수를 읽지 못했습니다.")?;
    if sizes.len() != count {
        return Err("PDF의 모든 페이지 크기를 읽지 못했습니다.".into());
    }
    let mut pages = Vec::with_capacity(count);
    for number in 1..=count {
        let mut size = *sizes
            .get(&number)
            .ok_or("PDF 페이지 크기가 누락되었습니다.")?;
        match rotations.get(&number).copied().unwrap_or(0) {
            0 | 180 => {}
            90 | 270 => std::mem::swap(&mut size.width, &mut size.height),
            _ => return Err("PDF 페이지 회전 값이 올바르지 않습니다.".into()),
        }
        if !size.valid() {
            return Err("PDF 페이지 크기가 올바르지 않습니다.".into());
        }
        pages.push(size);
    }
    Ok(pages)
}

fn decode_png(path: &Path) -> Result<Pixels, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_TOOL_BYTES {
        return Err("미리보기 이미지가 너무 큽니다.".into());
    }
    let mut png = Vec::new();
    file.take(MAX_TOOL_BYTES)
        .read_to_end(&mut png)
        .map_err(|error| error.to_string())?;
    // Check dimensions before decompression, including untrusted cache files.
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return Err("PNG 헤더가 올바르지 않습니다.".into());
    }
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width > RASTER_SIZE || height > RASTER_SIZE {
        return Err("PNG 크기가 올바르지 않습니다.".into());
    }
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let pixels = Pixels::new();
    pixels.set(image.width(), image.height(), image.into_raw());
    Ok(pixels)
}

fn run_tool(mut command: Command, capture: &Path) -> Result<String, String> {
    let log = File::create(capture).map_err(|error| error.to_string())?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            log.try_clone().map_err(|error| error.to_string())?,
        ))
        .stderr(Stdio::from(log));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if started.elapsed() >= TOOL_TIMEOUT
            || fs::metadata(capture).is_ok_and(|metadata| metadata.len() > MAX_TOOL_BYTES)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err("PDF 미리보기 처리가 제한 시간 또는 출력 크기를 초과했습니다.".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let file = File::open(capture).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_TOOL_BYTES {
        return Err("PDF 정보 출력이 너무 큽니다.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_TOOL_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let output = String::from_utf8_lossy(&bytes).into_owned();
    if !status.success() {
        return Err(if output.trim().is_empty() {
            format!("Poppler 종료 코드: {:?}", status.code())
        } else {
            output
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_includes_all_pages_and_applies_rotation() {
        let mut metadata = "Pages: 42\n".to_string();
        for page in 1..=42 {
            metadata.push_str(&format!(
                "Page {page:4} size: 612 x 792 pts (letter)\nPage {page:4} rot: {}\n",
                if page == 42 { 90 } else { 0 }
            ));
        }
        let pages = parse_page_sizes(&metadata).unwrap();
        assert_eq!(pages.len(), 42);
        assert_eq!((pages[41].width, pages[41].height), (792.0, 612.0));
    }

    #[test]
    fn incomplete_or_invalid_geometry_is_rejected() {
        assert!(parse_page_sizes("Pages: 2\nPage 1 size: 612 x 792 pts\n").is_err());
        assert!(parse_page_sizes("Pages: 1\nPage 1 size: NaN x 792 pts\n").is_err());
        assert!(
            parse_page_sizes("Pages: 1\nPage 1 size: 612 x 792 pts\nPage 1 rot: 45\n").is_err()
        );
        assert!(parse_page_sizes("Pages: 1\nPage 2 size: 612 x 792 pts\n").is_err());
    }

    #[test]
    fn changed_pdf_never_reuses_a_previous_success_marker() {
        let nonce = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("runetex-pdf-cache-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let pdf = directory.join("document.pdf");
        let marker = directory.join(METADATA_FILE);
        fs::write(&pdf, b"previous completed PDF").unwrap();
        let metadata = Metadata {
            version: CACHE_VERSION,
            pdf_name: "document.pdf".into(),
            identity: identity(&pdf).unwrap(),
            pages: vec![PageSize {
                width: 612.0,
                height: 792.0,
            }],
        };
        fs::write(&marker, serde_json::to_vec(&metadata).unwrap()).unwrap();
        assert!(read_cached_document(&pdf, PathBuf::new()).is_some());
        fs::write(&pdf, b"partial replacement").unwrap();
        assert!(read_cached_document(&pdf, PathBuf::new()).is_none());
        fs::remove_file(marker).unwrap();
        fs::remove_file(pdf).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn cached_page_can_reopen_without_launching_the_renderer() {
        let directory =
            std::env::temp_dir().join(format!("runetex-page-cache-{}", temporary_tag()));
        fs::create_dir(&directory).unwrap();
        let cached = directory.join("page-37.png");
        image::RgbaImage::from_pixel(2, 3, image::Rgba([20, 40, 60, 255]))
            .save(&cached)
            .unwrap();
        let original = fs::read(&cached).unwrap();
        let modified = fs::metadata(&cached).unwrap().modified().unwrap();
        let document = Document {
            pdf_path: directory.join("unavailable.pdf"),
            pages: vec![
                PageSize {
                    width: 2.0,
                    height: 3.0
                };
                37
            ],
            renderer: directory.join("unavailable-renderer"),
            cache_dir: directory.clone(),
        };
        assert_eq!(render_page(&document, 36).unwrap().size(), (2, 3));
        assert_eq!(fs::read(&cached).unwrap(), original);
        assert_eq!(fs::metadata(&cached).unwrap().modified().unwrap(), modified);
        fs::remove_file(cached).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
