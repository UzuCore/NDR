//! C# 파일 목록 규칙: FileSystemService.ListLocal의 열 글(종류, 속성, 날짜), MainWindow.SortPaneItems의 정렬,
//! FileGroupingService의 묶음 키, FileTypeCatalog의 묶음 분류, ExtensionColorConverter의 글자 색.
use crate::fsops::Entry;
use crate::parity::{dotnet_extension, dotnet_stem};
use serde_json::Value;
use std::{cmp::Ordering, ffi::OsStr, os::windows::ffi::OsStrExt, time::SystemTime};

pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
const FILE_ATTRIBUTE_PINNED: u32 = 0x0008_0000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

/// C# AttributeText: R/H/S/A 자리마다 글자, 없으면 '-'.
pub fn attribute_text(attributes: u32) -> String {
    [(FILE_ATTRIBUTE_READONLY, 'R'), (FILE_ATTRIBUTE_HIDDEN, 'H'), (FILE_ATTRIBUTE_SYSTEM, 'S'), (FILE_ATTRIBUTE_ARCHIVE, 'A')]
        .iter()
        .map(|(bit, letter)| if attributes & bit != 0 { *letter } else { '-' })
        .collect()
}

/// C# ListLocal의 Kind: 폴더는 "폴더", 확장자가 없으면 "파일", 있으면 "{EXT} 파일"(대문자). 영어는 같은 모양의 영어.
pub fn type_name(name: &str, is_dir: bool, korean: bool) -> String {
    if is_dir {
        return if korean { "폴더" } else { "Folder" }.into();
    }
    let extension = dotnet_extension(name);
    if extension.is_empty() {
        return if korean { "파일" } else { "File" }.into();
    }
    let upper = extension.trim_start_matches('.').to_uppercase();
    if korean { format!("{upper} 파일") } else { format!("{upper} File") }
}

/// SYSTEMTIME(현지 시각).
fn local_systemtime(time: SystemTime) -> windows_sys::Win32::Foundation::SYSTEMTIME {
    use chrono::{Datelike, Timelike};
    let local = chrono::DateTime::<chrono::Local>::from(time);
    windows_sys::Win32::Foundation::SYSTEMTIME {
        wYear: local.year().clamp(1601, 30827) as u16,
        wMonth: local.month() as u16,
        wDayOfWeek: local.weekday().num_days_from_sunday() as u16,
        wDay: local.day() as u16,
        wHour: local.hour() as u16,
        wMinute: local.minute() as u16,
        wSecond: local.second() as u16,
        wMilliseconds: (local.nanosecond() / 1_000_000).min(999) as u16,
    }
}

/// 사용자 지역 설정 값(GetLocaleInfoW).
fn locale_info(kind: u32) -> Option<String> {
    let mut buffer = vec![0u16; 16];
    // SAFETY: the buffer length is passed with the buffer.
    let written = unsafe { windows_sys::Win32::Globalization::GetLocaleInfoW(0x400, kind, buffer.as_mut_ptr(), buffer.len() as i32) };
    (written > 1).then(|| String::from_utf16_lossy(&buffer[..written as usize - 1]))
}

/// .NET 현재 문화권의 날짜 구분 기호('/' 자리, LOCALE_SDATE)와 시간 구분 기호(':' 자리, LOCALE_STIME).
pub fn date_separators() -> (String, String) {
    (locale_info(0x1D).unwrap_or_else(|| "/".into()), locale_info(0x1E).unwrap_or_else(|| ":".into()))
}

/// .NET 사용자 지정 형식을 Win32 그림 문자열로: '/'·':'는 문화권 구분 기호, "…"와 '\x'는 글자 그대로(작은따옴표로 감쌈).
fn dotnet_to_picture(format: &str) -> String {
    let (date_separator, time_separator) = date_separators();
    let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
    let mut out = String::new();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' => out.push_str(&quote(&date_separator)),
            ':' => out.push_str(&quote(&time_separator)),
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push_str(&quote(&next.to_string()));
                }
            }
            '"' | '\'' => {
                let mut literal = String::new();
                for next in chars.by_ref() {
                    if next == c {
                        break;
                    }
                    literal.push(next);
                }
                out.push_str(&quote(&literal));
            }
            other => out.push(other),
        }
    }
    out
}

/// GetDateFormatW / GetTimeFormatW(LOCALE_USER_DEFAULT, 그림 문자열). .NET 사용자 지정 형식과 같은 글자(yyyy, MM, dd, HH, mm, ss, tt…)를 쓴다.
fn win32_format(time: SystemTime, format: &str, date: bool) -> Option<String> {
    use windows_sys::Win32::Globalization::{GetDateFormatW, GetTimeFormatW};
    const LOCALE_USER_DEFAULT: u32 = 0x400;
    if format.is_empty() {
        return Some(String::new());
    }
    let st = local_systemtime(time);
    let picture = wide(&dotnet_to_picture(format));
    let mut buffer = vec![0u16; 256];
    // SAFETY: SYSTEMTIME and both strings live through the call; the buffer length is passed.
    let written = unsafe {
        if date {
            GetDateFormatW(LOCALE_USER_DEFAULT, 0, &st, picture.as_ptr(), buffer.as_mut_ptr(), buffer.len() as i32)
        } else {
            GetTimeFormatW(LOCALE_USER_DEFAULT, 0, &st, picture.as_ptr(), buffer.as_mut_ptr(), buffer.len() as i32)
        }
    };
    (written > 0).then(|| String::from_utf16_lossy(&buffer[..written as usize - 1]))
}

