//! C# file list rules: column texts of FileSystemService.ListLocal (type, attributes, date), sorting of MainWindow.SortPaneItems,
//! group keys of FileGroupingService, group categories of FileTypeCatalog, text colors of ExtensionColorConverter.
use crate::fsops::Entry;
use crate::parity::{list_extension, list_stem};
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

/// C# AttributeText: a letter per R/H/S/A slot, '-' when absent.
pub fn attribute_text(attributes: u32) -> String {
    [(FILE_ATTRIBUTE_READONLY, 'R'), (FILE_ATTRIBUTE_HIDDEN, 'H'), (FILE_ATTRIBUTE_SYSTEM, 'S'), (FILE_ATTRIBUTE_ARCHIVE, 'A')]
        .iter()
        .map(|(bit, letter)| if attributes & bit != 0 { *letter } else { '-' })
        .collect()
}

/// Original GetAttrString: in "____", position 0 A (archive), 1 H (hidden), 2 R (read-only), 3 S (system).
pub fn attr_string(attributes: u32) -> String {
    let mut text = ['_'; 4];
    for (bit, slot, letter) in [(FILE_ATTRIBUTE_ARCHIVE, 0, 'A'), (FILE_ATTRIBUTE_HIDDEN, 1, 'H'), (FILE_ATTRIBUTE_READONLY, 2, 'R'), (FILE_ATTRIBUTE_SYSTEM, 3, 'S')] {
        if attributes & bit != 0 {
            text[slot] = letter;
        }
    }
    text.iter().collect()
}

/// Kind of C# ListLocal: folders are "folder", "file" without an extension, otherwise "{EXT} file" (upper case). English uses the same form.
pub fn type_name(name: &str, is_dir: bool, korean: bool) -> String {
    if is_dir {
        return if korean { "폴더" } else { "Folder" }.into();
    }
    let extension = list_extension(name);
    if extension.is_empty() {
        return if korean { "파일" } else { "File" }.into();
    }
    let upper = extension.trim_start_matches('.').to_uppercase();
    if korean { format!("{upper} 파일") } else { format!("{upper} File") }
}

/// SYSTEMTIME (local time).
fn local_systemtime(time: SystemTime) -> windows_sys::Win32::Foundation::SYSTEMTIME {
    systemtime_of(&chrono::DateTime::<chrono::Local>::from(time))
}

fn systemtime_of(local: &chrono::DateTime<chrono::Local>) -> windows_sys::Win32::Foundation::SYSTEMTIME {
    use chrono::{Datelike, Timelike};
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

/// User locale value (GetLocaleInfoW).
fn locale_info(kind: u32) -> Option<String> {
    let mut buffer = vec![0u16; 16];
    // SAFETY: the buffer length is passed with the buffer.
    let written = unsafe { windows_sys::Win32::Globalization::GetLocaleInfoW(0x400, kind, buffer.as_mut_ptr(), buffer.len() as i32) };
    (written > 1).then(|| String::from_utf16_lossy(&buffer[..written as usize - 1]))
}

/// Date separator ('/' slot, LOCALE_SDATE) and time separator (':' slot, LOCALE_STIME) of the current .NET culture.
pub fn date_separators() -> (String, String) {
    (locale_info(0x1D).unwrap_or_else(|| "/".into()), locale_info(0x1E).unwrap_or_else(|| ":".into()))
}

/// Turns a .NET custom format into a Win32 picture string: '/' and ':' are culture separators, "..." and '\x' are literal (wrapped in single quotes).
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

/// GetDateFormatW / GetTimeFormatW (LOCALE_USER_DEFAULT, picture string). Uses the same letters as .NET custom formats (yyyy, MM, dd, HH, mm, ss, tt...).
/// Fixed (speed): previously each item read the locale separators twice (GetLocaleInfoW) and rebuilt the picture string, so building cell texts for a 20,000-file list
/// took nearly 70 ms. The picture string is reused for a few seconds (locale changes are picked up soon), and texts for the same day and time are reused too.
fn win32_format(time: SystemTime, format: &str, date: bool) -> Option<String> {
    win32_format_st(&local_systemtime(time), format, date)
}

fn win32_format_st(st: &windows_sys::Win32::Foundation::SYSTEMTIME, format: &str, date: bool) -> Option<String> {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};
    thread_local! {
        static PICTURES: RefCell<(Option<Instant>, HashMap<String, Vec<u16>>)> = RefCell::new((None, HashMap::new()));
        static TEXTS: RefCell<HashMap<(bool, String, i64), String>> = RefCell::new(HashMap::new());
    }
    if format.is_empty() {
        return Some(String::new());
    }
    // Date text depends only on the day, time text only on the second.
    let slot = if date { (st.wYear as i64) * 10_000 + (st.wMonth as i64) * 100 + st.wDay as i64 } else { (st.wHour as i64) * 3600 + (st.wMinute as i64) * 60 + st.wSecond as i64 };
    let fresh = PICTURES.with(|p| {
        let mut p = p.borrow_mut();
        let stale = p.0.is_none_or(|at| at.elapsed() > Duration::from_secs(5));
        if stale {
            p.0 = Some(Instant::now());
            p.1.clear();
        }
        stale
    });
    if fresh {
        TEXTS.with(|t| t.borrow_mut().clear());
    }
    let key = (date, format.to_owned(), slot);
    if let Some(text) = TEXTS.with(|t| t.borrow().get(&key).cloned()) {
        return Some(text);
    }
    let picture = PICTURES.with(|p| p.borrow_mut().1.entry(format.to_owned()).or_insert_with(|| wide(&dotnet_to_picture(format))).clone());
    let text = win32_format_with(st, &picture, date)?;
    TEXTS.with(|t| {
        let mut t = t.borrow_mut();
        if t.len() > 20_000 {
            t.clear();
        }
        t.insert(key, text.clone());
    });
    Some(text)
}

