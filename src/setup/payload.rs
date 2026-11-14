use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path},
    sync::atomic::{AtomicBool, Ordering},
};

const MAGIC: &[u8; 16] = b"RUNETEX_SETUP_V1";
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;

#[derive(Clone, Deserialize)]
pub struct PayloadFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub offset: u64,
}

#[derive(Deserialize)]
pub struct PayloadManifest {
    pub app_id: String,
    pub version: String,
    pub files: Vec<PayloadFile>,
}

pub struct Payload {
    pub manifest: PayloadManifest,
    source: File,
    start: u64,
}

impl Payload {
    pub fn open_current() -> Result<Self, String> {
        let path = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::open(&path)
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let mut source = File::open(path).map_err(|e| e.to_string())?;
        let size = source.metadata().map_err(|e| e.to_string())?.len();
        let end = image_end(&mut source, size)?;
        // Authenticode may align the certificate table to eight bytes after the
        // payload. Only that alignment padding is allowed after our footer.
        let mut footer = None;
        for padding in 0..8u64 {
            if let Some(position) = end.checked_sub(40 + padding) {
                source
                    .seek(SeekFrom::Start(position))
                    .map_err(|e| e.to_string())?;
                let mut bytes = [0u8; 40];
                source.read_exact(&mut bytes).map_err(|e| e.to_string())?;
                if &bytes[..16] == MAGIC {
                    footer = Some((position, bytes));
                    break;
                }
            }
        }
        let (position, bytes) =
            footer.ok_or("설치 데이터가 없습니다. 배포용 설치 파일을 실행하세요")?;
        let start = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let manifest_start = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let manifest_length = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
        if start < 64
            || manifest_start < start
            || manifest_length == 0
            || manifest_length > MAX_MANIFEST
            || manifest_start.checked_add(manifest_length) != Some(position)
        {
            return Err("설치 데이터의 범위가 올바르지 않습니다".into());
        }
        source
            .seek(SeekFrom::Start(manifest_start))
            .map_err(|e| e.to_string())?;
        let mut json = vec![0; manifest_length as usize];
        source.read_exact(&mut json).map_err(|e| e.to_string())?;
        let manifest: PayloadManifest =
            serde_json::from_slice(&json).map_err(|e| format!("설치 목록 오류: {e}"))?;
        if manifest.app_id != "RuneTeX"
            || manifest.version != env!("CARGO_PKG_VERSION")
            || manifest.files.is_empty()
            || manifest.files.len() > 50_000
        {
            return Err("이 설치 프로그램과 설치 데이터의 버전이 일치하지 않습니다".into());
        }
        let mut names = HashSet::new();
        let mut offset = 0u64;
        for file in &manifest.files {
            if !safe_relative_path(&file.path)
                || !names.insert(file.path.to_lowercase())
                || file.path.eq_ignore_ascii_case("uninstall.exe")
                || file.path.eq_ignore_ascii_case(".runetex-install.json")
                || file.offset != offset
                || file.sha256.len() != 64
                || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(format!(
                    "설치 목록의 파일이 올바르지 않습니다: {}",
                    file.path
                ));
            }
            offset = offset
                .checked_add(file.size)
                .ok_or("설치 데이터 크기 오류")?;
        }
        if start.checked_add(offset) != Some(manifest_start)
            || ![
                "runetex.exe",
                "tools/texlab/texlab.exe",
                "tools/poppler/library/bin/pdftoppm.exe",
            ]
            .iter()
            .all(|name| names.contains(*name))
        {
            return Err("필수 실행 파일이 빠졌거나 설치 데이터가 손상되었습니다".into());
        }
        Ok(Self {
            manifest,
            source,
            start,
        })
    }

    pub fn copy_file(
        &mut self,
        file: &PayloadFile,
        destination: &Path,
        cancel: &AtomicBool,
        progress: impl Fn(u64),
    ) -> Result<(), String> {
        self.source
            .seek(SeekFrom::Start(self.start + file.offset))
            .map_err(|e| e.to_string())?;
        let mut output = File::create(destination).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut remaining = file.size;
        let mut buffer = [0u8; 64 * 1024];
        while remaining > 0 {
            if cancel.load(Ordering::Relaxed) {
                return Err("설치를 취소했습니다".into());
            }
            let count = remaining.min(buffer.len() as u64) as usize;
            self.source
                .read_exact(&mut buffer[..count])
                .map_err(|e| e.to_string())?;
            output
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            hash.update(&buffer[..count]);
            remaining -= count as u64;
            progress(count as u64);
        }
        if format!("{:x}", hash.finalize()) != file.sha256.to_ascii_lowercase() {
            return Err(format!("파일 무결성 확인 실패: {}", file.path));
        }
        output.sync_all().map_err(|e| e.to_string())
    }

    pub fn verify(&mut self) -> Result<(), String> {
        let mut buffer = [0u8; 64 * 1024];
        for file in &self.manifest.files {
            self.source
                .seek(SeekFrom::Start(self.start + file.offset))
                .map_err(|e| e.to_string())?;
            let mut hash = Sha256::new();
            let mut remaining = file.size;
            while remaining > 0 {
                let count = remaining.min(buffer.len() as u64) as usize;
                self.source
                    .read_exact(&mut buffer[..count])
                    .map_err(|e| e.to_string())?;
                hash.update(&buffer[..count]);
                remaining -= count as u64;
            }
            if format!("{:x}", hash.finalize()) != file.sha256.to_ascii_lowercase() {
                return Err(format!("파일 무결성 확인 실패: {}", file.path));
            }
        }
        Ok(())
    }
}