/// 원본 FUN_00422068(같은 이름 확인 창): GetDateFormatW(LOCALE_SYSTEM_DEFAULT 0x800, DATE_LONGDATE 2)와
/// GetTimeFormatW(0x800, LOCALE_NOUSEROVERRIDE 0x80000000) — 시스템 긴 날짜와 시간. 읽지 못하면 "yyyy-MM-dd", "HH:mm:ss".
pub fn long_date_and_time(time: SystemTime) -> (String, String) {
    use windows_sys::Win32::Globalization::{GetDateFormatW, GetTimeFormatW};
    let st = local_systemtime(time);
    let mut date = vec![0u16; 128];
    let mut clock = vec![0u16; 64];
    // SAFETY: SYSTEMTIME lives through the calls; the buffer lengths are passed; null formats use the locale defaults.
    let (d, t) = unsafe {
        (
            GetDateFormatW(0x800, 2, &st, std::ptr::null(), date.as_mut_ptr(), date.len() as i32),
            GetTimeFormatW(0x800, 0x8000_0000, &st, std::ptr::null(), clock.as_mut_ptr(), clock.len() as i32),
        )
    };
    let local = chrono::DateTime::<chrono::Local>::from(time);
    (
        if d > 0 { String::from_utf16_lossy(&date[..d as usize - 1]) } else { local.format("%Y-%m-%d").to_string() },
        if t > 0 { String::from_utf16_lossy(&clock[..t as usize - 1]) } else { local.format("%H:%M:%S").to_string() },
    )
}

/// .NET DateTime.ToString("g", CurrentCulture): 짧은 날짜 + 짧은 시간(GetDateFormatW DATE_SHORTDATE, GetTimeFormatW TIME_NOSECONDS).
pub fn general_date_text(time: SystemTime) -> String {
    use windows_sys::Win32::Globalization::{GetDateFormatW, GetTimeFormatW};
    const LOCALE_USER_DEFAULT: u32 = 0x400;
    let st = local_systemtime(time);
    let mut date = vec![0u16; 128];
    let mut clock = vec![0u16; 128];
    // SAFETY: SYSTEMTIME lives through the calls; the buffer lengths are passed; null formats use the locale defaults.
    let (d, t) = unsafe {
        (
            GetDateFormatW(LOCALE_USER_DEFAULT, 1, &st, std::ptr::null(), date.as_mut_ptr(), date.len() as i32),
            GetTimeFormatW(LOCALE_USER_DEFAULT, 2, &st, std::ptr::null(), clock.as_mut_ptr(), clock.len() as i32),
        )
    };
    if d > 0 && t > 0 {
        format!("{} {}", String::from_utf16_lossy(&date[..d as usize - 1]), String::from_utf16_lossy(&clock[..t as usize - 1]))
    } else {
        chrono::DateTime::<chrono::Local>::from(time).format("%Y-%m-%d %H:%M").to_string()
    }
}

/// FormatDate(value, DateFormat, TimeFormat, UseDateAge): 날짜 없음은 빈 글. 날짜 나이는 원본 형식(`age_date_text`,
/// 한국어/영어 기본 글). 그 밖은 "날짜형식 시간형식", 형식이 틀리면 "yyyy-MM-dd HH:mm".
pub fn date_text(modified: Option<SystemTime>, date_format: &str, time_format: &str, use_age: bool, korean: bool, now: SystemTime) -> String {
    if use_age {
        let labels = if korean { ["오늘", "어제", "%d일 전"] } else { ["Today", "Yesterday", "%d days ago"] };
        return age_date_text(modified, date_format, time_format, labels, now);
    }
    let Some(time) = modified else { return String::new() };
    match (win32_format(time, date_format, true), win32_format(time, time_format, false)) {
        (Some(date), Some(clock)) => format!("{date} {clock}"),
        _ => chrono::DateTime::<chrono::Local>::from(time).format("%Y-%m-%d %H:%M").to_string(),
    }
}

