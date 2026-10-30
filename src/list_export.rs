//! C# ListExportWindow의 값·이스케이프·미리보기 규칙(SaveListAsync는 UTF-8 BOM으로 쓴다).
use crate::parity::{dotnet_extension, dotnet_stem};
use std::time::SystemTime;

/// 열 열쇠와 언어 키, 기본 글, 처음 켜짐.
pub const COLUMNS: [(&str, &str, &str, bool); 7] = [
    ("Name", "Caption.chkFileName", "File Name", true),
    ("Extension", "Caption.chkExtension", "Extension", true),
    ("Size", "Caption.chkFileSize", "Size", true),
    ("Date", "Caption.chkDate", "Date", true),
    ("Time", "Caption.chkTime", "Time", true),
    ("Attributes", "Caption.chkAttributes", "Attributes", false),
    ("Description", "Caption.chkDescription", "Comment", false),
];
pub const EDITABLE_ROW_LIMIT: usize = 2000;
pub const SAMPLE_ROW_LIMIT: usize = 200;
pub const EDITABLE_CHARACTER_LIMIT: usize = 1_000_000;

/// C# ExportRow(FileItem)에서 쓰는 값.
#[derive(Clone, Debug, Default)]
pub struct ExportRow {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub attributes: String,
    pub description: String,
}

fn win32_short(time: SystemTime, date: bool) -> String {
    use chrono::{Datelike, Timelike};
    use windows_sys::Win32::Globalization::{GetDateFormatW, GetTimeFormatW};
    let local = chrono::DateTime::<chrono::Local>::from(time);
    let st = windows_sys::Win32::Foundation::SYSTEMTIME {
        wYear: local.year().clamp(1601, 30827) as u16,
        wMonth: local.month() as u16,
        wDayOfWeek: local.weekday().num_days_from_sunday() as u16,
        wDay: local.day() as u16,
        wHour: local.hour() as u16,
        wMinute: local.minute() as u16,
        wSecond: local.second() as u16,
        wMilliseconds: 0,
    };
    let mut buffer = vec![0u16; 128];
    // SAFETY: SYSTEMTIME and the buffer live through the call; DATE_SHORTDATE / TIME_NOSECONDS pictures come from the locale.
    let written = unsafe {
        if date {
            GetDateFormatW(0x400, 0x1, &st, std::ptr::null(), buffer.as_mut_ptr(), buffer.len() as i32)
        } else {
            GetTimeFormatW(0x400, 0x2, &st, std::ptr::null(), buffer.as_mut_ptr(), buffer.len() as i32)
        }
    };
    if written > 0 { String::from_utf16_lossy(&buffer[..written as usize - 1]) } else { String::new() }
}

/// C# Value: 이름(폴더는 전체, 파일은 확장자 뺀 이름), 확장자(점 없음), 크기("#,##0" 또는 숫자), 날짜("d"), 시간("t"), 속성, 설명.
pub fn value(row: &ExportRow, column: &str, thousands: bool) -> String {
    match column {
        "Name" => if row.is_dir { row.name.clone() } else { dotnet_stem(&row.name).to_owned() },
        "Extension" => if row.is_dir { String::new() } else { dotnet_extension(&row.name).trim_start_matches('.').to_owned() },
        "Size" if row.is_dir => String::new(),
        "Size" => if thousands { crate::parity::format_size(row.size, "B", true).trim_end_matches(" B").to_owned() } else { row.size.to_string() },
        "Date" => row.modified.map(|t| win32_short(t, true)).unwrap_or_default(),
        "Time" => row.modified.map(|t| win32_short(t, false)).unwrap_or_default(),
        "Attributes" => row.attributes.clone(),
        "Description" => row.description.clone(),
        _ => String::new(),
    }
}

/// C# Escape: 구분자·따옴표·줄바꿈이 있으면 따옴표로 감싸고 안의 따옴표는 두 번.
pub fn escape(value: &str, delimiter: &str) -> String {
    if !value.contains(delimiter) && !value.contains(['"', '\r', '\n']) {
        return value.to_owned();
    }
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// 한 줄(끝에 CRLF).
pub fn line(row: &ExportRow, columns: &[&str], delimiter: &str, thousands: bool) -> String {
    let values: Vec<String> = columns.iter().map(|c| escape(&value(row, c, thousands), delimiter)).collect();
    format!("{}\r\n", values.join(delimiter))
}

/// 전체 내보내기 글.
pub fn export_text(rows: &[ExportRow], columns: &[&str], delimiter: &str, thousands: bool) -> String {
    rows.iter().map(|row| line(row, columns, delimiter, thousands)).collect()
}

/// C# RebuildPreview: 2000줄·100만 자 안이면 전부(편집 가능), 넘으면 처음 200줄만(읽기 전용). (글, 완전한가).
pub fn preview(rows: &[ExportRow], columns: &[&str], delimiter: &str, thousands: bool) -> (String, bool) {
    let mut text = String::new();
    let mut complete = rows.len() <= EDITABLE_ROW_LIMIT;
    for row in rows.iter().take(EDITABLE_ROW_LIMIT) {
        text.push_str(&line(row, columns, delimiter, thousands));
        if text.len() > EDITABLE_CHARACTER_LIMIT {
            complete = false;
            break;
        }
    }
    if !complete {
        text = rows.iter().take(SAMPLE_ROW_LIMIT).map(|row| line(row, columns, delimiter, thousands)).collect();
    }
    (text, complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_and_escaping_follow_listexportwindow() {
        let file = ExportRow { name: "report.final.TXT".into(), size: 1_234_567, attributes: "R--A".into(), description: "say \"hi\", ok".into(), ..Default::default() };
        let folder = ExportRow { name: "photos.2024".into(), is_dir: true, ..Default::default() };
        assert_eq!(value(&file, "Name", false), "report.final");
        assert_eq!(value(&file, "Extension", false), "TXT");
        assert_eq!(value(&file, "Size", true), "1,234,567");
        assert_eq!(value(&file, "Size", false), "1234567");
        assert_eq!((value(&folder, "Name", false).as_str(), value(&folder, "Extension", false).as_str(), value(&folder, "Size", true).as_str()), ("photos.2024", "", ""));
        assert_eq!(value(&file, "Date", false), "", "no time: empty");
        assert_eq!(escape("a,b", ","), "\"a,b\"");
        assert_eq!(escape("a\tb", ","), "a\tb");
        assert_eq!(line(&file, &["Name", "Description"], ",", false), "report.final,\"say \"\"hi\"\", ok\"\r\n");
        let many: Vec<ExportRow> = (0..2001).map(|i| ExportRow { name: format!("f{i}"), ..Default::default() }).collect();
        let (text, complete) = preview(&many, &["Name"], "\t", false);
        assert!(!complete);
        assert_eq!(text.lines().count(), SAMPLE_ROW_LIMIT);
        let (text, complete) = preview(&many[..3], &["Name"], "\t", false);
        assert!(complete && text == "f0\r\nf1\r\nf2\r\n");
    }
}
