//! C# OriginalShortcutEditor: NexusFile 5.4.1 단축키 창(fShortcut, FUN_00455a78)과 단축키 지정 창(fEditShortCut: FormShortCut 0x4212b8,
//! eShortCutChange 0x4216d0, lbRemainKeysClick 0x421ba4, 예약 키 FUN_00420610, 남은 키 FUN_0047cda8, 캡션 FUN_00420c84).
//! 수식 키 상태는 Delphi TShiftState 비트: 1 = Shift, 2 = Alt, 4 = Ctrl.
use crate::compat::{shortcut_to_text, text_to_shortcut};

/// 목록 한 줄(지도 순서 번호, 이름, 캡션, 키 글).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub index: usize,
    pub name: String,
    pub caption: String,
    pub key_text: String,
}

/// 지정 창 검사 결과.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validation {
    pub tip: String,
    pub tip2: String,
    pub ok_enabled: bool,
}

/// 캡션에 쓰는 언어 접근: 메인 폼 원래 값('&' 포함)과 [RES] 값.
pub struct Texts<'a> {
    pub main_form_raw: &'a dyn Fn(&str) -> Option<String>,
    pub resource: &'a dyn Fn(&str, &str) -> String,
}

pub fn name_of(line: &str) -> &str {
    line.split_once('=').map_or("", |(name, _)| name)
}

pub fn value_of(line: &str) -> &str {
    line.split_once('=').map_or("", |(_, value)| value)
}

/// (키, 상태).
pub fn split(shortcut: u16) -> (u16, u8) {
    (shortcut & 0xff, u8::from(shortcut & 0x2000 != 0) | (u8::from(shortcut & 0x8000 != 0) << 1) | (u8::from(shortcut & 0x4000 != 0) << 2))
}

/// FUN_007655d0.
pub fn compose(key: u16, state: u8) -> u16 {
    if key > 0xff {
        return 0;
    }
    let mut value = key;
    if state & 1 != 0 {
        value += 0x2000;
    }
    if state & 4 != 0 {
        value += 0x4000;
    }
    if state & 2 != 0 {
        value += 0x8000;
    }
    value
}

fn format_s(pattern: &str, value: &str) -> String {
    match pattern.find("%s") {
        Some(at) => format!("{}{value}{}", &pattern[..at], &pattern[at + 2..]),
        None => pattern.to_owned(),
    }
}

/// FUN_00420c84 / FUN_00455a78의 캡션: 메인 폼 "Menu.이름"에서 "&X. " 머리, '&', "..."를 뗀다. 작업 폴더는 WFGOTO/WFSET,
/// 메뉴가 아닌 이름은 FUNC_이름. list_variant(FUN_00455a78)는 4자보다 짧은 캡션에도 작업 폴더 형식을 쓴다.
pub fn caption(name: &str, texts: &Texts<'_>, list_variant: bool) -> String {
    let Some(raw) = (texts.main_form_raw)(&format!("Menu.{name}")) else {
        return (texts.resource)(&format!("FUNC_{}", name.to_uppercase()), name);
    };
    let mut caption: &str = &raw;
    let owned;
    if caption.chars().count() >= 4 {
        if let Some(rest) = caption.strip_prefix('&') {
            caption = rest;
            if !caption.starts_with(|c: char| c.is_ascii_digit()) {
                let mut chars = caption.chars();
                chars.next();
                caption = chars.as_str();
            }
        }
        caption = caption.strip_prefix('.').unwrap_or(caption);
        caption = caption.strip_prefix(' ').unwrap_or(caption);
        owned = caption.replace('&', "").replace("...", "");
        caption = &owned;
    } else if !list_variant {
        return caption.to_owned();
    }
    if name.contains("miGoToWorkFolder") {
        return format_s(&(texts.resource)("WFGOTO", "Go to %s"), caption);
    }
    if name.contains("miWorkFolder") {
        return format_s(&(texts.resource)("WFSET", "Set %s"), caption);
    }
    caption.to_owned()
}