/// 원본 TNXCustomFileListView::DateToStr(0x7df294)의 날짜 나이: 달력 날짜 차이(FUN_007df08c, 앞날이면 -1)가 0이면
/// labels[0](TODAY), 1이면 labels[1](YESTERDAY), 2~30이면 labels[2](DATEAGE의 %d = 날수), 그 밖은 날짜 형식.
/// 열 글은 GetFieldMaxWidth처럼 "날짜 시간"(TimeToStr = 시간 형식).
pub fn age_date_text(modified: Option<SystemTime>, date_format: &str, time_format: &str, labels: [&str; 3], now: SystemTime) -> String {
    let Some(time) = modified else { return String::new() };
    let file_day = chrono::DateTime::<chrono::Local>::from(time).date_naive();
    let today = chrono::DateTime::<chrono::Local>::from(now).date_naive();
    let days = if file_day > today { -1 } else { (today - file_day).num_days() };
    let age = match days {
        0 => Some(labels[0].to_owned()),
        1 => Some(labels[1].to_owned()),
        2..=30 => Some(labels[2].replace("%d", &days.to_string())),
        _ => None,
    };
    match (age.or_else(|| win32_format(time, date_format, true)), win32_format(time, time_format, false)) {
        (Some(date), Some(clock)) => format!("{date} {clock}"),
        _ => chrono::DateTime::<chrono::Local>::from(time).format("%Y-%m-%d %H:%M").to_string(),
    }
}

/// StrCmpLogicalW(UseLogicalSort) 또는 현재 문화권 대소문자 무시 비교(CompareStringW NORM_IGNORECASE).
pub fn compare_text(a: &str, b: &str, logical: bool) -> Ordering {
    let (a, b) = (wide(a), wide(b));
    // SAFETY: both buffers are NUL-terminated and live through the call.
    let result = unsafe {
        if logical {
            windows_sys::Win32::UI::Shell::StrCmpLogicalW(a.as_ptr(), b.as_ptr())
        } else {
            const LOCALE_USER_DEFAULT: u32 = 0x400;
            const NORM_IGNORECASE: u32 = 0x1;
            windows_sys::Win32::Globalization::CompareStringW(LOCALE_USER_DEFAULT, NORM_IGNORECASE, a.as_ptr(), -1, b.as_ptr(), -1) - 2
        }
    };
    result.cmp(&0)
}

/// SortPaneItems의 설정(패널마다): 폴더·파일 정렬 방식과 내림차순, 논리 정렬.
#[derive(Clone, Debug)]
pub struct SortOptions {
    pub folder_mode: String,
    pub folder_descending: bool,
    pub file_mode: String,
    pub file_descending: bool,
    pub logical: bool,
}

fn sort_base(entry: &Entry) -> &str {
    if entry.is_dir { &entry.name } else { dotnet_stem(&entry.name) }
}

fn sort_extension(entry: &Entry) -> &str {
    if entry.is_dir { "" } else { dotnet_extension(&entry.name) }
}

/// LegacyPriorityRank: 폴더 1, 고정(Pinned) 2, 보통 3, 열 때 받기(RecallOnOpen) 4.
fn priority_rank(entry: &Entry) -> u8 {
    if entry.is_dir {
        1
    } else if entry.attributes & FILE_ATTRIBUTE_RECALL_ON_OPEN != 0 {
        4
    } else if entry.attributes & FILE_ATTRIBUTE_PINNED != 0 {
        2
    } else {
        3
    }
}

fn compare_by_mode(a: &Entry, b: &Entry, mode: &str, logical: bool, color: &dyn Fn(&Entry) -> u32) -> Ordering {
    let base = || compare_text(sort_base(a), sort_base(b), logical);
    let ext = || compare_text(sort_extension(a), sort_extension(b), false);
    match mode {
        "None" => Ordering::Equal,
        "Extension" => ext().then_with(base),
        "Size" => a.size.cmp(&b.size).then_with(base).then_with(ext),
        "Modified" => a.modified.cmp(&b.modified).then_with(ext).then_with(base),
        "Attributes" => a.attributes.cmp(&b.attributes).then_with(ext).then_with(base),
        "Color" => color(a).cmp(&color(b)).then_with(ext).then_with(base),
        _ => base().then_with(ext),
    }
}

/// C# SortPaneItems(묶음 전): 폴더(폴더 방식) 다음 파일(우선순위 순위 오름차순, 그 안에서 파일 방식). 안정 정렬이라
/// "None"은 목록을 받은 순서. 내림차순은 순위를 뺀 열쇠 전체를 거꾸로(OrderByDescending.ThenByDescending).
pub fn sort_entries(entries: &mut Vec<Entry>, options: &SortOptions, color: &dyn Fn(&Entry) -> u32) {
    let (mut folders, mut files): (Vec<Entry>, Vec<Entry>) = std::mem::take(entries).into_iter().partition(|e| e.is_dir);
    folders.sort_by(|a, b| {
        let order = compare_by_mode(a, b, &options.folder_mode, options.logical, color);
        if options.folder_descending { order.reverse() } else { order }
    });
    files.sort_by(|a, b| {
        let order = compare_by_mode(a, b, &options.file_mode, options.logical, color);
        priority_rank(a).cmp(&priority_rank(b)).then(if options.file_descending { order.reverse() } else { order })
    });
    entries.extend(folders);
    entries.extend(files);
}

