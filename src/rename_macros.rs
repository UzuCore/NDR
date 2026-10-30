//! C# AdvancedRenameMacroStore: NexusFile advrename.save(절 = 매크로 이름, 줄 = 동작) 읽기·쓰기.
use crate::compat::RenameRule;
use crate::split::{ansi_bytes, ansi_text};
use std::{fs, path::Path};

pub const FILE_NAME: &str = "advrename.save";

/// C# RenameMacroPreset. legacy_rows: 읽었지만 동작으로 바꿀 수 없는 줄(그대로 다시 쓴다).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenameMacro {
    pub name: String,
    pub rules: Vec<RenameRule>,
    pub legacy_rows: Vec<String>,
}

impl PartialEq for RenameRule {
    fn eq(&self, other: &Self) -> bool {
        serde_json::to_value(self).ok() == serde_json::to_value(other).ok()
    }
}

fn decode(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |rest: &[u8], big: bool| -> Result<String, String> {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
        String::from_utf16(&units).map_err(|e| e.to_string())
    };
    let utf32 = |rest: &[u8], big: bool| -> Result<String, String> {
        rest.chunks_exact(4)
            .map(|c| {
                let value = if big { u32::from_be_bytes([c[0], c[1], c[2], c[3]]) } else { u32::from_le_bytes([c[0], c[1], c[2], c[3]]) };
                char::from_u32(value).ok_or_else(|| "Invalid UTF-32 text".to_string())
            })
            .collect()
    };
    if let Some(rest) = bytes.strip_prefix(&[0x00, 0x00, 0xfe, 0xff]) {
        utf32(rest, true)
    } else if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe, 0x00, 0x00]) {
        utf32(rest, false)
    } else if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        String::from_utf8(rest.to_vec()).map_err(|e| e.to_string())
    } else if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        utf16(rest, true)
    } else if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        utf16(rest, false)
    } else {
        Ok(ansi_text(bytes))
    }
}

fn parse_int(value: &str) -> Option<i64> {
    value.trim().parse::<i32>().ok().map(i64::from)
}

fn parse_target(value: &str) -> String {
    if value.eq_ignore_ascii_case("ext") { "Extension".into() } else { "Name".into() }
}

/// TryParsePosition: first/last(/all) 또는 숫자.
fn parse_position(value: &str, allow_all: bool) -> Option<(String, i64)> {
    if value.eq_ignore_ascii_case("first") {
        Some((if allow_all { "First" } else { "Prefix" }.into(), 1))
    } else if value.eq_ignore_ascii_case("last") {
        Some((if allow_all { "Last" } else { "Suffix" }.into(), 1))
    } else if allow_all && value.eq_ignore_ascii_case("all") {
        Some(("All".into(), 1))
    } else {
        parse_int(value).map(|index| ("Position".into(), index))
    }
}

/// TryParseRule.
pub fn parse_rule(row: &str) -> Option<RenameRule> {
    let fields: Vec<&str> = row.split(';').collect();
    let mut rule = RenameRule::default();
    match (fields[0].to_lowercase().as_str(), fields.len()) {
        ("ins", 4) => {
            rule.action = "Insert".into();
            rule.text = fields[1].into();
            (rule.position, rule.index) = parse_position(fields[2], false)?;
            rule.target = parse_target(fields[3]);
        }
        ("rep", 5) => {
            rule.action = "Replace".into();
            rule.find = fields[1].into();
            rule.replace = fields[2].into();
            (rule.position, rule.index) = parse_position(fields[3], true)?;
            rule.target = parse_target(fields[4]);
        }
        ("del", 6) => {
            rule.action = "Delete".into();
            let from = fields[1];
            if from.eq_ignore_ascii_case("<first>") {
                rule.delete_from_mode = "FromStart".into();
            } else if from.eq_ignore_ascii_case("<last>") {
                rule.delete_from_mode = "FromEnd".into();
            } else if let Some(index) = parse_int(from) {
                rule.delete_from_mode = "Position".into();
                rule.index = index;
            } else {
                rule.delete_from_mode = "Delimiter".into();
                rule.find = if from.len() >= 2 && from.starts_with('"') && from.ends_with('"') { from[1..from.len() - 1].into() } else { from.into() };
            }
            rule.delete_to_delimiter = fields[2].into();
            rule.delete_to_mode = if fields[2].is_empty() { "Count" } else { "Delimiter" }.into();
            rule.count = parse_int(fields[3])?;
            rule.delete_direction = if fields[4].eq_ignore_ascii_case("forward") { "LeftToRight" } else { "RightToLeft" }.into();
            rule.target = parse_target(fields[5]);
        }
        ("cas", 3) => {
            rule.action = "Case".into();
            rule.case_mode = match fields[1].to_lowercase().as_str() {
                "lower" => "Lower",
                "upperword" => "WordFirst",
                "upperfirst" => "FirstLetter",
                _ => "Upper",
            }
            .into();
            rule.target = parse_target(fields[2]);
        }
        ("num", 6) => {
            rule.action = "Number".into();
            rule.start = parse_int(fields[1])?;
            rule.step = parse_int(fields[2])?;
            rule.digits = usize::try_from(parse_int(fields[3])?).ok()?;
            (rule.position, rule.index) = parse_position(fields[4], false)?;
            rule.target = parse_target(fields[5]);
        }
        ("ext", 2) => {
            rule.action = "Extension".into();
            rule.target = "Extension".into();
            rule.text = fields[1].into();
        }
        _ => return None,
    }
    Some(rule)
}

