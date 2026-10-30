//! C# NDR(원본 NexusFile 이식본)의 동작 규칙 중 화면과 무관한 부분.
//! 각 함수 주석에 원본 함수 주소와 C# NDR 쪽 이름을 적는다.
use crate::names;
use std::collections::BTreeSet;

/// FUN_00485bbc(focusedOnly false) — C# `OpenAllSelectedItems`: 표시한 항목을 목록 순서대로, 없으면 포커스 항목.
/// Rust 목록에는 ".."와 드라이브 줄이 없으므로 그 제외 규칙은 필요 없다.
pub fn collect_items(len: usize, marked: &BTreeSet<usize>, cursor: Option<usize>) -> Vec<usize> {
    let marked: Vec<usize> = marked.iter().copied().filter(|index| *index < len).collect();
    if !marked.is_empty() {
        return marked;
    }
    cursor.filter(|index| *index < len).into_iter().collect()
}

/// CopyPathToClipboard(0x43317c)의 Tag: 0 전체 경로, 1 파일 이름, 2 경로.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPart {
    Full,
    Name,
    Folder,
}

/// 항목이 있는 곳. 로컬은 shlwapi 규칙, 압축 파일은 C# NDR의 "archive!/entry" 형식, 원격은 '/' 경로.
#[derive(Clone, Debug)]
pub enum ItemPath<'a> {
    Local(&'a str),
    Archive { archive: &'a str, key: &'a str, name: &'a str },
    Remote(&'a str),
}

fn part_of(item: &ItemPath<'_>, part: PathPart) -> String {
    match *item {
        ItemPath::Local(path) => match part {
            PathPart::Full => path.into(),
            PathPart::Name => names::find_file_name(path),
            PathPart::Folder => names::remove_file_spec(path),
        },
        ItemPath::Archive { archive, key, name } => {
            let trimmed = key.trim_end_matches('/');
            let folder = trimmed.rfind('/').map_or("", |at| &trimmed[..at]);
            match part {
                PathPart::Full => format!("{archive}!/{key}"),
                PathPart::Name => name.into(),
                PathPart::Folder => format!("{archive}!/{folder}"),
            }
        }
        ItemPath::Remote(path) => {
            let trimmed = path.trim_end_matches('/');
            let at = trimmed.rfind('/');
            match part {
                PathPart::Full => path.into(),
                PathPart::Name => at.map_or(trimmed, |at| &trimmed[at + 1..]).into(),
                PathPart::Folder => match at {
                    Some(0) | None => "/".into(),
                    Some(at) => trimmed[..at].into(),
                },
            }
        }
    }
}

/// CopyPathToClipboard(0x43317c) — C# `CopyPathToClipboard`: 항목이 없으면 복사하지 않음(None). Shift면 역슬래시를 두 번.
/// CRLF로 잇고 끝 줄바꿈 없음.
pub fn copy_path_text(items: &[ItemPath<'_>], part: PathPart, escape_backslashes: bool) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let lines: Vec<String> = items
        .iter()
        .map(|item| part_of(item, part))
        .map(|line| if escape_backslashes { line.replace('\\', "\\\\") } else { line })
        .collect();
    Some(lines.join("\r\n").trim_end_matches(['\r', '\n']).to_owned())
}

/// FUN_00437828 — C# `OriginalFindInList`: 커서 다음 항목부터 끝까지, 이어서 처음부터 커서까지.
/// prefix면 StrCmpNIW 접두 일치, 아니면 StrStrIW 부분 일치(빈 글은 찾지 못함). 커서가 없으면 첫 항목으로 본다(NDR 차이).
pub fn find_in_list(names: &[String], cursor: Option<usize>, text: &str, prefix: bool) -> Option<usize> {
    if names.is_empty() {
        return None;
    }
    let start = cursor.unwrap_or(0).min(names.len() - 1);
    (1..=names.len()).map(|offset| (start + offset) % names.len()).find(|&index| {
        if prefix {
            names::starts_with_ignore_case(&names[index], text)
        } else {
            names::contains_ignore_case(&names[index], text)
        }
    })
}

/// miFindNextClick(0x437cd8, 내부 FindNext) — C# `FindNextQuickSearchMatch`가 고르는 검색:
/// 입력 중인 빠른 검색 글자가 있으면 접두 검색, 없으면 마지막 찾기 문자열로 부분 검색, 둘 다 없으면 없음.
pub fn find_next_query<'a>(typed: &'a str, last_find: &'a str) -> Option<(&'a str, bool)> {
    if !typed.is_empty() {
        Some((typed, true))
    } else if !last_find.is_empty() {
        Some((last_find, false))
    } else {
        None
    }
}

/// r62 내부 명령(#NAME) — C# `ExecuteFunctionKeyMacroAsync`의 표. 반환: 메뉴 명령 ID 또는 특수 동작.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InternalCommand {
    None,
    CopyToOther { source: usize, moving: bool },
    CopyActiveToOther { moving: bool },
    FindNext,
    ExchangePath,
    ShowPathHistory,
    FocusAddressBar,
    SelectAll { with_folders: bool },
    SelectCurrentAndMove,
    /// 원본 내부 명령 0x2a "Select": 커서 항목의 선택만 뒤집는다(커서는 그대로).
    SelectCurrent,
    DeselectAll,
    CloseTab,
    Back,
    Forward,
    NewTab,
    FavoritesMenu,
    Menu(&'static str),
    Unknown,
}

/// "#name[:argument]"의 이름 부분으로 C# 표와 같은 동작을 고른다(대소문자 무시). 0 = 왼쪽, 1 = 오른쪽 패널.
pub fn internal_command(name: &str) -> InternalCommand {
    use InternalCommand::*;
    let name = name.split(':').next().unwrap_or("").trim().to_ascii_uppercase();
    match name.as_str() {
        "NONE" => None,
        "COPYTOLEFT" => CopyToOther { source: 1, moving: false },
        "COPYTORIGHT" => CopyToOther { source: 0, moving: false },
        "MOVETOLEFT" => CopyToOther { source: 1, moving: true },
        "MOVETORIGHT" => CopyToOther { source: 0, moving: true },
        "FINDNEXT" => FindNext,
        "EXCHANGEPATH" => ExchangePath,
        "SHOWPATHHISTORY" => ShowPathHistory,
        "FOCUSADDRESSBAR" => FocusAddressBar,
        "SELECTALL" => SelectAll { with_folders: false },
        "SELECTALLWITHFOLDER" => SelectAll { with_folders: true },
        "SELECTCURRANDMOVE" => SelectCurrentAndMove,
        "DESELECTALL" => DeselectAll,
        "CLOSETAB" => CloseTab,
        "BACK" => Back,
        "FORWARD" => Forward,
        "NEWTAB" => NewTab,
        "FAVORITES" => FavoritesMenu,
        "OPEN" => Menu("open"),
        "OPENWITHPARAM" => Menu("openWithParameters"),
        "COPYTO" => Menu("copyToFolder"),
        "MOVETO" => Menu("moveToFolder"),
        "COPY" | "EDITCOPY" => Menu("copy"),
        "CUT" | "EDITCUT" => Menu("cut"),
        "PASTE" | "EDITPASTE" => Menu("paste"),
        "DELETE" => Menu("deletePermanent"),
        "RECYCLEBIN" => Menu("deleteRecycle"),
        "SHRED" => Menu("shred"),
        "RENAME" => Menu("rename"),
        "ADVRENAME" => Menu("advancedRename"),
        "CHANGEATTR" => Menu("attributes"),
        "PROPERTIES" => Menu("properties"),
        "EDITDESCRIPTION" => Menu("description"),
        "NEWFOLDER" => Menu("newFolder"),
        "REFRESH" => Menu("refresh"),
        "GOTODESKTOP" => Menu("desktop"),
        "GOTODOCUMENTS" => Menu("documents"),
        "GOTOMUSIC" => Menu("music"),
        "GOTOPICTURES" => Menu("pictures"),
        "GOTOVIDEOS" => Menu("videos"),
        "GOTODOWNLOADS" => Menu("downloads"),
        "VIEW" => Menu("textView"),
        "EDIT" => Menu("textEdit"),
        "ADDTOARCHIVE" => Menu("archiveAdd"),
        "EXTRACT" => Menu("extractSelected"),
        "AUTOEXTRACT" => Menu("extractAuto"),
        "NETWORKDIALOG" => Menu("browseNetwork"),
        "SELECTSAMENAME" => Menu("selectSameName"),
        "SELECTSAMEEXT" => Menu("selectSameExt"),
        // 원본 FUN_00443f14 case 0x2a: Select(ItemIndex, !선택됨). (C# 표는 advancedSelect를 열었다.)
        "SELECT" => SelectCurrent,
        "FTPSITEMANAGER" => Menu("sites"),
        "OPENALLSELECTED" => Menu("openAll"),
        // 고친 것: 원본 FUN_00443f14의 점프 표는 0x2f(OpenAllSelected)·0x30(FolderTree)을 아무것도 안 하는 칸(0x4446dc)으로 둔다.
        // 기능키 이름(FUNC_FOLDERTREE "폴더 트리")대로 F10 폴더 선택 창(miFolderTreeClick)을 연다.
        "FOLDERTREE" => Menu("folderTree"),
        "GOTOROOT" => Menu("goRoot"),
        "GOTOPARENT" => Menu("goParent"),
        "GOTOLASTDEST" => Menu("goLastDestination"),
        "INVERTSELECTION" => Menu("invertSelection"),
        "COMMANDLINE" => Menu("command"),
        "SPLITFILE" => Menu("split"),
        "JOINFILES" => Menu("join"),
        "OPENRECYCLEBIN" => Menu("openRecycle"),
        _ => Unknown,
    }
}

/// C# QuickFilterAsync(도구 모음 빠른 찾기 칸·검색 칸에서 Enter): 앞뒤 공백을 떼고, 비면 필터 해제,
/// '*'나 '?'가 있으면 그대로, 없으면 "*글자*".
pub fn quick_filter_pattern(query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        String::new()
    } else if query.contains(['*', '?']) {
        query.into()
    } else {
        format!("*{query}*")
    }
}

/// C# RefreshPaneAsync의 FilterPattern: ';'로 나눈 마스크 중 하나라도 맞으면 보인다(빈 패턴은 모두 보임).
pub fn matches_filter(pattern: &str, name: &str) -> bool {
    let masks: Vec<&str> = pattern.split(';').map(str::trim).filter(|mask| !mask.is_empty()).collect();
    masks.is_empty() || masks.iter().any(|mask| matches_simple_expression(mask, name))
}

#[derive(Clone, Copy, PartialEq)]
enum Token {
    Star,
    One,
    Char(char),
}

/// .NET FileSystemName.MatchesSimpleExpression(mask, name, ignoreCase: true): '*'는 0자 이상, '?'는 정확히 한 글자,
/// '\'는 다음 글자를 글자 그대로. 빈 마스크나 빈 이름은 맞지 않는다. 대소문자는 서수 대문자 비교(OrdinalIgnoreCase).
pub fn matches_simple_expression(mask: &str, name: &str) -> bool {
    if mask.is_empty() || name.is_empty() {
        return false;
    }
    let mut tokens = Vec::new();
    let mut chars = mask.chars();
    while let Some(c) = chars.next() {
        tokens.push(match c {
            '*' => Token::Star,
            '?' => Token::One,
            '\\' => Token::Char(chars.next().unwrap_or('\\')),
            other => Token::Char(other),
        });
    }
    let upper = |c: char| {
        let mut mapped = c.to_uppercase();
        match (mapped.next(), mapped.next()) {
            (Some(single), None) => single,
            _ => c,
        }
    };
    let name: Vec<char> = name.chars().collect();
    let (mut t, mut n) = (0usize, 0usize);
    let mut backtrack: Option<(usize, usize)> = None;
    while n < name.len() {
        match tokens.get(t) {
            Some(Token::Star) => {
                backtrack = Some((t, n));
                t += 1;
                continue;
            }
            Some(Token::One) => {
                t += 1;
                n += 1;
                continue;
            }
            Some(Token::Char(c)) if upper(*c) == upper(name[n]) => {
                t += 1;
                n += 1;
                continue;
            }
            _ => {}
        }
        match backtrack {
            Some((star, from)) => {
                t = star + 1;
                n = from + 1;
                backtrack = Some((star, from + 1));
            }
            None => return false,
        }
    }
    tokens[t..].iter().all(|token| *token == Token::Star)
}

/// C# 빠른 찾기 입력이 끊겼다고 보는 간격(QuickSearchSequenceTimeout).
pub const QUICK_SEARCH_TIMEOUT_MS: u128 = 1100;

/// 숫자에 세 자리마다 ','(ko-KR/en-US 현재 문화권의 "N" 형식).
fn group_thousands(integer: &str) -> String {
    let digits: Vec<char> = integer.chars().collect();
    let mut out = String::new();
    for (index, digit) in digits.iter().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(*digit);
    }
    out
}