/// FileTypeCatalog의 묶음 분류(확장자는 점 포함, 대소문자 무시).
pub fn group_category(extension: &str) -> &'static str {
    const TABLE: &[(&str, &[&str])] = &[
        ("Archives", &[".7z", ".ace", ".alz", ".arj", ".br", ".bz2", ".cab", ".deb", ".dmg", ".egg", ".esd", ".gz", ".iso", ".lz", ".lz4", ".lzh", ".lzma", ".lzo", ".pkg", ".rar", ".rpm", ".tar", ".tgz", ".wim", ".xar", ".xz", ".z", ".zip", ".zst", ".zstd"]),
        ("Images", &[".arw", ".avif", ".bmp", ".cr2", ".cr3", ".dng", ".emf", ".gif", ".heic", ".heif", ".ico", ".jfi", ".jfif", ".jif", ".jpe", ".jpeg", ".jpg", ".jxl", ".nef", ".orf", ".png", ".psd", ".raw", ".rw2", ".svg", ".tif", ".tiff", ".webp", ".wmf", ".xcf"]),
        ("Audio", &[".aac", ".ac3", ".aif", ".aiff", ".alac", ".amr", ".ape", ".dff", ".dsf", ".flac", ".m4a", ".m4b", ".mid", ".midi", ".mka", ".mp3", ".oga", ".ogg", ".opus", ".wav", ".wma"]),
        ("Video", &[".3g2", ".3gp", ".asf", ".avi", ".f4v", ".flv", ".m2ts", ".m4v", ".mkv", ".mov", ".mp4", ".mpe", ".mpeg", ".mpg", ".mxf", ".ogv", ".rm", ".rmvb", ".ts", ".vob", ".webm", ".wmv"]),
        ("Programs", &[".appinstaller", ".appx", ".appxbundle", ".bat", ".cmd", ".com", ".cpl", ".dll", ".drv", ".exe", ".msi", ".msix", ".msixbundle", ".msp", ".msu", ".ocx", ".ps1", ".psd1", ".psm1", ".reg", ".scr", ".sys", ".vbs", ".wsf"]),
        ("Code", &[".bash", ".c", ".cc", ".cfg", ".conf", ".cpp", ".cs", ".csproj", ".css", ".go", ".h", ".hpp", ".html", ".htm", ".ini", ".java", ".js", ".json", ".jsx", ".lua", ".php", ".py", ".rb", ".rs", ".sh", ".sln", ".sql", ".toml", ".ts", ".tsx", ".xml", ".yaml", ".yml"]),
        ("Documents", &[".djvu", ".doc", ".docm", ".docx", ".dot", ".dotm", ".dotx", ".epub", ".md", ".odf", ".odg", ".odt", ".pages", ".pdf", ".rtf", ".tex", ".txt"]),
        ("Spreadsheets", &[".csv", ".numbers", ".ods", ".xls", ".xlsb", ".xlsm", ".xlsx", ".xlt", ".xltm", ".xltx"]),
        ("Presentations", &[".key", ".odp", ".pot", ".potm", ".potx", ".pps", ".ppsm", ".ppsx", ".ppt", ".pptm", ".pptx"]),
    ];
    // C#은 나중에 더한 분류가 이긴다(.ts는 Video 뒤 Code, .html은 Code).
    let lower = extension.to_ascii_lowercase();
    TABLE.iter().rev().find(|(_, list)| list.contains(&lower.as_str())).map_or("Other", |(name, _)| name)
}

/// FileGroupingService.GetGroupKey: "순서|이름". 폴더는 "01|Folders"(분류 묶음에서 폴더 분류가 Auto가 아니면 "05|Folders").
pub fn group_key(entry: &Entry, mode: &str, folder_category_mode: &str) -> String {
    if entry.is_dir {
        return if mode != "Category" || folder_category_mode == "Auto" { "01|Folders".into() } else { "05|Folders".into() };
    }
    match mode {
        "Extension" => {
            let extension = dotnet_extension(&entry.name);
            if extension.is_empty() { "10|No extension".into() } else { format!("10|{}", extension.to_lowercase()) }
        }
        "Size" => match entry.size {
            0 => "10|Empty",
            s if s < 1024 => "11|Under 1 KB",
            s if s < 1024 * 1024 => "12|1 KB–1 MB",
            s if s < 100 * 1024 * 1024 => "13|1–100 MB",
            s if s < 1024 * 1024 * 1024 => "14|100 MB–1 GB",
            _ => "15|1 GB and larger",
        }
        .into(),
        "Time" => match entry.modified {
            None => "10|Unknown date".into(),
            Some(time) => format!("10|{}", chrono::DateTime::<chrono::Local>::from(time).format("%Y-%m")),
        },
        "Category" => {
            let order = match group_category(dotnet_extension(&entry.name)) {
                "Archives" => "10|Archives",
                "Documents" => "20|Documents",
                "Spreadsheets" => "30|Spreadsheets",
                "Presentations" => "40|Presentations",
                "Images" => "50|Images",
                "Audio" => "60|Audio",
                "Video" => "70|Video",
                "Programs" => "80|Programs",
                "Code" => "90|Code",
                _ => "99|Other",
            };
            order.into()
        }
        _ => "00|".into(),
    }
}