/// FUN_00455a78: 지도 순서, 거르기는 캡션+키+이름(공백 뺌)을 대소문자 무시로.
pub fn rows(lines: &[String], filter: &str, texts: &Texts<'_>) -> Vec<Row> {
    let needle = filter.replace(' ', "").to_lowercase();
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let name = name_of(line).to_owned();
            let key_text = shortcut_to_text(text_to_shortcut(value_of(line)));
            let caption = caption(&name, texts, true);
            let haystack = format!("{}{}{}", caption.replace(' ', ""), key_text.replace(' ', ""), name.replace(' ', "")).to_lowercase();
            (needle.is_empty() || haystack.contains(&needle)).then_some(Row { index, name, caption, key_text })
        })
        .collect()
}

/// FUN_0047cc18: 값 글이 정식 글과 같은 첫 줄의 이름.
pub fn assigned_name(lines: &[String], shortcut: u16) -> Option<String> {
    let text = shortcut_to_text(shortcut);
    lines.iter().find(|line| value_of(line) == text).map(|line| name_of(line).to_owned())
}

const RESERVED_TEXTS: [&str; 16] = ["Escape", "Left", "Right", "Up", "Down", "PageDown", "PageUp", "Home", "End", "Spacebar", "[", "]", "Ctrl+PageDown", "Ctrl+PageUp", "Ctrl+Home", "Ctrl+End"];
const LETTERS_AND_DIGITS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const PLAIN_PUNCTUATION: [u16; 9] = [0xdb, 0xdd, 0xbb, 0xbd, 0xc0, 0xbc, 0xbe, 0xde, 0xba];
const SHIFT_PUNCTUATION: [u16; 5] = [0xdb, 0xdd, 0xbb, 0xbd, 0xc0];
const TOOL_BUTTONS: [&str; 10] = ["tbFile", "tbFolder", "tbEdit", "tbView", "tbSystem", "tbArchive", "tbNetwork", "tbTools", "tbHelp", "tbDonation"];

fn letter_or_digit(key: u16) -> bool {
    char::from_u32(u32::from(key)).is_some_and(|c| LETTERS_AND_DIGITS.contains(c))
}

/// FUN_00420610: 지정할 수 없는 키.
pub fn is_reserved(key: u16, state: u8, main_form_raw: &dyn Fn(&str) -> Option<String>) -> bool {
    let text = shortcut_to_text(compose(key, state));
    if RESERVED_TEXTS.iter().any(|r| r.eq_ignore_ascii_case(&text)) || key == 0x09 || key == 0x1b {
        return true;
    }
    let (shift, alt, ctrl, none) = (state & 1 != 0, state & 2 != 0, state & 4 != 0, state == 0);
    if none && ((0x70..=0x77).contains(&key) || key == 0x7b) {
        return true;
    }
    if shift && !ctrl && !alt && (0x70..=0x79).contains(&key) {
        return true;
    }
    if shift && ctrl && !alt && (0x70..=0x79).contains(&key) {
        return true;
    }
    if !shift && ctrl && !alt && (0x70..=0x7a).contains(&key) {
        return true;
    }
    if none && (letter_or_digit(key) || PLAIN_PUNCTUATION.contains(&key)) {
        return true;
    }
    if shift && !ctrl && !alt && (letter_or_digit(key) || SHIFT_PUNCTUATION.contains(&key)) {
        return true;
    }
    if !shift && !ctrl && alt {
        for button in TOOL_BUTTONS {
            let Some(caption) = main_form_raw(&format!("ToolButton.{button}")) else { continue };
            let chars: Vec<char> = caption.chars().collect();
            let Some(at) = chars.iter().position(|c| *c == '&') else { continue };
            let Some(next) = chars.get(at + 1) else { continue };
            let mut accelerator = *next as u32;
            if accelerator >= 0x5b {
                accelerator -= 0x20;
            }
            if accelerator == u32::from(key) {
                return true;
            }
        }
    }
    false
}

const KEY_NAMES: [&str; 41] = [
    "Escape", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "Backspace", "Enter", "Insert", "Delete", "Home", "End", "PageUp", "PageDown", "Spacebar", "Left",
    "Right", "Up", "Down", "Num /", "Num *", "Num -", "Num +", "Num .", "Num 0", "Num 1", "Num 2", "Num 3", "Num 4", "Num 5", "Num 6", "Num 7", "Num 8", "Num 9",
];
const PUNCTUATION_CHARS: &str = "`-=[]\\;',./";

