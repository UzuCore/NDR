//! C# FolderComparisonService (comparison and difference copy) and the result texts of FolderComparisonWindow (state, kind, size, date; TSV/HTML save).
use crate::listing::compare_text;
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DifferenceKind {
    Same,
    LeftOnly,
    RightOnly,
    Different,
}

/// C# FolderDifference (the date of a missing side is None).
#[derive(Clone, Debug, PartialEq)]
pub struct Difference {
    /// Left name (the right name when missing on the left). Sync and save use this path.
    pub relative: String,
    /// The name as in the right list (original lvList Columns[3]: the right list item). Equal to relative when missing on the right.
    pub right_relative: String,
    pub kind: DifferenceKind,
    pub left_size: u64,
    pub right_size: u64,
    pub left_modified: Option<SystemTime>,
    pub right_modified: Option<SystemTime>,
    pub left_dir: bool,
    pub right_dir: bool,
}

/// C# ExistingSyncFileBehavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExistingBehavior {
    Skip,
    Overwrite,
    OverwriteIfSourceIsNewer,
    OverwriteAndReplaceTypeConflicts,
}

/// C# FolderSyncResult.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncResult {
    pub copied: u64,
    pub created_directories: u64,
    pub skipped_existing: u64,
    pub errors: Vec<String>,
}

/// The three reports of C# FolderComparisonProgress(Message, Completed, Total). Text is [`CompareProgress::message`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareProgress {
    /// Index: every 256 items "searching {left/right folder} · N checked" (no total).
    Scanning { left: bool, scanned: usize },
    /// Index end: "{...} search done · N items" (scanned / scanned).
    Scanned { left: bool, count: usize, scanned: usize },
    /// Compare: "comparing files" (completed / total).
    Comparing { completed: usize, total: usize },
}

/// .NET "N0" (thousands separators).
fn n0(value: usize) -> String {
    crate::parity::format_size(value as u64, "B", true).trim_end_matches(" B").to_owned()
}

impl CompareProgress {
    pub fn message(&self, korean: bool) -> String {
        let label = |left: bool| match (left, korean) {
            (true, true) => "왼쪽 폴더",
            (false, true) => "오른쪽 폴더",
            (true, false) => "Left folder",
            (false, false) => "Right folder",
        };
        match (*self, korean) {
            (Self::Scanning { left, scanned }, true) => format!("{} 검색 중 · {}개 확인", label(left), n0(scanned)),
            (Self::Scanning { left, scanned }, false) => format!("Scanning {} · {} checked", label(left).to_lowercase(), n0(scanned)),
            (Self::Scanned { left, count, .. }, true) => format!("{} 검색 완료 · {}개 항목", label(left), n0(count)),
            (Self::Scanned { left, count, .. }, false) => format!("{} scanned · {} items", label(left), n0(count)),
            (Self::Comparing { .. }, true) => "파일 비교 중".into(),
            (Self::Comparing { .. }, false) => "Comparing files".into(),
        }
    }