fn win32_format_with(st: &windows_sys::Win32::Foundation::SYSTEMTIME, picture: &[u16], date: bool) -> Option<String> {
    use windows_sys::Win32::Globalization::{GetDateFormatW, GetTimeFormatW};
    const LOCALE_USER_DEFAULT: u32 = 0x400;
    let mut buffer = [0u16; 256];
    // SAFETY: SYSTEMTIME and both strings live through the call; the buffer length is passed.
    let written = unsafe {
        if date {
            GetDateFormatW(LOCALE_USER_DEFAULT, 0, st, picture.as_ptr(), buffer.as_mut_ptr(), buffer.len() as i32)
        } else {
            GetTimeFormatW(LOCALE_USER_DEFAULT, 0, st, picture.as_ptr(), buffer.as_mut_ptr(), buffer.len() as i32)
        }
    };
    (written > 0).then(|| String::from_utf16_lossy(&buffer[..written as usize - 1]))
}

/// Original FUN_00422068 (name conflict window): GetDateFormatW(LOCALE_SYSTEM_DEFAULT 0x800, DATE_LONGDATE 2) and
/// GetTimeFormatW(0x800, LOCALE_NOUSEROVERRIDE 0x80000000), the system long date and time. "yyyy-MM-dd", "HH:mm:ss" when unreadable.
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

/// .NET DateTime.ToString("g", CurrentCulture): short date + short time (GetDateFormatW DATE_SHORTDATE, GetTimeFormatW TIME_NOSECONDS).
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

/// FormatDate(value, DateFormat, TimeFormat, UseDateAge): empty text without a date. Date ages use the original form (`age_date_text`,
/// Korean/English defaults). Otherwise "date format time format", "yyyy-MM-dd HH:mm" when a format is invalid.
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

/// Date age of original TNXCustomFileListView::DateToStr (0x7df294): when the calendar day difference (FUN_007df08c, -1 for a future date) is 0,
/// labels[0] (TODAY); 1 gives labels[1] (YESTERDAY), 2-30 labels[2] (DATEAGE with %d = days), otherwise the date format.
/// The column text is "date time" like GetFieldMaxWidth (TimeToStr = time format).
pub fn age_date_text(modified: Option<SystemTime>, date_format: &str, time_format: &str, labels: [&str; 3], now: SystemTime) -> String {
    date_time_parts(modified, date_format, time_format, Some(labels), now).map(|(date, clock)| format!("{date} {clock}")).unwrap_or_default()
}