pub fn safe_relative_path(value: &str) -> bool {
    if value.is_empty()
        || value.contains('\\')
        || value
            .chars()
            .any(|c| c.is_control() || ":*?\"<>|".contains(c))
        || Path::new(value)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return false;
    }
    value.split('/').all(|part| {
        if part.is_empty() || part.ends_with(['.', ' ']) {
            return false;
        }
        let stem = part.split('.').next().unwrap().to_ascii_uppercase();
        !["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
            && !(stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    })
}

fn image_end(file: &mut File, size: u64) -> Result<u64, String> {
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).map_err(|e| e.to_string())?;
    if &dos[..2] != b"MZ" {
        return Err("Windows 실행 파일이 아닙니다".into());
    }
    let pe = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    if pe < 64 || pe.checked_add(24).is_none_or(|end| end > size) {
        return Err("실행 파일 헤더 오류".into());
    }
    file.seek(SeekFrom::Start(pe)).map_err(|e| e.to_string())?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"PE\0\0" {
        return Err("실행 파일 헤더 오류".into());
    }
    let optional_size = u16::from_le_bytes(header[20..22].try_into().unwrap()) as u64;
    if pe
        .checked_add(24 + optional_size)
        .is_none_or(|end| end > size)
    {
        return Err("실행 파일 헤더 오류".into());
    }
    let mut magic = [0u8; 2];
    file.read_exact(&mut magic).map_err(|e| e.to_string())?;
    let directories = match u16::from_le_bytes(magic) {
        0x20b => 112,
        0x10b => 96,
        _ => return Err("지원하지 않는 실행 파일 형식".into()),
    };
    if optional_size < directories + 5 * 8 {
        return Err("실행 파일 데이터 디렉터리 오류".into());
    }
    file.seek(SeekFrom::Start(pe + 24 + directories - 4))
        .map_err(|e| e.to_string())?;
    let mut count = [0u8; 4];
    file.read_exact(&mut count).map_err(|e| e.to_string())?;
    if u32::from_le_bytes(count) <= 4 {
        return Err("실행 파일 데이터 디렉터리 오류".into());
    }
    file.seek(SeekFrom::Start(pe + 24 + directories + 4 * 8))
        .map_err(|e| e.to_string())?;
    let mut certificate = [0u8; 8];
    file.read_exact(&mut certificate)
        .map_err(|e| e.to_string())?;
    let offset = u32::from_le_bytes(certificate[..4].try_into().unwrap()) as u64;
    let length = u32::from_le_bytes(certificate[4..].try_into().unwrap()) as u64;
    if offset == 0 && length == 0 {
        return Ok(size);
    }
    if offset < pe + 24 + optional_size
        || !offset.is_multiple_of(8)
        || length < 8
        || offset.checked_add(length) != Some(size)
    {
        return Err("실행 파일 서명 범위 오류".into());
    }
    Ok(offset)
}
