//! C# 컴퓨터 청소: CleanupScanProgressWindow.Scan(정리 위치 검색), CleanupOptionsWindow.ParsePatterns,
//! FileSystemName.MatchesWin32Expression(DOS 와일드카드), FileOperationService.DeleteCleanupFiles.
use std::{
    collections::{HashMap, HashSet},
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// C# 기본 UserDefinedCleanupExtensions.
pub const DEFAULT_EXTENSIONS: &str = "tmp;$$$;bak;old;chk;bkg;wbk;prv;syd;1st";

/// C# CleanupCandidate(Name, FullPath, Size).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub full_path: PathBuf,
    pub size: u64,
}

impl Candidate {
    /// DirectoryPath: Path.GetDirectoryName(FullPath) ?? FullPath.
    pub fn directory(&self) -> String {
        self.full_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(&self.full_path).to_string_lossy().into_owned()
    }

    /// SizeBytesText: long.ToString()(천 단위 구분 없음).
    pub fn size_text(&self) -> String {
        self.size.to_string()
    }
}

/// ParsePatterns: ';'로 나눈 조각마다(빈 조각도) "*." + 조각.
pub fn parse_patterns(text: &str) -> Vec<String> {
    text.split(';').map(|extension| format!("*.{extension}")).collect()
}

/// FileSystemName.TranslateWin32Expression: "*"·"*.*"·빈 글은 "*", 끝의 "*."은 '<'(DOS_STAR),
/// 뒤에 ?나 *가 오는 '.'은 '"'(DOS_DOT), '?'는 '>'(DOS_QM).
pub fn translate_win32_expression(expression: &str) -> String {
    if expression.is_empty() || expression == "*" || expression == "*.*" {
        return "*".into();
    }
    let chars: Vec<char> = expression.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '.' if i >= 1 && i == chars.len() - 1 && chars[i - 1] == '*' => {
                if let Some(last) = out.last_mut() {
                    *last = '<';
                }
            }
            '.' if i + 1 < chars.len() && matches!(chars[i + 1], '?' | '*') => out.push('"'),
            '?' => out.push('>'),
            _ => out.push(c),
        }
    }
    out.into_iter().collect()
}

fn same_char(a: char, b: char) -> bool {
    a == b || a.to_uppercase().eq(b.to_uppercase())
}

/// FileSystemName.MatchesWin32Expression(expression, name, ignoreCase: true). 빈 이름은 맞지 않는다.
/// '*' = 아무 글자 여럿, '<' = 마지막 '.'을 넘지 않는 글자 여럿, '>' = 한 글자(‘.’이나 끝에서는 없음), '"' = '.' 하나 또는 끝.
pub fn matches_win32_expression(expression: &str, name: &str) -> bool {
    let pattern: Vec<char> = translate_win32_expression(expression).chars().collect();
    let name: Vec<char> = name.chars().collect();
    if pattern.is_empty() || name.is_empty() {
        return false;
    }
    let last_dot = name.iter().rposition(|&c| c == '.');
    // memo[p][n]: pattern[p..]가 name[n..]에 맞는지.
    let mut memo = vec![vec![None::<bool>; name.len() + 1]; pattern.len() + 1];
    fn go(p: usize, n: usize, pattern: &[char], name: &[char], last_dot: Option<usize>, memo: &mut Vec<Vec<Option<bool>>>) -> bool {
        if let Some(known) = memo[p][n] {
            return known;
        }
        let result = match pattern.get(p) {
            None => n == name.len(),
            Some('*') => (n..=name.len()).any(|k| go(p + 1, k, pattern, name, last_dot, memo)),
            Some('<') => {
                let limit = match last_dot {
                    Some(dot) if dot >= n => dot,
                    _ => name.len(),
                };
                (n..=limit).any(|k| go(p + 1, k, pattern, name, last_dot, memo))
            }
            Some('>') => {
                if n == name.len() || name[n] == '.' {
                    go(p + 1, n, pattern, name, last_dot, memo)
                } else {
                    go(p + 1, n + 1, pattern, name, last_dot, memo)
                }
            }
            Some('"') => {
                if n == name.len() {
                    go(p + 1, n, pattern, name, last_dot, memo)
                } else if name[n] == '.' {
                    go(p + 1, n + 1, pattern, name, last_dot, memo)
                } else {
                    false
                }
            }
            Some(&c) => n < name.len() && same_char(c, name[n]) && go(p + 1, n + 1, pattern, name, last_dot, memo),
        };
        memo[p][n] = Some(result);
        result
    }
    go(0, 0, &pattern, &name, last_dot, &mut memo)
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { Err("Cancelled".into()) } else { Ok(()) }
}