    /// (done, total). An indeterminate progress bar without a total.
    pub fn counts(&self) -> (usize, Option<usize>) {
        match *self {
            Self::Scanning { scanned, .. } => (scanned, None),
            Self::Scanned { scanned, .. } => (scanned, Some(scanned)),
            Self::Comparing { completed, total } => (completed, Some(total)),
        }
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { Err("Cancelled".into()) } else { Ok(()) }
}

fn full(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn trimmed(path: &Path) -> String {
    let text = full(path).to_string_lossy().into_owned();
    if text.len() > 3 { text.trim_end_matches('\\').to_owned() } else { text }
}

fn is_within(root: &str, candidate: &str) -> bool {
    let prefix = format!("{}\\", root.trim_end_matches('\\'));
    candidate.to_lowercase().starts_with(&prefix.to_lowercase()) || candidate.eq_ignore_ascii_case(root.trim_end_matches('\\'))
}

/// EnsureDistinctRoots: Err when equal or nested.
pub fn ensure_distinct_roots(first: &Path, second: &Path) -> Result<(), String> {
    let (a, b) = (trimmed(first), trimmed(second));
    if crate::names::equals_ignore_case(&a, &b) || is_within(&a, &b) || is_within(&b, &a) {
        return Err("비교하거나 동기화할 두 폴더는 서로 같거나 포함 관계일 수 없습니다.".into());
    }
    Ok(())
}

struct Item {
    length: u64,
    modified: SystemTime,
    dir: bool,
}

/// Index: skips reparse points (not descending either), depth first when subfolders are included. Relative path -> entry, unreadable ones go to issues.
/// Reports progress every 256 checked entries (including errors) and at the end.
fn index(root: &Path, recursive: bool, left: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(CompareProgress)) -> Result<(HashMap<String, (String, Item)>, Vec<String>), String> {
    if !root.is_dir() {
        return Err(format!("폴더를 찾을 수 없습니다: {}", root.display()));
    }
    let mut result = HashMap::new();
    let mut issues = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut scanned = 0usize;
    while let Some(directory) = pending.pop() {
        cancelled(cancel)?;
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                issues.push(format!("{}: {error}", directory.display()));
                continue;
            }
        };
        for entry in entries {
            cancelled(cancel)?;
            scanned += 1;
            if scanned & 255 == 0 {
                progress(CompareProgress::Scanning { left, scanned });
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    issues.push(format!("{}: {error}", directory.display()));
                    continue;
                }
            };
            let path = entry.path();
            // DirEntry::metadata (on Windows the FindNextFileW values, not following links) means the same as symlink_metadata and does not open each file.
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(error) => {
                    issues.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            if metadata.file_attributes() & 0x400 != 0 {
                continue;
            }
            let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
            let dir = metadata.is_dir();
            result.insert(relative.to_lowercase(), (relative, Item { length: if dir { 0 } else { metadata.len() }, modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), dir }));
            if dir && recursive {
                pending.push(path);
            }
        }
    }
    progress(CompareProgress::Scanned { left, count: result.len(), scanned });
    Ok((result, issues))
}

fn contents_equal(left: &Path, right: &Path, cancel: &AtomicBool) -> std::io::Result<bool> {
    let (mut a, mut b) = (File::open(left)?, File::open(right)?);
    let (mut x, mut y) = (vec![0u8; 128 * 1024], vec![0u8; 128 * 1024]);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("Cancelled"));
        }
        let read = |file: &mut File, buffer: &mut [u8]| -> std::io::Result<usize> {
            let mut total = 0;
            while total < buffer.len() {
                let n = file.read(&mut buffer[total..])?;
                if n == 0 {
                    break;
                }
                total += n;
            }
            Ok(total)
        };
        let (n, m) = (read(&mut a, &mut x)?, read(&mut b, &mut y)?);
        if n != m {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
        if x[..n] != y[..m] {
            return Ok(false);
        }
    }
}

/// CompareAsync: the union of both sides' relative paths in culture order. Folders compare equal, files by size -> (content or modification time).
/// Stopping gives Err("Cancelled") (sync never copies from partial results).
pub fn compare(left: &Path, right: &Path, recursive: bool, contents: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(CompareProgress)) -> Result<(Vec<Difference>, Vec<String>), String> {
    match compare_partial(left, right, recursive, contents, cancel, progress)? {
        (_, _, true) => Err("Cancelled".into()),
        (results, issues, false) => Ok((results, issues)),
    }
}