fn group_rank(key: &str) -> u8 {
    if key.starts_with("00|") {
        0
    } else if key.starts_with("01|") {
        1
    } else {
        2
    }
}

/// SortPaneItems 끝: 묶음이 있으면 묶음 키 순서(순위, 그다음 문화권 비교, 묶음 내림차순이면 거꾸로)로 다시 안정 정렬한다.
pub fn sort_groups(entries: &mut Vec<Entry>, mode: &str, folder_category_mode: &str, descending: bool) {
    if mode == "None" {
        return;
    }
    let mut keyed: Vec<(String, Entry)> = std::mem::take(entries).into_iter().map(|e| (group_key(&e, mode, folder_category_mode), e)).collect();
    keyed.sort_by(|(ka, _), (kb, _)| {
        group_rank(ka).cmp(&group_rank(kb)).then_with(|| {
            let order = compare_text(ka, kb, false);
            if descending { order.reverse() } else { order }
        })
    });
    entries.extend(keyed.into_iter().map(|(_, e)| e));
}

/// GroupHeaderNameConverter의 머리 글: "Folders"는 [RES] FOLDERS, 알려진 이름은 언어 항목 키, 나머지는 이름 그대로.
pub fn group_caption_key(key: &str) -> (&str, Option<&'static str>) {
    let label = key.split_once('|').map_or(key, |(_, label)| label);
    let caption = match label {
        "Folders" => Some("RES.FOLDERS"),
        "Archives" => Some("CategoryArchives"),
        "Documents" => Some("CategoryDocuments"),
        "Spreadsheets" => Some("GroupSpreadsheets"),
        "Presentations" => Some("GroupPresentations"),
        "Images" => Some("CategoryPictures"),
        "Audio" => Some("CategoryAudio"),
        "Video" => Some("CategoryVideo"),
        "Programs" => Some("CategoryApplications"),
        "Code" => Some("GroupCode"),
        "Other" => Some("CategoryOther"),
        "No extension" => Some("GroupNoExtension"),
        "Unknown date" => Some("GroupUnknownDate"),
        "Empty" => Some("GroupEmpty"),
        "Under 1 KB" => Some("GroupUnder1KB"),
        "1 KB–1 MB" => Some("Group1KBTo1MB"),
        "1–100 MB" => Some("Group1To100MB"),
        "100 MB–1 GB" => Some("Group100MBTo1GB"),
        "1 GB and larger" => Some("Group1GBPlus"),
        _ => None,
    };
    (label, caption)
}

/// NexusFileColorMapService의 기본 요소 색.
pub fn default_element_color(key: &str) -> Option<&'static str> {
    Some(match key.to_ascii_uppercase().as_str() {
        "NXBACK" => "000000",
        "NXFORE" => "C0C0C0",
        "NXFOLDER" => "EA2222",
        "NXDRIVE" => "8080FF",
        "NXREADONLY" => "FFFF00",
        "NXHIDDEN" => "FF0000",
        "NXDESC" => "3682AF",
        "NXSIZE" => "606060",
        "NXTIME" => "808080",
        "NXATTR" => "606060",
        _ => return None,
    })
}

fn hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(hex, 16).ok()?;
    Some(((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
}

/// 색상표 값 찾기(대소문자 무시).
fn lookup<'a>(map: Option<&'a serde_json::Map<String, Value>>, key: &str) -> Option<&'a str> {
    map?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).and_then(|(_, v)| v.as_str())
}

/// ExtensionColorConverter: 폴더는 NXFOLDER, 숨김은 NXHIDDEN, 읽기 전용은 NXREADONLY(색상표를 쓸 때),
/// 그 밖은 확장자 색, 없으면 NXFORE. 색상표를 쓰지 않으면 확장자 색만. 결과 없음은 기본 글자 색.
pub fn entry_color(entry: &Entry, extension_colors: Option<&serde_json::Map<String, Value>>, nexus_colors: Option<&serde_json::Map<String, Value>>) -> Option<(u8, u8, u8)> {
    let color = if entry.is_dir {
        lookup(nexus_colors, "NXFOLDER")
    } else if entry.attributes & FILE_ATTRIBUTE_HIDDEN != 0 && lookup(nexus_colors, "NXHIDDEN").is_some() {
        lookup(nexus_colors, "NXHIDDEN")
    } else if entry.attributes & FILE_ATTRIBUTE_READONLY != 0 && lookup(nexus_colors, "NXREADONLY").is_some() {
        lookup(nexus_colors, "NXREADONLY")
    } else {
        lookup(extension_colors, dotnet_extension(&entry.name))
    };
    let color = color.filter(|c| !c.trim().is_empty()).or_else(|| lookup(nexus_colors, "NXFORE"));
    color.and_then(hex_color)
}