fn trimmed_key(path: &Path) -> String {
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().into_owned();
    let trimmed = if full.len() > 3 { full.trim_end_matches(['\\', '/']) } else { &full };
    trimmed.to_lowercase()
}

/// 원본 TfCleanup 사용자 지정 검색(btnSearchClick 0x41c338 → FUN_0041bedc): 위치마다, 폴더 안의 하위 폴더를 먼저 차례로
/// (들어갈 때 `entering`(폴더) — 원본 lResult 글), 그다음 그 폴더의 파일을 확장자 패턴마다(";" 순서, 패턴이 없으면 모두) 찾는다.
/// 재분석 지점 폴더는 들어가지 않고 같은 파일은 한 번. 멈춤(`stop`, 원본 +0x434)이면 그때까지 찾은 것을 돌려준다.
pub fn search_until_stopped(roots: &[(PathBuf, Vec<String>)], stop: &AtomicBool, entering: &mut dyn FnMut(&Path)) -> Vec<Candidate> {
    fn walk(directory: &Path, patterns: &[String], stop: &AtomicBool, entering: &mut dyn FnMut(&Path), found: &mut Vec<Candidate>, seen: &mut HashSet<String>, depth: usize) {
        if depth > 256 || stop.load(Ordering::Relaxed) {
            return;
        }
        let Ok(entries) = fs::read_dir(directory) else { return };
        let mut folders = Vec::new();
        let mut files = Vec::new();
        for entry in entries.flatten() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let Ok(metadata) = entry.metadata() else { continue };
            let attributes = metadata.file_attributes();
            if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                    folders.push(entry.path());
                }
            } else {
                files.push((entry.file_name().to_string_lossy().into_owned(), entry.path(), metadata.len()));
            }
        }
        for folder in folders {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            entering(&folder);
            walk(&folder, patterns, stop, entering, found, seen, depth + 1);
        }
        let all = [String::new()];
        let patterns: &[String] = if patterns.is_empty() { &all } else { patterns };
        for pattern in patterns {
            for (name, path, size) in &files {
                if (pattern.is_empty() || matches_win32_expression(pattern, name)) && seen.insert(path.to_string_lossy().to_lowercase()) {
                    found.push(Candidate { name: name.clone(), full_path: path.clone(), size: *size });
                }
            }
        }
    }
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for (root, patterns) in roots {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        entering(root);
        walk(root, patterns, stop, entering, &mut found, &mut seen, 0);
    }
    found
}

/// CleanupScanProgressWindow.Scan: 위치마다 깊이 우선(없는 위치·열지 못한 폴더는 건너뜀, 같은 폴더는 한 번),
/// 재분석 지점 폴더는 들어가지 않는다. 파일은 패턴(없으면 모두)에 맞으면 후보(전체 경로 기준 한 번).
/// 진행: 패턴을 통과한 파일에서 확인한 항목 수가 256의 배수일 때, 끝에 (확인 수, 후보 수, "").
pub fn scan(roots: &[(PathBuf, Vec<String>)], cancel: &AtomicBool, progress: &mut dyn FnMut(u64, usize, &Path)) -> Result<Vec<Candidate>, String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut scanned = 0u64;
    for (root, patterns) in roots {
        cancelled(cancel)?;
        if !root.is_dir() {
            continue;
        }
        let mut pending = vec![root.clone()];
        let mut visited = HashSet::new();
        while let Some(directory) = pending.pop() {
            cancelled(cancel)?;
            if !visited.insert(trimmed_key(&directory)) {
                continue;
            }
            let Ok(entries) = fs::read_dir(&directory) else { continue };
            for entry in entries {
                cancelled(cancel)?;
                scanned += 1;
                let Ok(entry) = entry else { continue };
                let Ok(metadata) = entry.metadata() else { continue };
                let attributes = metadata.file_attributes();
                let path = entry.path();
                if attributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY) == (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY) {
                    continue;
                }
                if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    pending.push(path);
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if !patterns.is_empty() && !patterns.iter().any(|p| matches_win32_expression(p, &name)) {
                    continue;
                }
                if seen.insert(path.to_string_lossy().to_lowercase()) {
                    files.push(Candidate { name, full_path: path.clone(), size: metadata.len() });
                }
                if scanned % 256 == 0 {
                    progress(scanned, files.len(), &path);
                }
            }
        }
    }
    cancelled(cancel)?;
    progress(scanned, files.len(), Path::new(""));
    Ok(files)
}