/// Between date text and time text in the list date cell. Original DrawListItem (field 4) draws DateToStr at the cell left and TimeToStr at the right
/// (DT_RIGHT) separately. U+00A0 is used so it is not confused with ordinary spaces of date ages ("5일 전", "5 days ago") or format text.
pub const LIST_DATE_TIME_GAP: char = '\u{a0}';

/// List date cell text: (date text)U+00A0(time text). With labels, date ages (UseDateAge, same rule as `age_date_text`).
pub fn list_date_text(modified: Option<SystemTime>, date_format: &str, time_format: &str, labels: Option<[&str; 3]>, now: SystemTime) -> String {
    date_time_parts(modified, date_format, time_format, labels, now).map(|(date, clock)| format!("{date}{LIST_DATE_TIME_GAP}{clock}")).unwrap_or_default()
}

/// (date text, time text) of original DateToStr (0x7df294) and TimeToStr (0x7df550). ("yyyy-MM-dd", "HH:mm") when a format is invalid.
fn date_time_parts(modified: Option<SystemTime>, date_format: &str, time_format: &str, labels: Option<[&str; 3]>, now: SystemTime) -> Option<(String, String)> {
    let time = modified?;
    // Fixed (speed): the local time conversion (chrono Local) done four times per item is now done once, and today's date is reused for the same now.
    let local = chrono::DateTime::<chrono::Local>::from(time);
    let age = labels.and_then(|labels| {
        let file_day = local.date_naive();
        let today = today_of(now);
        let days = if file_day > today { -1 } else { (today - file_day).num_days() };
        match days {
            0 => Some(labels[0].to_owned()),
            1 => Some(labels[1].to_owned()),
            2..=30 => Some(labels[2].replace("%d", &days.to_string())),
            _ => None,
        }
    });
    let st = systemtime_of(&local);
    match (age.or_else(|| win32_format_st(&st, date_format, true)), win32_format_st(&st, time_format, false)) {
        (Some(date), Some(clock)) => Some((date, clock)),
        _ => Some((local.format("%Y-%m-%d").to_string(), local.format("%H:%M").to_string())),
    }
}

/// Local date of `now` (converted once when the same now is asked repeatedly).
fn today_of(now: SystemTime) -> chrono::NaiveDate {
    use std::cell::Cell;
    thread_local! {
        static TODAY: Cell<Option<(SystemTime, chrono::NaiveDate)>> = const { Cell::new(None) };
    }
    TODAY.with(|cell| match cell.get() {
        Some((at, day)) if at == now => day,
        _ => {
            let day = chrono::DateTime::<chrono::Local>::from(now).date_naive();
            cell.set(Some((now, day)));
            day
        }
    })
}

/// StrCmpLogicalW (UseLogicalSort) or a case-insensitive comparison in the current culture (CompareStringW NORM_IGNORECASE).
pub fn compare_text(a: &str, b: &str, logical: bool) -> Ordering {
    compare_wide(&wide(a), &wide(b), logical)
}

/// Base of compare_text: between UTF-16 texts already ending with NUL.
fn compare_wide(a: &[u16], b: &[u16], logical: bool) -> Ordering {
    // SAFETY: both buffers are NUL-terminated and live through the call.
    let result = unsafe {
        if logical {
            windows_sys::Win32::UI::Shell::StrCmpLogicalW(a.as_ptr(), b.as_ptr())
        } else {
            const LOCALE_USER_DEFAULT: u32 = 0x400;
            const NORM_IGNORECASE: u32 = 0x1;
            match windows_sys::Win32::Globalization::CompareStringW(LOCALE_USER_DEFAULT, NORM_IGNORECASE, a.as_ptr(), -1, b.as_ptr(), -1) {
                // On failure (0) the result used to be always "less", which could break the sort order (Rust sort may panic then). Compares by UTF-16 value order.
                0 => return a.cmp(b),
                raw => raw - 2,
            }
        }
    };
    result.cmp(&0)
}