/// Original btnOkClick (0x41d464): stopping (+0x410) inside the comparison loop keeps the rows inserted so far in lvList. (results, unreadable, whether stopped).
/// Stopping while reading the lists gives Err("Cancelled").
pub fn compare_partial(left: &Path, right: &Path, recursive: bool, contents: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(CompareProgress)) -> Result<(Vec<Difference>, Vec<String>, bool), String> {
    let (left, right) = (full(left), full(right));
    ensure_distinct_roots(&left, &right)?;
    let (a, issues_left) = index(&left, recursive, true, cancel, progress)?;
    let (b, issues_right) = index(&right, recursive, false, cancel, progress)?;
    let mut issues = [issues_left, issues_right].concat();
    let mut keys: Vec<(&String, &String)> = a.iter().map(|(k, (name, _))| (k, name)).collect();
    for (key, (name, _)) in &b {
        if !a.contains_key(key) {
            keys.push((key, name));
        }
    }
    keys.sort_by(|x, y| compare_text(x.1, y.1, false));
    let total = keys.len();
    let mut results = Vec::with_capacity(total);
    progress(CompareProgress::Comparing { completed: 0, total });
    for (index, (key, name)) in keys.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok((results, issues, true));
        }
        let right_name = b.get(key).map_or_else(|| name.clone(), |(n, _)| n.clone());
        let difference = match (a.get(key).map(|(_, i)| i), b.get(key).map(|(_, i)| i)) {
            (None, Some(r)) => Difference { relative: name.clone(), right_relative: right_name, kind: DifferenceKind::RightOnly, left_size: 0, right_size: r.length, left_modified: None, right_modified: Some(r.modified), left_dir: false, right_dir: r.dir },
            (Some(l), None) => Difference { relative: name.clone(), right_relative: right_name, kind: DifferenceKind::LeftOnly, left_size: l.length, right_size: 0, left_modified: Some(l.modified), right_modified: None, left_dir: l.dir, right_dir: false },
            (Some(l), Some(r)) => {
                let same = if l.dir != r.dir {
                    false
                } else if l.dir {
                    true
                } else if contents {
                    // Original FUN_0041cd10: different when the size (GetFileSize) differs, otherwise compares content in 0xffff chunks.
                    if l.length != r.length {
                        false
                    } else {
                        match contents_equal(&left.join(name), &right.join(name), cancel) {
                            Ok(same) => same,
                            Err(error) => {
                                if cancel.load(Ordering::Relaxed) {
                                    return Ok((results, issues, true));
                                }
                                issues.push(format!("{name}: 파일 내용을 비교하지 못했습니다. {error}"));
                                false
                            }
                        }
                    }
                } else {
                    // Original 0x41d464: with content comparison off, only xilesShell CompareFileTime (last modification time) == 0 is checked (size is not).
                    l.modified == r.modified
                };
                Difference { relative: name.clone(), right_relative: right_name, kind: if same { DifferenceKind::Same } else { DifferenceKind::Different }, left_size: l.length, right_size: r.length, left_modified: Some(l.modified), right_modified: Some(r.modified), left_dir: l.dir, right_dir: r.dir }
            }
            (None, None) => continue,
        };
        results.push(difference);
        if index & 63 == 0 || index + 1 == total {
            progress(CompareProgress::Comparing { completed: index + 1, total });
        }
    }
    Ok((results, issues, false))
}

/// Result window text (FolderComparisonRow).
pub fn status_text(kind: DifferenceKind, korean: bool) -> &'static str {
    match (kind, korean) {
        (DifferenceKind::Same, true) => "같음",
        (DifferenceKind::LeftOnly, true) => "왼쪽에만 있음",
        (DifferenceKind::RightOnly, true) => "오른쪽에만 있음",
        (DifferenceKind::Different, true) => "다름",
        (DifferenceKind::Same, false) => "Same",
        (DifferenceKind::LeftOnly, false) => "Left only",
        (DifferenceKind::RightOnly, false) => "Right only",
        (DifferenceKind::Different, false) => "Different",
    }
}

pub fn type_text(d: &Difference, korean: bool) -> &'static str {
    match (d.left_dir != d.right_dir && d.kind != DifferenceKind::LeftOnly && d.kind != DifferenceKind::RightOnly, d.left_dir || d.right_dir, korean) {
        (true, _, true) => "파일/폴더 충돌",
        (true, _, false) => "File/folder conflict",
        (false, true, true) => "폴더",
        (false, true, false) => "Folder",
        (false, false, true) => "파일",
        (false, false, false) => "File",
    }
}

pub fn size_text(modified: Option<SystemTime>, dir: bool, size: u64, korean: bool) -> String {
    match (modified, dir) {
        (None, _) => "—".into(),
        (Some(_), true) => if korean { "폴더" } else { "Folder" }.into(),
        (Some(_), false) => crate::parity::format_size(size, "Auto", true),
    }
}

pub fn time_text(modified: Option<SystemTime>) -> String {
    modified.map_or_else(|| "—".into(), |t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string())
}

fn escape_tsv(value: &str) -> String {
    if value.contains(['\t', '\r', '\n', '"']) { format!("\"{}\"", value.replace('"', "\"\"")) } else { value.to_owned() }
}

