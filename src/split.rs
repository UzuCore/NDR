//! File split and join of C# FileSystemService (SplitFile, WriteSplitMetadata, FindNumberedParts, HasContinuousNumberedParts,
//! SuggestJoinedFileName, JoinOrderedFiles) and SplitFileWindow.TryParseSize / SelectDefaultSplitSize.
use crate::names;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const BUFFER: usize = 131_072;

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { Err("Cancelled".into()) } else { Ok(()) }
}

fn temporary_suffix() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:016x}{:08x}", nanos as u64, std::process::id())
}

/// Original FUN_0040d86c (split size cbSplitSizeChange/miSplitFileClick, archive volume miArcAddToArchiveClick): the text upper-cased; the unit
/// is 2^30 if it contains "GB", else 2^20 for "MB", else 1024 for "KB", else 1 (bytes); the number is the text before the first space or
/// letter read by StrToInt64ExW (leading white space, optional sign, decimal digits up to the first other character; no digits = 0),
/// times the unit. Fixed: the original copied the text into a 32-character buffer without a length check.
pub fn original_size_value(text: &str) -> i64 {
    let upper = text.to_uppercase();
    let unit: i64 = if upper.contains("GB") {
        1 << 30
    } else if upper.contains("MB") {
        1 << 20
    } else if upper.contains("KB") {
        1024
    } else {
        1
    };
    let end = upper.find(|c: char| c == ' ' || c.is_alphabetic()).unwrap_or(upper.len());
    let number = upper[..end].trim_start();
    let (negative, digits) = match number.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, number.strip_prefix('+').unwrap_or(number)),
    };
    let digits: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return 0;
    }
    let value = digits.bytes().fold(0i64, |v, d| v.wrapping_mul(10).wrapping_add(i64::from(d - b'0')));
    (if negative { value.wrapping_neg() } else { value }).wrapping_mul(unit)
}

/// Split size: the original rule (`original_size_value`). Fixed: a value of 0 or less was passed on to the splitter; it is rejected here.
pub fn parse_split_size(text: &str) -> Option<u64> {
    u64::try_from(original_size_value(text)).ok().filter(|&size| size > 0)
}

/// Original TfFileSplit cbSplitSize Items (DFM).
pub const SPLIT_SIZE_PRESETS: [&str; 7] = ["1440 KB", "2880 KB", "5 MB", "10 MB", "20 MB", "50 MB", "100 MB"];

/// Original TntFormShow (0x4236e4): Text "720 KB" (0x8e2792) below 1440 KB, otherwise ItemIndex 0/2/3/4/5/6 below 5/20/50/100/200 MB and above,
/// then cbSplitSizeChange (0x423958) fills the piece count.
pub fn default_split_size(length: u64) -> &'static str {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    if length < 1440 * KIB {
        return "720 KB";
    }
    SPLIT_SIZE_PRESETS[match length {
        l if l < 5 * MIB => 0,
        l if l < 20 * MIB => 2,
        l if l < 50 * MIB => 3,
        l if l < 100 * MIB => 4,
        l if l < 200 * MIB => 5,
        _ => 6,
    }]
}

/// Piece count (at least 1).
pub fn part_count(length: u64, part_size: u64) -> u64 {
    length.div_ceil(part_size.max(1)).max(1)
}

/// C# FileSplitResult.
#[derive(Debug, Default, PartialEq)]
pub struct SplitResult {
    pub parts: Vec<PathBuf>,
    pub source_size: u64,
    /// Upper-case hexadecimal without leading zeros (.NET "X"). Empty text when no pieces were made.
    pub crc32: String,
}

/// Original FUN_0043b8b8: save folder + PathRemoveExtensionW(file name). The whole name when the result is empty (".bashrc").
pub fn split_prefix(destination: &Path, file_name: &str) -> PathBuf {
    let stem = names::remove_extension(file_name);
    destination.join(if stem.is_empty() { file_name } else { &stem })
}