/// 원본 FileListChangedBeforeSort(0x7d7924)의 항목 색(+0x34, 이름 칸 글색): 폴더(종류 1)는 GetBasic(2) NXFOLDER,
/// 파일(종류 3)은 확장자 색(GetExtension 0x7d3894, 없으면 GetBasic(1) NXFORE), 그 밖(드라이브 종류 4·종류 2)은 GetBasic(3) NXDRIVE.
/// 숨김·읽기 전용은 이름 색을 바꾸지 않는다(크기 칸만 NXHIDDEN·NXREADONLY).
pub fn original_item_color(entry: &Entry, extension_colors: Option<&serde_json::Map<String, Value>>, basic: impl Fn(&str) -> (u8, u8, u8)) -> (u8, u8, u8) {
    if entry.drive {
        basic("NXDRIVE")
    } else if entry.is_dir {
        basic("NXFOLDER")
    } else {
        lookup(extension_colors, dotnet_extension(&entry.name)).and_then(hex_color).unwrap_or_else(|| basic("NXFORE"))
    }
}

/// 원본 기본 색상표(default\nexusfile.col [Ext])의 확장자 색. 확장자 색이 하나도 없을 때의 처음 값(settings.rs).
pub const ORIGINAL_EXTENSION_COLORS: &[(&str, &[&str])] = &[
    ("#E10000", &["bak", "tmp"]),
    ("#D857EA", &["bmp", "gif", "jpeg", "jpg", "png", "psd", "psp", "tif", "wmf"]),
    ("#408080", &["cap", "doc", "docx", "gal", "gul", "hwp", "pdf", "rtf", "txt"]),
    ("#808080", &["cfg", "inf", "ini", "reg"]),
    ("#804000", &["chm", "hlp"]),
    ("#FF00FF", &["dll", "sys"]),
    ("#8080FF", &["htm", "html", "url"]),
    ("#FF8040", &["asf", "avi", "mov", "mpeg", "mpg", "ram", "rm", "wmv", "mp4", "mkv", "swf", "tp", "ts"]),
    ("#EFF41C", &["bat", "lnk", "pif", "cmd"]),
    ("#00FF40", &["exe", "msi"]),
    ("#D65A36", &["mid", "mp2", "mp3", "wav", "ogg", "wma"]),
    ("#0B8AB5", &["ppt", "xls", "xlsx", "pptx"]),
    ("#5FA7DC", &["c", "cpp", "as", "php", "asp", "pas", "vb", "cs", "jsp", "js", "pl", "h", "hpp"]),
    ("#808000", &["ace", "alz", "arj", "cab", "gz", "lzh", "rar", "tar", "zip", "7z", "iso", "egg"]),
];

