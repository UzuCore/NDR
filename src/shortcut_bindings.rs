//! C# ShortcutBindingService: NDR shortcut definitions (Id, name, scope, default key, action), keys overridden by the KeyboardShortcuts setting,
//! reading and comparing key text ("Ctrl+Shift+F1"). Key names are handled as WPF Key names (D0-D9, Back, OemMinus, Left, F1 ...).
use crate::settings::Settings;

/// ShortcutDefinition(Id, Command, Scope, DefaultGesture, ActionId).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub id: String,
    pub command: &'static str,
    pub scope: &'static str,
    pub default_gesture: String,
    pub action: &'static str,
}

const PANEL: &str = "파일 패널";
const TREE: &str = "폴더 트리";
const RENAME: &str = "고급 이름 변경";

/// (Id, name, scope, default key, action), in the same order as C# Definitions.
const TABLE: &[(&str, &str, &str, &str, &str)] = &[
    ("historyBack", "뒤로 이동", PANEL, "Alt+Left", ""),
    ("historyForward", "앞으로 이동", PANEL, "Alt+Right", ""),
    ("goParent", "상위 폴더", PANEL, "Alt+Up", ""),
    ("lastDestination", "마지막 복사/이동 대상 위치로 이동", PANEL, "Ctrl+Backspace", ""),
    ("addressEdit", "주소 경로 편집", PANEL, "Ctrl+L", ""),
    ("copyToFolder", "다른 폴더로 복사", PANEL, "Alt+C", ""),
    ("moveToFolder", "다른 폴더로 이동", PANEL, "Alt+M", ""),
    ("copy", "파일 복사", PANEL, "Ctrl+C", ""),
    ("cut", "파일 잘라내기", PANEL, "Ctrl+X", ""),
    ("paste", "붙여넣기", PANEL, "Ctrl+V", ""),
    ("rename", "이름 변경", PANEL, "F2", ""),
    ("copyToOther", "다른 패널로 복사", PANEL, "F3", ""),
    ("moveToOther", "다른 패널로 이동", PANEL, "F4", ""),
    ("copyLeft", "왼쪽 패널로 복사", PANEL, "Ctrl+Left", ""),
    ("copyRight", "오른쪽 패널로 복사", PANEL, "Ctrl+Right", ""),
    ("moveLeft", "왼쪽 패널로 이동", PANEL, "Ctrl+Alt+Left", ""),
    ("moveRight", "오른쪽 패널로 이동", PANEL, "Ctrl+Alt+Right", ""),
    ("refreshF5", "새로 고침", PANEL, "F5", ""),
    ("batchRename", "고급 이름 변경", PANEL, "F6", ""),
    ("advancedRename", "고급 이름 변경", PANEL, "Shift+Alt+R", ""),
    ("newFolder", "새 폴더", PANEL, "F7", ""),
    ("deleteF8", "삭제", PANEL, "F8", ""),
    ("deleteKey", "삭제", PANEL, "Delete", ""),
    ("deleteAltD", "휴지통으로 삭제", PANEL, "Alt+D", ""),
    ("deletePermanent", "영구 삭제", PANEL, "Shift+Delete", ""),
    ("shredShiftAltDelete", "완전 삭제", PANEL, "Shift+Alt+Delete", ""),
    ("shredShiftAltD", "완전 삭제", PANEL, "Shift+Alt+D", ""),
    ("open", "열기 / 실행", PANEL, "Enter", ""),
    ("openWithParameters", "선택 항목을 인수와 함께 열기", PANEL, "Ctrl+Enter", ""),
    ("extractSelected", "선택 압축 풀기", PANEL, "Ctrl+E", ""),
    ("autoExtract", "자동 압축 풀기", PANEL, "Alt+Q", ""),
    ("newTab", "새 탭", PANEL, "Ctrl+T", ""),
    ("closeTab", "현재 탭 닫기", PANEL, "Ctrl+F4", ""),
    ("focusSearch", "검색 입력으로 이동", PANEL, "Ctrl+F", ""),
    ("refreshCtrlR", "새로 고침", PANEL, "Ctrl+R", ""),
    ("selectAll", "전체 선택", PANEL, "Ctrl+U", ""),
    ("cycleSelection", "파일/폴더 선택 순환", PANEL, "Alt+U", ""),
    ("toggleSelection", "항목 선택/해제", PANEL, "Space", ""),
    ("browseFolder", "폴더 선택 창", PANEL, "F10", ""),
    ("favoritesMenu", "즐겨찾기 메뉴", PANEL, "F11", ""),
    ("shortcutsHelp", "단축키 안내", PANEL, "F12", ""),
    ("treeRename", "선택 폴더 이름 변경", TREE, "F2", ""),
    ("treeFindNext", "다음 폴더 찾기", TREE, "F3", ""),
    ("treeRefresh", "폴더 트리 새로 고침", TREE, "F5", ""),
    ("treeRefreshRecursive", "하위 폴더 트리 새로 고침", TREE, "F6", ""),
    ("treeNewFolder", "폴더 트리에 새 폴더", TREE, "F7", ""),
    ("treeProperties", "선택 폴더 속성", TREE, "F9", ""),
    ("advancedTabInsert", "고급 이름 변경: 삽입", RENAME, "Alt+1", ""),
    ("advancedTabReplace", "고급 이름 변경: 바꾸기", RENAME, "Alt+2", ""),
    ("advancedTabDelete", "고급 이름 변경: 삭제", RENAME, "Alt+3", ""),
    ("advancedTabCase", "고급 이름 변경: 대소문자", RENAME, "Alt+4", ""),
    ("advancedTabNumber", "고급 이름 변경: 번호", RENAME, "Alt+5", ""),
    ("advancedTabExtension", "고급 이름 변경: 확장자", RENAME, "Alt+6", ""),
    ("legacyArchiveCopy", "압축 파일에 추가 (복사)", PANEL, "Ctrl+A", "archiveAdd"),
    ("legacyArchiveMove", "압축 파일에 추가 (이동)", PANEL, "Ctrl+Shift+A", "archiveMove"),
    ("legacyDocuments", "문서 폴더", PANEL, "Ctrl+W", "documents"),
    ("legacyDesktop", "바탕 화면", PANEL, "Ctrl+D", "desktop"),
    ("legacyPictures", "사진 폴더", PANEL, "Ctrl+Q", "pictures"),
    ("legacyMusic", "음악 폴더", PANEL, "Ctrl+S", "music"),
    ("legacyVideos", "동영상 폴더", PANEL, "Ctrl+O", "videos"),
    ("legacyTemp", "임시 폴더", PANEL, "Ctrl+P", "temp"),
    ("legacyRecycleBin", "휴지통", PANEL, "Ctrl+B", "openRecycle"),
    ("legacyEmptyRecycleBin", "휴지통 비우기", PANEL, "Ctrl+Shift+B", "emptyRecycle"),
    ("legacyEmptyTemp", "임시 폴더 비우기", PANEL, "Ctrl+Shift+P", "emptyTemp"),
    ("legacyCreateEmptyFile", "빈 파일 만들기", PANEL, "Ctrl+N", "newEmpty"),
    ("legacyDesktopShortcut", "바탕 화면 바로 가기 만들기", PANEL, "Ctrl+Shift+I", "shortcut"),
    ("legacyQuickLaunchShortcut", "빠른 실행 바로가기 만들기", PANEL, "Ctrl+Shift+Q", "quickLaunchShortcut"),
    ("legacyPasteShortcut", "바로 가기로 붙여넣기", PANEL, "Ctrl+Shift+V", "pasteShortcut"),
    ("legacyRestoreSelection", "마지막 선택 복구", PANEL, "Shift+Backspace", "restoreSelection"),
    ("legacyChangeAttributes", "속성 및 시간 변경", PANEL, "Ctrl+Z", "attributes"),
    ("legacyEditDescription", "설명 추가 또는 편집", PANEL, "Alt+Enter", "description"),
    ("legacyProperties", "Windows 속성", PANEL, "F9", "properties"),
    ("legacyFindNext", "다음 항목 찾기", PANEL, "Ctrl+Shift+F", "findNext"),
    ("legacySearch", "파일 및 폴더 검색", PANEL, "Ctrl+H", "search"),
    ("legacyFilterOrInstall", "필터 / 압축 파일 설치", PANEL, "Ctrl+I", "filterOrInstall"),
    ("legacyExtractCurrent", "현재 폴더에 압축 풀기", PANEL, "Ctrl+Shift+E", "extractCurrent"),
    ("legacyShred", "분쇄 후 삭제", PANEL, "Ctrl+Shift+D", "shred"),
    ("legacyTextView", "텍스트 보기", PANEL, "Alt+V", "textView"),
    ("legacyTextEdit", "텍스트 편집", PANEL, "Alt+B", "textEdit"),
    ("legacyControlPanel", "제어판", PANEL, "Alt+F12", "controlPanel"),
    ("legacyInstalledApps", "설치된 앱", PANEL, "Alt+F11", "addRemovePrograms"),
    ("legacyOptions", "환경 설정", PANEL, "Ctrl+F12", "options"),
    ("legacyHelp", "도움말", PANEL, "F1", "help"),
    ("legacyExit", "종료", PANEL, "Alt+X", "exit"),
    ("legacyToggleHidden", "숨김 항목 표시 전환", PANEL, "Alt+Z", "toggleHidden"),
    ("legacyRename", "이름 변경", PANEL, "Alt+R", "rename"),
    ("legacyNewFolder", "새 폴더", PANEL, "Alt+K", "newFolder"),
    ("legacySetWorkFolder1", "작업 폴더 1 저장", PANEL, "Ctrl+1", "workSet:0"),
    ("legacySetWorkFolder2", "작업 폴더 2 저장", PANEL, "Ctrl+2", "workSet:1"),
    ("legacySetWorkFolder3", "작업 폴더 3 저장", PANEL, "Ctrl+3", "workSet:2"),
    ("legacySetWorkFolder4", "작업 폴더 4 저장", PANEL, "Ctrl+4", "workSet:3"),
    ("legacySetWorkFolder5", "작업 폴더 5 저장", PANEL, "Ctrl+5", "workSet:4"),
    ("legacyGoWorkFolder1", "작업 폴더 1로 이동", PANEL, "Alt+1", "workOpen:0"),
    ("legacyGoWorkFolder2", "작업 폴더 2로 이동", PANEL, "Alt+2", "workOpen:1"),
    ("legacyGoWorkFolder3", "작업 폴더 3으로 이동", PANEL, "Alt+3", "workOpen:2"),
    ("legacyGoWorkFolder4", "작업 폴더 4로 이동", PANEL, "Alt+4", "workOpen:3"),
    ("legacyGoWorkFolder5", "작업 폴더 5로 이동", PANEL, "Alt+5", "workOpen:4"),
    ("legacyColumnAuto", "열 자동 배치", PANEL, "Ctrl+Alt+0", "column:0"),
    ("legacyColumn1", "1열", PANEL, "Ctrl+Alt+1", "column:1"),
    ("legacyColumn2", "2열", PANEL, "Ctrl+Alt+2", "column:2"),
    ("legacyColumn3", "3열", PANEL, "Ctrl+Alt+3", "column:3"),
    ("legacyColumn4", "4열", PANEL, "Ctrl+Alt+4", "column:4"),
    ("legacyColumn5", "5열", PANEL, "Ctrl+Alt+5", "column:5"),
    ("legacyColumn6", "6열", PANEL, "Ctrl+Alt+6", "column:6"),
    ("legacySortAttributes", "속성순 정렬", PANEL, "Ctrl+Alt+B", "sort:Attributes"),
    ("legacySortColor", "색상순 정렬", PANEL, "Ctrl+Alt+C", "sort:Color"),
    ("legacySortExtension", "확장자순 정렬", PANEL, "Ctrl+Alt+E", "sort:Extension"),
    ("legacySortName", "이름순 정렬", PANEL, "Ctrl+Alt+N", "sort:Name"),
    ("legacySortNone", "정렬 안 함", PANEL, "Ctrl+Alt+O", "sort:None"),
    ("legacySortSize", "크기순 정렬", PANEL, "Ctrl+Alt+S", "sort:Size"),
    ("legacySortTime", "시간순 정렬", PANEL, "Ctrl+Alt+T", "sort:Modified"),
    ("legacySortDescending", "내림차순 정렬", PANEL, "Ctrl+Alt+-", "sortDescending"),
    ("legacyCopyFileName", "파일 이름 복사", PANEL, "Ctrl+Alt+A", "copyName"),
    ("legacyCopyFullPath", "전체 경로 복사", PANEL, "Ctrl+Alt+F", "copyFullPath"),
    ("legacyCopyPath", "경로 복사", PANEL, "Ctrl+Alt+R", "copyPathOnly"),
    ("legacyJoinFiles", "분할 파일 합치기", PANEL, "Ctrl+Alt+J", "join"),
    ("legacySplitFile", "파일 분할", PANEL, "Ctrl+Alt+L", "split"),
];