/// C# SplitFile: makes nothing when the source is at most the piece size. Fails before writing if any piece to create ("{prefix}.001"...) already exists
/// (approved fix). Pieces are written under temp names then moved; on failure created pieces are deleted. Progress is (pieces done, bytes written).
pub fn split_file(path: &Path, part_size: u64, destination: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, &Path)) -> Result<SplitResult, String> {
    if part_size == 0 {
        return Err("Part size must be greater than zero".into());
    }
    let source = std::path::absolute(path).map_err(|e| e.to_string())?;
    let destination = std::path::absolute(destination).map_err(|e| e.to_string())?;
    if !destination.is_dir() {
        return Err(format!("Split destination folder does not exist: {}", destination.display()));
    }
    let file_name = source.file_name().ok_or("Missing file name")?.to_string_lossy().into_owned();
    let prefix = split_prefix(&destination, &file_name).to_string_lossy().into_owned();
    let mut input = File::open(&source).map_err(|e| e.to_string())?;
    let length = input.metadata().map_err(|e| e.to_string())?.len();
    if length <= part_size {
        return Ok(SplitResult { parts: vec![], source_size: length, crc32: String::new() });
    }
    let count = part_count(length, part_size);
    for number in 1..=count {
        let planned = format!("{prefix}.{number:03}");
        if Path::new(&planned).exists() {
            return Err(format!("A split output part with this file name already exists: {planned}"));
        }
    }
    let mut created: Vec<PathBuf> = Vec::new();
    let mut hasher = crc32fast::Hasher::new();
    let mut buffer = vec![0u8; BUFFER];
    let mut written = 0u64;
    let result = (|| {
        for number in 1..=count {
            cancelled(cancel)?;
            let part = PathBuf::from(format!("{prefix}.{number:03}"));
            let temporary = PathBuf::from(format!("{}.{}.tmp", part.display(), temporary_suffix()));
            let outcome = (|| {
                let mut target = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|e| e.to_string())?;
                let mut remaining = part_size.min(length - written);
                while remaining > 0 {
                    cancelled(cancel)?;
                    let want = remaining.min(BUFFER as u64) as usize;
                    let read = input.read(&mut buffer[..want]).map_err(|e| e.to_string())?;
                    if read == 0 {
                        return Err("The source file changed while it was being split.".to_string());
                    }
                    target.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
                    hasher.update(&buffer[..read]);
                    remaining -= read as u64;
                    written += read as u64;
                    progress(created.len() as u64, written, &part);
                }
                target.sync_all().map_err(|e| e.to_string())?;
                drop(target);
                fs::rename(&temporary, &part).map_err(|e| e.to_string())
            })();
            let _ = fs::remove_file(&temporary);
            outcome?;
            created.push(part.clone());
            progress(created.len() as u64, written, &part);
        }
        cancelled(cancel)
    })();
    if let Err(error) = result {
        for part in &created {
            let _ = fs::remove_file(part);
        }
        return Err(error);
    }
    Ok(SplitResult { parts: created, source_size: length, crc32: format!("{:X}", hasher.finalize()) })
}

/// Bytes in the ANSI code page (CP_ACP) when convertible, None when some character cannot be converted.
pub fn ansi_bytes(text: &str) -> Option<Vec<u8>> {
    use windows_sys::Win32::Globalization::{CP_ACP, WideCharToMultiByte};
    const WC_NO_BEST_FIT_CHARS: u32 = 0x400;
    let wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() {
        return Some(vec![]);
    }
    let mut used_default = 0i32;
    // SAFETY: the first call measures, the second fills a buffer of that size.
    unsafe {
        let size = WideCharToMultiByte(CP_ACP, WC_NO_BEST_FIT_CHARS, wide.as_ptr(), wide.len() as i32, std::ptr::null_mut(), 0, std::ptr::null(), &mut used_default);
        if size <= 0 || used_default != 0 {
            return None;
        }
        let mut out = vec![0u8; size as usize];
        WideCharToMultiByte(CP_ACP, WC_NO_BEST_FIT_CHARS, wide.as_ptr(), wide.len() as i32, out.as_mut_ptr(), size, std::ptr::null(), &mut used_default);
        (used_default == 0).then_some(out)
    }
}

pub(crate) fn ansi_text(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar};
    if bytes.is_empty() {
        return String::new();
    }
    // SAFETY: measured then filled buffer.
    unsafe {
        let size = MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0);
        let mut out = vec![0u16; size.max(0) as usize];
        MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), bytes.len() as i32, out.as_mut_ptr(), size);
        String::from_utf16_lossy(&out)
    }
}

