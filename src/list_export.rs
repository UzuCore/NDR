//! 원본 TfSaveList(파일 목록 저장하기) 규칙. 미리보기 줄은 FUN_004519a0, 저장은 btnSaveClick(0x452030)의
//! mPreview.Lines.SaveToFile(이름, TEncoding.Unicode) = BOM FF FE + UTF-16LE, 줄마다 CRLF.
use crate::parity::{dotnet_extension, dotnet_stem};
use std::time::SystemTime;

/// 원본 TNXFileItem에서 쓰는 값: 이름(+4, 파일은 확장자 뺀 이름, 폴더는 전체), 확장자(+8, 점 없음), 크기(+0x20),
/// 시각(+0x28), 속성, 설명(+0x10).
#[derive(Clone, Debug, Default)]
pub struct ExportRow {
    pub name: String,
    pub extension: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub attributes: u32,
    pub description: String,
}

impl ExportRow {
    /// 목록 이름을 목록의 이름·확장자 열처럼 나눈다(폴더는 나누지 않는다).
    pub fn new(file_name: &str, is_dir: bool, size: u64, modified: Option<SystemTime>, attributes: u32, description: String) -> Self {
        let (name, extension) = if is_dir {
            (file_name.to_owned(), String::new())
        } else {
            (dotnet_stem(file_name).to_owned(), dotnet_extension(file_name).trim_start_matches('.').to_owned())
        };
        Self { name, extension, is_dir, size, modified, attributes, description }
    }
    /// 원본 TNXFileItem.GetFileName(0x7d25dc): 폴더는 이름 그대로, 그 밖은 확장자가 있으면 "%s.%s".
    pub fn file_name(&self) -> String {
        if self.is_dir || self.extension.is_empty() { self.name.clone() } else { format!("{}.{}", self.name, self.extension) }
    }
}

/// 저장할 열(gbColumns). chkFileName은 늘 켜져 있어 따로 두지 않는다. 처음 값은 DFM 그대로.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Columns {
    pub extension: bool,
    pub size: bool,
    pub size_comma: bool,
    pub date: bool,
    pub time: bool,
    pub attributes: bool,
    pub description: bool,
}

impl Default for Columns {
    fn default() -> Self {
        Self { extension: true, size: true, size_comma: false, date: true, time: true, attributes: false, description: false }
    }
}

/// rgDelimiter: 0 탭, 1 쉼표, 2 eUserDelimiter. 결과가 비면 탭(0x8fbeee).
pub fn delimiter(index: usize, custom: &str) -> String {
    let text = match index {
        0 => "\t",
        1 => ",",
        _ => custom,
    };
    if text.is_empty() { "\t".into() } else { text.into() }
}

/// FloatToStrF(크기, ffNumber, 15, 0): 천 단위 쉼표.
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 시각의 (년, 월, 일, 시, 분, 초). 시각이 없으면 FILETIME 0 = 1601-01-01 00:00:00.
/// 고친 것: 원본은 UTC FILETIME(+0x28)을 FileTimeToSystemTime으로 바로 바꿔 목록 표시(DateToStr, 지역 시각)와
/// 다른 UTC 날짜·시간을 썼다. 여기서는 목록처럼 지역 시각을 쓴다.
fn parts(time: Option<SystemTime>) -> (i32, u32, u32, u32, u32, u32) {
    use chrono::{Datelike, Timelike};
    match time {
        Some(time) => {
            let local = chrono::DateTime::<chrono::Local>::from(time);
            (local.year(), local.month(), local.day(), local.hour(), local.minute(), local.second())
        }
        None => (1601, 1, 1, 0, 0, 0),
    }
}

/// FUN_004519a0의 한 줄: 이름(chkExtension이면 GetFileName, 아니면 +4 이름) 뒤에 켠 열마다 구분자 + 값.
/// 크기는 chkSizeComma면 천 단위 쉼표, 아니면 숫자. 날짜 "%d/%02d/%02d", 시간 "%02d:%02d:%02d", 속성 GetAttrString, 설명.
/// 값을 따옴표로 감싸지 않는다.
pub fn line(row: &ExportRow, columns: &Columns, delimiter: &str) -> String {
    let mut text = if columns.extension { row.file_name() } else { row.name.clone() };
    let (year, month, day, hour, minute, second) = parts(row.modified);
    if columns.size {
        text.push_str(delimiter);
        text.push_str(&if columns.size_comma { grouped(row.size) } else { row.size.to_string() });
    }
    if columns.date {
        text.push_str(&format!("{delimiter}{year}/{month:02}/{day:02}"));
    }
    if columns.time {
        text.push_str(&format!("{delimiter}{hour:02}:{minute:02}:{second:02}"));
    }
    if columns.attributes {
        text.push_str(delimiter);
        text.push_str(&crate::listing::attr_string(row.attributes));
    }
    if columns.description {
        text.push_str(delimiter);
        text.push_str(&row.description);
    }
    text
}

/// 미리보기(mPreview.Lines): 항목마다 한 줄.
pub fn preview(rows: &[ExportRow], columns: &Columns, delimiter: &str) -> String {
    rows.iter().map(|row| line(row, columns, delimiter)).collect::<Vec<_>>().join("\n")
}

/// 메모 글의 줄(TMemo Lines): CR·LF·CRLF로 나누고, 끝 줄바꿈 뒤의 빈 줄은 줄로 치지 않는다.
pub fn memo_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split("\r\n").flat_map(|part| part.split(['\r', '\n'])).collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// Lines.SaveToFile(TEncoding.Unicode)의 바이트: BOM FF FE, 줄마다 CRLF를 붙인 UTF-16LE. 줄이 없으면 None
/// (btnSaveClick은 Lines.Count > 0일 때만 저장 대화 상자를 연다).
pub fn file_bytes(text: &str) -> Option<Vec<u8>> {
    let lines = memo_lines(text);
    if lines.is_empty() {
        return None;
    }
    let mut bytes = vec![0xFF, 0xFE];
    for line in lines {
        for unit in line.encode_utf16().chain("\r\n".encode_utf16()) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_follow_the_original_savelist() {
        let file = ExportRow::new("report.final.TXT", false, 1_234_567, None, 0x21, "say, \"hi\"".into());
        let folder = ExportRow::new("photos.2024", true, 0, None, 0x10, String::new());
        assert_eq!((file.name.as_str(), file.extension.as_str(), file.file_name().as_str()), ("report.final", "TXT", "report.final.TXT"));
        assert_eq!(folder.file_name(), "photos.2024");
        let defaults = Columns::default();
        assert_eq!(line(&file, &defaults, "\t"), "report.final.TXT\t1234567\t1601/01/01\t00:00:00");
        let all = Columns { extension: false, size_comma: true, attributes: true, description: true, ..defaults };
        assert_eq!(line(&file, &all, ","), "report.final,1,234,567,1601/01/01,00:00:00,A_R_,say, \"hi\"", "no quoting");
        assert_eq!(line(&folder, &Columns { size: false, date: false, time: false, ..defaults }, "\t"), "photos.2024");
        assert_eq!((delimiter(0, "x"), delimiter(1, "x"), delimiter(2, "::"), delimiter(2, "")), ("\t".to_owned(), ",".to_owned(), "::".to_owned(), "\t".to_owned()));
        assert_eq!(file_bytes(""), None);
        assert_eq!(file_bytes("a\nb\n").unwrap(), [0xFF, 0xFE, b'a', 0, b'\r', 0, b'\n', 0, b'b', 0, b'\r', 0, b'\n', 0]);
        assert_eq!(memo_lines("\n"), [""], "one empty line still counts");
    }
}