/// FUN_0047cda8 + FUN_00420610 거르기: 한 수식 키 상태에서 비어 있고 지정할 수 있는 키.
pub fn available_keys(lines: &[String], state: u8, main_form_raw: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let used: std::collections::HashSet<u16> = lines
        .iter()
        .map(|line| split(text_to_shortcut(value_of(line))))
        .filter(|(key, line_state)| *line_state == state && *key != 0)
        .map(|(key, _)| key)
        .collect();
    LETTERS_AND_DIGITS
        .chars()
        .map(String::from)
        .chain(PUNCTUATION_CHARS.chars().map(String::from))
        .chain(KEY_NAMES.iter().map(|s| (*s).to_owned()))
        .filter(|candidate| {
            let (key, _) = split(text_to_shortcut(candidate));
            !used.contains(&key) && !is_reserved(key, state, main_form_raw)
        })
        .collect()
}

/// eShortCutChange 0x4216d0.
pub fn validate(text: &str, original: &str, lines: &[String], texts: &Texts<'_>) -> Validation {
    let (key, state) = split(text_to_shortcut(text));
    if text.is_empty() || text == original || key == 0 {
        return Validation { tip: String::new(), tip2: String::new(), ok_enabled: !(state != 0 && key == 0) };
    }
    if is_reserved(key, state, texts.main_form_raw) {
        return Validation { tip: (texts.resource)("SHORTCUT_PREVENTED", "Cannot be used"), tip2: String::new(), ok_enabled: false };
    }
    if let Some(assigned) = assigned_name(lines, compose(key, state)) {
        return Validation { tip: (texts.resource)("SHORTCUT_ALREADY_ASSIGNED", "Already assigned to:"), tip2: caption(&assigned, texts, false), ok_enabled: false };
    }
    Validation { tip: String::new(), tip2: String::new(), ok_enabled: true }
}

/// FormShortCut 0x4212b8: 누른 키의 글(키는 칸에 들어가지 않는다).
pub fn captured_text(virtual_key: u16, state: u8) -> String {
    shortcut_to_text(compose(virtual_key, state))
}

/// lbRemainKeysClick 0x421ba4: "Num +" 하나를 빼고 마지막 '+'까지 남긴 뒤 항목을 붙인다.
pub fn replace_key_part(text: &str, item: &str) -> String {
    let stripped = match text.find("Num +") {
        Some(at) => format!("{}{}", &text[..at], &text[at + 5..]),
        None => text.to_owned(),
    };
    match stripped.rfind('+') {
        Some(at) => format!("{}{item}", &stripped[..=at]),
        None => item.to_owned(),
    }
}

/// FUN_0047d288은 같은 이름의 첫 줄을 고치지만 NDR은 두 번 누른 줄을 고친다(승인된 수정: Delete처럼 이름이 겹치는 경우).
pub fn set_line(lines: &[String], index: usize, text: &str) -> Vec<String> {
    let mut result = lines.to_vec();
    if let Some(line) = result.get_mut(index) {
        *line = format!("{}={}", name_of(line), shortcut_to_text(text_to_shortcut(text)));
    }
    result
}