/// Path.ChangeExtension(first piece, ".crc").
fn metadata_path(first_part: &Path) -> PathBuf {
    first_part.with_extension("crc")
}

/// C# WriteSplitMetadata (success path of original FUN_0046431c): three lines "filename=, size=, crc32=" (CRLF) in ANSI, or UTF-8 BOM when the name cannot be written in ANSI
/// (approved fix). The size uses the full 64 bits (approved fix). Not written when the .crc path is the source file. Returns the written path or None.
pub fn write_split_metadata(first_part: &Path, source: &Path, source_size: u64, crc32: &str) -> Result<Option<PathBuf>, String> {
    let path = std::path::absolute(metadata_path(first_part)).map_err(|e| e.to_string())?;
    let source_full = std::path::absolute(source).map_err(|e| e.to_string())?;
    if names::equals_ignore_case(&path.to_string_lossy(), &source_full.to_string_lossy()) {
        return Ok(None);
    }
    let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let text = format!("filename={name}\r\nsize={source_size}\r\ncrc32={crc32}\r\n");
    let bytes = ansi_bytes(&text).unwrap_or_else(|| [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat());
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(Some(path))
}

fn numbered_suffix(name: &str) -> Option<(&str, &str, bool)> {
    // ".partN" (case-insensitive) or ".NNN" (three or more digits).
    let lower = name.to_ascii_lowercase();
    if let Some(at) = lower.rfind(".part") {
        let digits = &name[at + 5..];
        if at > 0 && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            return Some((&name[..at], digits, false));
        }
    }
    let at = name.rfind('.')?;
    let digits = &name[at + 1..];
    (at > 0 && digits.len() >= 3 && digits.chars().all(|c| c.is_ascii_digit())).then_some((&name[..at], digits, true))
}

/// C# FindNumberedParts: for "name.partN" every .partN with the same prefix, for "name.001" every number of three or more digits with the same prefix. In number order.
/// Starting from an old-style piece other than 001, such as ".002", finds nothing.
pub fn find_numbered_parts(part: &Path) -> Vec<PathBuf> {
    let Some(name) = part.file_name().map(|n| n.to_string_lossy().into_owned()) else { return vec![] };
    let Some((prefix, digits, legacy)) = numbered_suffix(&name) else { return vec![] };
    if legacy && digits != "001" {
        return vec![];
    }
    let Some(directory) = std::path::absolute(part).ok().and_then(|p| p.parent().map(Path::to_path_buf)).filter(|d| d.is_dir()) else { return vec![] };
    let mut found: Vec<(u128, PathBuf)> = fs::read_dir(&directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| {
            let other = entry.file_name().to_string_lossy().into_owned();
            let (other_prefix, number, other_legacy) = numbered_suffix(&other)?;
            (other_legacy == legacy && names::equals_ignore_case(other_prefix, prefix)).then(|| Some((number.parse::<u128>().ok()?, entry.path()))).flatten()
        })
        .collect();
    found.sort_by(|(a, pa), (b, pb)| a.cmp(b).then_with(|| pa.to_string_lossy().to_lowercase().cmp(&pb.to_string_lossy().to_lowercase())));
    found.into_iter().map(|(_, path)| path).collect()
}

/// C# HasContinuousNumberedParts: whether the numbers run from 1 without gaps.
pub fn has_continuous_numbered_parts(parts: &[PathBuf]) -> bool {
    !parts.is_empty()
        && parts.iter().enumerate().all(|(index, path)| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .and_then(|name| numbered_suffix(&name).and_then(|(_, digits, _)| digits.parse::<u128>().ok()))
                == Some(index as u128 + 1)
        })
}