/// SerializeRule.
pub fn serialize_rule(rule: &RenameRule) -> String {
    let target = if rule.target.eq_ignore_ascii_case("Extension") { "ext" } else { "name" };
    let position = match rule.position.as_str() {
        "Prefix" => "first".to_owned(),
        "Suffix" => "last".to_owned(),
        "First" | "Last" | "All" => rule.position.clone(),
        _ => rule.index.to_string(),
    };
    match rule.action.as_str() {
        "Insert" => format!("ins;{};{position};{target}", rule.text),
        "Replace" => format!("rep;{};{};{position};{target}", rule.find, rule.replace),
        "Delete" => {
            let from = match rule.delete_from_mode.as_str() {
                "FromStart" => "<first>".to_owned(),
                "FromEnd" => "<last>".to_owned(),
                "Delimiter" => format!("\"{}\"", rule.find),
                _ => rule.index.to_string(),
            };
            let to = if rule.delete_to_mode == "Delimiter" { rule.delete_to_delimiter.as_str() } else { "" };
            let direction = if rule.delete_direction == "LeftToRight" { "forward" } else { "backward" };
            format!("del;{from};{to};{};{direction};{target}", rule.count)
        }
        "Case" => format!("cas;{};{target}", match rule.case_mode.as_str() {
            "Lower" => "lower",
            "WordFirst" => "upperword",
            "FirstLetter" => "upperfirst",
            _ => "upper",
        }),
        "Number" => format!("num;{};{};{};{position};{target}", rule.start, rule.step, rule.digits),
        _ => format!("ext;{}", rule.text),
    }
}

/// CanSave: 이름에 줄바꿈·제어 문자, 알 수 없는 동작, 값에 ';'·줄바꿈이 있으면 그 값과 함께 Err.
pub fn can_save(macros: &[RenameMacro]) -> Result<(), String> {
    for item in macros {
        if item.name.chars().any(char::is_control) || item.legacy_rows.iter().any(|row| row.contains(['\r', '\n'])) {
            return Err(item.name.clone());
        }
        for rule in &item.rules {
            let values: Vec<&str> = match rule.action.as_str() {
                "Insert" | "Extension" => vec![&rule.text],
                "Replace" => vec![&rule.find, &rule.replace],
                "Delete" => vec![&rule.find, &rule.delete_to_delimiter],
                "Case" | "Number" => vec![],
                other => return Err(other.to_owned()),
            };
            if let Some(value) = values.into_iter().find(|v| v.contains([';', '\r', '\n'])) {
                return Err(value.to_owned());
            }
        }
    }
    Ok(())
}

/// Read: "[이름]" 절, 빈 줄은 건너뛰고, 읽을 수 없는 줄은 legacy_rows.
pub fn read(path: &Path) -> Result<Vec<RenameMacro>, String> {
    let text = decode(&fs::read(path).map_err(|e| e.to_string())?)?;
    let mut macros: Vec<RenameMacro> = Vec::new();
    let mut current: Option<usize> = None;
    for line in text.split("\r\n").flat_map(|l| l.split('\n')).flat_map(|l| l.split('\r')) {
        if line.starts_with('[') {
            let name = if line.len() >= 2 && line.ends_with(']') { &line[1..line.len() - 1] } else { "" };
            current = if name.trim().is_empty() {
                None
            } else {
                macros.push(RenameMacro { name: name.into(), ..Default::default() });
                Some(macros.len() - 1)
            };
            continue;
        }
        let Some(index) = current else { continue };
        if line.is_empty() {
            continue;
        }
        match parse_rule(line) {
            Some(rule) => macros[index].rules.push(rule),
            None => macros[index].legacy_rows.push(line.into()),
        }
    }
    Ok(macros)
}