/// DeleteCleanupFiles 결과.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeleteResult {
    pub errors: Vec<String>,
    pub successful: u64,
    pub cancelled: bool,
}

fn set_attributes(path: &Path, attributes: u32) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: NUL-terminated path for the call.
    if unsafe { windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(wide.as_ptr(), attributes) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// DeleteCleanupFiles: 파일만(폴더가 됐으면 오류), 읽기 전용·숨김·시스템이면 보관 속성만 남기고 지운다. 지운 파일의 설명을 폴더마다
/// 한 번에 지운다. 오류는 "경로: 글". 취소하면 남은 것은 두고 멈춘다. 진행은 (끝낸 수, 경로).
pub fn delete_files(paths: &[PathBuf], cancel: &AtomicBool, progress: &mut dyn FnMut(u64, &Path)) -> DeleteResult {
    let mut result = DeleteResult::default();
    let mut descriptions: HashMap<String, (PathBuf, Vec<String>)> = HashMap::new();
    let mut completed = 0u64;
    for path in paths {
        if cancel.load(Ordering::Relaxed) {
            result.cancelled = true;
            break;
        }
        let outcome = (|| -> Result<(), String> {
            let attributes = fs::symlink_metadata(path).map_err(|e| e.to_string())?.file_attributes();
            if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                return Err("The cleanup target is no longer a file.".into());
            }
            if attributes & (FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) != 0 {
                set_attributes(path, FILE_ATTRIBUTE_ARCHIVE).map_err(|e| e.to_string())?;
            }
            fs::remove_file(path).map_err(|e| e.to_string())
        })();
        match outcome {
            Ok(()) => {
                result.successful += 1;
                let full = std::path::absolute(path).unwrap_or_else(|_| path.clone());
                if let (Some(directory), Some(name)) = (full.parent(), full.file_name().map(|n| n.to_string_lossy().into_owned())) {
                    if !crate::compat::is_description_file(&name) {
                        descriptions.entry(directory.to_string_lossy().to_lowercase()).or_insert_with(|| (directory.to_path_buf(), vec![])).1.push(name);
                    }
                }
            }
            Err(error) => result.errors.push(format!("{}: {error}", path.display())),
        }
        completed += 1;
        progress(completed, path);
    }
    for (directory, names) in descriptions.into_values() {
        if let Err(error) = crate::compat::remove_descriptions(&directory, &names) {
            result.errors.push(format!("{}: {error}", directory.join("descript.dat").display()));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_expressions_follow_filesystemname() {
        assert_eq!(translate_win32_expression("*."), "<");
        assert_eq!(translate_win32_expression("*.*"), "*");
        assert_eq!(translate_win32_expression("a?.b*"), "a>.b*");
        assert_eq!(translate_win32_expression("a.*"), "a\"*");
        assert!(matches_win32_expression("*.tmp", "Work.TMP"), "ignore case");
        assert!(!matches_win32_expression("*.tmp", "work.tmpx"));
        assert!(matches_win32_expression("*.$$$", "a.$$$"));
        assert!(matches_win32_expression("*.", "README"), "*. = no extension");
        assert!(!matches_win32_expression("*.", "read.me"));
        assert!(matches_win32_expression("*.b?k", "x.bak") && !matches_win32_expression("*.b?k", "x.bk"), "? takes one character inside a name");
        assert!(matches_win32_expression("*.ba?", "x.ba") && matches_win32_expression("a?.txt", "a.txt"), "? matches nothing at the end or before a dot");
        assert!(matches_win32_expression("a.*", "a") && matches_win32_expression("a.*", "a.txt"), "DOS_DOT");
        assert!(matches_win32_expression("*", "anything") && !matches_win32_expression("*.tmp", ""));
        assert_eq!(parse_patterns("tmp;;bak"), ["*.tmp", "*.", "*.bak"], "empty pieces are kept");
    }

    /// 원본 FUN_0041bedc: 하위 폴더를 먼저, 그다음 이 폴더 파일을 패턴 순서로. 멈추면 찾은 것까지.
    #[test]
    fn search_follows_the_original_order_and_stops() {
        let root = std::env::temp_dir().join(format!("ndr-cleanup-search-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(root.join("sub")).unwrap();
        for name in ["a.bak", "b.tmp", "keep.txt", "sub/c.tmp"] {
            fs::write(root.join(name), b"1").unwrap();
        }
        let stop = AtomicBool::new(false);
        let mut entered = Vec::new();
        let found = search_until_stopped(&[(root.clone(), parse_patterns("tmp;bak"))], &stop, &mut |p| entered.push(p.to_path_buf()));
        assert_eq!(found.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["c.tmp", "b.tmp", "a.bak"]);
        assert_eq!(entered, [root.clone(), root.join("sub")]);
        stop.store(true, Ordering::Relaxed);
        assert!(search_until_stopped(&[(root.clone(), vec![])], &stop, &mut |_| {}).is_empty(), "stopped before starting");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scan_and_delete_follow_the_cleanup_services() {
        let root = std::env::temp_dir().join(format!("ndr-cleanup-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(root.join("sub")).unwrap();
        for name in ["a.tmp", "b.TXT", "sub/c.bak", "sub/d.tmp"] {
            fs::write(root.join(name), b"12345").unwrap();
        }
        crate::compat::set_description(&root, "a.tmp", "gone", true).unwrap();
        crate::compat::set_description(&root, "b.TXT", "kept", true).unwrap();
        let cancel = AtomicBool::new(false);
        let patterns = parse_patterns("tmp;bak");
        let mut last = None;
        let mut found = scan(&[(root.clone(), patterns.clone()), (root.join("sub"), vec![]), (root.join("missing"), vec![])], &cancel, &mut |s, m, p| last = Some((s, m, p.to_path_buf()))).unwrap();
        found.sort_by(|a, b| a.full_path.cmp(&b.full_path));
        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a.tmp", "c.bak", "d.tmp"], "each path once; descript.dat and b.TXT do not match; the plain root adds nothing new");
        assert_eq!(found[0].size_text(), "5");
        assert_eq!(found[1].directory(), root.join("sub").to_string_lossy());
        assert_eq!(last.map(|(_, m, p)| (m, p)), Some((3, PathBuf::new())), "the final report");
        cancel.store(true, Ordering::Relaxed);
        assert!(scan(&[(root.clone(), vec![])], &cancel, &mut |_, _, _| {}).is_err(), "cancel");
        cancel.store(false, Ordering::Relaxed);
        // 읽기 전용·숨김 파일도 지우고, 지운 파일의 설명만 지운다. 폴더는 오류.
        set_attributes(&root.join("a.tmp"), FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_HIDDEN).unwrap();
        let mut steps = vec![];
        let result = delete_files(&[root.join("a.tmp"), root.join("sub"), root.join("sub/c.bak")], &cancel, &mut |n, _| steps.push(n));
        assert_eq!((result.successful, result.errors.len(), result.cancelled, steps), (2, 1, false, vec![1, 2, 3]));
        assert!(result.errors[0].ends_with("The cleanup target is no longer a file."));
        assert!(!root.join("a.tmp").exists() && !root.join("sub/c.bak").exists());
        let left = crate::compat::read_descriptions(&root, false).unwrap();
        assert_eq!(left.into_iter().collect::<Vec<_>>(), [("b.TXT".to_owned(), "kept".to_owned())], "only the deleted file loses its description");
        cancel.store(true, Ordering::Relaxed);
        let stopped = delete_files(&[root.join("b.TXT")], &cancel, &mut |_, _| {});
        assert!(stopped.cancelled && root.join("b.TXT").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