/// 저장 글. FUN_0047cb40은 머리 없이 저장해 다음 시작에 기본값과 섞이지만, NDR은 머리를 넣어 고친 줄이 그대로 읽힌다(승인된 수정).
pub fn to_file_text(lines: &[String]) -> String {
    format!("NEXUSFILESHORTCUT1.0\r\n{}\r\n", lines.join("\r\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts_fixture() -> (impl Fn(&str) -> Option<String>, impl Fn(&str, &str) -> String) {
        let raw = |key: &str| match key {
            "Menu.miOpen" => Some("&O. &Open...".to_string()),
            "Menu.miCopy" => Some("Cop".to_string()),
            "Menu.miGoToWorkFolder1" => Some("&1. Work Folder 1".to_string()),
            "ToolButton.tbFile" => Some("&File".to_string()),
            _ => None,
        };
        let resource = |key: &str, fallback: &str| match key {
            "WFGOTO" => "Go to %s".to_string(),
            "FUNC_FINDNEXT" => "Find next".to_string(),
            "SHORTCUT_PREVENTED" => "Cannot be used".to_string(),
            "SHORTCUT_ALREADY_ASSIGNED" => "Already assigned to:".to_string(),
            _ => fallback.to_string(),
        };
        (raw, resource)
    }

    #[test]
    fn captions_rows_and_validation_follow_the_original() {
        let (raw, resource) = texts_fixture();
        let texts = Texts { main_form_raw: &raw, resource: &resource };
        assert_eq!(caption("miOpen", &texts, true), "Open");
        assert_eq!(caption("miCopy", &texts, false), "Cop", "short captions are kept as they are");
        assert_eq!(caption("miGoToWorkFolder1", &texts, true), "Go to 1. Work Folder 1", "a digit after & is kept with its dot");
        assert_eq!(caption("FindNext", &texts, true), "Find next", "internal commands use FUNC_");
        let lines: Vec<String> = ["miOpen=F3", "miCopy=Ctrl+C", "FindNext=Ctrl+L", "Delete=Shift+Delete", "Delete=Shift+Alt+D"].iter().map(|s| s.to_string()).collect();
        let all = rows(&lines, "", &texts);
        assert_eq!(all.len(), 5);
        assert_eq!(all[0], Row { index: 0, name: "miOpen".into(), caption: "Open".into(), key_text: "F3".into() });
        assert_eq!(rows(&lines, "ctrl+ l", &texts).iter().map(|r| r.index).collect::<Vec<_>>(), [2], "spaces removed, case-insensitive");
        assert_eq!(validate("", "F3", &lines, &texts).ok_enabled, true);
        assert_eq!(validate("F3", "F3", &lines, &texts).ok_enabled, true, "unchanged");
        assert_eq!(validate("A", "F3", &lines, &texts).tip, "Cannot be used", "plain letters are reserved");
        assert_eq!(validate("Alt+F", "F3", &lines, &texts).tip, "Cannot be used", "tool button accelerators are reserved");
        let taken = validate("Ctrl+C", "F3", &lines, &texts);
        assert_eq!((taken.tip.as_str(), taken.tip2.as_str(), taken.ok_enabled), ("Already assigned to:", "Cop", false));
        assert!(validate("Ctrl+Shift+K", "F3", &lines, &texts).ok_enabled);
        assert!(validate("Ctrl+", "F3", &lines, &texts).ok_enabled, "TextToShortCut gives 0 for a modifier without a key");
    }

    #[test]
    fn keys_text_and_lines_follow_the_original() {
        let (raw, _) = texts_fixture();
        let lines: Vec<String> = ["miOpen=Ctrl+A", "Delete=Shift+Delete", "Delete=Shift+Alt+D"].iter().map(|s| s.to_string()).collect();
        let free = available_keys(&lines, 4, &raw);
        assert!(!free.contains(&"A".to_string()), "Ctrl+A is used");
        assert!(free.contains(&"B".to_string()));
        assert!(!free.contains(&"F1".to_string()), "Ctrl+F1..F11 are reserved");
        assert!(!available_keys(&lines, 0, &raw).contains(&"B".to_string()), "plain letters are reserved");
        assert_eq!(captured_text(0x4B, 4 | 1), "Ctrl+Shift+K");
        assert_eq!(replace_key_part("Ctrl+Shift+K", "Q"), "Ctrl+Shift+Q");
        assert_eq!(replace_key_part("Ctrl+Num +", "Q"), "Ctrl+Q");
        assert_eq!(replace_key_part("K", "Q"), "Q");
        let updated = set_line(&lines, 2, "Shift+Alt+X");
        assert_eq!(updated[2], "Delete=Shift+Alt+X", "the double-clicked duplicate line, canonical text");
        assert_eq!(updated[1], "Delete=Shift+Delete");
        assert_eq!(to_file_text(&updated[..1]), "NEXUSFILESHORTCUT1.0\r\nmiOpen=Ctrl+A\r\n");
        assert_eq!(split(compose(0x44, 3)), (0x44, 3));
    }
}
