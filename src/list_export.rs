//! Rules of the original TfSaveList (Save File List). Preview lines follow FUN_004519a0; saving is btnSaveClick (0x452030)
//! mPreview.Lines.SaveToFile(name, TEncoding.Unicode) = BOM FF FE + UTF-16LE, CRLF per line.
use crate::parity::{dotnet_extension, dotnet_stem};
use std::time::SystemTime;

/// Values used from the original TNXFileItem: name (+4, the name without extension for files, whole for folders), extension (+8, no dot), size (+0x20),
/// time (+0x28), attributes, description (+0x10).
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
    /// Splits a list name like the list's name and extension columns (folders are not split).
    pub fn new(file_name: &str, is_dir: bool, size: u64, modified: Option<SystemTime>, attributes: u32, description: String) -> Self {
        let (name, extension) = if is_dir {
            (file_name.to_owned(), String::new())
        } else {
            (dotnet_stem(file_name).to_owned(), dotnet_extension(file_name).trim_start_matches('.').to_owned())
        };
        Self { name, extension, is_dir, size, modified, attributes, description }
    }
    /// Original TNXFileItem.GetFileName (0x7d25dc): folders keep the name, others "%s.%s" when there is an extension.
    pub fn file_name(&self) -> String {
        if self.is_dir || self.extension.is_empty() { self.name.clone() } else { format!("{}.{}", self.name, self.extension) }
    }
}

/// Columns to save (gbColumns). chkFileName is always on and not stored. Defaults as in the DFM.
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

/// rgDelimiter: 0 tab, 1 comma, 2 eUserDelimiter. A tab when the result is empty (0x8fbeee).
pub fn delimiter(index: usize, custom: &str) -> String {
    let text = match index {
        0 => "\t",
        1 => ",",
        _ => custom,
    };
    if text.is_empty() { "\t".into() } else { text.into() }
}

/// FloatToStrF(size, ffNumber, 15, 0): thousands separators.
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

/// (year, month, day, hour, minute, second) of the time. Without a time, FILETIME 0 = 1601-01-01 00:00:00.
/// Fixed: the original converted the UTC FILETIME (+0x28) directly with FileTimeToSystemTime and wrote UTC date and time
/// that differed from the list display (DateToStr, local time). Here local time is used like the list.
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

/// One line of FUN_004519a0: the name (GetFileName with chkExtension, otherwise the +4 name), then delimiter + value for each enabled column.
/// Size with thousands separators when chkSizeComma, otherwise a number. Date "%d/%02d/%02d", time "%02d:%02d:%02d", attributes GetAttrString, description.
/// Values are not quoted.
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

/// Preview (mPreview.Lines): one line per item.
pub fn preview(rows: &[ExportRow], columns: &Columns, delimiter: &str) -> String {
    rows.iter().map(|row| line(row, columns, delimiter)).collect::<Vec<_>>().join("\n")
}

/// Lines of memo text (TMemo Lines): split on CR, LF and CRLF; an empty line after the final line break does not count.
pub fn memo_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split("\r\n").flat_map(|part| part.split(['\r', '\n'])).collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// Bytes of Lines.SaveToFile(TEncoding.Unicode): BOM FF FE and UTF-16LE with CRLF after each line. None without lines
/// (btnSaveClick opens the save dialog only when Lines.Count > 0).
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