const FAVORITE_ACTIONS: [&str; 20] = [
    "favoriteOpen:0", "favoriteOpen:1", "favoriteOpen:2", "favoriteOpen:3", "favoriteOpen:4", "favoriteOpen:5", "favoriteOpen:6",
    "favoriteOpen:7", "favoriteOpen:8", "favoriteOpen:9", "favoriteOpen:10", "favoriteOpen:11", "favoriteOpen:12", "favoriteOpen:13",
    "favoriteOpen:14", "favoriteOpen:15", "favoriteOpen:16", "favoriteOpen:17", "favoriteOpen:18", "favoriteOpen:19",
];

/// C# Definitions (the table + 20 favorites: Shift+F1-F10, Ctrl+Shift+F1-F10).
pub fn definitions() -> Vec<Definition> {
    let mut list: Vec<Definition> = TABLE
        .iter()
        .map(|&(id, command, scope, gesture, action)| Definition { id: id.into(), command, scope, default_gesture: gesture.into(), action })
        .collect();
    for (index, action) in FAVORITE_ACTIONS.iter().enumerate() {
        list.push(Definition {
            id: format!("favoriteOpen{}", index + 1),
            command: "즐겨찾기로 이동",
            scope: PANEL,
            default_gesture: if index < 10 { format!("Shift+F{}", index + 1) } else { format!("Ctrl+Shift+F{}", index - 9) },
            action,
        });
    }
    list
}