/// .NET 숫자 형식 "N{decimals}"(thousands) 또는 "0.##…"(뒤 0 생략).
fn format_number(value: f64, decimals: usize, thousands: bool, trim: bool) -> String {
    let text = format!("{value:.decimals$}");
    let (integer, fraction) = text.split_once('.').map_or((text.as_str(), ""), |(i, f)| (i, f));
    let integer = if thousands { group_thousands(integer) } else { integer.to_owned() };
    let fraction = if trim { fraction.trim_end_matches('0') } else { fraction };
    if fraction.is_empty() { integer } else { format!("{integer}.{fraction}") }
}

/// C# FileSystemService.FormatSize(size, SizeUnit, UseThousandsSeparator): 단위를 고르면 그 단위(B는 정수, 나머지 소수 둘째 자리),
/// "Auto"면 1024로 나누며 B는 정수, 나머지는 소수 첫째 자리.
pub fn format_size(size: u64, unit: &str, thousands: bool) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if !unit.eq_ignore_ascii_case("Auto") {
        if let Some(requested) = UNITS.iter().position(|u| u.eq_ignore_ascii_case(unit)) {
            let value = size as f64 / 1024f64.powi(requested as i32);
            let text = if requested == 0 {
                format_number(value, 0, thousands, false)
            } else {
                format_number(value, 2, thousands, !thousands)
            };
            return format!("{text} {}", UNITS[requested]);
        }
    }
    let mut value = size as f64;
    let mut index = 0;
    while value >= 1024.0 && index < UNITS.len() - 1 {
        value /= 1024.0;
        index += 1;
    }
    if index == 0 {
        return format!("{} B", format_number(size as f64, 0, thousands, false));
    }
    format!("{} {}", format_number(value, 1, thousands, !thousands), UNITS[index])
}

