//! Screen-independent behavior rules of C# NDR (the earlier port of the original NexusFile).
//! Each function comment notes the original function address and the C# NDR name.
use crate::names;
use std::collections::BTreeSet;

/// FUN_00485bbc(focusedOnly false), C# `OpenAllSelectedItems`: marked items in list order, otherwise the focused item.
/// The Rust list has no ".." or drive rows, so their exclusion rules are not needed.
pub fn collect_items(len: usize, marked: &BTreeSet<usize>, cursor: Option<usize>) -> Vec<usize> {
    let marked: Vec<usize> = marked.iter().copied().filter(|index| *index < len).collect();
    if !marked.is_empty() {
        return marked;
    }
    cursor.filter(|index| *index < len).into_iter().collect()
}

/// Tag of CopyPathToClipboard (0x43317c): 0 full path, 1 file name, 2 path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPart {
    Full,
    Name,
    Folder,
}

/// Where an item lives. Local uses shlwapi rules, archives the C# NDR "archive!/entry" form, remote '/' paths.
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

/// CopyPathToClipboard (0x43317c), C# `CopyPathToClipboard`: nothing is copied without items (None). Shift doubles the backslashes.
/// Joined with CRLF, no trailing line break.
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

/// FUN_00437828, C# `OriginalFindInList`: from the item after the cursor to the end, then from the start up to the cursor.
/// With prefix, StrCmpNIW prefix match, otherwise StrStrIW substring match (empty text finds nothing). Without a cursor the first item is used (NDR difference).
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

/// miFindNextClick (0x437cd8, internal FindNext), the search C# `FindNextQuickSearchMatch` chooses:
/// a prefix search when quick search text is being typed, otherwise a substring search with the last find string, nothing when neither exists.
pub fn find_next_query<'a>(typed: &'a str, last_find: &'a str) -> Option<(&'a str, bool)> {
    if !typed.is_empty() {
        Some((typed, true))
    } else if !last_find.is_empty() {
        Some((last_find, false))
    } else {
        None
    }
}

/// r62 internal commands (#NAME), the table of C# `ExecuteFunctionKeyMacroAsync`. Returns a menu command ID or a special action.
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
    /// Original internal command 0x2a "Select": flips only the cursor item's selection (the cursor stays).
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

/// Picks the same action as the C# table from the name part of "#name[:argument]" (case-insensitive). 0 = left, 1 = right pane.
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
        // Original FUN_00443f14 case 0x2a: Select(ItemIndex, !selected). (The C# table opened advancedSelect.)
        "SELECT" => SelectCurrent,
        "FTPSITEMANAGER" => Menu("sites"),
        "OPENALLSELECTED" => Menu("openAll"),
        // Fixed: the jump table of original FUN_00443f14 maps 0x2f (OpenAllSelected) and 0x30 (FolderTree) to a do-nothing slot (0x4446dc).
        // Following the function key name (FUNC_FOLDERTREE "folder tree"), it opens the F10 folder picker window (miFolderTreeClick).
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

/// C# QuickFilterAsync (Enter in the toolbar quick search or search field): trims spaces; empty clears the filter,
/// text with '*' or '?' is used as is, otherwise "*text*".
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

/// FilterPattern of C# RefreshPaneAsync: visible when any ';'-separated mask matches (an empty pattern shows everything).
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

/// .NET FileSystemName.MatchesSimpleExpression(mask, name, ignoreCase: true): '*' is zero or more characters, '?' exactly one,
/// '\' takes the next character literally. Empty masks or names do not match. Case uses ordinal upper-case comparison (OrdinalIgnoreCase).
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

/// Gap after which C# quick search input counts as interrupted (QuickSearchSequenceTimeout).
pub const QUICK_SEARCH_TIMEOUT_MS: u128 = 1100;

/// ',' every three digits ("N" format of the current ko-KR/en-US culture).
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

/// .NET number format "N{decimals}" (thousands) or "0.##..." (trailing zeros omitted).
fn format_number(value: f64, decimals: usize, thousands: bool, trim: bool) -> String {
    let text = format!("{value:.decimals$}");
    let (integer, fraction) = text.split_once('.').map_or((text.as_str(), ""), |(i, f)| (i, f));
    let integer = if thousands { group_thousands(integer) } else { integer.to_owned() };
    let fraction = if trim { fraction.trim_end_matches('0') } else { fraction };
    if fraction.is_empty() { integer } else { format!("{integer}.{fraction}") }
}