/// SaveTo: ANSI(못 쓰면 UTF-8 BOM), CRLF, 임시 파일 뒤 바꾸기.
pub fn save(path: &Path, macros: &[RenameMacro]) -> Result<(), String> {
    can_save(macros).map_err(|value| format!("NexusFile's advrename.save format cannot store semicolons or line breaks in macro values ({value})."))?;
    let mut lines = Vec::new();
    for item in macros {
        lines.push(format!("[{}]", item.name));
        lines.extend(item.rules.iter().map(serialize_rule));
        lines.extend(item.legacy_rows.iter().cloned());
    }
    let text = if lines.is_empty() { String::new() } else { format!("{}\r\n", lines.join("\r\n")) };
    let bytes = ansi_bytes(&text).unwrap_or_else(|| [&[0xef, 0xbb, 0xbf][..], text.as_bytes()].concat());
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    }
    let temporary = path.with_extension("save.tmp");
    let result = fs::write(&temporary, bytes).and_then(|_| fs::rename(&temporary, path)).map_err(|e| e.to_string());
    let _ = fs::remove_file(&temporary);
    result
}

/// Load: 프로필의 advrename.save, 없으면 실행 파일 옆의 것(가능하면 프로필로 복사), 없으면 설정 사본(가능하면 파일로).
pub fn load(profile: &Path, beside_executable: &Path, configured: Vec<RenameMacro>) -> Result<Vec<RenameMacro>, String> {
    let user = profile.join(FILE_NAME);
    if user.is_file() {
        return read(&user);
    }
    let beside = beside_executable.join(FILE_NAME);
    if beside.is_file() {
        let imported = read(&beside)?;
        if can_save(&imported).is_ok() {
            let _ = save(&user, &imported);
        }
        return Ok(imported);
    }
    if !configured.is_empty() && can_save(&configured).is_ok() {
        let _ = save(&user, &configured);
    }
    Ok(configured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advrename_save_round_trip() {
        let root = std::env::temp_dir().join(format!("ndr-macros-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&root).unwrap();
        let text = "[사진 정리]\r\nins;IMG_;first;name\r\nrep;a;b;all;name\r\ndel;\"(\";);0;forward;name\r\ncas;upperword;name\r\nnum;1;2;3;last;name\r\next;jpg\r\nweird row\r\n\r\n[]\r\nins;x;first;name\r\n";
        let path = root.join(FILE_NAME);
        fs::write(&path, text.as_bytes()).unwrap();
        let mut macros = read(&path).unwrap();
        assert_eq!(macros.len(), 1, "an empty section name starts no macro");
        let rules = &macros[0].rules;
        assert_eq!(rules.len(), 6);
        assert_eq!((rules[0].action.as_str(), rules[0].position.as_str(), rules[0].text.as_str()), ("Insert", "Prefix", "IMG_"));
        assert_eq!((rules[1].position.as_str(), rules[1].find.as_str()), ("All", "a"));
        assert_eq!((rules[2].delete_from_mode.as_str(), rules[2].find.as_str(), rules[2].delete_to_mode.as_str(), rules[2].delete_to_delimiter.as_str()), ("Delimiter", "(", "Delimiter", ")"));
        assert_eq!(rules[3].case_mode, "WordFirst");
        assert_eq!((rules[4].start, rules[4].step, rules[4].digits, rules[4].position.as_str()), (1, 2, 3, "Suffix"));
        assert_eq!(macros[0].legacy_rows, ["weird row"]);
        save(&path, &macros).unwrap();
        assert_eq!(read(&path).unwrap(), macros, "round trip");
        macros[0].rules[0].text = "a;b".into();
        assert_eq!(can_save(&macros), Err("a;b".into()));
        assert!(save(&path, &macros).is_err());
        let profile = root.join("profile");
        let loaded = load(&profile, &root, vec![]).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(profile.join(FILE_NAME).is_file(), "imported beside the executable, copied to the profile");
        fs::remove_dir_all(root).unwrap();
    }
}