/// Settings of SortPaneItems (per pane): folder and file sort modes with descending flags, logical sort.
#[derive(Clone, Debug)]
pub struct SortOptions {
    pub folder_mode: String,
    pub folder_descending: bool,
    pub file_mode: String,
    pub file_descending: bool,
    pub logical: bool,
}

fn sort_base(entry: &Entry) -> &str {
    if entry.is_dir { &entry.name } else { list_stem(&entry.name) }
}

fn sort_extension(entry: &Entry) -> &str {
    if entry.is_dir { "" } else { list_extension(&entry.name) }
}

/// LegacyPriorityRank: folders 1, pinned 2, normal 3, recall on open 4.
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

/// C# SortPaneItems (before grouping): folders (folder mode) then files (priority rank ascending, then the file mode within). The sort is stable, so
/// "None" is the received order. Descending reverses the whole key except the rank (OrderByDescending.ThenByDescending).
pub fn sort_entries(entries: &mut Vec<Entry>, options: &SortOptions, color: &dyn Fn(&Entry) -> u32) {
    let (mut folders, mut files): (Vec<Entry>, Vec<Entry>) = std::mem::take(entries).into_iter().partition(|e| e.is_dir);
    let (folder_mode, folder_descending) = effective_folder_sort(options);
    sort_group(&mut folders, folder_mode, folder_descending, options.logical, color, false);
    sort_group(&mut files, &options.file_mode, options.file_descending, options.logical, color, true);
    entries.extend(folders);
    entries.extend(files);
}

/// Folder sort "Auto" = original foldersort 0 (the default). TNXFileList sort FUN_007d3350: with folder type 0 the folders take the file
/// type, except extension (2), size (3) and color (6), which become name (1); no file sort (0) leaves both unsorted. The folder comparer
/// FUN_007d32ac reverses with the same descending flag (+0x24) as the files (the original has no separate folder descend setter).
/// Previously NDR sorted folders by name unless the C# folder sort was set, so a time sort left the folders in name order.
pub fn effective_folder_sort(options: &SortOptions) -> (&str, bool) {
    if options.folder_mode != "Auto" {
        return (&options.folder_mode, options.folder_descending);
    }
    let mode = match options.file_mode.as_str() {
        "None" => "None",
        "Modified" => "Modified",
        "Attributes" => "Attributes",
        _ => "Name",
    };
    (mode, options.file_descending)
}

/// One group of sort_entries (folders or files). Same order as compare_by_mode, but fixed (speed): comparison texts (UTF-16) and colors are built once
/// per item. Previously each comparison converted both names to UTF-16 (about 300,000 times for 20,000 items) and sorting froze the screen for over 70 ms.
fn sort_group(list: &mut Vec<Entry>, mode: &str, descending: bool, logical: bool, color: &dyn Fn(&Entry) -> u32, priority: bool) {
    struct Key {
        base: Vec<u16>,
        ext: Vec<u16>,
        color: u32,
        rank: u8,
    }
    let keys: Vec<Key> = list
        .iter()
        .map(|e| Key { base: wide(sort_base(e)), ext: wide(sort_extension(e)), color: if mode == "Color" { color(e) } else { 0 }, rank: if priority { priority_rank(e) } else { 0 } })
        .collect();
    let mut order: Vec<usize> = (0..list.len()).collect();
    order.sort_by(|&i, &j| {
        let (a, b, ka, kb) = (&list[i], &list[j], &keys[i], &keys[j]);
        let base = || compare_wide(&ka.base, &kb.base, logical);
        let ext = || compare_wide(&ka.ext, &kb.ext, false);
        let by_mode = match mode {
            "None" => Ordering::Equal,
            "Extension" => ext().then_with(base),
            "Size" => a.size.cmp(&b.size).then_with(base).then_with(ext),
            "Modified" => a.modified.cmp(&b.modified).then_with(ext).then_with(base),
            "Attributes" => a.attributes.cmp(&b.attributes).then_with(ext).then_with(base),
            "Color" => ka.color.cmp(&kb.color).then_with(ext).then_with(base),
            _ => base().then_with(ext),
        };
        ka.rank.cmp(&kb.rank).then(if descending { by_mode.reverse() } else { by_mode })
    });
    let mut taken: Vec<Option<Entry>> = std::mem::take(list).into_iter().map(Some).collect();
    *list = order.into_iter().filter_map(|i| taken[i].take()).collect();
}