fn html(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// Headers of the result text: (select/copy, state, kind, relative path, left size, right size, left modified date, right modified date).
/// C# always uses Korean text. An English UI writes English (difference list of the handoff document).
pub fn column_titles(korean: bool) -> [&'static str; 8] {
    if korean {
        ["복사", "상태", "종류", "상대 경로", "왼쪽 크기", "오른쪽 크기", "왼쪽 수정 날짜", "오른쪽 수정 날짜"]
    } else {
        ["Copy", "Status", "Type", "Relative path", "Left size", "Right size", "Left modified", "Right modified"]
    }
}

fn yes_no(value: bool, korean: bool) -> &'static str {
    match (value, korean) {
        (true, true) => "예",
        (false, true) => "아니요",
        (true, false) => "Yes",
        (false, false) => "No",
    }
}

fn unreadable_heading(korean: bool) -> &'static str {
    if korean { "읽지 못한 항목" } else { "Unreadable items" }
}

/// TSV of SaveResults (every extension other than .html). The first column is "select" (yes/no).
pub fn report_tsv(rows: &[(Difference, bool)], issues: &[String], korean: bool) -> String {
    let titles = column_titles(korean);
    let mut out = format!("{}\t{}\r\n", if korean { "선택" } else { "Selected" }, titles[1..].join("\t"));
    for (d, include) in rows {
        let cells = [
            yes_no(*include, korean).to_owned(),
            escape_tsv(status_text(d.kind, korean)),
            escape_tsv(type_text(d, korean)),
            escape_tsv(&d.relative),
            escape_tsv(&size_text(d.left_modified, d.left_dir, d.left_size, korean)),
            escape_tsv(&size_text(d.right_modified, d.right_dir, d.right_size, korean)),
            escape_tsv(&time_text(d.left_modified)),
            escape_tsv(&time_text(d.right_modified)),
        ];
        out.push_str(&cells.join("\t"));
        out.push_str("\r\n");
    }
    if !issues.is_empty() {
        out.push_str(&format!("\r\n{}\r\n", unreadable_heading(korean)));
        for issue in issues {
            out.push_str(&escape_tsv(issue));
            out.push_str("\r\n");
        }
    }
    out
}

/// HTML of SaveResults.
pub fn report_html(title: &str, left: &str, right: &str, rows: &[(Difference, bool)], issues: &[String], select_for_sync: bool, korean: bool) -> String {
    let titles = column_titles(korean);
    let mut out = String::new();
    let mut line = |text: &str| {
        out.push_str(text);
        out.push_str("\r\n");
    };
    line("<!doctype html>");
    line(if korean { "<html lang=\"ko\">" } else { "<html lang=\"en\">" });
    line("<head>");
    line("<meta charset=\"utf-8\">");
    line("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">");
    line(&format!("<title>{}</title>", html(title)));
    line("<style>body{font:14px Segoe UI,Arial,sans-serif;margin:24px;color:#222}h1{font-size:22px}p{overflow-wrap:anywhere}table{border-collapse:collapse;width:100%;margin-top:16px}th,td{border:1px solid #bbb;padding:6px 8px;text-align:left;vertical-align:top}th{background:#edf1f5;position:sticky;top:0}tbody tr:nth-child(even){background:#f7f8fa}.path{overflow-wrap:anywhere}.issues{margin-top:24px}.issues li{overflow-wrap:anywhere}</style>");
    line("</head>");
    line("<body>");
    line(&format!("<h1>{}</h1>", html(title)));
    line(&format!("<p><strong>{}:</strong> {}</p>", if korean { "왼쪽" } else { "Left" }, html(left)));
    line(&format!("<p><strong>{}:</strong> {}</p>", if korean { "오른쪽" } else { "Right" }, html(right)));
    line("<table><thead><tr>");
    if select_for_sync {
        line(&format!("<th>{}</th>", titles[0]));
    }
    line(&format!("{}</tr></thead><tbody>", titles[1..].iter().map(|t| format!("<th>{t}</th>")).collect::<String>()));
    for (d, include) in rows {
        line("<tr>");
        if select_for_sync {
            line(&format!("<td>{}</td>", yes_no(*include, korean)));
        }
        line(&format!("<td>{}</td>", html(status_text(d.kind, korean))));
        line(&format!("<td>{}</td>", html(type_text(d, korean))));
        line(&format!("<td class=\"path\">{}</td>", html(&d.relative)));
        line(&format!("<td>{}</td>", html(&size_text(d.left_modified, d.left_dir, d.left_size, korean))));
        line(&format!("<td>{}</td>", html(&size_text(d.right_modified, d.right_dir, d.right_size, korean))));
        line(&format!("<td>{}</td>", html(&time_text(d.left_modified))));
        line(&format!("<td>{}</td>", html(&time_text(d.right_modified))));
        line("</tr>");
    }
    line("</tbody></table>");
    if !issues.is_empty() {
        line(&format!("<section class=\"issues\"><h2>{}</h2><ul>", unreadable_heading(korean)));
        for issue in issues {
            line(&format!("<li>{}</li>", html(issue)));
        }
        line("</ul></section>");
    }
    line("</body>");
    line("</html>");
    out
}

/// EnsureNoReparseTraversal: inside the root, and the traversed path (including the destination) has no reparse points.
fn ensure_no_reparse(root: &Path, candidate: &Path, include_target: bool) -> Result<(), String> {
    let root = full(root);
    let candidate = full(candidate);
    let relative = candidate.strip_prefix(&root).map_err(|_| format!("경로가 루트 폴더를 벗어납니다: {}", candidate.display()))?;
    let parts: Vec<_> = relative.components().collect();
    let count = if include_target { parts.len() } else { parts.len().saturating_sub(1) };
    let mut current = root.clone();
    for part in &parts[..count] {
        current.push(part);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(format!("재분석 지점(심볼릭 링크 또는 연결 지점)을 통한 파일 작업은 건너뜁니다: {}", current.display()));
            }
        }
    }
    Ok(())
}