/// .NET Path.GetExtension(이름): 마지막 '.'부터(점 포함). 점이 끝이면 빈 문자열.
pub fn dotnet_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(at) if at + 1 < name.len() => &name[at..],
        _ => "",
    }
}

/// .NET Path.GetFileNameWithoutExtension(이름).
pub fn dotnet_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(at) => &name[..at],
        None => name,
    }
}

/// 파일 목록의 확장자(점 포함): `dotnet_extension`과 같되, 맨 앞 점은 확장자로 보지 않는다(".gitconfig"는 이름만).
/// 고친 것(사용자 요청): 원본·C# 판은 ".gitconfig"를 빈 이름 + 확장자 "gitconfig"로 보였다.
pub fn list_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(at) if at > 0 && at + 1 < name.len() => &name[at..],
        _ => "",
    }
}

/// 파일 목록의 확장자 없는 이름(`list_extension`과 짝).
pub fn list_stem(name: &str) -> &str {
    &name[..name.len() - list_extension(name).len()]
}

/// C# SelectQuickSearchMatch: 시작은 커서 항목(커서가 없거나 마지막 항목이면 0, cycle_from_next면 다음 항목).
/// 거기서 한 바퀴 돌며 먼저 StrCmpNIW 접두 일치, 없으면 StrStrIW 부분 일치.
pub fn quick_search_match(names: &[String], cursor: Option<usize>, query: &str, cycle_from_next: bool) -> Option<usize> {
    if query.is_empty() || names.is_empty() {
        return None;
    }
    let count = names.len();
    let start = match cursor.filter(|index| *index < count) {
        None => 0,
        Some(index) if index == count - 1 => 0,
        Some(index) => (index + usize::from(cycle_from_next)) % count,
    };
    let find = |substring: bool| {
        (0..count).map(|offset| (start + offset) % count).find(|index| {
            if substring { names::contains_ignore_case(&names[*index], query) } else { names::starts_with_ignore_case(&names[*index], query) }
        })
    };
    find(false).or_else(|| find(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn item_collection_marks_then_focus() {
        let marked: BTreeSet<usize> = [3, 1].into_iter().collect();
        assert_eq!(collect_items(5, &marked, Some(0)), vec![1, 3], "marked items in list order");
        assert_eq!(collect_items(5, &BTreeSet::new(), Some(2)), vec![2]);
        assert!(collect_items(0, &BTreeSet::new(), Some(0)).is_empty());
        assert!(collect_items(2, &[7].into_iter().collect(), None).is_empty());
    }

    #[test]
    fn copy_path_matches_copypathtoclipboard() {
        let items = [ItemPath::Local(r"C:\work\a.txt"), ItemPath::Local(r"C:\b.txt")];
        assert_eq!(copy_path_text(&items, PathPart::Full, false).unwrap(), "C:\\work\\a.txt\r\nC:\\b.txt");
        assert_eq!(copy_path_text(&items, PathPart::Name, false).unwrap(), "a.txt\r\nb.txt");
        assert_eq!(copy_path_text(&items, PathPart::Folder, false).unwrap(), "C:\\work\r\nC:\\");
        assert_eq!(copy_path_text(&items[..1], PathPart::Full, true).unwrap(), r"C:\\work\\a.txt");
        assert!(copy_path_text(&[], PathPart::Full, false).is_none(), "nothing collected, nothing copied");
        let archive = [ItemPath::Archive { archive: r"C:\x.zip", key: "dir/one.txt", name: "one.txt" }];
        assert_eq!(copy_path_text(&archive, PathPart::Full, false).unwrap(), r"C:\x.zip!/dir/one.txt");
        assert_eq!(copy_path_text(&archive, PathPart::Folder, false).unwrap(), r"C:\x.zip!/dir");
        let remote = [ItemPath::Remote("/pub/file.bin"), ItemPath::Remote("/top")];
        assert_eq!(copy_path_text(&remote, PathPart::Name, false).unwrap(), "file.bin\r\ntop");
        assert_eq!(copy_path_text(&remote, PathPart::Folder, false).unwrap(), "/pub\r\n/");
    }

    #[test]
    fn find_follows_fun_00437828() {
        let names = n(&["folder1", "alpha.txt", "Alphabet.doc", "beta.txt", "gamma.log"]);
        assert_eq!(find_in_list(&names, Some(0), "PHA", false), Some(1));
        assert_eq!(find_in_list(&names, Some(1), "PHA", false), Some(2), "continues after the cursor");
        assert_eq!(find_in_list(&names, Some(2), "PHA", false), Some(1), "wraps to the top");
        assert_eq!(find_in_list(&names, Some(1), "alpha.txt", false), Some(1), "the cursor item itself is checked last");
        assert_eq!(find_in_list(&names, Some(0), "", false), None, "StrStrIW: empty text finds nothing");
        assert_eq!(find_in_list(&names, Some(0), "zzz", false), None);
        assert_eq!(find_in_list(&names, Some(0), "G", true), Some(4), "prefix search");
        assert_eq!(find_in_list(&names, Some(0), "eta", true), None, "prefix only, no substring fallback");
        assert_eq!(find_in_list(&[], Some(0), "a", false), None);
        assert_eq!(find_in_list(&names, None, "fold", false), Some(0), "no cursor counts as the first item (NDR deviation)");
    }

    #[test]
    fn find_next_prefers_typed_text() {
        assert_eq!(find_next_query("G", "beta"), Some(("G", true)));
        assert_eq!(find_next_query("", "beta"), Some(("beta", false)));
        assert_eq!(find_next_query("", ""), None);
    }

    #[test]
    fn internal_commands_match_the_csharp_table() {
        use InternalCommand::*;
        assert_eq!(internal_command("CopyToLeft"), CopyToOther { source: 1, moving: false });
        assert_eq!(internal_command("#ignored"), Unknown);
        assert_eq!(internal_command("MoveToRight:arg"), CopyToOther { source: 0, moving: true });
        assert_eq!(internal_command("Select"), SelectCurrent);
        assert_eq!(internal_command("FolderTree"), Menu("folderTree"));
        assert_eq!(internal_command("Favorites"), FavoritesMenu);
        assert_eq!(internal_command("OpenRecycleBin"), Menu("openRecycle"));
        assert_eq!(internal_command("none"), None);
        assert_eq!(internal_command("SelectAllWithFolder"), SelectAll { with_folders: true });
    }

    #[test]
    fn quick_filter_and_simple_expression_masks() {
        assert_eq!(quick_filter_pattern("  report "), "*report*");
        assert_eq!(quick_filter_pattern("*.txt"), "*.txt");
        assert_eq!(quick_filter_pattern("a?c"), "a?c");
        assert_eq!(quick_filter_pattern("   "), "");
        assert!(matches_filter("", "anything"));
        assert!(matches_filter("*.TXT; report-??.csv", "notes.txt"));
        assert!(matches_filter("*.txt;report-??.csv", "Report-01.CSV"));
        assert!(!matches_filter("*.txt;report-??.csv", "report-1.csv"), "? is exactly one character");
        assert!(matches_simple_expression("*", "x"));
        assert!(!matches_simple_expression("*", ""), "an empty name never matches");
        assert!(!matches_simple_expression("", "x"));
        assert!(matches_simple_expression("a*b*c", "aXbYbZc"));
        assert!(!matches_simple_expression("a*b", "ab c"));
        assert!(matches_simple_expression("*한글*", "새 한글 문서.txt"));
        assert!(matches_simple_expression("\\*x", "*x"), "backslash escapes the next character");
        assert!(!matches_simple_expression("\\*x", "ax"));
        assert!(!matches_simple_expression("*.*", "noext"), "no implicit DOS *.* rule");
    }

    #[test]
    fn quick_search_prefix_first_then_substring() {
        let names: Vec<String> = ["folder1", "alpha.txt", "Alphabet.doc", "beta.txt", "gamma.log"].iter().map(|s| s.to_string()).collect();
        assert_eq!(quick_search_match(&names, Some(0), "al", false), Some(1));
        assert_eq!(quick_search_match(&names, Some(1), "al", false), Some(1), "the cursor item itself first");
        assert_eq!(quick_search_match(&names, Some(1), "al", true), Some(2), "cycle from the next item");
        assert_eq!(quick_search_match(&names, Some(4), "g", false), Some(4), "last item: start from the top, wrap to it");
        assert_eq!(quick_search_match(&names, Some(0), "eta", false), Some(3), "no prefix hit: substring");
        assert_eq!(quick_search_match(&names, Some(0), "ETA.T", false), Some(3));
        assert_eq!(quick_search_match(&names, None, "zz", false), None);
        assert_eq!(quick_search_match(&[], Some(0), "a", false), None);
    }

    #[test]
    fn size_text_follows_formatsize() {
        assert_eq!(format_size(999, "Auto", true), "999 B");
        assert_eq!(format_size(1_234_567, "Auto", true), "1.2 MB");
        assert_eq!(format_size(1024, "Auto", false), "1 KB", "0.# drops a zero fraction");
        assert_eq!(format_size(1536, "Auto", false), "1.5 KB");
        assert_eq!(format_size(1_234_567, "B", true), "1,234,567 B");
        assert_eq!(format_size(1_234_567, "b", false), "1234567 B");
        assert_eq!(format_size(1536, "KB", true), "1.50 KB");
        assert_eq!(format_size(1536, "KB", false), "1.5 KB");
        assert_eq!(format_size(2_000_000_000_000, "GB", true), "1,862.65 GB");
        assert_eq!(format_size(5, "XB", true), "5 B", "unknown unit: automatic");
        assert_eq!((dotnet_extension("a.tar.gz"), dotnet_stem("a.tar.gz")), (".gz", "a.tar"));
        assert_eq!((dotnet_extension(".gitignore"), dotnet_stem(".gitignore")), (".gitignore", ""));
        assert_eq!((dotnet_extension("name."), dotnet_extension("noext")), ("", ""));
        // 목록: 맨 앞 점은 이름의 일부.
        assert_eq!((list_stem(".gitconfig"), list_extension(".gitconfig")), (".gitconfig", ""));
        assert_eq!((list_stem(".config.json"), list_extension(".config.json")), (".config", ".json"));
        assert_eq!((list_stem("a.tar.gz"), list_extension("a.tar.gz")), ("a.tar", ".gz"));
        assert_eq!((list_stem("name."), list_extension("name.")), ("name.", ""));
    }
}