/// Original TNXCustomFileListView::SizeToStr (0x7df870): list size cell text. unit = SizeUnit (Auto, B, KB, MB, GB = 0-4 of original +0x318),
/// comma = UseThousandsSeparator (+0x31c, original [Display] usesizecomma).
/// - Auto (FUN_007df5a4): from 1000 GB TB ((size>>30)/1024), from 1000 MB (0x3e800000) GB ((size>>20)/1024),
///   from 1000 KB (0xfa000) MB ((size>>10)/1024), from 1024 KB, otherwise "%d" (no unit). Values of 100 or more use Round(v),
///   10 or more Round(v*10)/10, otherwise Round(v*100)/100 (Delphi Round = round half to even) -> FloatToStrF(ffFixed, 10, digits) + " " + unit.
///   Unit table 0x92d414 = "b","KB","MB","GB","TB". No thousands separators.
/// - A chosen unit uses FloatToText (ffFixed, or ffNumber with commas, 20, digits): B without decimals or unit text, KB/MB/GB two decimals + " KB" and so on.
pub fn size_to_str(size: u64, unit: &str, comma: bool) -> String {
    const UNITS: [&str; 5] = ["b", "KB", "MB", "GB", "TB"];
    // Integer quotient as fixed-point text with `digits` decimals (q is value x 10^digits).
    let fixed = |q: u128, digits: u32, comma: bool| {
        let factor = 10u128.pow(digits);
        let integer = (q / factor).to_string();
        let integer = if comma { group_thousands(&integer) } else { integer };
        if digits == 0 { integer } else { format!("{integer}.{:0width$}", q % factor, width = digits as usize) }
    };
    let requested = ["auto", "b", "kb", "mb", "gb"].iter().position(|u| u.eq_ignore_ascii_case(unit)).unwrap_or(0);
    if requested > 0 {
        let power = (requested - 1) as u32;
        if power == 0 {
            return fixed(size as u128, 0, comma);
        }
        // Decimal rounding of FloatToText (half up).
        let divisor = 1024u128.pow(power);
        let q = (size as u128 * 100 * 2 + divisor) / (2 * divisor);
        return format!("{} {}", fixed(q, 2, comma), UNITS[requested - 1]);
    }
    let (index, mantissa) = if size >= 1_073_741_824_000 {
        (4, size >> 30)
    } else if size >= 0x3e80_0000 {
        (3, size >> 20)
    } else if size >= 0xfa000 {
        (2, size >> 10)
    } else if size >= 0x400 {
        (1, size)
    } else {
        return size.to_string();
    };
    let digits = if mantissa >= 100 * 1024 { 0 } else if mantissa >= 10 * 1024 { 1 } else { 2 };
    // Round(v x 10^digits), v = mantissa / 1024, rounding half to even.
    let numerator = mantissa as u128 * 10u128.pow(digits);
    let (mut q, rest) = (numerator / 1024, numerator % 1024);
    if rest * 2 > 1024 || (rest * 2 == 1024 && q % 2 == 1) {
        q += 1;
    }
    format!("{} {}", fixed(q, digits, false), UNITS[index])
}

/// C# FileSystemService.FormatSize(size, SizeUnit, UseThousandsSeparator): a chosen unit uses that unit (B as an integer, others two decimals);
/// "Auto" divides by 1024 with B as an integer and others one decimal.
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

/// .NET Path.GetExtension(name): from the last '.' (including the dot). Empty when the dot is last.
pub fn dotnet_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(at) if at + 1 < name.len() => &name[at..],
        _ => "",
    }
}

/// .NET Path.GetFileNameWithoutExtension(name).
pub fn dotnet_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(at) => &name[..at],
        None => name,
    }
}

/// File list extension (with the dot): like `dotnet_extension`, but a leading dot is not an extension (".gitconfig" is a name only).
/// Fixed (user request): the original and C# versions showed ".gitconfig" as an empty name with extension "gitconfig".
pub fn list_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(at) if at > 0 && at + 1 < name.len() => &name[at..],
        _ => "",
    }
}

/// File list name without extension (pairs with `list_extension`).
pub fn list_stem(name: &str) -> &str {
    &name[..name.len() - list_extension(name).len()]
}

/// C# SelectQuickSearchMatch: starts at the cursor item (0 without a cursor or at the last item, the next item with cycle_from_next).
/// Goes around once from there, first StrCmpNIW prefix matches, otherwise StrStrIW substring matches.
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
    fn list_size_text_follows_the_original_sizetostr() {
        // Auto: below 1024 a number without unit, three significant digits.
        assert_eq!(size_to_str(0, "Auto", true), "0");
        assert_eq!(size_to_str(1023, "Auto", true), "1023");
        assert_eq!(size_to_str(1024, "Auto", true), "1.00 KB");
        assert_eq!(size_to_str(1536, "Auto", false), "1.50 KB");
        assert_eq!(size_to_str(1152, "Auto", false), "1.12 KB", "Round is round-half-even");
        assert_eq!(size_to_str(10240, "Auto", true), "10.0 KB");
        assert_eq!(size_to_str(102_400, "Auto", true), "100 KB");
        assert_eq!(size_to_str(1_023_999, "Auto", true), "1000 KB");
        assert_eq!(size_to_str(1_024_000, "Auto", true), "0.98 MB", "1000 KB and above is MB");
        assert_eq!(size_to_str(1_234_567, "Auto", true), "1.18 MB");
        assert_eq!(size_to_str(5 << 30, "Auto", true), "5.00 GB");
        assert_eq!(size_to_str(2_000_000_000_000, "Auto", true), "1.82 TB");
        // Chosen unit: B has no unit text, commas use ffNumber.
        assert_eq!(size_to_str(1_234_567, "B", true), "1,234,567");
        assert_eq!(size_to_str(1_234_567, "B", false), "1234567");
        assert_eq!(size_to_str(1_234_567, "KB", true), "1,205.63 KB");
        assert_eq!(size_to_str(128, "KB", false), "0.13 KB", "FloatToText rounds half up");
        assert_eq!(size_to_str(1_234_567, "MB", false), "1.18 MB");
        assert_eq!(size_to_str(2_000_000_000_000, "GB", true), "1,862.65 GB");
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
        // List: a leading dot is part of the name.
        assert_eq!((list_stem(".gitconfig"), list_extension(".gitconfig")), (".gitconfig", ""));
        assert_eq!((list_stem(".config.json"), list_extension(".config.json")), (".config", ".json"));
        assert_eq!((list_stem("a.tar.gz"), list_extension("a.tar.gz")), ("a.tar", ".gz"));
        assert_eq!((list_stem("name."), list_extension("name.")), ("name.", ""));
    }
}