/// Removes replaced items (.ndr-sync-replaced-...) (following the Recycle Bin setting).
fn delete_backup(path: &Path, recycle: bool) -> Vec<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    crate::fsops::run(crate::fsops::Operation::Delete { sources: vec![path.to_path_buf()], recycle }, tx, std::sync::Arc::new(AtomicBool::new(false)));
    rx.try_iter().filter_map(|event| match event {
        crate::fsops::Event::Error { path, message } => Some(format!("{}: {message}", path.display())),
        _ => None,
    }).collect()
}

fn unique(directory: &Path, prefix: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    directory.join(format!("{prefix}{nanos:x}{:x}", std::process::id()))
}

/// CreateDirectoriesWithoutReparse: creates folders from the root to the destination folder (a blocking file is replaced only when replacement is allowed) and returns the count created.
fn create_directories(root: &Path, destination: &Path, replace_files: bool, recycle: bool, errors: &mut Vec<String>) -> Result<u64, String> {
    let root = full(root);
    let destination = full(destination);
    let relative = destination.strip_prefix(&root).map_err(|_| format!("Destination folder escaped its root: {}", destination.display()))?.to_path_buf();
    let mut current = root;
    let mut created = 0;
    for part in relative.components() {
        current.push(part);
        if current.is_file() {
            if !replace_files {
                return Err(format!("A file blocks the required destination folder: {}", current.display()));
            }
            let backup = unique(current.parent().unwrap_or(Path::new(".")), ".ndr-sync-replaced-");
            fs::rename(&current, &backup).map_err(|e| e.to_string())?;
            if let Err(error) = fs::create_dir(&current) {
                let _ = fs::rename(&backup, &current);
                return Err(error.to_string());
            }
            errors.extend(delete_backup(&backup, recycle));
            created += 1;
        } else if !current.is_dir() {
            fs::create_dir(&current).map_err(|e| e.to_string())?;
            created += 1;
        }
        if fs::symlink_metadata(&current).map(|m| m.file_attributes() & 0x400 != 0).unwrap_or(false) {
            return Err(format!("Synchronization cannot traverse a reparse point: {}", current.display()));
        }
    }
    Ok(created)
}