/// TryGetTColorSortKey: "RRGGBB" → Delphi TColor(0x00BBGGRR). 색 정렬은 기본 글자 색과 같거나 색이 없으면 맨 뒤.
pub fn color_sort_key(entry: &Entry, extension_colors: Option<&serde_json::Map<String, Value>>, nexus_colors: Option<&serde_json::Map<String, Value>>) -> u32 {
    let tcolor = |value: Option<&str>| value.and_then(hex_color).map(|(r, g, b)| (u32::from(b) << 16) | (u32::from(g) << 8) | u32::from(r));
    let element = |key: &str| lookup(nexus_colors, key).filter(|v| hex_color(v).is_some()).or_else(|| default_element_color(key));
    if entry.is_dir {
        return tcolor(element("NXFOLDER")).unwrap_or(u32::MAX);
    }
    let Some(file) = tcolor(lookup(extension_colors, dotnet_extension(&entry.name))) else { return u32::MAX };
    if tcolor(element("NXFORE")) == Some(file) { u32::MAX } else { file }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, time::Duration};

    fn entry(name: &str, is_dir: bool, size: u64, attributes: u32) -> Entry {
        Entry { path: PathBuf::from(name), name: name.into(), is_dir, size, modified: None, hidden: attributes & 2 != 0, attributes, drive: false }
    }
    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }
    fn options(folder: &str, file: &str, descending: bool) -> SortOptions {
        SortOptions { folder_mode: folder.into(), folder_descending: descending, file_mode: file.into(), file_descending: descending, logical: true }
    }

    #[test]
    fn column_texts_follow_listlocal() {
        assert_eq!(attribute_text(0x21), "R--A");
        assert_eq!(attribute_text(0x6), "-HS-");
        assert_eq!(type_name("report.pdf", false, true), "PDF 파일");
        assert_eq!(type_name("README", false, true), "파일");
        assert_eq!(type_name("folder.v1", true, false), "Folder");
        assert_eq!(type_name("a.tar.GZ", false, false), "GZ File");
        let now = SystemTime::now();
        assert_eq!(date_text(None, "yyyy-MM-dd", "HH:mm", false, true, now), "");
        // 원본 DateToStr: 달력 날짜 차이 0/1/2~30 → TODAY/YESTERDAY/DATEAGE, 뒤에 시간. 31일부터와 앞날은 날짜 형식.
        let clock = |t: SystemTime| chrono::DateTime::<chrono::Local>::from(t).format("%H:%M").to_string();
        let midday = chrono::Local::now().date_naive().and_hms_opt(12, 0, 0).unwrap().and_local_timezone(chrono::Local).unwrap();
        let noon: SystemTime = midday.into();
        assert_eq!(date_text(Some(noon - Duration::from_secs(60)), "yyyy-MM-dd", "HH:mm", true, true, noon), "오늘 11:59");
        assert_eq!(date_text(Some(noon - Duration::from_secs(86_400)), "yyyy-MM-dd", "HH:mm", true, false, noon), "Yesterday 12:00");
        let five = noon - Duration::from_secs(5 * 86_400);
        assert_eq!(date_text(Some(five), "yyyy-MM-dd", "HH:mm", true, true, noon), format!("5일 전 {}", clock(five)));
        assert_eq!(age_date_text(Some(five), "yyyy-MM-dd", "HH:mm", ["T", "Y", "%d ago"], noon), format!("5 ago {}", clock(five)));
        let old = noon - Duration::from_secs(31 * 86_400);
        let expected = chrono::DateTime::<chrono::Local>::from(old).format("%Y-%m-%d %H:%M").to_string();
        assert_eq!(date_text(Some(old), "yyyy-MM-dd", "HH:mm", true, true, noon), expected, "31 days or more: the formats");
        let future = noon + Duration::from_secs(2 * 86_400);
        let expected = chrono::DateTime::<chrono::Local>::from(future).format("%Y-%m-%d %H:%M").to_string();
        assert_eq!(date_text(Some(future), "yyyy-MM-dd", "HH:mm", true, true, noon), expected, "a future date: the formats");
        let (date_sep, time_sep) = date_separators();
        let local = chrono::DateTime::<chrono::Local>::from(old);
        let expected = format!("{}{date_sep}{}{date_sep}{} {}{time_sep}{}{time_sep}{}", local.format("%y"), local.format("%m"), local.format("%d"), local.format("%H"), local.format("%M"), local.format("%S"));
        assert_eq!(date_text(Some(old), "yy/MM/dd", "HH:mm:ss", false, true, now), expected, "'/' and ':' are the culture separators");
        let expected = format!("{}/{} {}", local.format("%Y"), local.format("%m"), local.format("%Hh"));
        assert_eq!(date_text(Some(old), "yyyy'/'MM", "HH\\h", false, true, now), expected, "quoted and escaped literals");
    }

    #[test]
    fn sorting_follows_sortpaneitems() {
        let mut list = vec![
            entry("b10.txt", false, 5, 0x20),
            entry("B2.txt", false, 9, 0x20),
            entry("zeta", true, 0, 0x10),
            entry("a.doc", false, 1, 0x20),
            entry("Alpha", true, 0, 0x10),
            entry("pinned.txt", false, 1, 0x20 | FILE_ATTRIBUTE_PINNED),
            entry("cloud.txt", false, 1, 0x20 | FILE_ATTRIBUTE_RECALL_ON_OPEN),
        ];
        let none = |_: &Entry| u32::MAX;
        sort_entries(&mut list, &options("Name", "Name", false), &none);
        assert_eq!(names(&list), ["Alpha", "zeta", "pinned.txt", "a.doc", "B2.txt", "b10.txt", "cloud.txt"], "folders, then pinned, normal (logical), recall");
        sort_entries(&mut list, &options("Name", "Size", true), &none);
        assert_eq!(names(&list), ["zeta", "Alpha", "pinned.txt", "B2.txt", "b10.txt", "a.doc", "cloud.txt"], "descending keeps the priority rank ascending");
        sort_entries(&mut list, &options("Name", "Extension", false), &none);
        assert_eq!(names(&list), ["Alpha", "zeta", "pinned.txt", "a.doc", "B2.txt", "b10.txt", "cloud.txt"]);
        let mut plain = vec![entry("x2", false, 0, 0), entry("x10", false, 0, 0)];
        sort_entries(&mut plain, &SortOptions { logical: false, ..options("Name", "Name", false) }, &none);
        assert_eq!(names(&plain), ["x10", "x2"], "without logical sort: culture text order");
        let mut original = vec![entry("c", false, 0, 0), entry("a", false, 0, 0)];
        sort_entries(&mut original, &options("None", "None", false), &none);
        assert_eq!(names(&original), ["c", "a"], "None keeps the listing order");
    }

    #[test]
    fn groups_follow_filegroupingservice() {
        assert_eq!(group_key(&entry("x.ZIP", false, 1, 0), "Extension", "Auto"), "10|.zip");
        assert_eq!(group_key(&entry("README", false, 1, 0), "Extension", "Auto"), "10|No extension");
        assert_eq!(group_key(&entry("big", false, 2 * 1024 * 1024 * 1024, 0), "Size", "Auto"), "15|1 GB and larger");
        assert_eq!(group_key(&entry("e", false, 0, 0), "Size", "Auto"), "10|Empty");
        assert_eq!(group_key(&entry("dir", true, 0, 0), "Category", "Auto"), "01|Folders");
        assert_eq!(group_key(&entry("dir", true, 0, 0), "Category", "Category"), "05|Folders");
        assert_eq!(group_key(&entry("clip.ts", false, 1, 0), "Category", "Auto"), "90|Code", "last-added-wins: .ts is Code");
        assert_eq!(group_key(&entry("photo.JPG", false, 1, 0), "Category", "Auto"), "50|Images");
        assert_eq!(group_key(&entry("x.unknown", false, 1, 0), "Category", "Auto"), "99|Other");
        let mut list = vec![entry("b.txt", false, 1, 0), entry("dir", true, 0, 0), entry("a.zip", false, 1, 0), entry("c.txt", false, 1, 0)];
        sort_groups(&mut list, "Extension", "Auto", false);
        assert_eq!(names(&list), ["dir", "b.txt", "c.txt", "a.zip"], "folders first, then groups, stable inside");
        sort_groups(&mut list, "Extension", "Auto", true);
        assert_eq!(names(&list), ["dir", "a.zip", "b.txt", "c.txt"], "group descending keeps folders first");
        assert_eq!(group_caption_key("10|Archives"), ("Archives", Some("CategoryArchives")));
        assert_eq!(group_caption_key("10|.zip"), (".zip", None));
    }

    #[test]
    fn original_item_colors_follow_file_list_changed_before_sort() {
        // 원본 0x7d7924: 폴더 NXFOLDER, 파일은 확장자 색(없으면 NXFORE), 드라이브 NXDRIVE. 숨김·읽기 전용은 이름 색을 안 바꾼다.
        let basic = |key: &str| match key {
            "NXFOLDER" => (234, 34, 34),
            "NXDRIVE" => (128, 128, 255),
            _ => (192, 192, 192),
        };
        let colors = serde_json::json!({".zip": "#808000"});
        let map = colors.as_object();
        assert_eq!(original_item_color(&entry("docs", true, 0, 0x10), map, basic), (234, 34, 34));
        assert_eq!(original_item_color(&entry("a.ZIP", false, 1, 0), map, basic), (128, 128, 0));
        assert_eq!(original_item_color(&entry("a.dat", false, 1, 2), map, basic), (192, 192, 192));
        assert_eq!(original_item_color(&entry("b.zip", false, 1, 1), map, basic), (128, 128, 0));
        let mut drive = entry("C:", true, 0, 0);
        drive.drive = true;
        assert_eq!(original_item_color(&drive, map, basic), (128, 128, 255));
    }
    #[test]
    fn colors_follow_extensioncolorconverter() {
        let ext: serde_json::Map<String, Value> = serde_json::from_str(r##"{".txt":"#112233"}"##).unwrap();
        let nexus: serde_json::Map<String, Value> = serde_json::from_str(r#"{"NXFOLDER":"EA2222","NXHIDDEN":"FF0000","NXFORE":"C0C0C0"}"#).unwrap();
        assert_eq!(entry_color(&entry("a.TXT", false, 0, 0), Some(&ext), None), Some((0x11, 0x22, 0x33)));
        assert_eq!(entry_color(&entry("a.doc", false, 0, 0), Some(&ext), None), None);
        assert_eq!(entry_color(&entry("dir", true, 0, 0), Some(&ext), Some(&nexus)), Some((0xEA, 0x22, 0x22)));
        assert_eq!(entry_color(&entry("h.txt", false, 0, FILE_ATTRIBUTE_HIDDEN), Some(&ext), Some(&nexus)), Some((0xFF, 0, 0)));
        assert_eq!(entry_color(&entry("r.txt", false, 0, FILE_ATTRIBUTE_READONLY), Some(&ext), Some(&nexus)), Some((0x11, 0x22, 0x33)), "no NXREADONLY: extension");
        assert_eq!(entry_color(&entry("a.doc", false, 0, 0), Some(&ext), Some(&nexus)), Some((0xC0, 0xC0, 0xC0)), "NXFORE fallback");
        assert_eq!(color_sort_key(&entry("a.txt", false, 0, 0), Some(&ext), None), 0x332211);
        assert_eq!(color_sort_key(&entry("a.doc", false, 0, 0), Some(&ext), None), u32::MAX);
        assert_eq!(color_sort_key(&entry("dir", true, 0, 0), None, None), 0x2222EA, "default NXFOLDER");
    }
}