/// Modifiers (WPF ModifierKeys: Alt 1, Control 2, Shift 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// GetGesture: KeyboardShortcuts[Id] when present, otherwise the default key.
pub fn gesture(settings: &Settings, id: &str) -> Option<String> {
    let configured = settings.values.get("KeyboardShortcuts").and_then(|v| v.get(id)).and_then(|v| v.as_str());
    if let Some(value) = configured {
        return Some(value.to_owned());
    }
    TABLE.iter().find(|row| row.0 == id).map(|row| row.3.to_owned()).or_else(|| {
        let index = id.strip_prefix("favoriteOpen")?.parse::<usize>().ok()?.checked_sub(1)?;
        (index < 20).then(|| if index < 10 { format!("Shift+F{}", index + 1) } else { format!("Ctrl+Shift+F{}", index - 9) })
    })
}

/// Bindable WPF Key names (case-insensitive). Like Enum.TryParse, returns the canonical name.
fn key_name(part: &str) -> Option<String> {
    if part.len() == 1 && part.as_bytes()[0].is_ascii_digit() {
        return Some(format!("D{part}"));
    }
    if part.eq_ignore_ascii_case("Backspace") {
        return Some("Back".into());
    }
    if part == "-" || part == "Minus" {
        return Some("OemMinus".into());
    }
    const ALIASES: &[(&str, &str)] = &[("Return", "Enter"), ("Escape", "Escape"), ("Esc", "Escape"), ("Prior", "PageUp"), ("Next", "PageDown"), ("Capital", "CapsLock")];
    if let Some((_, name)) = ALIASES.iter().find(|(alias, _)| alias.eq_ignore_ascii_case(part)) {
        return Some((*name).into());
    }
    const NAMES: &[&str] = &[
        "Enter", "Tab", "Space", "Back", "Delete", "Insert", "Home", "End", "PageUp", "PageDown", "Left", "Right", "Up", "Down", "Escape",
        "OemMinus", "OemPlus", "OemComma", "OemPeriod", "Apps", "Pause", "CapsLock", "Scroll", "NumLock", "PrintScreen", "Add", "Subtract",
        "Multiply", "Divide", "Decimal",
    ];
    if let Some(name) = NAMES.iter().find(|name| name.eq_ignore_ascii_case(part)) {
        return Some((*name).into());
    }
    let upper = part.to_ascii_uppercase();
    if upper.len() == 1 && upper.as_bytes()[0].is_ascii_uppercase() {
        return Some(upper);
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some(format!("F{n}"));
    }
    if let Some(n) = upper.strip_prefix("NUMPAD").and_then(|n| n.parse::<u8>().ok()).filter(|n| *n <= 9) {
        return Some(format!("NumPad{n}"));
    }
    if let Some(n) = upper.strip_prefix('D').and_then(|n| n.parse::<u8>().ok()).filter(|n| *n <= 9 && upper.len() == 2) {
        return Some(format!("D{n}"));
    }
    None
}