/// CopyDifferencesAsync: copies differing items and items only on the source side (temp file -> rename, modification time kept). Extra files in the destination are not deleted.
pub fn copy_differences(source_root: &Path, destination_root: &Path, differences: &[Difference], source_is_left: bool, behavior: ExistingBehavior, recycle: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, &Path)) -> Result<SyncResult, String> {
    let (source_root, destination_root) = (full(source_root), full(destination_root));
    ensure_distinct_roots(&source_root, &destination_root)?;
    if !source_root.is_dir() || !destination_root.is_dir() {
        return Err("Both synchronization roots must exist.".into());
    }
    let replace_types = behavior == ExistingBehavior::OverwriteAndReplaceTypeConflicts;
    let mut result = SyncResult::default();
    let mut bytes = 0u64;
    for item in differences {
        cancelled(cancel)?;
        let wanted = item.kind == DifferenceKind::Different || if source_is_left { item.kind == DifferenceKind::LeftOnly } else { item.kind == DifferenceKind::RightOnly };
        if !wanted {
            continue;
        }
        let relative = item.relative.replace('/', "\\");
        if Path::new(&relative).is_absolute() || relative.split('\\').any(|p| p.is_empty() || p == "..") {
            return Err(format!("Unsafe relative path in comparison result: {}", item.relative));
        }
        let source = source_root.join(&relative);
        let destination = destination_root.join(&relative);
        let outcome = (|| -> Result<(), String> {
            let source_dir = if source_is_left { item.left_dir } else { item.right_dir };
            if source_dir {
                if !source.is_dir() {
                    result.errors.push(format!("Source folder is missing: {}", item.relative));
                    return Ok(());
                }
                ensure_no_reparse(&source_root, &source, true)?;
                if destination.is_file() && !replace_types {
                    if behavior == ExistingBehavior::Skip {
                        result.skipped_existing += 1;
                    } else {
                        result.errors.push(format!("Destination is a file where a folder is required: {}", item.relative));
                    }
                    return Ok(());
                }
                result.created_directories += create_directories(&destination_root, &destination, replace_types, recycle, &mut result.errors)?;
                return Ok(());
            }
            if !source.is_file() {
                result.errors.push(format!("Source file is missing: {}", item.relative));
                return Ok(());
            }
            ensure_no_reparse(&source_root, &source, true)?;
            let directory = destination.parent().map(Path::to_path_buf).unwrap_or_else(|| destination_root.clone());
            result.created_directories += create_directories(&destination_root, &directory, replace_types, recycle, &mut result.errors)?;
            ensure_no_reparse(&destination_root, &destination, true)?;
            let (exists, is_dir) = (destination.is_file(), destination.is_dir());
            if is_dir && !replace_types {
                if behavior == ExistingBehavior::Skip {
                    result.skipped_existing += 1;
                } else {
                    result.errors.push(format!("Destination is a folder where a file is required: {}", item.relative));
                }
                return Ok(());
            }
            if exists && behavior == ExistingBehavior::Skip {
                result.skipped_existing += 1;
                return Ok(());
            }
            let source_time = fs::metadata(&source).and_then(|m| m.modified()).map_err(|e| e.to_string())?;
            if exists && behavior == ExistingBehavior::OverwriteIfSourceIsNewer && source_time <= fs::metadata(&destination).and_then(|m| m.modified()).map_err(|e| e.to_string())? {
                result.skipped_existing += 1;
                return Ok(());
            }
            let temporary = unique(&directory, ".ndr-sync-");
            let copied = (|| -> Result<(), String> {
                let mut input = File::open(&source).map_err(|e| e.to_string())?;
                let mut output = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|e| e.to_string())?;
                let mut buffer = vec![0u8; 1024 * 1024];
                loop {
                    cancelled(cancel)?;
                    let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                    bytes += n as u64;
                    progress(result.copied, bytes, &source);
                }
                output.flush().map_err(|e| e.to_string())?;
                output.set_modified(source_time).map_err(|e| e.to_string())?;
                drop(output);
                ensure_no_reparse(&destination_root, &destination, true)?;
                if is_dir {
                    let backup = unique(&directory, ".ndr-sync-replaced-");
                    fs::rename(&destination, &backup).map_err(|e| e.to_string())?;
                    if let Err(error) = fs::rename(&temporary, &destination) {
                        let _ = fs::rename(&backup, &destination);
                        return Err(error.to_string());
                    }
                    result.errors.extend(delete_backup(&backup, recycle));
                } else {
                    use std::os::windows::ffi::OsStrExt;
                    let wide = |p: &Path| p.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<u16>>();
                    let flags = if exists { windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING } else { 0 };
                    // SAFETY: both paths are NUL-terminated for the call.
                    if unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileExW(wide(&temporary).as_ptr(), wide(&destination).as_ptr(), flags) } == 0 {
                        return Err(std::io::Error::last_os_error().to_string());
                    }
                }
                Ok(())
            })();
            let _ = fs::remove_file(&temporary);
            copied?;
            result.copied += 1;
            progress(result.copied, bytes, &source);
            Ok(())
        })();
        if let Err(error) = outcome {
            cancelled(cancel)?;
            result.errors.push(format!("{}: {error}", item.relative));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_and_copy_follow_foldercomparisonservice() {
        let root = std::env::temp_dir().join(format!("ndr-compare-{}-{}", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        let (left, right) = (root.join("left"), root.join("right"));
        fs::create_dir_all(left.join("sub")).unwrap();
        fs::create_dir_all(&right).unwrap();
        fs::write(left.join("same.txt"), b"same").unwrap();
        fs::copy(left.join("same.txt"), right.join("same.txt")).unwrap();
        fs::write(left.join("diff.txt"), b"left!").unwrap();
        fs::write(right.join("diff.txt"), b"right").unwrap();
        fs::write(left.join("sub/only.txt"), b"x").unwrap();
        fs::write(right.join("right-only.txt"), b"y").unwrap();
        let cancel = AtomicBool::new(false);
        assert!(ensure_distinct_roots(&left, &left.join("sub")).is_err());
        let (flat, _) = compare(&left, &right, false, true, &cancel, &mut |_| {}).unwrap();
        assert!(flat.iter().all(|d| !d.relative.contains('\\')), "not recursive");
        let mut reports = vec![];
        let (results, issues) = compare(&left, &right, true, true, &cancel, &mut |p| reports.push(p)).unwrap();
        assert!(issues.is_empty());
        assert_eq!(reports[0], CompareProgress::Scanned { left: true, count: 4, scanned: 4 }, "left: 3 files, 1 folder");
        assert_eq!(reports[1], CompareProgress::Scanned { left: false, count: 3, scanned: 3 });
        assert_eq!(reports[2], CompareProgress::Comparing { completed: 0, total: 5 });
        assert_eq!(*reports.last().unwrap(), CompareProgress::Comparing { completed: 5, total: 5 });
        assert_eq!(reports[0].message(true), "왼쪽 폴더 검색 완료 · 4개 항목");
        assert_eq!(reports[1].message(false), "Right folder scanned · 3 items");
        assert_eq!(CompareProgress::Scanning { left: true, scanned: 1024 }.message(true), "왼쪽 폴더 검색 중 · 1,024개 확인");
        assert_eq!(CompareProgress::Scanning { left: false, scanned: 256 }.counts(), (256, None));
        assert_eq!(reports[2].message(true), "파일 비교 중");
        let kind = |name: &str| results.iter().find(|d| d.relative == name).map(|d| d.kind);
        assert_eq!(kind("same.txt"), Some(DifferenceKind::Same), "same contents");
        assert_eq!(kind("diff.txt"), Some(DifferenceKind::Different), "same size, other contents");
        assert_eq!(kind("sub"), Some(DifferenceKind::LeftOnly));
        assert_eq!(kind("sub\\only.txt"), Some(DifferenceKind::LeftOnly));
        assert_eq!(kind("right-only.txt"), Some(DifferenceKind::RightOnly));
        let synced = copy_differences(&left, &right, &results, true, ExistingBehavior::Overwrite, false, &cancel, &mut |_, _, _| {}).unwrap();
        assert_eq!((synced.copied, synced.created_directories, synced.errors.len()), (2, 1, 0));
        assert_eq!(fs::read(right.join("diff.txt")).unwrap(), b"left!");
        assert!(right.join("sub/only.txt").is_file() && right.join("right-only.txt").is_file(), "extra files are kept");
        let skip = copy_differences(&left, &right, &results, true, ExistingBehavior::Skip, false, &cancel, &mut |_, _, _| {}).unwrap();
        assert_eq!((skip.copied, skip.skipped_existing), (0, 2));
        let rows: Vec<(Difference, bool)> = results.iter().cloned().map(|d| (d, true)).collect();
        let tsv = report_tsv(&rows, &[], true);
        assert!(tsv.starts_with("선택\t상태\t종류\t상대 경로\t왼쪽 크기\t오른쪽 크기\t왼쪽 수정 날짜\t오른쪽 수정 날짜\r\n"));
        assert!(tsv.contains("예\t다름\t파일\tdiff.txt\t5 B\t5 B\t"));
        let english = report_tsv(&rows, &["bad".into()], false);
        assert!(english.starts_with("Selected\tStatus\tType\tRelative path\t"));
        assert!(english.contains("Yes\tDifferent\tFile\tdiff.txt\t5 B\t5 B\t"));
        assert!(english.contains("Yes\tLeft only\tFolder\tsub\tFolder\t—\t"), "a one-sided folder is a folder, not a conflict");
        assert!(english.ends_with("\r\nUnreadable items\r\nbad\r\n"));
        let page = report_html("t<1>", "L", "R", &rows, &["bad".into()], true, true);
        assert!(page.contains("<title>t&lt;1&gt;</title>") && page.contains("<th>복사</th>") && page.contains("<h2>읽지 못한 항목</h2>"));
        assert!(!report_html("t", "L", "R", &rows, &[], false, false).contains("<th>Copy</th>"), "the copy column is sync-only");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn time_comparison_follows_original_comparefiletime() {
        let root = std::env::temp_dir().join(format!("ndr-compare-time-{}-{}", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        let (left, right) = (root.join("left"), root.join("right"));
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        let stamp = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        for (name, left_bytes, right_bytes, right_time) in [("size.txt", &b"short"[..], &b"much longer"[..], stamp), ("time.txt", &b"same"[..], &b"same"[..], stamp + std::time::Duration::from_secs(60))] {
            fs::write(left.join(name), left_bytes).unwrap();
            fs::write(right.join(name), right_bytes).unwrap();
            File::options().write(true).open(left.join(name)).unwrap().set_modified(stamp).unwrap();
            File::options().write(true).open(right.join(name)).unwrap().set_modified(right_time).unwrap();
        }
        let cancel = AtomicBool::new(false);
        let kind = |results: &[Difference], name: &str| results.iter().find(|d| d.relative == name).map(|d| d.kind);
        let (by_time, _) = compare(&left, &right, false, false, &cancel, &mut |_| {}).unwrap();
        assert_eq!(kind(&by_time, "size.txt"), Some(DifferenceKind::Same), "original time mode ignores the size");
        assert_eq!(kind(&by_time, "time.txt"), Some(DifferenceKind::Different), "other last-write time");
        let (by_contents, _) = compare(&left, &right, false, true, &cancel, &mut |_| {}).unwrap();
        assert_eq!(kind(&by_contents, "size.txt"), Some(DifferenceKind::Different), "FUN_0041cd10 checks the size first");
        assert_eq!(kind(&by_contents, "time.txt"), Some(DifferenceKind::Same), "same bytes, time ignored");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_keeps_rows_and_right_names_keep_their_case() {
        let root = std::env::temp_dir().join(format!("ndr-compare-partial-{}-{}", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        let (left, right) = (root.join("left"), root.join("right"));
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs::write(left.join(name), name).unwrap();
        }
        fs::write(right.join("B.TXT"), "b.txt").unwrap();
        // Original lvList cell 3 is the right list name as is ("B.TXT"), cell 1 the left name.
        let cancel = AtomicBool::new(false);
        let (rows, _, stopped) = compare_partial(&left, &right, false, true, &cancel, &mut |_| {}).unwrap();
        assert!(!stopped);
        let b = rows.iter().find(|d| d.relative == "b.txt").unwrap();
        assert_eq!((b.right_relative.as_str(), b.kind), ("B.TXT", DifferenceKind::Same));
        assert_eq!(rows.iter().find(|d| d.relative == "a.txt").unwrap().right_relative, "a.txt", "left-only rows keep the left name");
        // Original btnStopClick (+0x410): stopping in the comparison loop keeps the rows so far. The sync compare returns Err.
        let mut seen = 0;
        let (rows, _, stopped) = compare_partial(&left, &right, false, true, &cancel, &mut |report| {
            if let CompareProgress::Comparing { completed, .. } = report {
                seen += 1;
                if completed >= 1 {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
        })
        .unwrap();
        assert!(stopped && seen >= 2);
        assert_eq!(rows.iter().map(|d| d.relative.as_str()).collect::<Vec<_>>(), ["a.txt"], "rows compared before the stop");
        cancel.store(false, Ordering::Relaxed);
        let mut stop_now = |_: CompareProgress| cancel.store(true, Ordering::Relaxed);
        assert_eq!(compare(&left, &right, false, true, &cancel, &mut stop_now).unwrap_err(), "Cancelled");
        fs::remove_dir_all(root).unwrap();
    }
}
