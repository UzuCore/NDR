//! Local file tools. All writers stage output beside its target before commit.
use crate::fsops::{
    self, Entry, check_cancel, commit_file, ensure_plain_path, open_read, unique_path,
};
use sha2::Digest;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
/// C# ChecksumService.ComputeFileAsync: 한 번 읽어 CRC32(X8), MD5, SHA1, SHA256(대문자 16진).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checksums {
    pub crc32: String,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}
pub fn checksums(path: &Path, cancel: &AtomicBool) -> Result<Checksums, String> {
    use md5::Digest;
    let mut input = open_read(path)?;
    let (mut md5, mut sha1, mut sha256, mut crc) = (md5::Md5::new(), sha1::Sha1::new(), sha2::Sha256::new(), crc32fast::Hasher::new());
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        check_cancel(cancel)?;
        let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let data = &buffer[..n];
        md5.update(data);
        sha1.update(data);
        sha256.update(data);
        crc.update(data);
    }
    Ok(Checksums { crc32: format!("{:08X}", crc.finalize()), md5: format!("{:X}", md5.finalize()), sha1: format!("{:X}", sha1.finalize()), sha256: format!("{:X}", sha256.finalize()) })
}pub fn checksum(path: &Path, algorithm: &str) -> Result<String, String> {
    ensure_plain_path(path)?;
    let mut input = open_read(path)?;
    let mut md5 = md5::Md5::new();
    let mut sha1 = sha1::Sha1::new();
    let mut sha256 = sha2::Sha256::new();
    let mut crc = crc32fast::Hasher::new();
    let algo = algorithm.to_ascii_lowercase().replace('-', "");
    if !["md5", "sha1", "sha256", "crc32"].contains(&algo.as_str()) {
        return Err("Supported algorithms: CRC32, MD5, SHA1, SHA256".into());
    }
    let mut buf = vec![0; 1024 * 1024];
    loop {
        let n = input.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        let data = &buf[..n];
        match algo.as_str() {
            "md5" => md5.update(data),
            "sha1" => sha1.update(data),
            "sha256" => sha256.update(data),
            _ => crc.update(data),
        }
    }
    Ok(match algo.as_str() {
        "md5" => format!("{:X}", md5.finalize()),
        "sha1" => format!("{:X}", sha1.finalize()),
        "sha256" => format!("{:X}", sha256.finalize()),
        _ => format!("{:08X}", crc.finalize()),
    })
}
pub fn split_file(
    path: &Path,
    destination: &Path,
    part_size: u64,
    cancel: &AtomicBool,
) -> Result<Vec<PathBuf>, String> {
    if part_size == 0 {
        return Err("Part size must be greater than zero".into());
    }
    check_cancel(cancel)?;
    ensure_plain_path(path)?;
    ensure_plain_path(destination)?;
    fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let len = fs::metadata(path).map_err(|e| e.to_string())?.len();
    let count = len.div_ceil(part_size).max(1);
    if count > 100_000 {
        return Err("Too many split parts (limit 100000)".into());
    }
    let name = path
        .file_name()
        .ok_or("Missing file name")?
        .to_string_lossy();
    let outputs: Vec<_> = (1..=count)
        .map(|i| destination.join(format!("{name}.{i:03}")))
        .collect();
    if outputs.iter().any(|p| fs::symlink_metadata(p).is_ok()) {
        return Err("A split part already exists".into());
    }
    let mut staged = Vec::new();
    let mut committed = Vec::new();
    let result = (|| {
        let mut input = open_read(path)?;
        let mut buf = vec![0; 1024 * 1024];
        let mut total_read = 0u64;
        for target in &outputs {
            check_cancel(cancel)?;
            let stage = unique_path(destination, "split");
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)
                .map_err(|e| e.to_string())?;
            staged.push(stage.clone());
            let mut remaining = part_size;
            while remaining > 0 {
                check_cancel(cancel)?;
                let want = remaining.min(buf.len() as u64) as usize;
                let n = input.read(&mut buf[..want]).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                remaining -= n as u64;
                total_read += n as u64;
            }
            out.sync_all().map_err(|e| e.to_string())?;
            drop(out);
            check_cancel(cancel)?;
            commit_file(&stage, target, false)?;
            committed.push(target.clone());
        }
        if total_read != len {
            return Err("Input shrank while splitting".into());
        }
        if input.read(&mut [0; 1]).map_err(|e| e.to_string())? != 0 {
            return Err("Input grew while splitting".into());
        }
        Ok(outputs)
    })();
    for s in staged {
        let _ = fs::remove_file(s);
    }
    if result.is_err() {
        for p in committed {
            let _ = fs::remove_file(p);
        }
    }
    result
}
pub fn join_files(parts: &[PathBuf], output: &Path, cancel: &AtomicBool) -> Result<(), String> {
    if parts.is_empty() {
        return Err("No input parts".into());
    }
    check_cancel(cancel)?;
    ensure_plain_path(output)?;
    if output.exists() {
        return Err("Join output already exists".into());
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = unique_path(parent, "join");
    let result = (|| {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stage)
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0; 1024 * 1024];
        for p in parts {
            ensure_plain_path(p)?;
            let mut input = open_read(p)?;
            loop {
                check_cancel(cancel)?;
                let n = input.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            }
        }
        out.sync_all().map_err(|e| e.to_string())?;
        drop(out);
        check_cancel(cancel)?;
        commit_file(&stage, output, false)
    })();
    let _ = fs::remove_file(stage);
    result
}
fn contains(path: &Path, text: &str, cancel: &AtomicBool) -> Result<bool, String> {
    let mut needles = vec![
        text.as_bytes().to_vec(),
        text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    ];
    let (encoded, _, errors) = encoding_rs::EUC_KR.encode(text);
    if !errors {
        needles.push(encoded.into_owned())
    }
    let overlap = needles
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(1)
        .saturating_sub(1);
    let mut input = open_read(path)?;
    let mut buf = vec![0; 64 * 1024];
    let mut carry = Vec::new();
    loop {
        check_cancel(cancel)?;
        let n = input.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(false);
        }
        carry.extend_from_slice(&buf[..n]);
        if needles
            .iter()
            .any(|needle| carry.windows(needle.len()).any(|w| w == needle))
        {
            return Ok(true);
        }
        let keep = overlap.min(carry.len());
        carry.drain(..carry.len() - keep);
    }
}
pub fn search(
    root: &Path,
    pattern: &str,
    content: &str,
    recursive: bool,
    cancel: &AtomicBool,
) -> Result<Vec<Entry>, String> {
    ensure_plain_path(root)?;
    let mut masks = Vec::new();
    for mask in pattern
        .split([';', ',', '|'])
        .filter(|s| !s.trim().is_empty())
    {
        let mut expression = String::from("(?i)^");
        for c in mask.trim().chars() {
            match c {
                '*' => expression.push_str(".*"),
                '?' => expression.push('.'),
                _ => expression.push_str(&regex::escape(&c.to_string())),
            }
        }
        expression.push('$');
        masks.push(regex::Regex::new(&expression).map_err(|e| e.to_string())?)
    }
    let mut pending = vec![root.to_owned()];
    let mut found = Vec::new();
    while let Some(dir) = pending.pop() {
        check_cancel(cancel)?;
        for e in fsops::list_dir(&dir, true)? {
            check_cancel(cancel)?;
            let m = fs::symlink_metadata(&e.path).map_err(|e| e.to_string())?;
            if fsops::is_reparse(&m) {
                continue;
            }
            if e.is_dir && recursive {
                pending.push(e.path.clone())
            }
            if (masks.is_empty() || masks.iter().any(|m| m.is_match(&e.name)))
                && (content.is_empty() || (!e.is_dir && contains(&e.path, content, cancel)?))
            {
                found.push(e)
            }
        }
    }
    Ok(found)
}
fn inventory(root: &Path, cancel: &AtomicBool) -> Result<BTreeMap<PathBuf, (bool, u64)>, String> {
    ensure_plain_path(root)?;
    let mut result = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(p) = pending.pop() {
        check_cancel(cancel)?;
        let m = fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
        if fsops::is_reparse(&m) {
            return Err(format!("Reparse point cannot be compared: {}", p.display()));
        }
        let key = p.strip_prefix(root).map_err(|e| e.to_string())?.to_owned();
        if !key.as_os_str().is_empty() || m.is_file() {
            result.insert(key, (m.is_dir(), m.len()));
        }
        if m.is_dir() {
            for e in fs::read_dir(&p).map_err(|e| e.to_string())? {
                pending.push(e.map_err(|e| e.to_string())?.path());
            }
        }
    }
    Ok(result)
}
fn equal_bytes(a: &Path, b: &Path, cancel: &AtomicBool) -> Result<bool, String> {
    let mut a = open_read(a)?;
    let mut b = open_read(b)?;
    let mut x = vec![0; 1024 * 1024];
    let mut y = vec![0; 1024 * 1024];
    loop {
        check_cancel(cancel)?;
        let an = a.read(&mut x).map_err(|e| e.to_string())?;
        let mut bn = 0;
        while bn < an {
            let n = b.read(&mut y[bn..an]).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            bn += n;
        }
        if an != bn || x[..an] != y[..bn] {
            return Ok(false);
        }
        if an == 0 {
            return Ok(b.read(&mut [0; 1]).map_err(|e| e.to_string())? == 0);
        }
    }
}
/// Differences use stable human-readable prefixes: left-only, right-only, type, size, content.
pub fn compare(
    left: &Path,
    right: &Path,
    content: bool,
    cancel: &AtomicBool,
) -> Result<Vec<String>, String> {
    let a = inventory(left, cancel)?;
    let b = inventory(right, cancel)?;
    let mut differences = Vec::new();
    for (k, (dir, size)) in &a {
        check_cancel(cancel)?;
        let reason = match b.get(k) {
            None => Some("left-only"),
            Some((d, _)) if d != dir => Some("type"),
            Some((_, s)) if !dir && s != size => Some("size"),
            Some(_) if !dir && content && !equal_bytes(&left.join(k), &right.join(k), cancel)? => {
                Some("content")
            }
            _ => None,
        };
        if let Some(reason) = reason {
            differences.push(format!("{reason}: {}", k.display()))
        }
    }
    for k in b.keys() {
        if !a.contains_key(k) {
            differences.push(format!("right-only: {}", k.display()))
        }
    }
    Ok(differences)
}
/// Sum ordinary file sizes without following directory links.
pub fn directory_size(path: &Path, cancel: &AtomicBool) -> Result<u64, String> {
    inventory(path, cancel)?
        .values()
        .filter(|(dir, _)| !*dir)
        .try_fold(0u64, |total, (_, size)| {
            total
                .checked_add(*size)
                .ok_or("Directory size overflow".into())
        })
}
/// C# FolderSizeSummary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderSizeSummary {
    pub total_bytes: u64,
    pub files: u64,
    pub folders: u64,
    /// 읽지 못한 항목(최대 100개, "경로: 메시지").
    pub errors: Vec<String>,
}