/// C# TryParse: "Ctrl"/"Control", "Shift", "Alt" are modifiers; "Win" fails; only one key.
pub fn parse(gesture: &str) -> Option<(String, Modifiers)> {
    let mut modifiers = Modifiers::default();
    let mut key = None;
    let parts: Vec<&str> = gesture.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    for part in parts {
        if part.eq_ignore_ascii_case("Ctrl") || part.eq_ignore_ascii_case("Control") {
            modifiers.ctrl = true;
        } else if part.eq_ignore_ascii_case("Shift") {
            modifiers.shift = true;
        } else if part.eq_ignore_ascii_case("Alt") {
            modifiers.alt = true;
        } else if part.eq_ignore_ascii_case("Win") || part.eq_ignore_ascii_case("Windows") {
            return None;
        } else if key.is_none() {
            key = Some(key_name(part)?);
        } else {
            return None;
        }
    }
    key.map(|key| (key, modifiers))
}

/// Matches(id, settings, key, modifiers): whether the setting or default key exactly matches this key and modifiers.
pub fn matches(settings: &Settings, id: &str, key: &str, modifiers: Modifiers) -> bool {
    gesture(settings, id).and_then(|g| parse(&g)).is_some_and(|(k, m)| k == key && m == modifiers)
}

/// C# Format(key, modifiers): "Ctrl+Alt+Shift+key" (D0-D9 as digits, Back as Backspace, OemMinus as "-").
pub fn format(key: &str, modifiers: Modifiers) -> String {
    let mut parts: Vec<String> = Vec::new();
    if modifiers.ctrl {
        parts.push("Ctrl".into());
    }
    if modifiers.alt {
        parts.push("Alt".into());
    }
    if modifiers.shift {
        parts.push("Shift".into());
    }
    let name = match key {
        "Back" => "Backspace".to_owned(),
        "OemMinus" => "-".to_owned(),
        k if k.len() == 2 && k.starts_with('D') && k.as_bytes()[1].is_ascii_digit() => k[1..].to_owned(),
        k => k.to_owned(),
    };
    parts.push(name);
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gestures_parse_and_match_like_csharp() {
        let mut settings = Settings::default();
        assert_eq!(parse("Ctrl+Alt+-"), Some(("OemMinus".into(), Modifiers { ctrl: true, alt: true, shift: false })));
        assert_eq!(parse("Shift+Backspace"), Some(("Back".into(), Modifiers { shift: true, ..Default::default() })));
        assert_eq!(parse("ctrl+1"), Some(("D1".into(), Modifiers { ctrl: true, ..Default::default() })));
        assert_eq!(parse("Win+E"), None);
        assert_eq!(parse("Ctrl+A+B"), None);
        assert_eq!(parse("Ctrl"), None);
        assert!(matches(&settings, "historyBack", "Left", Modifiers { alt: true, ..Default::default() }));
        assert!(!matches(&settings, "historyBack", "Left", Modifiers::default()));
        assert!(matches(&settings, "favoriteOpen12", "F2", Modifiers { ctrl: true, shift: true, alt: false }));
        settings.values.insert("KeyboardShortcuts".into(), serde_json::json!({"refreshF5": "Ctrl+F9"}));
        assert!(matches(&settings, "refreshF5", "F9", Modifiers { ctrl: true, ..Default::default() }));
        assert!(!matches(&settings, "refreshF5", "F5", Modifiers::default()));
        assert_eq!(definitions().len(), TABLE.len() + 20);
        assert_eq!(gesture(&settings, "nothing"), None);
    }
}