/// TryReadSplitMetadataFileName: the "filename=" value (file name part) of the .crc next to the first piece.
fn metadata_file_name(first_part: &Path) -> Option<String> {
    let bytes = fs::read(metadata_path(first_part)).ok()?;
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        String::from_utf16_lossy(&rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        String::from_utf16_lossy(&rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>())
    } else {
        String::from_utf8(bytes.clone()).unwrap_or_else(|_| ansi_text(&bytes))
    };
    text.split('\n').find_map(|line| {
        let value = line.get(..9).filter(|head| head.eq_ignore_ascii_case("filename=")).map(|_| &line[9..])?;
        let value = value.trim_end_matches(['\r', '\0']);
        let name = names::find_file_name(value);
        (!name.trim().is_empty()).then_some(name)
    })
}

/// Original miJoinFileClick (0x43bec8): the first file with its extension renamed to ".crc" gives the "filename" value; otherwise (".001" header,
/// FUN_0043bd20) the first file's name without its extension (PathRemoveExtension + PathFindFileName). ".partN" (C#) gives the prefix.
/// Previously a plain file proposed the C# "name.joined.ext".
pub fn suggest_joined_file_name(input: &Path) -> String {
    let name = input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match numbered_suffix(&name) {
        Some((prefix, _, false)) => prefix.to_owned(),
        Some((prefix, _, true)) => metadata_file_name(input).unwrap_or_else(|| prefix.to_owned()),
        // The original reads "<first file>.crc" for any first file (PathRenameExtension + PathFileExists), not only for ".NNN" pieces.
        None => metadata_file_name(input).unwrap_or_else(|| {
            let extension = crate::parity::dotnet_extension(&name);
            name[..name.len() - extension.len()].to_owned()
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextKind {
    Ascii,
    Utf8,
    Utf16Le,
    Utf16Be,
}

/// LooksLikeLegacyUtf8: whether, before the first NUL, the first non-ASCII is a 3-byte UTF-8 character that is a 2-byte (high bit) character in ANSI.
/// Original FUN_00465338 (lstrlenA, 0xE0/0x80 masks, WideCharToMultiByte CP_ACP, & 0x8080).
fn looks_like_legacy_utf8(sample: &[u8]) -> bool {
    let sample = &sample[..sample.iter().position(|b| *b == 0).unwrap_or(sample.len())];
    let first = sample.iter().position(|b| *b > 0x7F).unwrap_or(sample.len());
    if sample.len() - first < 3 {
        return false;
    }
    let (a, b, c) = (sample[first], sample[first + 1], sample[first + 2]);
    if a & 0xF0 != 0xE0 || b & 0xC0 != 0x80 || c & 0xC0 != 0x80 {
        return false;
    }
    let code = (u32::from(a & 0x0F) << 12) | (u32::from(b & 0x3F) << 6) | u32::from(c & 0x3F);
    let Some(ch) = char::from_u32(code) else { return false };
    ansi_bytes(&ch.to_string()).is_some_and(|encoded| encoded.len() == 2 && encoded[0] & 0x80 != 0 && encoded[1] & 0x80 != 0)
}

/// InspectLegacyTextFile: looks at the first 4096 bytes for a BOM (followed by at least one byte) or a UTF-8 shape. (kind, BOM length).
/// Original FUN_0046541c: at most 0x1000 bytes; FF FE / FE FF with 3+ bytes = Unicode, EF BB BF with 4+ bytes = UTF-8, otherwise FUN_00465338 (the
/// legacy UTF-8 shape) = UTF-8, else the default code page.
fn inspect_text(path: &Path) -> Result<(TextKind, usize), String> {
    let mut sample = vec![0u8; 4096];
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut length = 0;
    while length < sample.len() {
        let read = file.read(&mut sample[length..]).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        length += read;
    }
    let bytes = &sample[..length];
    Ok(if bytes.len() >= 3 && bytes[..2] == [0xFF, 0xFE] {
        (TextKind::Utf16Le, 2)
    } else if bytes.len() >= 3 && bytes[..2] == [0xFE, 0xFF] {
        (TextKind::Utf16Be, 2)
    } else if bytes.len() >= 4 && bytes[..3] == [0xEF, 0xBB, 0xBF] {
        (TextKind::Utf8, 3)
    } else if looks_like_legacy_utf8(bytes) {
        (TextKind::Utf8, 0)
    } else {
        (TextKind::Ascii, 0)
    })
}

fn decode(bytes: &[u8], kind: TextKind) -> String {
    match kind {
        TextKind::Ascii => bytes.iter().map(|b| if *b < 0x80 { *b as char } else { '?' }).collect(),
        TextKind::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        TextKind::Utf16Le => String::from_utf16_lossy(&bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()),
        TextKind::Utf16Be => String::from_utf16_lossy(&bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>()),
    }
}

fn encode(text: &str, kind: TextKind) -> Vec<u8> {
    match kind {
        TextKind::Ascii => text.chars().map(|c| if (c as u32) < 0x80 { c as u8 } else { b'?' }).collect(),
        TextKind::Utf8 => text.as_bytes().to_vec(),
        _ => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
    }
}

/// StreamReader.ReadLine: splits at "\r\n", "\r", "\n" with no trailing empty line.
fn read_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find(['\r', '\n']) {
            Some(at) => {
                lines.push(&rest[..at]);
                let skip = if rest[at..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[at + skip..];
            }
            None => {
                lines.push(rest);
                break;
            }
        }
    }
    lines
}

/// C# JoinOrderedFiles: concatenates the inputs in order into a temp file, then moves it to the result name (overwrite only with explicit consent).
/// Joining text files: rewrites every line with CRLF; the output encoding is the last file's (UTF-16BE becomes UTF-16LE), except that when a non-ASCII file exists
/// and the last one is ASCII, UTF-16LE. The BOM follows the output encoding (none for ASCII). Progress is (files done, bytes written).
/// Original text join FUN_00465588: each file goes through TStringList.LoadFromFile with the encoding from FUN_0046541c and AddStrings, then
/// SaveToFile with the last encoding, or TEncoding.Unicode when some file was not the default and the last one is. Fixed: FUN_0046541c gives
/// FF FE and FE FF the same encoding (FUN_00523990, little-endian Unicode), so a big-endian file comes out garbled; NDR decodes it as UTF-16BE.
pub fn join_ordered_files(inputs: &[PathBuf], output: &Path, join_text: bool, allow_overwrite: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, &Path)) -> Result<PathBuf, String> {
    if inputs.is_empty() {
        return Err("합칠 입력 파일이 없습니다.".into());
    }
    let sources: Vec<PathBuf> = inputs.iter().map(|p| std::path::absolute(p).unwrap_or_else(|_| p.clone())).collect();
    if sources.iter().any(|p| !p.is_file()) {
        return Err("입력 파일을 찾을 수 없습니다.".into());
    }
    let destination = std::path::absolute(output).map_err(|e| e.to_string())?;
    if sources.iter().any(|p| names::equals_ignore_case(&p.to_string_lossy(), &destination.to_string_lossy())) {
        return Err("입력 파일을 결과 파일로 덮어쓸 수 없습니다.".into());
    }
    let directory = destination.parent().filter(|d| d.is_dir()).ok_or("결과 폴더를 찾을 수 없습니다.")?.to_path_buf();
    let kinds = if join_text { Some(sources.iter().map(|s| inspect_text(s)).collect::<Result<Vec<_>, _>>()?) } else { None };
    let file_name = destination.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = directory.join(format!(".{file_name}.{}.tmp", temporary_suffix()));
    let result = (|| {
        let mut out = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|e| e.to_string())?;
        let mut written = 0u64;
        if let Some(kinds) = &kinds {
            let non_default = kinds.iter().any(|(k, _)| *k != TextKind::Ascii);
            let last = kinds.last().map(|(k, _)| *k).unwrap_or(TextKind::Ascii);
            let output_kind = match (non_default, last) {
                (true, TextKind::Ascii) => TextKind::Utf16Le,
                (_, TextKind::Utf16Be) => TextKind::Utf16Le,
                (_, kind) => kind,
            };
            let preamble: &[u8] = match output_kind {
                TextKind::Ascii => &[],
                TextKind::Utf8 => &[0xEF, 0xBB, 0xBF],
                _ => &[0xFF, 0xFE],
            };
            out.write_all(preamble).map_err(|e| e.to_string())?;
            written += preamble.len() as u64;
            for (index, (source, (kind, bom))) in sources.iter().zip(kinds).enumerate() {
                cancelled(cancel)?;
                let bytes = fs::read(source).map_err(|e| e.to_string())?;
                let text = decode(&bytes[(*bom).min(bytes.len())..], *kind);
                for line in read_lines(&text) {
                    let encoded = encode(&format!("{line}\r\n"), output_kind);
                    out.write_all(&encoded).map_err(|e| e.to_string())?;
                    written += encoded.len() as u64;
                }
                progress(index as u64 + 1, written, source);
            }
        } else {
            // Original join engine FUN_00464d2c: opens the result with CREATE_ALWAYS and copies each piece in order (progress kinds 4/3 = totals,
            // 0 = next piece, 1 = bytes, 2 = done; cancel = 0x4c7). Fixed: when the first piece ends in ".001" it reads its first 256 bytes and,
            // if the first '/' there follows a digit, skips every byte up to and including that '/' (FUN_008bc5a5 strchr 0x2f, SetFilePointer).
            // The original splitter (FUN_0046431c) writes no such header, so this cut real data (for example a text starting "2024/01/01");
            // NDR copies every byte.
            let mut buffer = vec![0u8; BUFFER];
            for (index, source) in sources.iter().enumerate() {
                let mut input = File::open(source).map_err(|e| e.to_string())?;
                loop {
                    cancelled(cancel)?;
                    let read = input.read(&mut buffer).map_err(|e| e.to_string())?;
                    if read == 0 {
                        break;
                    }
                    out.write_all(&buffer[..read]).map_err(|e| e.to_string())?;
                    written += read as u64;
                    progress(index as u64, written, source);
                }
                progress(index as u64 + 1, written, source);
            }
        }
        cancelled(cancel)?;
        out.sync_all().map_err(|e| e.to_string())?;
        drop(out);
        move_into_place(&temporary, &destination, allow_overwrite)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|_| destination)
}

/// File.Move(temp, destination, allowOverwrite): MoveFileExW (with or without REPLACE_EXISTING).
fn move_into_place(from: &Path, to: &Path, overwrite: bool) -> Result<(), String> {
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW};
    let wide = |p: &Path| p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (a, b) = (wide(from), wide(to));
    // SAFETY: both paths are NUL-terminated and live through the call.
    if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), if overwrite { MOVEFILE_REPLACE_EXISTING } else { 0 }) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ndr-split-{}-{}-{label}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn split_sizes_follow_the_original_reader() {
        assert_eq!(parse_split_size("1440 KB"), Some(1440 * 1024));
        assert_eq!(parse_split_size("5 mb"), Some(5 << 20), "upper-cased first");
        assert_eq!(parse_split_size("2.5 MB"), Some(2 << 20), "StrToInt64ExW stops at the dot");
        assert_eq!(parse_split_size("100"), Some(100), "no unit: bytes");
        assert_eq!(parse_split_size("1g"), Some(1), "a bare G is not a unit");
        assert_eq!(parse_split_size("1GB"), Some(1 << 30));
        assert_eq!(parse_split_size("10 bytes"), Some(10));
        assert_eq!(original_size_value("분할 안함"), 0);
        assert_eq!(original_size_value(" 5 MB"), 0, "a leading space ends the number at once");
        assert_eq!(original_size_value("-3 KB"), -3072);
        assert_eq!(parse_split_size("0 MB"), None);
        assert_eq!(parse_split_size("-3 KB"), None, "Fixed: the original passed it on");
        assert_eq!(parse_split_size("abc"), None);
        assert_eq!(default_split_size(1000), "720 KB");
        assert_eq!(default_split_size(3 * 1024 * 1024), "1440 KB");
        assert_eq!(default_split_size(30 * 1024 * 1024), "10 MB");
        assert_eq!(default_split_size(5 << 30), "100 MB");
        assert_eq!(part_count(10, 3), 4);
        assert_eq!(part_count(0, 3), 1);
    }

    #[test]
    fn split_and_join_round_trip_with_metadata() {
        let root = fixture("roundtrip");
        let source = root.join("data.bin");
        let payload: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&source, &payload).unwrap();
        let cancel = AtomicBool::new(false);
        let mut ticks = 0;
        let result = split_file(&source, 4096, &root, &cancel, &mut |_, _, _| ticks += 1).unwrap();
        assert_eq!(result.parts, vec![root.join("data.001"), root.join("data.002"), root.join("data.003")], "stem without the last extension");
        assert_eq!(result.crc32, format!("{:X}", crc32fast::hash(&payload)));
        assert!(ticks > 0);
        assert_eq!(write_split_metadata(&result.parts[0], &source, result.source_size, &result.crc32).unwrap(), Some(root.join("data.crc")));
        assert_eq!(fs::read_to_string(root.join("data.crc")).unwrap(), format!("filename=data.bin\r\nsize=10000\r\ncrc32={}\r\n", result.crc32));
        assert!(split_file(&source, 4096, &root, &cancel, &mut |_, _, _| {}).unwrap_err().contains("already exists"), "existing parts are refused before writing");
        assert!(split_file(&source, 20_000, &root, &cancel, &mut |_, _, _| {}).unwrap().parts.is_empty(), "smaller than one part: nothing");
        let parts = find_numbered_parts(&root.join("data.001"));
        assert_eq!(parts.len(), 3);
        assert!(has_continuous_numbered_parts(&parts));
        assert!(find_numbered_parts(&root.join("data.002")).is_empty(), "legacy parts must start at 001");
        assert_eq!(suggest_joined_file_name(&root.join("data.001")), "data.bin", "the .crc name");
        // The original reads "<first>.crc" for any first file: joining data.crc itself proposes data.bin, not "data".
        assert_eq!(suggest_joined_file_name(&root.join("data.crc")), "data.bin", "a first file with a .crc next to it");
        let joined = join_ordered_files(&parts, &root.join("out.bin"), false, false, &cancel, &mut |_, _, _| {}).unwrap();
        assert_eq!(fs::read(joined).unwrap(), payload);
        assert!(join_ordered_files(&parts, &root.join("out.bin"), false, false, &cancel, &mut |_, _, _| {}).is_err(), "never replaces without consent");
        assert!(join_ordered_files(&parts, &root.join("out.bin"), false, true, &cancel, &mut |_, _, _| {}).is_ok(), "replaces with consent");
        assert!(join_ordered_files(&parts, &parts[0], false, true, &cancel, &mut |_, _, _| {}).is_err(), "an input cannot be the output");
        fs::remove_file(root.join("data.002")).unwrap();
        assert!(!has_continuous_numbered_parts(&find_numbered_parts(&root.join("data.001"))));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn names_and_text_join_follow_the_csharp_rules() {
        let root = fixture("text");
        assert_eq!(split_prefix(&root, ".bashrc"), root.join(".bashrc"), "an empty stem keeps the whole name");
        assert_eq!(split_prefix(&root, "movie.part 1"), root.join("movie.part 1"), "an extension with a space is kept");
        assert_eq!(suggest_joined_file_name(Path::new("C:/x/movie.mkv.part3")), "movie.mkv");
        assert_eq!(suggest_joined_file_name(Path::new("C:/x/notes.txt")), "notes", "original: the name without its extension");
        assert_eq!(suggest_joined_file_name(Path::new("C:/x/README")), "README");
        fs::write(root.join("a.txt"), b"one\ntwo").unwrap();
        fs::write(root.join("b.txt"), b"three\r\n").unwrap();
        let cancel = AtomicBool::new(false);
        join_ordered_files(&[root.join("a.txt"), root.join("b.txt")], &root.join("ab.txt"), true, false, &cancel, &mut |_, _, _| {}).unwrap();
        assert_eq!(fs::read(root.join("ab.txt")).unwrap(), b"one\r\ntwo\r\nthree\r\n", "ASCII in, ASCII out, CRLF lines");
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend("가\n".encode_utf16().flat_map(u16::to_le_bytes));
        fs::write(root.join("c.txt"), utf16).unwrap();
        join_ordered_files(&[root.join("c.txt"), root.join("a.txt")], &root.join("ca.txt"), true, false, &cancel, &mut |_, _, _| {}).unwrap();
        let expected: Vec<u8> = [0xFF, 0xFE].into_iter().chain("가\r\none\r\ntwo\r\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(fs::read(root.join("ca.txt")).unwrap(), expected, "non-ASCII input with an ASCII last file: UTF-16LE");
        fs::remove_dir_all(root).unwrap();
    }
}