/// C# FolderSizeService.CalculateAsync: 루트마다 재분석 지점은 건너뛰고, 파일 루트는 파일 하나, 폴더 루트는 폴더 하나로 센 뒤
/// 깊이 우선으로 내려간다(링크는 따라가지 않음). 오류는 100개까지 모은다. 취소되면 None.
pub fn folder_size_summary(roots: &[PathBuf], cancel: &AtomicBool) -> Option<FolderSizeSummary> {
    use std::os::windows::fs::MetadataExt;
    const REPARSE: u32 = 0x400;
    const DIRECTORY: u32 = 0x10;
    let mut summary = FolderSizeSummary::default();
    let error = |summary: &mut FolderSizeSummary, path: &Path, message: String| {
        if summary.errors.len() < 100 {
            summary.errors.push(format!("{}: {message}", path.display()));
        }
    };
    for root in roots {
        if check_cancel(cancel).is_err() {
            return None;
        }
        let metadata = match fs::symlink_metadata(root) {
            Ok(m) => m,
            Err(e) => {
                error(&mut summary, root, e.to_string());
                continue;
            }
        };
        let attributes = metadata.file_attributes();
        if attributes & REPARSE != 0 {
            continue;
        }
        if attributes & DIRECTORY == 0 {
            summary.total_bytes += metadata.len();
            summary.files += 1;
            continue;
        }
        summary.folders += 1;
        let mut pending = vec![root.clone()];
        while let Some(folder) = pending.pop() {
            if check_cancel(cancel).is_err() {
                return None;
            }
            let entries = match fs::read_dir(&folder) {
                Ok(entries) => entries,
                Err(e) => {
                    error(&mut summary, &folder, e.to_string());
                    continue;
                }
            };
            let mut children = Vec::new();
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(e) => {
                        error(&mut summary, &folder, e.to_string());
                        break;
                    }
                };
                let path = entry.path();
                let metadata = match entry.metadata() {
                    Ok(m) => m,
                    Err(e) => {
                        error(&mut summary, &path, e.to_string());
                        continue;
                    }
                };
                let attributes = metadata.file_attributes();
                if attributes & REPARSE != 0 {
                    continue;
                }
                if attributes & DIRECTORY != 0 {
                    summary.folders += 1;
                    children.push(path);
                } else {
                    summary.total_bytes += metadata.len();
                    summary.files += 1;
                }
            }
            // 스택이므로 거꾸로 넣어 목록 순서대로 내려간다.
            pending.extend(children.into_iter().rev());
        }
    }
    Some(summary)
}