/// Group categories of FileTypeCatalog (extensions with the dot, case-insensitive).
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
    // In C# a later category wins (.ts is Code after Video, .html is Code).
    let lower = extension.to_ascii_lowercase();
    TABLE.iter().rev().find(|(_, list)| list.contains(&lower.as_str())).map_or("Other", |(name, _)| name)
}

/// FileGroupingService.GetGroupKey: "order|name". Folders are "01|Folders" ("05|Folders" when the folder category is not Auto in category grouping).
pub fn group_key(entry: &Entry, mode: &str, folder_category_mode: &str) -> String {
    if entry.is_dir {
        return if mode != "Category" || folder_category_mode == "Auto" { "01|Folders".into() } else { "05|Folders".into() };
    }
    match mode {
        "Extension" => {
            let extension = list_extension(&entry.name);
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
            let order = match group_category(list_extension(&entry.name)) {
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

/// End of SortPaneItems: with grouping, sorts stably again by group key order (rank, then culture comparison, reversed for descending groups).
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

/// Header text of GroupHeaderNameConverter: "Folders" is [RES] FOLDERS, known names use language item keys, others the name as is.
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

/// Base element colors of NexusFileColorMapService.
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

/// Looks up a color table value (case-insensitive).
fn lookup<'a>(map: Option<&'a serde_json::Map<String, Value>>, key: &str) -> Option<&'a str> {
    map?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).and_then(|(_, v)| v.as_str())
}

/// ExtensionColorConverter: folders NXFOLDER, hidden NXHIDDEN, read-only NXREADONLY (when the color table is used),
/// others the extension color, NXFORE when none. Without the color table only extension colors. No result means the default text color.
pub fn entry_color(entry: &Entry, extension_colors: Option<&serde_json::Map<String, Value>>, nexus_colors: Option<&serde_json::Map<String, Value>>) -> Option<(u8, u8, u8)> {
    let color = if entry.is_dir {
        lookup(nexus_colors, "NXFOLDER")
    } else if entry.attributes & FILE_ATTRIBUTE_HIDDEN != 0 && lookup(nexus_colors, "NXHIDDEN").is_some() {
        lookup(nexus_colors, "NXHIDDEN")
    } else if entry.attributes & FILE_ATTRIBUTE_READONLY != 0 && lookup(nexus_colors, "NXREADONLY").is_some() {
        lookup(nexus_colors, "NXREADONLY")
    } else {
        lookup(extension_colors, list_extension(&entry.name))
    };
    let color = color.filter(|c| !c.trim().is_empty()).or_else(|| lookup(nexus_colors, "NXFORE"));
    color.and_then(hex_color)
}

/// Item color of original FileListChangedBeforeSort (0x7d7924) (+0x34, name cell text color): folders (kind 1) GetBasic(2) NXFOLDER,
/// files (kind 3) the extension color (GetExtension 0x7d3894, otherwise GetBasic(1) NXFORE), others (drives kind 4, kind 2) GetBasic(3) NXDRIVE.
/// Hidden and read-only do not change the name color (only the size cell uses NXHIDDEN and NXREADONLY).
pub fn original_item_color(entry: &Entry, extension_colors: Option<&serde_json::Map<String, Value>>, basic: impl Fn(&str) -> (u8, u8, u8)) -> (u8, u8, u8) {
    if entry.drive {
        basic("NXDRIVE")
    } else if entry.is_dir {
        basic("NXFOLDER")
    } else {
        lookup(extension_colors, list_extension(&entry.name)).and_then(hex_color).unwrap_or_else(|| basic("NXFORE"))
    }
}

/// Extension colors of the original default color table (default\nexusfile.col [Ext]). The initial value when there are no extension colors at all (settings.rs).
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

/// TryGetTColorSortKey: "RRGGBB" -> Delphi TColor (0x00BBGGRR). Color sorting puts colors equal to the default text color, or missing colors, last.
pub fn color_sort_key(entry: &Entry, extension_colors: Option<&serde_json::Map<String, Value>>, nexus_colors: Option<&serde_json::Map<String, Value>>) -> u32 {
    let tcolor = |value: Option<&str>| value.and_then(hex_color).map(|(r, g, b)| (u32::from(b) << 16) | (u32::from(g) << 8) | u32::from(r));
    let element = |key: &str| lookup(nexus_colors, key).filter(|v| hex_color(v).is_some()).or_else(|| default_element_color(key));
    if entry.is_dir {
        return tcolor(element("NXFOLDER")).unwrap_or(u32::MAX);
    }
    let Some(file) = tcolor(lookup(extension_colors, list_extension(&entry.name))) else { return u32::MAX };
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

    /// Original FUN_007d3350 with foldersort 0: folders follow a time or attribute sort and its descending flag; extension, size and color
    /// sorts keep the folders in name order.
    #[test]
    fn auto_folder_sort_follows_the_file_sort() {
        let at = |name: &str, secs: u64| Entry { modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs)), ..entry(name, true, 0, 0x10) };
        let folders = || vec![at("Languages", 30), at("Skins", 10), at("Tools", 20)];
        let auto = |file: &str, descending: bool| {
            let mut list = folders();
            let options = SortOptions { folder_mode: "Auto".into(), folder_descending: false, file_mode: file.into(), file_descending: descending, logical: true };
            sort_entries(&mut list, &options, &|_| 0);
            list.into_iter().map(|e| e.name).collect::<Vec<_>>()
        };
        assert_eq!(auto("Modified", false), ["Skins", "Tools", "Languages"]);
        assert_eq!(auto("Modified", true), ["Languages", "Tools", "Skins"]);
        assert_eq!(auto("Size", false), ["Languages", "Skins", "Tools"]);
        assert_eq!(auto("Extension", true), ["Tools", "Skins", "Languages"]);
        assert_eq!(auto("None", false), ["Languages", "Skins", "Tools"]);
        assert_eq!(effective_folder_sort(&options("Name", "Modified", false)), ("Name", false));
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
        // Original DateToStr: calendar day difference 0/1/2-30 -> TODAY/YESTERDAY/DATEAGE, followed by the time. From 31 days and future dates the date format.
        let clock = |t: SystemTime| chrono::DateTime::<chrono::Local>::from(t).format("%H:%M").to_string();
        let midday = chrono::Local::now().date_naive().and_hms_opt(12, 0, 0).unwrap().and_local_timezone(chrono::Local).unwrap();
        let noon: SystemTime = midday.into();
        assert_eq!(date_text(Some(noon - Duration::from_secs(60)), "yyyy-MM-dd", "HH:mm", true, true, noon), "오늘 11:59");
        assert_eq!(date_text(Some(noon - Duration::from_secs(86_400)), "yyyy-MM-dd", "HH:mm", true, false, noon), "Yesterday 12:00");
        let five = noon - Duration::from_secs(5 * 86_400);
        assert_eq!(date_text(Some(five), "yyyy-MM-dd", "HH:mm", true, true, noon), format!("5일 전 {}", clock(five)));
        assert_eq!(age_date_text(Some(five), "yyyy-MM-dd", "HH:mm", ["T", "Y", "%d ago"], noon), format!("5 ago {}", clock(five)));
        // List cell: U+00A0 between date text (including date ages with spaces) and time text (drawn separately at the cell left and right).
        assert_eq!(list_date_text(Some(five), "yyyy-MM-dd", "HH:mm", Some(["T", "Y", "%d days ago"]), noon), format!("5 days ago\u{a0}{}", clock(five)));
        assert_eq!(list_date_text(None, "yyyy-MM-dd", "HH:mm", None, noon), "");
        let local = chrono::DateTime::<chrono::Local>::from(five);
        assert_eq!(list_date_text(Some(five), "yyyy-MM-dd", "HH:mm", None, noon), format!("{}\u{a0}{}", local.format("%Y-%m-%d"), clock(five)));
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
        // Original 0x7d7924: folders NXFOLDER, files the extension color (otherwise NXFORE), drives NXDRIVE. Hidden and read-only do not change the name color.
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