/// C# FileAttributeChanges: 세 상태 속성(None = 그대로), 바꿀 수정 시각, 하위 폴더 포함, 폴더에도 적용.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttributeChanges {
    pub archive: Option<bool>,
    pub hidden: Option<bool>,
    pub system: Option<bool>,
    pub read_only: Option<bool>,
    pub modified: Option<std::time::SystemTime>,
    pub recursive: bool,
    pub apply_to_folders: bool,
}

/// C# FileSystemService.EnumerateSafeEntries: 깊이 우선(스택), 재분석 지점은 내보내지만 내려가지 않는다.
fn safe_entries(root: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).map_err(|e| format!("{}: {e}", folder.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", folder.display()))?;
            // DirEntry::metadata(Windows: FindNextFileW 값, 링크를 따라가지 않음) = symlink_metadata와 같은 뜻이고 파일마다 열지 않는다.
            let Ok(metadata) = entry.metadata() else { continue };
            let attributes = metadata.file_attributes();
            if attributes & 0x400 != 0 {
                continue;
            }
            out.push(entry.path());
            if attributes & 0x10 != 0 {
                pending.push(entry.path());
            }
        }
    }
    Ok(())
}

/// C# FileOperationService.ChangeAttributesAndTime: 대상(하위 폴더 포함이면 그 안 전부, 중복은 대소문자 무시로 한 번)마다
/// 속성 비트를 켜고 끈 뒤 수정 시각을 바꾼다. "폴더에도 적용"이 꺼져 있으면 폴더는 건너뛴다. 오류는 "경로: 메시지"로 모은다.
pub fn change_attributes_and_time(paths: &[PathBuf], changes: &AttributeChanges) -> Vec<String> {
    use std::os::windows::fs::MetadataExt;
    let mut errors = Vec::new();
    let mut targets = Vec::new();
    for path in paths {
        let Ok(metadata) = fs::symlink_metadata(path) else { continue };
        targets.push(path.clone());
        if changes.recursive && metadata.file_attributes() & (0x10 | 0x400) == 0x10 {
            if let Err(error) = safe_entries(path, &mut targets) {
                errors.push(error);
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for path in targets {
        if !seen.insert(path.to_string_lossy().to_lowercase()) {
            continue;
        }
        let result = (|| {
            let attributes = fs::metadata(&path).map_err(|e| e.to_string())?.file_attributes();
            if !changes.apply_to_folders && attributes & 0x10 != 0 {
                return Ok(());
            }
            let update = |value: u32, bit: u32, state: Option<bool>| match state {
                Some(true) => value | bit,
                Some(false) => value & !bit,
                None => value,
            };
            let mut value = attributes;
            value = update(value, 0x20, changes.archive);
            value = update(value, 0x2, changes.hidden);
            value = update(value, 0x4, changes.system);
            value = update(value, 0x1, changes.read_only);
            // SetFileAttributesW가 받지 않는 비트(폴더·재분석·압축·암호·희소 등)는 뺀다.
            let settable = value & !(0x10 | 0x40 | 0x200 | 0x400 | 0x800 | 0x4000 | 0x8000 | 0x40000 | 0x400000);
            set_raw_attributes(&path, if settable == 0 { 0x80 } else { settable })?;
            if let Some(modified) = changes.modified {
                set_write_time(&path, modified)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(message) = result {
            errors.push(format!("{}: {message}", path.display()));
        }
    }
    errors
}

/// .NET File/Directory.SetLastWriteTime: FILE_WRITE_ATTRIBUTES로만 열어 SetFileTime(읽기 전용 항목과 폴더도 된다).
fn set_write_time(path: &Path, time: std::time::SystemTime) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE}, Storage::FileSystem::*};
    let since = time.duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?;
    let ticks = since.as_nanos() / 100 + 116_444_736_000_000_000;
    let filetime = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: NUL-terminated path; the handle is closed before returning.
    unsafe {
        let handle = CreateFileW(wide.as_ptr(), FILE_WRITE_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, std::ptr::null(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, std::ptr::null_mut());
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let ok = SetFileTime(handle, std::ptr::null(), std::ptr::null(), &filetime);
        let error = std::io::Error::last_os_error();
        CloseHandle(handle);
        if ok == 0 { Err(error.to_string()) } else { Ok(()) }
    }
}

fn set_raw_attributes(path: &Path, bits: u32) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: NUL-terminated path for the duration of the call.
    if unsafe { windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(wide.as_ptr(), bits) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

/// Toggle only the read-only bit, preserving all other attributes.
pub fn set_readonly(path: &Path, readonly: bool) -> Result<(), String> {
    ensure_plain_path(path)?;
    let mut permissions = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    permissions.set_readonly(readonly);
    fs::set_permissions(path, permissions).map_err(|e| e.to_string())
}
/// Read native Windows attribute bits (hidden/system/archive/read-only etc.).
#[cfg(windows)]
pub fn attributes(path: &Path) -> Result<u32, String> {
    use std::os::windows::fs::MetadataExt;
    fs::symlink_metadata(path)
        .map(|m| m.file_attributes())
        .map_err(|e| e.to_string())
}
/// Change explicit Windows attribute bits; reparse points are always rejected.
#[cfg(windows)]
pub fn set_attributes(path: &Path, bits: u32) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    ensure_plain_path(path)?;
    if bits & !0x20a7 != 0 {
        return Err("Unsupported attribute bits".into());
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetFileAttributesW(path: *const u16, bits: u32) -> i32;
    }
    let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { SetFileAttributesW(wide.as_ptr(), if bits == 0 { 0x80 } else { bits }) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchCategory {
    #[default]
    All,
    Documents,
    Pictures,
    Audio,
    Video,
    Archives,
    Applications,
    Folders,
    Other,
}
#[derive(Debug, Clone, Copy, Default)]
pub enum SearchTimeKind {
    #[default]
    Any,
    Modified,
    Created,
    Accessed,
}
#[derive(Debug, Clone)]
pub struct SearchCriteria {
    pub roots: Vec<PathBuf>,
    pub name_patterns: String,
    pub extension_patterns: String,
    pub content: String,
    pub recursive: bool,
    pub include_hidden: bool,
    pub include_files: bool,
    pub include_folders: bool,
    pub match_case: bool,
    pub minimum_bytes: Option<u64>,
    pub maximum_bytes: Option<u64>,
    pub date_from: Option<std::time::SystemTime>,
    pub date_to: Option<std::time::SystemTime>,
    pub time_kind: SearchTimeKind,
    pub category: SearchCategory,
}
impl Default for SearchCriteria {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            name_patterns: "*".into(),
            extension_patterns: String::new(),
            content: String::new(),
            recursive: true,
            include_hidden: false,
            include_files: true,
            include_folders: false,
            match_case: false,
            minimum_bytes: None,
            maximum_bytes: None,
            date_from: None,
            date_to: None,
            time_kind: SearchTimeKind::Any,
            category: SearchCategory::All,
        }
    }
}
fn masks(patterns: &str, case: bool) -> Result<Vec<regex::Regex>, String> {
    patterns
        .split([';', ',', '|'])
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let mut r = if case {
                String::from("^")
            } else {
                String::from("(?i)^")
            };
            for c in p.trim().chars() {
                match c {
                    '*' => r.push_str(".*"),
                    '?' => r.push('.'),
                    _ => r.push_str(&regex::escape(&c.to_string())),
                }
            }
            r.push('$');
            regex::Regex::new(&r).map_err(|e| e.to_string())
        })
        .collect()
}
fn category_matches(category: SearchCategory, entry: &Entry) -> bool {
    if category == SearchCategory::All {
        return true;
    }
    if category == SearchCategory::Folders {
        return entry.is_dir;
    }
    if entry.is_dir {
        return false;
    }
    let ext = entry
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let groups = [
        (
            SearchCategory::Documents,
            "csv djvu doc docm docx dot dotm dotx epub html htm json key md numbers odf odg odp ods odt pages pdf pot potm potx ppt pptm pptx pps ppsm ppsx rtf tex txt xls xlsb xlsm xlsx xlt xltm xltx xml yaml yml",
        ),
        (
            SearchCategory::Pictures,
            "arw avif bmp cr2 cr3 dng emf gif heic heif ico jfi jfif jif jpe jpeg jpg jxl nef orf png psd raw rw2 svg tif tiff webp wmf xcf",
        ),
        (
            SearchCategory::Audio,
            "aac ac3 aif aiff alac amr ape dff dsf flac m4a m4b mid midi mka mp3 oga ogg opus wav wma",
        ),
        (
            SearchCategory::Video,
            "3g2 3gp asf avi f4v flv m2ts m4v mkv mov mp4 mpe mpeg mpg mxf ogv rm rmvb ts vob webm wmv",
        ),
        (
            SearchCategory::Archives,
            "7z ace alz arj br bz2 cab deb dmg egg esd gz iso lz lz4 lzh lzma lzo pkg rar rpm tar tgz wim xar xz z zip zst zstd",
        ),
        (
            SearchCategory::Applications,
            "appinstaller appx appxbundle bat cmd com cpl dll drv exe msi msix msixbundle msp msu ocx ps1 psd1 psm1 reg scr sys vbs wsf",
        ),
    ];
    let mut any = false;
    for (c, list) in groups {
        let found = list.split_whitespace().any(|e| e == ext);
        any |= found;
        if c == category {
            return found;
        }
    }
    category == SearchCategory::Other && !any
}
fn decode_chunks(
    path: &Path,
    cancel: &AtomicBool,
    mut consume: impl FnMut(&str) -> Result<bool, String>,
) -> Result<bool, String> {
    ensure_plain_path(path)?;
    let mut file = open_read(path)?;
    let mut buffer = vec![0; 64 * 1024];
    let first = file.read(&mut buffer).map_err(|e| e.to_string())?;
    let sample = &buffer[..first];
    let (encoding, skip) = if sample.starts_with(&[0xef, 0xbb, 0xbf]) {
        (encoding_rs::UTF_8, 3)
    } else if sample.starts_with(&[0xff, 0xfe]) {
        (encoding_rs::UTF_16LE, 2)
    } else if sample.starts_with(&[0xfe, 0xff]) {
        (encoding_rs::UTF_16BE, 2)
    } else {
        let utf8 = match std::str::from_utf8(sample) {
            Ok(_) => true,
            Err(e) => e.error_len().is_none(),
        };
        (
            if utf8 {
                encoding_rs::UTF_8
            } else {
                encoding_rs::EUC_KR
            },
            0,
        )
    };
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let mut pending = &buffer[skip..first];
    let mut eof = first == 0;
    loop {
        check_cancel(cancel)?;
        let mut text = String::with_capacity(pending.len() * 4 + 16);
        let (result, used, errors) = decoder.decode_to_string(pending, &mut text, eof);
        if errors {
            return Err(format!(
                "Invalid {} text in {}",
                encoding.name(),
                path.display()
            ));
        }
        if consume(&text)? {
            return Ok(true);
        }
        if used < pending.len() {
            pending = &pending[used..];
            continue;
        }
        if eof {
            return Ok(false);
        }
        if matches!(result, encoding_rs::CoderResult::OutputFull) {
            pending = &[];
            continue;
        }
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        pending = &buffer[..n];
        eof = n == 0;
    }
}
fn contains_text(
    path: &Path,
    text: &str,
    match_case: bool,
    cancel: &AtomicBool,
) -> Result<bool, String> {
    if match_case {
        return contains(path, text, cancel);
    }
    let needle = text.to_lowercase();
    let mut carry = String::new();
    decode_chunks(path, cancel, |chunk| {
        carry.push_str(&chunk.to_lowercase());
        if carry.contains(&needle) {
            return Ok(true);
        }
        let keep = needle.len().saturating_sub(1).min(carry.len());
        let mut start = carry.len() - keep;
        while !carry.is_char_boundary(start) {
            start += 1
        }
        carry.drain(..start);
        Ok(false)
    })
}
pub fn search_advanced(
    criteria: &SearchCriteria,
    cancel: &AtomicBool,
) -> Result<Vec<Entry>, String> {
    search_advanced_with_progress(criteria, cancel, &mut |_, _, _| {})
}
/// [`search_advanced`] that reports (items checked, matches, current folder) for each folder and every 256 items
/// (C# SearchProgress(Scanned, Matches, CurrentPath) for SearchProgressWindow).
pub fn search_advanced_with_progress(
    criteria: &SearchCriteria,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize, usize, &Path),
) -> Result<Vec<Entry>, String> {
    if criteria.roots.is_empty() {
        return Err("At least one search root is required".into());
    }
    if criteria
        .minimum_bytes
        .zip(criteria.maximum_bytes)
        .is_some_and(|(a, b)| a > b)
        || criteria
            .date_from
            .zip(criteria.date_to)
            .is_some_and(|(a, b)| a > b)
    {
        return Err("Search range minimum exceeds maximum".into());
    }
    let name_masks = masks(&criteria.name_patterns, criteria.match_case)?;
    let extensions = criteria
        .extension_patterns
        .split([';', ',', '|'])
        .filter(|e| !e.trim().is_empty())
        .map(|e| e.trim().trim_start_matches("*.").trim_start_matches('.'))
        .collect::<Vec<_>>()
        .join(";");
    let ext_masks = masks(&extensions, criteria.match_case)?;
    let mut pending = criteria.roots.clone();
    let mut visited = std::collections::HashSet::new();
    let mut found = Vec::new();
    let mut scanned = 0usize;
    while let Some(root) = pending.pop() {
        check_cancel(cancel)?;
        ensure_plain_path(&root)?;
        let canonical = fs::canonicalize(&root).map_err(|e| e.to_string())?;
        if !visited.insert(canonical) {
            continue;
        }
        progress(scanned, found.len(), &root);
        for entry in fsops::list_dir(&root, criteria.include_hidden)? {
            check_cancel(cancel)?;
            scanned += 1;
            if scanned & 255 == 0 {
                progress(scanned, found.len(), &root);
            }
            let metadata = fs::symlink_metadata(&entry.path).map_err(|e| e.to_string())?;
            if fsops::is_reparse(&metadata) {
                continue;
            }
            if entry.is_dir && criteria.recursive {
                pending.push(entry.path.clone())
            }
            if (entry.is_dir
                && !criteria.include_folders
                && criteria.category != SearchCategory::Folders)
                || (!entry.is_dir
                    && (!criteria.include_files || criteria.category == SearchCategory::Folders))
                || !category_matches(criteria.category, &entry)
                || (!name_masks.is_empty() && !name_masks.iter().any(|m| m.is_match(&entry.name)))
            {
                continue;
            }
            if !entry.is_dir {
                let ext = entry
                    .path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if !ext_masks.is_empty() && !ext_masks.iter().any(|m| m.is_match(ext)) {
                    continue;
                }
                if criteria.minimum_bytes.is_some_and(|n| entry.size < n)
                    || criteria.maximum_bytes.is_some_and(|n| entry.size > n)
                {
                    continue;
                }
            }
            if criteria.date_from.is_some() || criteria.date_to.is_some() {
                let times = match criteria.time_kind {
                    SearchTimeKind::Modified => vec![metadata.modified()],
                    SearchTimeKind::Created => vec![metadata.created()],
                    SearchTimeKind::Accessed => vec![metadata.accessed()],
                    SearchTimeKind::Any => {
                        vec![metadata.modified(), metadata.created(), metadata.accessed()]
                    }
                };
                if !times.into_iter().filter_map(Result::ok).any(|t| {
                    criteria.date_from.is_none_or(|d| t >= d)
                        && criteria.date_to.is_none_or(|d| t <= d)
                }) {
                    continue;
                }
            }
            if !criteria.content.is_empty()
                && (entry.is_dir
                    || !contains_text(&entry.path, &criteria.content, criteria.match_case, cancel)?)
            {
                continue;
            }
            found.push(entry);
        }
    }
    progress(scanned, found.len(), Path::new(""));
    Ok(found)
}
/// Return cleanup candidates only. The caller presents the list and chooses deletion.
pub fn cleanup_scan(
    root: &Path,
    patterns: &str,
    cancel: &AtomicBool,
) -> Result<Vec<Entry>, String> {
    if patterns.trim().is_empty() {
        return Err("Cleanup patterns must be explicit".into());
    }
    search_advanced(
        &SearchCriteria {
            roots: vec![root.to_owned()],
            name_patterns: patterns.into(),
            include_hidden: true,
            ..Default::default()
        },
        cancel,
    )
}
/// Decode UTF-8/UTF-16 BOM or Korean ANSI inputs, then write one selected encoding.
pub fn merge_text_files(
    parts: &[PathBuf],
    output: &Path,
    encoding: &str,
    separator: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if parts.is_empty() {
        return Err("No text inputs".into());
    }
    check_cancel(cancel)?;
    ensure_plain_path(output)?;
    if output.exists() {
        return Err("Merge output already exists".into());
    }
    let label = encoding.to_ascii_lowercase();
    let utf16le = matches!(label.as_str(), "utf-16" | "utf-16le");
    let utf16be = label == "utf-16be";
    let selected = if label == "utf-8-bom" {
        encoding_rs::UTF_8
    } else {
        encoding_rs::Encoding::for_label(label.as_bytes()).ok_or("Unknown output encoding")?
    };
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = unique_path(parent, "merge-text");
    let result = (|| {
        let mut writer = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&stage)
            .map_err(|e| e.to_string())?;
        if utf16le {
            writer.write_all(&[0xff, 0xfe]).map_err(|e| e.to_string())?
        } else if utf16be {
            writer.write_all(&[0xfe, 0xff]).map_err(|e| e.to_string())?
        } else if label == "utf-8-bom" {
            writer
                .write_all(&[0xef, 0xbb, 0xbf])
                .map_err(|e| e.to_string())?
        }
        let mut write_text = |text: &str| -> Result<bool, String> {
            let bytes = if utf16le {
                text.encode_utf16().flat_map(u16::to_le_bytes).collect()
            } else if utf16be {
                text.encode_utf16().flat_map(u16::to_be_bytes).collect()
            } else {
                let (bytes, _, errors) = selected.encode(text);
                if errors {
                    return Err("Text cannot be represented in the output encoding".into());
                }
                bytes.into_owned()
            };
            writer.write_all(&bytes).map_err(|e| e.to_string())?;
            Ok(false)
        };
        for (i, path) in parts.iter().enumerate() {
            if i > 0 {
                write_text(separator)?;
            }
            decode_chunks(path, cancel, &mut write_text)?;
        }
        writer.sync_all().map_err(|e| e.to_string())?;
        drop(writer);
        check_cancel(cancel)?;
        commit_file(&stage, output, false)
    })();
    let _ = fs::remove_file(stage);
    result
}
/// Copy and verify an ordinary disk-image file. Raw disks/volumes are deliberately rejected.
pub fn copy_image(source: &Path, output: &Path, cancel: &AtomicBool) -> Result<(), String> {
    for path in [source, output] {
        let raw = path.to_string_lossy().replace('/', "\\");
        if raw.starts_with("\\\\.\\") || raw.starts_with("\\\\?\\") || raw.ends_with(':') {
            return Err("Raw device image operations require a validated removable device and are unavailable in this file-copy API".into());
        }
        ensure_plain_path(path)?;
    }
    if !fs::metadata(source).map_err(|e| e.to_string())?.is_file() {
        return Err("Image source must be an ordinary file".into());
    }
    join_files(&[source.to_owned()], output, cancel)?;
    if !equal_bytes(source, output, cancel)? {
        return Err("Image verification failed; source may have changed".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;
    pub(super) fn fixture() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "ndr-tools-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn checksums_match_known_vectors() {
        let p = fixture();
        fs::write(p.join("a"), b"abc").unwrap();
        assert_eq!(
            checksum(&p.join("a"), "sha256").unwrap(),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
        assert_eq!(checksum(&p.join("a"), "crc32").unwrap(), "352441C2");
        assert!(checksum(&p.join("a"), "unknown").is_err());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn split_join_roundtrip_and_preserve_existing_output() {
        let p = fixture();
        let flag = AtomicBool::new(false);
        fs::write(p.join("a"), b"abcdefgh").unwrap();
        let parts = split_file(&p.join("a"), &p, 3, &flag).unwrap();
        assert_eq!(parts.len(), 3);
        join_files(&parts, &p.join("joined"), &flag).unwrap();
        assert_eq!(fs::read(p.join("joined")).unwrap(), b"abcdefgh");
        assert!(join_files(&parts, &p.join("joined"), &AtomicBool::new(true)).is_err());
        assert_eq!(fs::read(p.join("joined")).unwrap(), b"abcdefgh");
        assert!(split_file(&p.join("a"), &p, 0, &flag).is_err());
        assert!(split_file(&p.join("a"), &p, 3, &flag).is_err());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn search_and_compare_find_real_differences() {
        let p = fixture();
        fs::create_dir_all(p.join("left/sub")).unwrap();
        fs::create_dir_all(p.join("right/sub")).unwrap();
        let korean = "\u{D55C}\u{AD6D}\u{C5B4}";
        fs::write(p.join("left/sub/a.txt"), format!("hello {korean}")).unwrap();
        fs::write(p.join("right/sub/a.txt"), "hello english").unwrap();
        let flag = AtomicBool::new(false);
        let found = search(&p.join("left"), "*.txt", korean, true, &flag).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            compare(&p.join("left"), &p.join("right"), true, &flag)
                .unwrap()
                .len(),
            1
        );
        fs::remove_dir_all(p).unwrap();
    }
}
#[cfg(test)]
mod advanced_tests {
    use super::*;
    #[test]
    fn attribute_changes_follow_changeattributesandtime() {
        use std::os::windows::fs::MetadataExt;
        let p = super::tests::fixture();
        fs::create_dir_all(p.join("tree/sub")).unwrap();
        fs::write(p.join("tree/sub/a.txt"), b"a").unwrap();
        fs::write(p.join("b.txt"), b"b").unwrap();
        let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        let changes = AttributeChanges { read_only: Some(true), archive: Some(false), modified: Some(when), recursive: true, apply_to_folders: false, ..Default::default() };
        let errors = change_attributes_and_time(&[p.join("tree"), p.join("b.txt"), p.join("B.TXT")], &changes);
        assert!(errors.is_empty(), "{errors:?}");
        let attributes = |path: &Path| fs::metadata(path).unwrap().file_attributes();
        assert_eq!(attributes(&p.join("tree/sub/a.txt")) & 0x21, 0x1, "recursive: read-only on, archive off");
        assert_eq!(attributes(&p.join("b.txt")) & 0x21, 0x1);
        assert_eq!(attributes(&p.join("tree")) & 0x1, 0, "folders skipped when Apply to Folders is off");
        assert_eq!(fs::metadata(p.join("b.txt")).unwrap().modified().unwrap(), when);
        let clear = AttributeChanges { read_only: Some(false), recursive: true, apply_to_folders: true, ..Default::default() };
        assert!(change_attributes_and_time(&[p.join("tree"), p.join("b.txt")], &clear).is_empty());
        assert_eq!(attributes(&p.join("tree/sub/a.txt")) & 0x1, 0);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn folder_size_summary_counts_like_foldersizeservice() {
        let p = super::tests::fixture();
        fs::create_dir_all(p.join("a/b")).unwrap();
        fs::write(p.join("a/one.bin"), [0u8; 10]).unwrap();
        fs::write(p.join("a/b/two.bin"), [0u8; 5]).unwrap();
        fs::write(p.join("single.bin"), [0u8; 7]).unwrap();
        let cancel = AtomicBool::new(false);
        let summary = folder_size_summary(&[p.join("a"), p.join("single.bin"), p.join("missing")], &cancel).unwrap();
        assert_eq!((summary.total_bytes, summary.files, summary.folders), (22, 3, 2), "the root folder counts");
        assert_eq!(summary.errors.len(), 1, "a missing root is an error");
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(folder_size_summary(&[p.join("a")], &cancel).is_none());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn advanced_search_filters_size_case_extensions_and_category() {
        let p = super::tests::fixture();
        fs::write(p.join("Report.TXT"), "HELLO \u{D55C}\u{AE00}").unwrap();
        fs::write(p.join("small.txt"), "x").unwrap();
        fs::write(p.join("image.png"), "HELLO").unwrap();
        let mut criteria = SearchCriteria {
            roots: vec![p.clone()],
            minimum_bytes: Some(4),
            extension_patterns: "txt".into(),
            content: "hello \u{D55C}\u{AE00}".into(),
            category: SearchCategory::Documents,
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        assert_eq!(search_advanced(&criteria, &cancel).unwrap().len(), 1);
        criteria.match_case = true;
        assert!(search_advanced(&criteria, &cancel).unwrap().is_empty());
        criteria.minimum_bytes = Some(100);
        assert!(search_advanced(&criteria, &cancel).unwrap().is_empty());
        fs::write(p.join("x.tmp"), b"garbage").unwrap();
        assert_eq!(cleanup_scan(&p, "*.tmp;*.bak", &cancel).unwrap().len(), 1);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn text_merge_handles_mixed_bom_encoding_and_image_copy_rejects_device_paths() {
        let p = super::tests::fixture();
        fs::write(p.join("a"), b"\xef\xbb\xbfhello").unwrap();
        let mut b = vec![0xff, 0xfe];
        b.extend("\u{D55C}\u{AE00}".encode_utf16().flat_map(u16::to_le_bytes));
        fs::write(p.join("b"), b).unwrap();
        let cancel = AtomicBool::new(false);
        merge_text_files(
            &[p.join("a"), p.join("b")],
            &p.join("merged"),
            "utf-8",
            "\n",
            &cancel,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(p.join("merged")).unwrap(),
            "hello\n\u{D55C}\u{AE00}"
        );
        assert!(
            copy_image(
                Path::new("\\\\.\\PhysicalDrive0"),
                &p.join("image"),
                &cancel
            )
            .is_err()
        );
        copy_image(&p.join("merged"), &p.join("image"), &cancel).unwrap();
        assert_eq!(
            fs::read(p.join("image")).unwrap(),
            fs::read(p.join("merged")).unwrap()
        );
        fs::remove_dir_all(p).unwrap();
    }
}
