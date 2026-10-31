//! NexusFile-compatible language, command, shortcut, rename and description formats.
use crate::settings::Settings;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn decode(bytes: &[u8], utf16_default: bool) -> String {
    let (encoding, skip) = if bytes.starts_with(&[255, 254]) {
        (Some(encoding_rs::UTF_16LE), 2)
    } else if bytes.starts_with(&[254, 255]) {
        (Some(encoding_rs::UTF_16BE), 2)
    } else if bytes.starts_with(&[239, 187, 191]) {
        (Some(encoding_rs::UTF_8), 3)
    } else if utf16_default {
        (Some(encoding_rs::UTF_16LE), 0)
    } else {
        (None, 0)
    };
    if let Some(encoding) = encoding {
        return encoding
            .decode_without_bom_handling(&bytes[skip..])
            .0
            .into_owned();
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => encoding_rs::EUC_KR
            .decode_without_bom_handling(bytes)
            .0
            .into_owned(),
    }
}
fn parse_resource(text: &str) -> HashMap<String, String> {
    let mut section = "Root";
    let mut values = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with([';', '#']) {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = &trimmed[1..trimmed.len() - 1];
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && !key.trim().is_empty()
        {
            values.insert(
                format!("{section}.{}", key.trim()).to_lowercase(),
                value.replace("\\n", "\n").replace("\\r", ""),
            );
        }
    }
    values
}

#[derive(Clone, Debug, Default)]
pub struct Language {
    pub code: String,
    values: HashMap<String, String>,
    fallback: HashMap<String, String>,
    keys_by_text: HashMap<String, String>,
}
impl Language {
    /// `directory` contains original *.lang resources. Missing languages fall back to Korean.
    pub fn load(directory: &Path, code: &str) -> Self {
        let read = |code: &str| {
            fs::read(directory.join(format!("{code}.lang")))
                .ok()
                .map(|b| parse_resource(&decode(&b, false)))
                .unwrap_or_default()
        };
        let code = if !code.contains(['/', '\\', ':'])
            && directory.join(format!("{code}.lang")).is_file()
        {
            code
        } else {
            "ko"
        };
        let korean = read("ko");
        let english = read("en");
        let mut keys_by_text = HashMap::new();
        for strings in [&korean, &english] {
            let mut entries: Vec<_> = strings.iter().collect();
            entries.sort_by_key(|(key, _)| {
                (!key.starts_with("res."), !key.starts_with("root."), *key)
            });
            for (key, value) in entries {
                let text = clean_language_text(value);
                if !text.is_empty() {
                    keys_by_text
                        .entry(text.to_lowercase())
                        .or_insert_with(|| key.clone());
                }
            }
        }
        Self {
            code: code.into(),
            values: if code.eq_ignore_ascii_case("ko") {
                korean
            } else if code.eq_ignore_ascii_case("en") {
                english.clone()
            } else {
                read(code)
            },
            fallback: english,
            keys_by_text,
        }
    }
    /// Accepts RES.KEY, Root.KEY, formMain.Menu.miOpen, Menu.miOpen or a bare root key.
    pub fn text(&self, key: &str, fallback: &str) -> String {
        clean_language_text(self.raw(key).as_deref().unwrap_or(fallback))
    }
    /// The stored value with its '&' accelerator markers (the original strips them itself), or None when absent.
    /// Same key rules as [`Language::text`].
    pub fn raw(&self, key: &str) -> Option<String> {
        let lower = key.to_lowercase();
        let mut keys = vec![lower.clone()];
        if let Some(rest) = lower
            .strip_prefix("root.")
            .or_else(|| lower.strip_prefix("res."))
        {
            keys = vec![format!("res.{rest}"), format!("root.{rest}")];
        } else if lower.starts_with("menu.") || lower.starts_with("toolbutton.") {
            keys.push(format!("formmain.{lower}"));
        } else if !lower.contains('.') {
            keys = vec![format!("res.{lower}"), format!("root.{lower}")];
        }
        keys.iter()
            .find_map(|key| self.values.get(key))
            .or_else(|| keys.iter().find_map(|key| self.fallback.get(key)))
            .cloned()
    }
    /// Resolve either source caption through cached original resource keys; performs no filesystem I/O.
    pub fn translate_pair(&self, korean: &str, english: &str) -> String {
        let fallback = if self.code.eq_ignore_ascii_case("ko") {
            korean
        } else {
            english
        };
        if let Some(key) = self
            .keys_by_text
            .get(&clean_language_text(korean).to_lowercase())
            .or_else(|| {
                self.keys_by_text
                    .get(&clean_language_text(english).to_lowercase())
            })
        {
            self.text(key, fallback)
        } else {
            fallback.into()
        }
    }
}
fn clean_language_text(value: &str) -> String {
    value
        .replace("&&", "\u{1}")
        .replace('&', "")
        .replace('\u{1}', "&")
}
pub fn available_codes(directory: &Path) -> Vec<String> {
    let mut codes: Vec<_> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file()
                || !path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("lang"))
            {
                return None;
            }
            path.file_stem()?.to_str().map(str::to_owned)
        })
        .collect();
    codes.sort_by_key(|code| code.to_lowercase());
    codes.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    codes
}

#[derive(Clone, Debug, Default)]
pub struct MacroPane {
    pub path: PathBuf,
    pub focused: Option<String>,
    pub selected: Vec<String>,
}

fn drive(path: &str) -> String {
    if path.as_bytes().get(1) == Some(&b':') {
        return format!("{}\\", &path[..2]);
    }
    if let Some(unc) = path.strip_prefix(r"\\") {
        let mut parts = unc.split('\\');
        if let (Some(server), Some(share)) = (parts.next(), parts.next()) {
            return format!(r"\\{server}\{share}\");
        }
    }
    if path.starts_with('/') {
        return "/".into();
    }
    String::new()
}
fn split_name(name: &str) -> (&str, &str) {
    name.rsplit_once('.').unwrap_or((name, ""))
}
fn pane_value(group: &str, pane: &MacroPane) -> String {
    let path = pane.path.to_string_lossy();
    let focused = pane.focused.as_deref().unwrap_or("");
    let (stem, ext) = if pane.path.join(focused).is_dir() {
        (focused, "")
    } else {
        split_name(focused)
    };
    match group {
        "SELECTED" => {
            let selected = if pane.selected.is_empty() {
                pane.focused.iter().map(String::as_str).collect::<Vec<_>>()
            } else {
                pane.selected.iter().map(String::as_str).collect()
            };
            selected
                .into_iter()
                .map(|name| {
                    let full = if name == ".." {
                        pane.path
                            .parent()
                            .unwrap_or(&pane.path)
                            .to_string_lossy()
                            .into_owned()
                    } else if path.is_empty() {
                        name.to_owned()
                    } else {
                        format!("{}\\{}", path.trim_end_matches(['\\', '/']), name)
                    };
                    format!("\"{full}\"")
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
        "PATH" => {
            if path.is_empty() || path.ends_with('\\') {
                path.into_owned()
            } else {
                format!("{path}\\")
            }
        }
        "DRIVE" => drive(&path),
        "FILENAME" => focused.to_owned(),
        "NAME" => stem.to_owned(),
        "EXTENSION" => ext.to_owned(),
        _ => String::new(),
    }
}
pub fn expand_command(
    text: &str,
    active: &MacroPane,
    left: &MacroPane,
    right: &MacroPane,
) -> String {
    let mut result = text.to_owned();
    for (legacy, group) in [
        ("!..!", "SELECTED"),
        ("!\\", "PATH"),
        ("!:", "DRIVE"),
        ("!.!", "FILENAME"),
        ("!,", "NAME"),
        (",!", "EXTENSION"),
    ] {
        for (token, pane) in [
            (legacy.to_owned(), active),
            (format!("$({group})"), active),
            (format!("$({group}1)"), left),
            (format!("$({group}2)"), right),
        ] {
            let regex = regex::RegexBuilder::new(&regex::escape(&token))
                .case_insensitive(true)
                .build()
                .expect("escaped token");
            result = regex
                .replace_all(&result, regex::NoExpand(&pane_value(group, pane)))
                .into_owned();
        }
    }
    result
}

/// Program macros are case sensitive in command fields, and do not expand environment variables.
pub fn expand_program_macros(text: &str, directory: &Path) -> String {
    let path = directory.to_string_lossy();
    let path = if path.len() > 3 {
        path.trim_end_matches('\\')
    } else {
        &path
    };
    text.replace("$(PROGRAMDIR)", path)
        .replace("$(PROGRAMDRIVE)", &drive(path))
}
pub fn strip_function_key_label(command: &str) -> &str {
    if command.starts_with('[') {
        command.split_once(']').map(|(_, c)| c).unwrap_or("")
    } else {
        command
    }
}
pub fn take_parameter_prompt(arguments: &mut String) -> bool {
    if let Some(index) = arguments.find("%p").or_else(|| arguments.find("%P")) {
        arguments.replace_range(index..index + 2, "");
        true
    } else {
        false
    }
}

/// C# NexusFileMacroService.ExpandSetting: %environment variables% and $(PROGRAMDIR)/$(PROGRAMDRIVE).
pub fn expand_setting(value: &str) -> String {
    let env = regex::Regex::new(r"%([^%]+)%").expect("constant regex");
    let value = env.replace_all(value, |capture: &regex::Captures<'_>| {
        std::env::var(&capture[1]).unwrap_or_else(|_| capture[0].to_owned())
    });
    let directory = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_owned))
        .unwrap_or_default();
    let macro_pattern =
        regex::Regex::new(r"(?i)\$\((PROGRAMDIR|PROGRAMDRIVE)\)").expect("constant regex");
    macro_pattern
        .replace_all(&value, |capture: &regex::Captures<'_>| {
            if capture[1].eq_ignore_ascii_case("PROGRAMDIR") {
                directory.to_string_lossy().into_owned()
            } else {
                drive(&directory.to_string_lossy())
            }
        })
        .into_owned()
}

/// Imports only the explicitly supplied source; never saves or touches a default profile.
/// The keys are the ones the original reads at startup in FUN_00474dac (nexusfile.ini [Window] maximized, dualview, … looked up by name)
/// and writes back in FUN_0046ff1c (one "%s=%d" / "%s=%s" line per key, FUN_0046fd54 / FUN_0046fe1c); its default shortcut list
/// (miOpen=Enter, …) is built by FUN_0047a788 at startup. NDR keeps one settings.json and only imports the ini.
pub fn import_ini(path: &Path) -> Result<Settings, String> {
    use serde_json::json;
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let sample = &bytes[..bytes.len().min(128)];
    let utf16 = bytes.len() >= 4
        && bytes.len().is_multiple_of(2)
        && sample
            .iter()
            .skip(1)
            .step_by(2)
            .filter(|&&b| b == 0)
            .count()
            >= sample.len() / 8;
    let mut raw = BTreeMap::new();
    let mut section = "";
    let content = decode(&bytes, utf16);
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with([';', '#']) || line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let name = format!("{section}.{}", key.trim());
            if let Some(existing) = raw
                .keys()
                .find(|key: &&String| key.eq_ignore_ascii_case(&name))
                .cloned()
            {
                raw.remove(&existing);
            }
            raw.insert(name, value.trim().trim_matches('"').to_owned());
        }
    }
    let parsed: HashMap<_, _> = raw
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.clone()))
        .collect();
    let get = |section: &str, key: &str| {
        parsed
            .get(&format!("{section}.{key}").to_lowercase())
            .map(String::as_str)
    };
    let mut settings = Settings::default();
    settings.values.insert("LegacyIniValues".into(), json!(raw));
    settings.set_text(
        "LegacyIniPath",
        fs::canonicalize(path)
            .map_err(|e| e.to_string())?
            .to_string_lossy(),
    );
    for (property, section, key, default) in [
        ("DualPaneMode", "Window", "dualview", true),
        ("WindowMaximized", "Window", "maximized", false),
        ("UseWindowAlpha", "Window", "usewndalpha", false),
        ("HideWhenMinimized", "Window", "hidewhenminimized", false),
        ("ShowToolBar", "Show", "stoolbar", true),
        ("ShowFunctionBar", "Show", "sbuttonbar", true),
        ("ShowFolderDiskInfoBar", "Show", "sfolderinfobar", true),
        ("ShowFileInfoBar", "Show", "sfileinfobar", true),
        ("ShowExtensionInfoBar", "Show", "sextinfobar", false),
        ("ShowVerticalBar", "Show", "svertbar", true),
        ("ShowDriveButtons", "Show", "sdrivebtns", false),
        ("ShowAddressBar", "Show", "saddressbar", true),
        ("ShowIcons", "Display", "showicon", true),
        ("ShowDriveList", "Display", "showdrivelist", true),
        ("ShowColumnLines", "Display", "showcolline", true),
        ("ShowRowLines", "Display", "showrowline", false),
        ("UseMenuAlpha", "Display", "usemenualpha", true),
        ("ShowFileType", "Display", "showfiletype", true),
        ("UseDateAge", "Display", "usedateage", true),
        (
            "UseThousandsSeparator",
            "Display",
            "thousandsseparator",
            true,
        ),
        ("AttachExtensionToName", "Display", "attachext", false),
        ("ConfirmDelete", "Confirm", "delconfirm", true),
        ("ConfirmDragDrop", "Confirm", "confirmdrop", true),
        ("SaveDriveFolders", "Operation", "savedrivefolders", true),
        (
            "RemoveReadOnlyWhenCopying",
            "Operation",
            "remreadonly",
            true,
        ),
        ("RenameWithExtension", "Operation", "renamewithext", true),
        ("SelectFolders", "Operation", "selectfolder", false),
        ("UseExplorerMenu", "Operation", "explorermenu", false),
        ("ShowDeleteButton", "Operation", "showdelbtn", true),
        (
            "UseExternalArchivePrograms",
            "Operation",
            "useextarc",
            false,
        ),
        (
            "CheckForUpdatesOnStartup",
            "Operation",
            "checkupdate",
            false,
        ),
        (
            "UseAlternateDriveHandling",
            "Operation",
            "hanshdrive",
            false,
        ),
        ("UseLogicalSort", "Operation", "uselogsort", true),
        ("UseTabs", "Operation", "usetab", true),
        ("UseOverlayIcons", "Operation", "overlay", true),
        ("UseCopyHandler", "Operation", "usecopyhandler", false),
        ("UseSingleInstance", "Operation", "runonce", false),
        ("UseDescriptionFile", "Operation", "useion", true),
        ("SaveFolderTreeState", "Tree", "tfileinroot", true),
        ("TreeWindowMatchesMainSize", "Tree", "tmainwndsize", true),
        ("TreeWindowMaximized", "Tree", "tmaximized", false),
        ("ShowHiddenFoldersInTree", "Tree", "tshowhidden", false),
        ("ShowFolderTreeIcons", "Tree", "tshowicon", true),
    ] {
        let value = get(section, key)
            .and_then(|s| {
                if s.eq_ignore_ascii_case("true") {
                    Some(true)
                } else if s.eq_ignore_ascii_case("false") {
                    Some(false)
                } else {
                    s.parse::<i64>().ok().map(|n| n != 0)
                }
            })
            .unwrap_or(default);
        settings.set_flag(property, value);
    }
    for (property, section, key, default) in [
        ("WindowLeft", "Window", "left", 10.),
        ("WindowTop", "Window", "top", 10.),
        ("WindowWidth", "Window", "width", 1450.),
        ("WindowHeight", "Window", "height", 900.),
        ("WindowAlpha", "Window", "wndalpha", 210.),
        ("CharacterCase", "Display", "charcase", 0.),
        ("ListLineSpace", "Display", "linespace", 0.),
        ("ThumbnailSize", "Display", "thumbsize", 120.),
        ("DefaultDeleteButton", "Operation", "defdelbtn", 2.),
        ("MaxRecentFileCount", "Operation", "mufcount", 10.),
        ("DefaultDestinationMode", "Operation", "defdest", 0.),
        ("PathClickMode", "Operation", "pathclick", 0.),
        ("TreeWindowLeft", "Tree", "tleft", 10.),
        ("TreeWindowTop", "Tree", "ttop", 10.),
        ("TreeWindowWidth", "Tree", "twidth", 800.),
        ("TreeWindowHeight", "Tree", "theight", 600.),
        ("FolderTreeItemWidth", "Tree", "titemwidth", 130.),
        ("FolderTreeDepth", "Tree", "tfullrefreshlevel", 10.),
        ("DefaultArchiveFormatIndex", "ArcDlg", "arcformat", 1.),
        (
            "DefaultArchiveCompressionPresetIndex",
            "ArcDlg",
            "arccomplevel",
            1.,
        ),
    ] {
        let decimal = matches!(
            property,
            "WindowLeft"
                | "WindowTop"
                | "WindowWidth"
                | "WindowHeight"
                | "TreeWindowLeft"
                | "TreeWindowTop"
                | "TreeWindowWidth"
                | "TreeWindowHeight"
        );
        let mut number = get(section, key)
            .and_then(|v| {
                if decimal {
                    v.parse::<f64>().ok()
                } else {
                    v.parse::<i64>().ok().map(|n| n as f64)
                }
            })
            .filter(|v| v.is_finite())
            .unwrap_or(default);
        if property == "PathClickMode" {
            number = number.clamp(0., 2.);
        }
        if property == "WindowAlpha" {
            number = number.clamp(0., 255.);
        }
        settings.values.insert(
            property.into(),
            if decimal {
                json!(number)
            } else {
                json!(number as i64)
            },
        );
    }
    for (property, section, key, default) in [
        ("DateFormat", "Display", "dateformat", "yyyy-MM-dd"),
        ("TimeFormat", "Display", "timeformat", "HH:mm"),
        ("ExplorerShortcutKey", "Operation", "shortcutkey", "Q"),
        ("CopyHandlerPath", "Operation", "copyhandler", ""),
        ("TextViewerPath", "ExtProg", "textviewer", "notepad.exe"),
        ("TextEditorPath", "ExtProg", "texteditor", "notepad.exe"),
        ("OpenWithParameters", "ExtProg", "OpenWithParam", ""),
    ] {
        settings.set_text(
            property,
            expand_setting(get(section, key).unwrap_or(default)),
        );
    }
    for (side, section, index) in [("Left", "View1", 1), ("Right", "View2", 2)] {
        for (property, key) in [("Path", "lastpath"), ("StartPath", "startpath")] {
            if let Some(v) = get(section, &format!("{key}{index}")) {
                settings.set_text(&format!("{side}{property}"), expand_setting(v));
            }
        }
        for (property, key) in [
            ("ShowHiddenFiles", "showhidden"),
            ("SortDescending", "sortdesc"),
            ("FolderSortDescending", "foldersortdesc"),
            ("AlwaysShowFolderSize", "alwaysshowsize"),
            ("StartPathEnabled", "usestartpath"),
            ("FolderTree", "showtree"),
        ] {
            let value = get(section, &format!("{key}{index}"))
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let key = if property == "FolderTree" {
                format!("Show{side}FolderTree")
            } else {
                format!("{side}{property}")
            };
            settings.set_flag(&key, value);
        }
        for (property, key, default) in [
            ("FolderTreeWidth", "treewidth", 120),
            ("PaneWidth", "viewwidth", 50),
            ("ScrollStep", "scrollstep", 2),
            ("ColumnCount", "listcolumn", 0),
        ] {
            settings.values.insert(
                format!("{side}{property}"),
                json!(
                    get(section, &format!("{key}{index}"))
                        .and_then(|v| v.parse::<i64>().ok())
                        .unwrap_or(default)
                ),
            );
        }
        for (property, key) in [("SortMode", "sort"), ("FolderSortMode", "foldersort")] {
            let mode = match get(section, &format!("{key}{index}")).unwrap_or("1") {
                "2" => "Extension",
                "3" => "Size",
                "4" => "Modified",
                "5" => "Attributes",
                "6" => "Color",
                _ => "Name",
            };
            settings.set_text(&format!("{side}{property}"), mode);
        }
        // Original ReadInteger(View, viewstyleN, 0): 0 (auto) when missing.
        let mode = match get(section, &format!("viewstyle{index}")).unwrap_or("0") {
            "1" => "Names",
            "2" => "Details",
            "3" => "Thumbnails",
            _ => "Auto",
        };
        settings.set_text(&format!("{side}ViewStyle"), mode);
    }
    // The size in the original listfont/treefont "\"name\",size,style" is in points (TFont.Size). ListFontSize is pixels, so it is converted
    // (NDR default 14 px when missing). FolderTreeFontSize stays in points.
    for (property, section, key, default_size, pixels) in [
        ("List", "Display", "listfont", crate::settings::DEFAULT_LIST_FONT_PIXELS, true),
        ("FolderTree", "Tree", "treefont", 9., false),
    ] {
        let font = get(section, key).unwrap_or("");
        let mut pieces = font.split(',');
        let family = pieces.next().unwrap_or("").trim().trim_matches('"');
        settings.set_text(
            &format!("{property}Font"),
            if family.is_empty() {
                "Segoe UI"
            } else {
                family
            },
        );
        let size = pieces
            .next()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .map(|points| if pixels { crate::settings::font_points_to_pixels(points) } else { points })
            .unwrap_or(default_size)
            .clamp(8., 32.);
        settings
            .values
            .insert(format!("{property}FontSize"), json!(size));
        // Original "\"name\",size,style": style = TFontStyles byte (1 bold, 2 italic, 4 underline, 8 strikeout).
        let style = pieces
            .next()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .map_or(0, |v| v & 0x0F);
        settings
            .values
            .insert(format!("{property}FontStyle"), json!(style));
    }
    let theme = match get("Window", "skin")
        .unwrap_or("black")
        .to_lowercase()
        .as_str()
    {
        "black" => "Black",
        "classic" => "Classic",
        "gray" | "grey" => "Gray",
        "white" | "light" => "Light",
        "system" => "System",
        _ => "Dark",
    };
    settings.set_text("Theme", theme);
    settings.set_flag(
        "StartWithRightPaneActive",
        get("Window", "lastfocus") == Some("1"),
    );
    settings.set_flag(
        "ShowFolderTree",
        settings.flag("ShowLeftFolderTree", false) || settings.flag("ShowRightFolderTree", false),
    );
    settings.set_flag(
        "NavigateOnPathClick",
        settings.values["PathClickMode"].as_f64() == Some(1.),
    );
    settings.set_flag(
        "HasWindowBounds",
        ["width", "height", "maximized"]
            .iter()
            .any(|k| get("Window", k).is_some()),
    );
    settings.set_flag(
        "HasTreeWindowBounds",
        ["twidth", "theight", "tmaximized"]
            .iter()
            .any(|k| get("Tree", k).is_some()),
    );
    settings.set_flag("ShowDeleteToolbarButton", true);
    // Original [Operation] completesound (setting +0xdc, completion sound 0x43d9fc) and playwav (+0xf0, 0x483c79 plays .wav inside the program)
    // are separate values. Both default to on (0x46f03a, 0x46f088).
    let on = |key: &str| get("Operation", key).map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(true);
    settings.set_flag("PlayCompletionSound", on("completesound"));
    settings.set_flag("PlayWaveInProgram", on("playwav"));
    if let Some(mode) = get("Operation", "esckey")
        .and_then(|v| v.parse::<u8>().ok())
        .filter(|v| *v <= 4)
    {
        settings
            .values
            .insert("EscapeKeyAction".into(), json!(mode));
    }
    let format = match get("ArcDlg", "arcformat").unwrap_or("1") {
        "1" => Some("zip"),
        "2" => Some("rar"),
        "3" => Some("7z"),
        "4" => Some("alz"),
        "5" => Some("arj"),
        _ => None,
    };
    settings
        .values
        .insert("DefaultArchiveFormatExtension".into(), json!(format));
    settings.set_strings(
        "WorkingFolders",
        (1..=5)
            .map(|n| expand_setting(get("WorkFolder", &format!("WF{n}")).unwrap_or("")))
            .collect(),
    );
    settings.values.insert(
        "FunctionKeys".into(),
        json!(raw_section(&content, "FuncKey")),
    );
    let programs: BTreeMap<_, _> = ["rar", "arj", "ace", "alz"]
        .iter()
        .map(|name| {
            (
                *name,
                expand_setting(get("ExtProg", &format!("{name}prog")).unwrap_or("")),
            )
        })
        .collect();
    settings
        .values
        .insert("ExternalPrograms".into(), json!(programs));
    let special: BTreeMap<_, _> = [
        ("Documents", "path_documents"),
        ("Pictures", "path_pictures"),
        ("Music", "path_musics"),
        ("Videos", "path_videos"),
        ("Downloads", "path_downloads"),
    ]
    .iter()
    .filter_map(|(name, key)| {
        get("Paths", key)
            .filter(|v| !v.is_empty())
            .map(|v| (*name, expand_setting(v)))
    })
    .collect();
    settings
        .values
        .insert("SpecialFolderPaths".into(), json!(special));
    let parent = path.parent().unwrap_or(Path::new("."));
    let shortcut = parent.join("shortcut.ini");
    if shortcut.is_file() {
        settings.set_text("OriginalShortcutMapText", read_shortcut_file(&shortcut)?);
    }
    let excludes = parent.join("treeexcludes.ini");
    if excludes.is_file() {
        let text = decode(&fs::read(excludes).map_err(|e| e.to_string())?, false);
        settings.set_strings(
            "FolderTreeExcludedPaths",
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with([';', '#']))
                .map(expand_setting)
                .collect(),
        );
    }
    import_sidecars(&mut settings, parent, path)?;
    settings.normalize();
    Ok(settings)
}
fn raw_section(content: &str, wanted: &str) -> BTreeMap<String, String> {
    let mut inside = false;
    let mut result = BTreeMap::new();
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            inside = line[1..line.len() - 1].eq_ignore_ascii_case(wanted);
            continue;
        }
        if inside
            && !line.starts_with([';', '#'])
            && let Some((k, v)) = line.split_once('=')
        {
            result.insert(k.trim().into(), v.trim().trim_matches('"').into());
        }
    }
    result
}

fn read_utf16_sidecar(path: &Path) -> Result<Option<String>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(decode(&bytes, true))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}
fn import_sidecars(settings: &mut Settings, parent: &Path, ini: &Path) -> Result<(), String> {
    use serde_json::json;
    if let Some(text) = read_utf16_sidecar(&parent.join("favorits.save"))? {
        let lines: Vec<_> = text.lines().collect();
        let mut slots = vec![(String::new(), String::new()); 20];
        let versioned = lines
            .first()
            .is_some_and(|line| matches!(*line, "NEXUSFILEFAV.VER2.1" | "NEXUSFILEFAV.VER2"));
        let separator = if lines.first() == Some(&"NEXUSFILEFAV.VER2") {
            '='
        } else {
            '\t'
        };
        let rows: Vec<_> = lines
            .iter()
            .map(|line| line.split_once(separator))
            .collect();
        for (index, row) in rows.iter().enumerate().skip(usize::from(versioned)) {
            let key = row.map(|(k, _)| k).unwrap_or("");
            let (slot, name) = if versioned {
                let prefix: String = key.chars().take(2).collect();
                let Some(slot) = prefix.parse::<usize>().ok().filter(|n| *n < 20) else {
                    continue;
                };
                (slot, key.chars().skip(2).take(255).collect::<String>())
            } else {
                if index >= 20 {
                    break;
                }
                (index, key.into())
            };
            let value = rows
                .iter()
                .flatten()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| (*value).to_owned())
                .unwrap_or_default();
            slots[slot] = (name, value);
        }
        settings.set_strings("Favorites", slots.iter().map(|(_, v)| v.clone()).collect());
        settings.set_strings("FavoriteNames", slots.into_iter().map(|(n, _)| n).collect());
    }
    // Original tabs.save writer FUN_0046dad8: "NEXUSFILETABS.VER1", then "[Tab%d]" per pane followed by that pane's tab lines, saved as Unicode.
    // The reader FUN_0046d4b4 takes the pane number from "[Tab%d]" and every non-empty line after it; an older file ("[Tabs%d]" sections of
    // name=value lines) is converted and saved again in the new form.
    if let Some(text) = read_utf16_sidecar(&parent.join("tabs.save"))? {
        let lines: Vec<_> = text.lines().collect();
        let mut panes: [Vec<String>; 2] = [Vec::new(), Vec::new()];
        if lines.first() == Some(&"NEXUSFILETABS.VER1") {
            let mut pane = Some(0);
            for line in lines.iter().skip(1) {
                if line.starts_with("[Tab") && line.len() >= 5 {
                    pane = line
                        .get(4..line.len() - 1)
                        .and_then(|s| s.parse::<usize>().ok())
                        .filter(|n| *n < 2);
                    continue;
                }
                if let Some(pane) = pane
                    && let Some((_, path)) = line.split_once('\t')
                    && !path.is_empty()
                {
                    panes[pane].push(path.into());
                }
            }
        } else {
            for (pane, paths) in panes.iter_mut().enumerate() {
                if let Some(index) = lines
                    .iter()
                    .position(|line| line.eq_ignore_ascii_case(&format!("[Tabs{pane}]")))
                {
                    for line in lines
                        .iter()
                        .skip(index + 1)
                        .take_while(|line| !line.is_empty() && !line.starts_with("[Tabs"))
                    {
                        if let Some((_, path)) = line.split_once('=')
                            && !path.is_empty()
                        {
                            paths.push(path.into());
                        }
                    }
                }
            }
        }
        for (pane, side) in panes.iter_mut().zip(["Left", "Right"]) {
            let use_start = settings.flag(&format!("{side}StartPathEnabled"), false);
            let start = settings.text(&format!("{side}StartPath"), "");
            let current = if use_start && !start.trim().is_empty() {
                start
            } else {
                settings.text(&format!("{side}Path"), "")
            };
            let index = pane
                .iter()
                .position(|path| path.eq_ignore_ascii_case(&current))
                .unwrap_or_else(|| {
                    if pane.len() == 1 && use_start {
                        pane[0] = current;
                        0
                    } else {
                        pane.push(current);
                        pane.len() - 1
                    }
                });
            settings.set_strings(&format!("{side}Tabs"), std::mem::take(pane));
            settings
                .values
                .insert(format!("Selected{side}Tab"), json!(index));
        }
    }
    for (file, key) in [
        ("history_cmd.save", "CommandHistory"),
        ("history_exe.save", "ExecutableHistory"),
    ] {
        if let Some(text) = read_utf16_sidecar(&parent.join(file))? {
            settings.set_strings(key, text.lines().map(str::to_owned).collect());
        }
    }
    if let Some(text) = read_utf16_sidecar(&parent.join("history_drive.save"))? {
        let mut known = HashSet::new();
        let mut folders = BTreeMap::new();
        for line in text.lines() {
            if let Some((name, path)) = line.split_once('=')
                && !name.is_empty()
                && known.insert(name.to_lowercase())
            {
                folders.insert(name, path);
            }
        }
        settings
            .values
            .insert("LastFoldersByDrive".into(), json!(folders));
    }
    if let Some(text) = read_utf16_sidecar(&parent.join("ftpsite.ini"))? {
        let mut sites = Vec::new();
        let mut header = None;
        let mut rows = Vec::new();
        for line in text.lines() {
            if line.starts_with('[') {
                if let Some(name) = header {
                    sites.push(import_remote_site(name, &rows)?);
                }
                header = Some(line);
                rows.clear();
            } else if header.is_some() {
                rows.push(line);
            }
        }
        if let Some(name) = header {
            sites.push(import_remote_site(name, &rows)?);
        }
        settings.values.insert("RemoteSites".into(), json!(sites));
    }
    if let Some(text) = read_utf16_sidecar(&parent.join("history_dest.save"))? {
        settings.values.insert(
            "MostUsedDestinations".into(),
            json!(import_destination_history(&text)),
        );
    }
    let col = ini.with_extension("col");
    if col.is_file() {
        let bytes = fs::read(col).map_err(|e| e.to_string())?;
        if let Ok(map) = crate::styles::ColorMap::parse(&decode(&bytes, false)) {
            let hex = |color: &[u8; 4]| format!("{:02X}{:02X}{:02X}", color[0], color[1], color[2]);
            let elements: BTreeMap<_, _> = map
                .elements
                .iter()
                .map(|(key, color)| (key.clone(), hex(color)))
                .collect();
            let extensions: BTreeMap<_, _> = map
                .extensions
                .iter()
                .map(|(key, color)| (key.clone(), format!("#{}", hex(color))))
                .collect();
            settings.set_flag("UseNexusFileColorMap", !elements.is_empty());
            settings
                .values
                .insert("NexusFileColorMapValues".into(), json!(elements));
            settings
                .values
                .insert("ExtensionColors".into(), json!(extensions));
        }
    }
    Ok(())
}
fn import_remote_site(header: &str, rows: &[&str]) -> Result<serde_json::Value, String> {
    let get = |wanted: &str| {
        rows.iter()
            .find_map(|line| {
                line.split_once('=')
                    .filter(|(key, _)| key.eq_ignore_ascii_case(wanted))
                    .map(|(_, value)| value)
            })
            .unwrap_or("")
    };
    let name: String = header
        .chars()
        .skip(1)
        .take(header.chars().count().saturating_sub(2))
        .collect();
    let password = decode_legacy_password(get("pass"));
    let encrypted = if password.is_empty() {
        String::new()
    } else {
        crate::native::protect_secret(&password)?
    };
    Ok(
        serde_json::json!({"Name":name,"Host":get("host"),"Port":get("port").parse::<i32>().unwrap_or(21),"User":get("user"),"EncryptedPassword":encrypted,"RemoteDirectory":get("remotedir"),"Passive":get("passive").parse::<i32>().map(|n|n!=0).unwrap_or(true),"Protocol":if get("servertype").eq_ignore_ascii_case("SFTP"){"SFTP"}else{"FTP"}}),
    )
}
/// Legacy FTP password RC4 compatibility. The returned secret is for immediate DPAPI protection.
pub fn decode_legacy_password(encoded: &str) -> String {
    let chars: Vec<_> = encoded.chars().collect();
    let mut data: Vec<u16> = chars
        .chunks(2)
        .map(|chunk| u16::from_str_radix(&chunk.iter().collect::<String>(), 16).unwrap_or(0))
        .collect();
    data.push(0);
    let mut state: [usize; 256] = std::array::from_fn(|i| i);
    let key = b"xxx.xiles.net";
    let mut j = 0;
    for i in 0..256 {
        j = (j + state[i] + key[i % key.len()] as usize) & 255;
        state.swap(i, j);
    }
    let mut i = 0;
    j = 0;
    for value in data.iter_mut().take(chars.len() / 2) {
        i = (i + 1) & 255;
        j = (j + state[i]) & 255;
        state.swap(i, j);
        *value ^= state[(state[i] + state[j]) & 255] as u16;
    }
    let end = data.iter().position(|v| *v == 0).unwrap_or(data.len());
    String::from_utf16_lossy(&data[..end])
}
/// Original FUN_0046ddf0: the date is read by position (Copy of characters 1-4, 6-7, 9-10, 12-13, 15-16, 18-19 + StrToInt), so the
/// separators written by the locale do not matter. Fixed: NDR accepted only "-" and ":" and dropped every entry written with "/" or ".".
fn positional_date_time(text: &str) -> Option<chrono::NaiveDateTime> {
    let chars: Vec<char> = text.trim().chars().collect();
    let number = |from: usize, to: usize| -> Option<u32> { chars.get(from..to)?.iter().collect::<String>().parse().ok() };
    chrono::NaiveDate::from_ymd_opt(number(0, 4)? as i32, number(5, 7)?, number(8, 10)?)?.and_hms_opt(number(11, 13)?, number(14, 16)?, number(17, 19)?)
}

/// Original destination history (dest file): FUN_0046e008 reads it (Unicode; "NEXUSFILEDEST.VER1", then tab-separated path, count, date,
/// kept only when newer than the age limit; an older "path=count" file is converted and saved again), FUN_0046e4dc writes it ("%s\t%d\t%s",
/// skipping folders that no longer exist, date = FormatDateTime("yyyy/mm/dd hh:nn:ss"), whose '/' and ':' become the locale separators).
fn import_destination_history(text: &str) -> Vec<serde_json::Value> {
    use chrono::{Local, SecondsFormat, TimeZone, Utc};
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return Vec::new();
    };
    let now = Local::now();
    let cutoff = now.naive_local() - chrono::Duration::days(60);
    if first == "NEXUSFILEDEST.VER1" {
        lines.filter_map(|line|{
            let fields:Vec<_>=line.split('\t').collect();if fields.len()!=3{return None;}
            let count=fields[1].trim().parse::<i32>().ok()?;let used=positional_date_time(fields[2])?;
            if used<=cutoff{return None;}
            let utc=Local.from_local_datetime(&used).latest()?.with_timezone(&Utc);
            Some(serde_json::json!({"Path":fields[0],"UseCount":count,"LastUsedUtc":utc.to_rfc3339_opts(SecondsFormat::Secs,true)}))
        }).collect()
    } else {
        std::iter::once(first).chain(lines).filter_map(|line|{
            let (path,count)=line.split_once('=')?;let count=count.trim().parse::<i32>().ok()?;
            Some(serde_json::json!({"Path":path,"UseCount":count,"LastUsedUtc":now.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs,true)}))
        }).collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenameSpec {
    pub find: String,
    pub replace: String,
    pub prefix: String,
    pub suffix: String,
    pub start: Option<u64>,
    pub digits: usize,
    pub case_mode: String,
    pub extension: Option<String>,
}

/// Ordered advanced rename rule, serialized with the original PascalCase field names.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct RenameRule {
    pub action: String,
    pub target: String,
    pub text: String,
    pub find: String,
    pub replace: String,
    pub position: String,
    pub case_mode: String,
    pub delete_from_mode: String,
    pub delete_to_mode: String,
    pub delete_direction: String,
    pub delete_to_delimiter: String,
    pub index: i64,
    pub count: i64,
    pub start: i64,
    pub step: i64,
    pub digits: usize,
}
impl Default for RenameRule {
    fn default() -> Self {
        Self {
            action: "Replace".into(),
            target: "Name".into(),
            text: String::new(),
            find: String::new(),
            replace: String::new(),
            position: "All".into(),
            case_mode: "Upper".into(),
            delete_from_mode: "Position".into(),
            delete_to_mode: "Count".into(),
            delete_direction: "LeftToRight".into(),
            delete_to_delimiter: String::new(),
            index: 1,
            count: 0,
            start: 1,
            step: 1,
            digits: 1,
        }
    }
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct RenamePreset {
    pub name: String,
    pub rules: Vec<RenameRule>,
}

/// Preview one item. Positions count Unicode characters, preserving valid supplementary characters.
pub fn rename_with_rules(
    file_name: &str,
    rules: &[RenameRule],
    item_index: usize,
    is_directory: bool,
) -> Result<String, String> {
    let (name, extension) = if is_directory {
        (file_name, "")
    } else {
        split_name(file_name)
    };
    let mut name = name.to_owned();
    let mut extension = extension.to_owned();
    for rule in rules {
        if rule.action == "Extension" {
            extension = rule.text.trim().trim_start_matches('.').into();
            continue;
        }
        let target = if rule.target == "Extension" {
            &mut extension
        } else {
            &mut name
        };
        *target = apply_rename_rule(target, rule, item_index)?;
        if rule.target == "Extension" {
            *target = target.trim_start_matches('.').into();
        }
    }
    let output = if extension.is_empty() {
        name
    } else {
        format!("{name}.{extension}")
    };
    validate_name(&output)?;
    Ok(output)
}
/// Builds previews using filesystem directory information. `apply_renames` commits this result.
pub fn rename_rules_preview(
    paths: &[PathBuf],
    rules: &[RenameRule],
) -> Result<Vec<String>, String> {
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    for (index, path) in paths.iter().enumerate() {
        let name = path
            .file_name()
            .ok_or("Cannot rename a filesystem root")?
            .to_str()
            .ok_or("File name is not valid Unicode")?;
        let target = rename_with_rules(name, rules, index, path.is_dir())?;
        let parent =
            fs::canonicalize(path.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
        if !seen.insert(path_key(&parent.join(&target))) {
            return Err(format!("Duplicate result name: {target}"));
        }
        output.push(target);
    }
    Ok(output)
}
fn character_offset(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}
fn apply_rename_rule(value: &str, rule: &RenameRule, item_index: usize) -> Result<String, String> {
    let insert = |text: &str| {
        let index = match rule.position.as_str() {
            "Suffix" | "Last" => value.len(),
            "Position" => character_offset(value, rule.index.saturating_sub(1).max(0) as usize),
            _ => 0,
        };
        format!("{}{text}{}", &value[..index], &value[index..])
    };
    match rule.action.as_str() {
        "Insert" => Ok(insert(&rule.text)),
        "Number" => {
            let number = rule
                .step
                .checked_mul(i64::try_from(item_index).map_err(|_| "Rename sequence overflow")?)
                .and_then(|v| rule.start.checked_add(v))
                .ok_or("Rename sequence overflow")?;
            let text = format!(
                "{}{:0width$}",
                if number < 0 { "-" } else { "" },
                number.unsigned_abs(),
                width = rule.digits.clamp(1, 250)
            );
            Ok(insert(&text))
        }
        "Replace" => {
            if rule.find.is_empty() {
                return Ok(value.into());
            }
            let regex = regex::RegexBuilder::new(&regex::escape(&rule.find))
                .case_insensitive(true)
                .build()
                .map_err(|e| e.to_string())?;
            if matches!(rule.position.as_str(), "First" | "Last") {
                let found = if rule.position == "First" {
                    regex.find(value)
                } else {
                    regex.find_iter(value).last()
                };
                Ok(if let Some(m) = found {
                    format!(
                        "{}{}{}",
                        &value[..m.start()],
                        rule.replace,
                        &value[m.end()..]
                    )
                } else {
                    value.into()
                })
            } else {
                Ok(regex
                    .replace_all(value, regex::NoExpand(&rule.replace))
                    .into_owned())
            }
        }
        "Case" => Ok(match rule.case_mode.as_str() {
            "Upper" => value.to_uppercase(),
            "Lower" => value.to_lowercase(),
            "First" | "FirstLetter" => {
                let mut chars = value.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase())
                    .unwrap_or_default()
            }
            _ => value
                .split(' ')
                .map(|word| {
                    let mut chars = word.chars();
                    chars
                        .next()
                        .map(|c| {
                            c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                        })
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" "),
        }),
        "Delete" => {
            let len = value.chars().count() as i64;
            if len == 0 {
                return Ok(value.into());
            }
            let forward = rule.delete_direction != "RightToLeft";
            let anchor = match rule.delete_from_mode.as_str() {
                "FromStart" => 0,
                "FromEnd" => len - 1,
                "Delimiter" => {
                    if rule.find.is_empty() {
                        return Ok(value.into());
                    }
                    let Some(index) = value.find(&rule.find) else {
                        return Ok(value.into());
                    };
                    let index = value[..index].chars().count() as i64;
                    if forward {
                        index + rule.find.chars().count() as i64
                    } else {
                        index - 1
                    }
                }
                _ => rule.index.saturating_sub(1),
            };
            if anchor < 0 {
                return Ok(value.into());
            }
            let other =
                if rule.delete_to_mode == "Delimiter" && !rule.delete_to_delimiter.is_empty() {
                    let Some(index) = value.find(&rule.delete_to_delimiter) else {
                        return Ok(value.into());
                    };
                    let index = value[..index].chars().count() as i64;
                    if forward { index - 1 } else { index }
                } else {
                    if rule.count <= 0 {
                        return Ok(value.into());
                    }
                    if forward {
                        anchor.saturating_add(rule.count - 1)
                    } else {
                        anchor.saturating_sub(rule.count - 1)
                    }
                };
            let start = anchor.min(other).max(0);
            let end = anchor.max(other).min(len - 1);
            if end < start {
                return Ok(value.into());
            }
            Ok(format!(
                "{}{}",
                &value[..character_offset(value, start as usize)],
                &value[character_offset(value, (end + 1) as usize)..]
            ))
        }
        _ => Ok(value.into()),
    }
}
pub fn validate_name(name: &str) -> Result<(), String> {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|p| {
            stem.strip_prefix(p).is_some_and(|s| {
                matches!(
                    s,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    if name.is_empty()
        || name.ends_with([' ', '.'])
        || name.chars().any(|c| c < ' ' || "\\/:*?\"<>|".contains(c))
        || name.encode_utf16().count() > 255
        || reserved
    {
        return Err(format!("Invalid file name: {name}"));
    }
    Ok(())
}
/// Applies replace/case/prefix/sequence/suffix to the stem and preserves the extension by default.
pub fn rename_preview(names: &[String], spec: &RenameSpec) -> Result<Vec<String>, String> {
    let mut output = Vec::with_capacity(names.len());
    let mut unique = HashSet::new();
    let find = if spec.find.is_empty() {
        None
    } else {
        Some(
            regex::RegexBuilder::new(&regex::escape(&spec.find))
                .case_insensitive(true)
                .build()
                .map_err(|e| e.to_string())?,
        )
    };
    for (index, name) in names.iter().enumerate() {
        let (stem, extension) = split_name(name);
        let stem = find
            .as_ref()
            .map(|r| {
                r.replace_all(stem, regex::NoExpand(&spec.replace))
                    .into_owned()
            })
            .unwrap_or_else(|| stem.to_owned());
        let stem = match spec.case_mode.to_ascii_lowercase().as_str() {
            "upper" => stem.to_uppercase(),
            "lower" => stem.to_lowercase(),
            "first" | "firstletter" => {
                let mut chars = stem.chars();
                chars
                    .next()
                    .map(|c| c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase())
                    .unwrap_or_default()
            }
            "title" => stem
                .split(' ')
                .map(|word| {
                    let mut c = word.chars();
                    c.next()
                        .map(|first| {
                            first.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()
                        })
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" "),
            _ => stem,
        };
        let number = match spec.start {
            Some(start) => format!(
                "{:0width$}",
                start
                    .checked_add(index as u64)
                    .ok_or("Rename sequence overflow")?,
                width = spec.digits.clamp(1, 250)
            ),
            None => String::new(),
        };
        let extension = spec
            .extension
            .as_deref()
            .map(|s| s.trim().trim_start_matches('.'))
            .unwrap_or(extension);
        let mut target = format!("{}{stem}{number}{}", spec.prefix, spec.suffix);
        if !extension.is_empty() {
            target.push('.');
            target.push_str(extension);
        }
        validate_name(&target)?;
        if !unique.insert(target.to_lowercase()) {
            return Err(format!("Duplicate result name: {target}"));
        }
        output.push(target);
    }
    Ok(output)
}

#[cfg(windows)]
fn move_no_replace(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<_> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileW(source.as_ptr(), destination.as_ptr())
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
#[cfg(not(windows))]
fn move_no_replace(source: &Path, destination: &Path) -> Result<(), String> {
    if fs::symlink_metadata(destination).is_ok() {
        return Err(format!("Target exists: {}", destination.display()));
    }
    fs::rename(source, destination).map_err(|e| e.to_string())
}
fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Preflights the entire batch, stages same-directory temporary names, then commits; supports cycles.
/// Rollback errors include recoverable temporary paths. Existing targets are never overwritten.
pub fn apply_renames(paths: &[PathBuf], names: &[String]) -> Result<(), String> {
    apply_renames_inner(paths, names, None)
}
fn apply_renames_inner(
    paths: &[PathBuf],
    names: &[String],
    fail_after: Option<usize>,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if paths.len() != names.len() {
        return Err("Source and target counts differ".into());
    }
    let mut plan = Vec::new();
    let mut sources = HashSet::new();
    let mut targets = HashSet::new();
    for (path, name) in paths.iter().zip(names) {
        validate_name(name)?;
        let parent = fs::canonicalize(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .map_err(|e| e.to_string())?;
        let file_name = path.file_name().ok_or("Cannot rename a filesystem root")?;
        let source = parent.join(file_name);
        fs::symlink_metadata(&source).map_err(|e| format!("{}: {e}", source.display()))?;
        let target = parent.join(name);
        if !sources.insert(path_key(&source)) {
            return Err(format!("Duplicate source: {}", source.display()));
        }
        if !targets.insert(path_key(&target)) {
            return Err(format!("Duplicate target: {}", target.display()));
        }
        plan.push((source, target));
    }
    for (source, target) in &plan {
        if plan
            .iter()
            .any(|(other, _)| other != source && source.starts_with(other))
        {
            return Err("A rename batch cannot contain an ancestor and its descendant".into());
        }
        if fs::symlink_metadata(target).is_ok() && !sources.contains(&path_key(target)) {
            return Err(format!("Target exists: {}", target.display()));
        }
    }
    plan.retain(|(source, target)| source != target);
    let mut descriptions = BTreeMap::<PathBuf, Vec<(String, String)>>::new();
    for (source, target) in &plan {
        let old = source.file_name().unwrap().to_string_lossy().into_owned();
        let new = target.file_name().unwrap().to_string_lossy().into_owned();
        if !is_description_file(&old) && !is_description_file(&new) {
            descriptions
                .entry(source.parent().unwrap().to_owned())
                .or_default()
                .push((old, new));
        }
    }
    let mut backups = BTreeMap::new();
    for parent in descriptions.keys() {
        let path = parent.join("descript.dat");
        let backup = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        backups.insert(path, backup);
    }
    let mut staged = Vec::<(PathBuf, PathBuf, PathBuf, bool)>::new();
    let mut touched = Vec::new();
    let outcome = (|| {
        for (source, target) in &plan {
            let temporary = loop {
                let p = source.parent().unwrap().join(format!(
                    ".ndr-rename-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                if fs::symlink_metadata(&p).is_err() {
                    break p;
                }
            };
            move_no_replace(source, &temporary)?;
            staged.push((source.clone(), target.clone(), temporary, false));
        }
        for (index, (_, target, temporary, committed)) in staged.iter_mut().enumerate() {
            if fail_after == Some(index) {
                return Err("Injected commit failure".into());
            }
            move_no_replace(temporary, target)?;
            *committed = true;
        }
        for (parent, renames) in &descriptions {
            touched.push(parent.join("descript.dat"));
            rename_descriptions(parent, renames, true)?;
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = outcome {
        let mut errors = vec![error];
        for path in touched.into_iter().rev() {
            let restore = match &backups[&path] {
                Some(bytes) => write_description_bytes(&path, bytes),
                None => remove_description_file(&path),
            };
            if let Err(e) = restore {
                errors.push(format!("Description rollback {}: {e}", path.display()));
            }
        }
        // Vacate all committed targets before restoring sources, otherwise cycles collide.
        for (_, target, temporary, committed) in staged.iter_mut().rev() {
            if *committed {
                match move_no_replace(target, temporary) {
                    Ok(()) => *committed = false,
                    Err(e) => errors.push(format!(
                        "Rollback {} -> {}: {e}",
                        target.display(),
                        temporary.display()
                    )),
                }
            }
        }
        for (source, _, temporary, committed) in staged.iter().rev() {
            if !committed && let Err(e) = move_no_replace(temporary, source) {
                errors.push(format!(
                    "Rollback {} -> {}: {e}",
                    temporary.display(),
                    source.display()
                ));
            }
        }
        return Err(errors.join("\n"));
    }
    Ok(())
}

pub fn is_description_file(name: &str) -> bool {
    name.eq_ignore_ascii_case("descript.dat") || name.eq_ignore_ascii_case("descript.ion")
}
fn description_data(directory: &Path) -> Result<Vec<(String, String)>, String> {
    let path = directory.join("descript.dat");
    let bytes = match fs::read(&path) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    Ok(decode(&bytes, true)
        .lines()
        .filter_map(|line| {
            line.split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .map(|(n, v)| (n.into(), v.into()))
        })
        .collect())
}
fn description_legacy(directory: &Path) -> Result<Vec<(String, String)>, String> {
    let bytes = match fs::read(directory.join("descript.ion")) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    Ok(decode(&bytes, false)
        .lines()
        .filter_map(|line| {
            let (name, value) = if let Some(quoted) = line.strip_prefix('"') {
                quoted.split_once('"').unwrap_or((quoted, ""))
            } else {
                let index = line.find([' ', '\t']).unwrap_or(line.len());
                (&line[..index], &line[index..])
            };
            if name.is_empty() {
                None
            } else {
                Some((name.into(), value.trim().into()))
            }
        })
        .collect())
}
/// Reads descript.dat first, optionally merges descript.ion; empty DAT values mask legacy entries.
/// Keys retain source spelling. Use case-insensitive lookup for Windows filenames.
pub fn read_descriptions(
    directory: &Path,
    include_legacy: bool,
) -> Result<BTreeMap<String, String>, String> {
    let mut entries = description_data(directory)?;
    if include_legacy {
        entries.extend(description_legacy(directory)?);
    }
    let mut known = HashSet::new();
    let mut result = BTreeMap::new();
    for (name, value) in entries {
        if known.insert(name.to_lowercase()) && !value.is_empty() {
            result.insert(name, value);
        }
    }
    Ok(result)
}
fn normalized_description(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value.replace(['\r', '\n'], " "))
    }
}
pub fn set_description(
    directory: &Path,
    name: &str,
    value: &str,
    include_legacy: bool,
) -> Result<(), String> {
    validate_name(name)?;
    if name.contains('=') {
        return Err("Description names cannot contain '='".into());
    }
    let mut entries = description_data(directory)?;
    entries.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
    if let Some(value) = normalized_description(value) {
        entries.push((name.into(), value));
    } else if include_legacy
        && description_legacy(directory)?
            .iter()
            .any(|(n, v)| n.eq_ignore_ascii_case(name) && !v.is_empty())
    {
        entries.push((name.into(), String::new()));
    }
    write_descriptions(directory, &entries)
}
pub fn rename_descriptions(
    directory: &Path,
    renames: &[(String, String)],
    include_legacy: bool,
) -> Result<(), String> {
    let map: HashMap<_, _> = renames
        .iter()
        .filter(|(old, new)| !is_description_file(old) && !is_description_file(new))
        .map(|(old, new)| (old.to_lowercase(), new))
        .collect();
    let mut entries = description_data(directory)?;
    let mut known: HashSet<_> = entries.iter().map(|(n, _)| n.to_lowercase()).collect();
    if include_legacy {
        for (name, value) in description_legacy(directory)? {
            if map.contains_key(&name.to_lowercase())
                && !value.is_empty()
                && known.insert(name.to_lowercase())
            {
                entries.push((name, value));
            }
        }
    }
    if !entries
        .iter()
        .any(|(n, _)| map.contains_key(&n.to_lowercase()))
    {
        return Ok(());
    }
    let targets: HashSet<_> = map.values().map(|s| s.to_lowercase()).collect();
    let mut written = HashSet::new();
    let mut output = Vec::new();
    for (name, value) in entries {
        if let Some(new) = map.get(&name.to_lowercase()) {
            if written.insert(new.to_lowercase()) {
                if new.contains('=') {
                    return Err("Description names cannot contain '='".into());
                }
                output.push(((*new).clone(), value));
            }
        } else if !targets.contains(&name.to_lowercase()) {
            output.push((name, value));
        }
    }
    write_descriptions(directory, &output)
}
pub fn remove_descriptions(directory: &Path, names: &[String]) -> Result<(), String> {
    let mut entries = description_data(directory)?;
    let count = entries.len();
    entries.retain(|(name, _)| !names.iter().any(|n| n.eq_ignore_ascii_case(name)));
    if entries.len() != count {
        write_descriptions(directory, &entries)?;
    }
    Ok(())
}
pub fn clear_descriptions(directory: &Path) -> Result<(), String> {
    remove_description_file(&directory.join("descript.dat"))
}

/// Metadata for one file operation that already committed. No files are copied by this helper.
#[derive(Clone, Debug)]
pub struct DescriptionTransfer {
    pub source_directory: PathBuf,
    pub source_name: String,
    pub destination_directory: PathBuf,
    pub destination_name: String,
    pub description: Option<String>,
    pub move_entry: bool,
}
/// Applies successful copy/move comment changes, grouping writes by directory; returns all metadata errors.
pub fn apply_description_transfers(transfers: &[DescriptionTransfer]) -> Vec<String> {
    #[derive(Default)]
    struct Changes {
        directory: PathBuf,
        removed: HashSet<String>,
        updates: BTreeMap<String, (String, Option<String>)>,
    }
    let mut directories = BTreeMap::<String, Changes>::new();
    for transfer in transfers {
        if !is_description_file(&transfer.source_name) && transfer.move_entry {
            let changes = directories
                .entry(path_key(&transfer.source_directory))
                .or_insert_with(|| Changes {
                    directory: transfer.source_directory.clone(),
                    ..Default::default()
                });
            changes.removed.insert(transfer.source_name.to_lowercase());
        }
        if !is_description_file(&transfer.destination_name) {
            let changes = directories
                .entry(path_key(&transfer.destination_directory))
                .or_insert_with(|| Changes {
                    directory: transfer.destination_directory.clone(),
                    ..Default::default()
                });
            changes.updates.insert(
                transfer.destination_name.to_lowercase(),
                (
                    transfer.destination_name.clone(),
                    transfer
                        .description
                        .as_deref()
                        .and_then(normalized_description),
                ),
            );
        }
    }
    let mut errors = Vec::new();
    for changes in directories.into_values() {
        let result = (|| {
            let entries = description_data(&changes.directory)?;
            let original = entries.len();
            let mut entries: Vec<_> = entries
                .into_iter()
                .filter(|(name, _)| {
                    !changes.removed.contains(&name.to_lowercase())
                        && !changes.updates.contains_key(&name.to_lowercase())
                })
                .collect();
            let mut changed = entries.len() != original;
            for (name, value) in changes.updates.into_values() {
                if let Some(value) = value {
                    validate_name(&name)?;
                    if name.contains('=') {
                        return Err("Description names cannot contain '='".into());
                    }
                    entries.push((name, value));
                    changed = true;
                }
            }
            if changed {
                write_descriptions(&changes.directory, &entries)?;
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            errors.push(format!(
                "{}: {error}",
                changes.directory.join("descript.dat").display()
            ));
        }
    }
    errors
}
/// Imports comments for listed names into DAT while retaining unlisted metadata and legacy source bytes.
pub fn migrate_legacy_descriptions(
    directory: &Path,
    listed_names: &[String],
    include_legacy: bool,
) -> Result<(), String> {
    if !include_legacy {
        return Ok(());
    }
    let mut entries = description_data(directory)?;
    let mut known: HashSet<_> = entries.iter().map(|(n, _)| n.to_lowercase()).collect();
    let mut legacy = BTreeMap::new();
    for (name, value) in description_legacy(directory)? {
        if !value.is_empty() {
            legacy.entry(name.to_lowercase()).or_insert(value);
        }
    }
    let mut changed = false;
    for name in listed_names {
        if known.contains(&name.to_lowercase()) {
            continue;
        }
        if let Some(value) = legacy.get(&name.to_lowercase()) {
            validate_name(name)?;
            if name.contains('=') {
                return Err("Description names cannot contain '='".into());
            }
            entries.push((name.clone(), value.clone()));
            known.insert(name.to_lowercase());
            changed = true;
        }
    }
    if changed {
        write_descriptions(directory, &entries)?;
    }
    Ok(())
}
fn write_descriptions(directory: &Path, entries: &[(String, String)]) -> Result<(), String> {
    let path = directory.join("descript.dat");
    if entries.is_empty() {
        return remove_description_file(&path);
    }
    let mut text = String::new();
    for (name, value) in entries {
        text.push_str(name);
        text.push('=');
        text.push_str(value);
        text.push_str("\r\n");
    }
    let mut bytes = vec![255, 254];
    bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    write_description_bytes(&path, &bytes)
}
#[cfg(windows)]
fn description_attributes(path: &Path, attributes: u32) {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(path.as_ptr(), attributes);
    }
}
#[cfg(not(windows))]
fn description_attributes(_path: &Path, _attributes: u32) {}
fn write_description_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    description_attributes(path, 0x20);
    let result = crate::settings::atomic_write(path, bytes);
    description_attributes(path, 0x22);
    result
}
fn remove_description_file(path: &Path) -> Result<(), String> {
    description_attributes(path, 0x20);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

const KEY_NAMES: &[&str] = &[
    "Escape",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "Backspace",
    "Enter",
    "Insert",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Spacebar",
    "Left",
    "Right",
    "Up",
    "Down",
    "Num /",
    "Num *",
    "Num -",
    "Num +",
    "Num .",
    "Num 0",
    "Num 1",
    "Num 2",
    "Num 3",
    "Num 4",
    "Num 5",
    "Num 6",
    "Num 7",
    "Num 8",
    "Num 9",
];
const KEY_CODES: &[u16] = &[
    0x1b, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x7b, 0x08, 0x0d, 0x2d,
    0x2e, 0x24, 0x23, 0x21, 0x22, 0x20, 0x25, 0x27, 0x26, 0x28, 0x6f, 0x6a, 0x6d, 0x6b, 0x6e, 0x60,
    0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
];
const PUNCTUATION: &str = "`-=[]\\;',./";
const PUNCTUATION_CODES: &[u16] = &[
    0xc0, 0xbd, 0xbb, 0xdb, 0xdd, 0xdc, 0xba, 0xde, 0xbc, 0xbe, 0xbf,
];
const SHIFTED: &str = "~!@#$%^&*()_+{}|:\"<>?";
const SHIFTED_CODES: &[u16] = &[
    0xc0, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x30, 0xbd, 0xbb, 0xdb, 0xdd, 0xdc,
    0xba, 0xde, 0xbc, 0xbe, 0xbf,
];

/// Original Delphi TShortCut: low byte VK, 0x2000 Shift, 0x4000 Ctrl, 0x8000 Alt.
/// Original FUN_0047a158 (text → shortcut): splits at '+' (not the last character), Ctrl/Alt/Shift set the state, one character is looked up
/// in the shifted table (adds Shift), then the punctuation table, else CharUpperA; Bksp/Esc/Ins/Del become the full names before the
/// 0x29-entry key-name table. The reverse is FUN_0047a458.
pub fn text_to_shortcut(text: &str) -> u16 {
    let mut key = 0;
    let mut state = 0;
    let mut start = 0;
    let mut tokens = Vec::new();
    for (index, ch) in text.char_indices() {
        if ch == '+' && index + 1 != text.len() {
            tokens.push(&text[start..index]);
            start = index + 1;
        }
    }
    tokens.push(&text[start..]);
    for token in tokens {
        match token {
            "Ctrl" => {
                state |= 0x4000;
                continue;
            }
            "Alt" => {
                state |= 0x8000;
                continue;
            }
            "Shift" => {
                state |= 0x2000;
                continue;
            }
            _ => {}
        }
        if token.chars().count() == 1 {
            let ch = token.chars().next().unwrap();
            if let Some(index) = SHIFTED.chars().position(|c| c == ch) {
                state |= 0x2000;
                key = SHIFTED_CODES[index];
            } else if let Some(index) = PUNCTUATION.chars().position(|c| c == ch) {
                key = PUNCTUATION_CODES[index];
            } else {
                key = if ch.is_ascii() {
                    ch.to_ascii_uppercase() as u16
                } else {
                    0x100
                };
            }
        } else {
            let name = match token.to_ascii_lowercase().as_str() {
                "bksp" => "Backspace",
                "esc" => "Escape",
                "ins" => "Insert",
                "del" => "Delete",
                _ => token,
            };
            if let Some(index) = KEY_NAMES.iter().position(|n| n.eq_ignore_ascii_case(name)) {
                key = KEY_CODES[index];
            }
        }
    }
    if key > 255 { 0 } else { key | state }
}
/// Original FUN_0047a458 (shortcut → text): "Ctrl+", "Shift+", "Alt+" in that order, then the key name table, the punctuation table, or
/// the character itself for '0'..'Z'.
pub fn shortcut_to_text(shortcut: u16) -> String {
    let mut result = String::new();
    for (mask, text) in [(0x4000, "Ctrl+"), (0x2000, "Shift+"), (0x8000, "Alt+")] {
        if shortcut & mask != 0 {
            result.push_str(text);
        }
    }
    let key = shortcut & 255;
    if let Some(index) = KEY_CODES.iter().position(|&k| k == key) {
        result.push_str(KEY_NAMES[index]);
    } else if let Some(index) = PUNCTUATION_CODES.iter().position(|&k| k == key) {
        result.push(PUNCTUATION.as_bytes()[index] as char);
    } else if (0x30..=0x5a).contains(&key) {
        result.push(char::from_u32(key as u32).unwrap());
    }
    result
}
pub fn read_shortcut_file(path: &Path) -> Result<String, String> {
    Ok(fs::read(path)
        .map_err(|e| e.to_string())?
        .iter()
        .map(|b| (b & 127) as char)
        .collect())
}
fn shortcut_lines(text: &str) -> Vec<String> {
    let text = text.split('\0').next().unwrap_or("");
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<_> = text.split('\n').map(str::to_owned).collect();
    if text.ends_with('\n') || text.is_empty() {
        lines.pop();
    }
    lines
}
fn shortcut_name(line: &str) -> &str {
    line.split_once('=').map(|(n, _)| n).unwrap_or("")
}
fn shortcut_value(line: &str) -> &str {
    line.split_once('=').map(|(_, v)| v).unwrap_or("")
}

#[derive(Clone, Debug)]
pub struct ShortcutMap {
    pub lines: Vec<String>,
    pub menu_shortcuts: BTreeMap<String, u16>,
}
impl ShortcutMap {
    /// Set one command binding while preserving unrelated lines and duplicate internal bindings.
    pub fn set_binding(&mut self, command: &str, binding: &str) -> Result<(), String> {
        if command.is_empty()
            || command.contains(['=', '\r', '\n', '\0'])
            || binding.contains(['\r', '\n', '\0'])
        {
            return Err("Invalid shortcut entry".into());
        }
        if !MENU_SHORTCUTS
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(command))
            && !INTERNAL_COMMANDS
                .iter()
                .any(|name| name.eq_ignore_ascii_case(command))
        {
            return Err(format!("Unknown shortcut command: {command}"));
        }
        let replacement = format!("{command}={binding}");
        // The first line controls internal dispatch, the last line controls a menu component.
        let menu = MENU_SHORTCUTS
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(command));
        let index = if menu {
            self.lines
                .iter()
                .rposition(|line| shortcut_name(line).eq_ignore_ascii_case(command))
        } else {
            self.lines
                .iter()
                .position(|line| shortcut_name(line).eq_ignore_ascii_case(command))
        };
        if let Some(index) = index {
            self.lines[index] = replacement;
        } else {
            self.lines.push(replacement);
        }
        let text = self.to_text();
        *self = Self::parse(Some(&text));
        Ok(())
    }
    pub fn to_text(&self) -> String {
        format!(
            "NEXUSFILESHORTCUT1.0\r\n{}{}",
            self.lines.join("\r\n"),
            if self.lines.is_empty() { "" } else { "\r\n" }
        )
    }
    /// A header replaces defaults; a headerless file overrides each default name using its first occurrence.
    pub fn parse(text: Option<&str>) -> Self {
        let mut lines = shortcut_lines(DEFAULT_SHORTCUT_TEXT);
        lines.remove(0);
        if let Some(text) = text {
            let supplied = shortcut_lines(text);
            if supplied
                .first()
                .is_some_and(|line| line == "NEXUSFILESHORTCUT1.0")
            {
                lines = supplied.into_iter().skip(1).collect();
            } else {
                for line in &mut lines {
                    if let Some(found) = supplied.iter().find(|entry| {
                        entry.contains('=')
                            && shortcut_name(entry).eq_ignore_ascii_case(shortcut_name(line))
                    }) {
                        *line = found.clone();
                    }
                }
            }
        }
        let mut menu_shortcuts: BTreeMap<_, _> = MENU_SHORTCUTS
            .iter()
            .map(|(n, k)| ((*n).to_owned(), *k))
            .collect();
        for line in &lines {
            if let Some((_, code)) = menu_shortcuts
                .iter_mut()
                .find(|(n, _)| n.eq_ignore_ascii_case(shortcut_name(line)))
            {
                *code = text_to_shortcut(shortcut_value(line));
            }
        }
        Self {
            lines,
            menu_shortcuts,
        }
    }
    /// Returns an original miMenuName or internal command name. Menu order takes precedence.
    pub fn resolve(&self, shortcut: u16) -> Option<String> {
        if shortcut == 0 {
            return None;
        }
        for (name, _) in MENU_SHORTCUTS {
            if self.menu_shortcuts.get(*name) == Some(&shortcut) {
                return Some((*name).into());
            }
        }
        let text = shortcut_to_text(shortcut);
        let line = self
            .lines
            .iter()
            .find(|line| shortcut_value(line) == text)?;
        let name = shortcut_name(line);
        INTERNAL_COMMANDS
            .iter()
            .skip(1)
            .find(|command| command.eq_ignore_ascii_case(name))
            .map(|s| (*s).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Writes the settings imported from the original default/nexusfile.ini to the NDR_DEFAULT_INI_DUMP file (for default comparisons; run by hand only).
    #[test]
    #[ignore]
    fn dump_original_default_ini() {
        let (Some(ini), Some(out)) = (std::env::var_os("NDR_DEFAULT_INI"), std::env::var_os("NDR_DEFAULT_INI_DUMP")) else { return };
        let settings = import_ini(Path::new(&ini)).unwrap();
        fs::write(out, serde_json::to_string_pretty(&settings.values).unwrap()).unwrap();
    }
    fn sandbox(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ndr-compat-{}-{label}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn macros_quote_unicode_and_respect_original_tokens() {
        let pane = MacroPane {
            path: PathBuf::from(r"C:\한 글"),
            focused: Some("문서.txt".into()),
            selected: vec!["a b.txt".into(), "문서.txt".into()],
        };
        assert_eq!(
            expand_command(
                "!..! | $(path) | !: | !.! | !, | ,! | %TEMP%",
                &pane,
                &pane,
                &pane
            ),
            "\"C:\\한 글\\a b.txt\" \"C:\\한 글\\문서.txt\" | C:\\한 글\\ | C:\\ | 문서.txt | 문서 | txt | %TEMP%"
        );
    }
    #[test]
    fn language_reads_original_korean_and_english_resources() {
        let directory = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Resources/Languages"));
        assert_eq!(
            Language::load(directory, "en").text("RES.RENAME", "missing"),
            "Rename"
        );
        assert_ne!(
            Language::load(directory, "ko").text("RES.RENAME", "missing"),
            "missing"
        );
        assert_ne!(
            Language::load(directory, "en").text("Menu.miOpen", "missing"),
            "missing"
        );
    }
    #[test]
    fn language_pair_translation_uses_cached_original_keys_and_custom_fallbacks() {
        let directory = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Resources/Languages"));
        let english = Language::load(directory, "en");
        let korean = Language::load(directory, "ko");
        assert_eq!(
            english.translate_pair("이름 바꾸기", "Rename"),
            english.text("RES.RENAME", "missing")
        );
        assert_eq!(
            korean.translate_pair("이름 바꾸기", "Rename"),
            korean.text("RES.RENAME", "missing")
        );
        assert_eq!(
            korean.translate_pair("사용자 고유 문구", "Custom phrase 123456"),
            "사용자 고유 문구"
        );
        assert_eq!(
            english.translate_pair("사용자 고유 문구", "Custom phrase 123456"),
            "Custom phrase 123456"
        );
        let codes = available_codes(directory);
        assert!(codes.contains(&"ko".to_owned()));
        assert!(codes.contains(&"en".to_owned()));
        let root = sandbox("cached-lang");
        fs::write(root.join("en.lang"), "[RES]\nRENAME=Rename\n").unwrap();
        fs::write(root.join("ko.lang"), "[RES]\nRENAME=이름 바꾸기\n").unwrap();
        let cached = Language::load(&root, "ko");
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            cached.translate_pair("이름 바꾸기", "Rename"),
            "이름 바꾸기"
        );
    }
    #[test]
    fn ini_maps_real_keys_and_preserves_unmapped_values() {
        let root = sandbox("ini");
        let file = root.join("nexusfile.ini");
        fs::write(&file, "[Window]\ndualview=0\nlastfocus=1\nskin=black\n[View1]\nlastpath1=C:\\한글\nshowhidden1=1\nsort1=3\n[Operation]\nuseion=0\n[Display]\nlistfont=\"Tahoma\",10,3\n[Tree]\ntreefont=Gulim,9\n[Future]\nunknown=keep\n").unwrap();
        let settings = import_ini(&file).unwrap();
        // Original "\"name\",size,style" (FUN_0044cd58 wsprintfW): style = TFontStyles byte. Size in points -> ListFontSize pixels (10 x 96 / 72 = 13).
        assert_eq!((settings.text("ListFont", ""), settings.values["ListFontSize"].as_f64(), settings.values["ListFontStyle"].as_i64()), ("Tahoma".to_owned(), Some(13.0), Some(3)));
        assert_eq!(settings.values["FolderTreeFontSize"].as_f64(), Some(9.0), "the folder tree size stays in points");
        assert_eq!((settings.text("FolderTreeFont", ""), settings.values["FolderTreeFontStyle"].as_i64()), ("Gulim".to_owned(), Some(0)));
        assert!(!settings.flag("DualPaneMode", true));
        assert!(settings.flag("StartWithRightPaneActive", false));
        assert_eq!(settings.text("LeftPath", ""), "C:\\한글");
        assert_eq!(settings.text("LeftSortMode", ""), "Size");
        assert!(!settings.flag("UseDescriptionFile", true));
        assert_eq!(settings.values["LegacyIniValues"]["Future.unknown"], "keep");
        assert_eq!(settings.values["ThumbnailSize"].as_u64(), Some(120));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn ini_keeps_literal_windows_backslashes_and_imports_sidecars() {
        let root = sandbox("sidecars");
        let ini = root.join("nexusfile.ini");
        fs::write(&ini, "[View1]\nlastpath1=C:\\new\\next\n").unwrap();
        let unicode = |text: &str| {
            let mut bytes = vec![255, 254];
            bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
            bytes
        };
        fs::write(
            root.join("favorits.save"),
            unicode("NEXUSFILEFAV.VER2.1\r\n00한글\tC:\\new\\next\r\n"),
        )
        .unwrap();
        fs::write(
            root.join("tabs.save"),
            unicode(
                "NEXUSFILETABS.VER1\r\n[Tab0]\r\nOne\tC:\\new\\next\r\n[Tab1]\r\nTwo\tD:\\docs\r\n",
            ),
        )
        .unwrap();
        fs::write(
            root.join("history_cmd.save"),
            unicode(" cmd one \r\n한글 명령\r\n"),
        )
        .unwrap();
        fs::write(
            root.join("history_drive.save"),
            unicode("C=C:\\first\r\nc=C:\\later\r\n"),
        )
        .unwrap();
        fs::write(root.join("ftpsite.ini"),unicode("[테스트]\r\nhost=example.invalid\r\nhost=ignored\r\nport=22\r\nuser=test\r\npass=\r\nservertype=SFTP\r\npassive=0\r\n")).unwrap();
        let settings = import_ini(&ini).unwrap();
        assert_eq!(settings.text("LeftPath", ""), r"C:\new\next");
        assert_eq!(settings.strings("Favorites")[0], r"C:\new\next");
        assert_eq!(settings.strings("FavoriteNames")[0], "한글");
        assert_eq!(settings.strings("LeftTabs")[0], r"C:\new\next");
        assert_eq!(
            settings.strings("CommandHistory"),
            [" cmd one ", "한글 명령"]
        );
        assert_eq!(settings.values["LastFoldersByDrive"]["C"], r"C:\first");
        assert_eq!(settings.values["RemoteSites"][0]["Protocol"], "SFTP");
        assert_eq!(settings.values["RemoteSites"][0]["Host"], "example.invalid");
        assert_eq!(settings.values["RemoteSites"][0]["EncryptedPassword"], "");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rename_cycles_work_and_collisions_never_modify_sources() {
        let root = sandbox("rename");
        let a = root.join("a.txt");
        let b = root.join("b.txt");
        fs::write(&a, b"AAA").unwrap();
        fs::write(&b, b"BBB").unwrap();
        assert!(apply_renames(std::slice::from_ref(&a), &["b.txt".into()]).is_err());
        assert_eq!(fs::read(&a).unwrap(), b"AAA");
        assert_eq!(fs::read(&b).unwrap(), b"BBB");
        assert!(apply_renames(std::slice::from_ref(&a), &["../outside".into()]).is_err());
        apply_renames(&[a.clone(), b.clone()], &["b.txt".into(), "a.txt".into()]).unwrap();
        assert_eq!(fs::read(a).unwrap(), b"BBB");
        assert_eq!(fs::read(b).unwrap(), b"AAA");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rename_partial_commit_rolls_back_cycles_and_descriptions() {
        let root = sandbox("rollback");
        let paths: Vec<_> = ["one", "two", "three"]
            .iter()
            .map(|n| root.join(n))
            .collect();
        for (index, path) in paths.iter().enumerate() {
            fs::write(path, [index as u8]).unwrap();
        }
        set_description(&root, "one", "first", true).unwrap();
        assert!(
            apply_renames_inner(
                &paths,
                &["two".into(), "three".into(), "one".into()],
                Some(2)
            )
            .is_err()
        );
        for (index, path) in paths.iter().enumerate() {
            assert_eq!(fs::read(path).unwrap(), [index as u8]);
        }
        assert_eq!(read_descriptions(&root, true).unwrap()["one"], "first");
        assert!(!fs::read_dir(&root).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".ndr-rename-")
        }));
        apply_renames(&paths, &["two".into(), "three".into(), "one".into()]).unwrap();
        assert_eq!(read_descriptions(&root, true).unwrap()["two"], "first");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn language_decodes_cp949_and_resolves_root_alias() {
        let root = sandbox("cp949");
        let (bytes, _, errors) = encoding_rs::EUC_KR.encode("[RES]\r\nRENAME=이름 바꾸기(&R)\r\n");
        assert!(!errors);
        fs::write(root.join("ko.lang"), bytes.as_ref()).unwrap();
        assert_eq!(
            Language::load(&root, "ko").text("Root.RENAME", "missing"),
            "이름 바꾸기(R)"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn description_encoding_and_legacy_mask_roundtrip() {
        let root = sandbox("description");
        fs::write(root.join("descript.ion"), "\"a b.txt\" legacy\n").unwrap();
        set_description(&root, "문서.txt", "한글\n설명", true).unwrap();
        let bytes = fs::read(root.join("descript.dat")).unwrap();
        assert!(bytes.starts_with(&[255, 254]));
        assert_eq!(
            read_descriptions(&root, true).unwrap()["문서.txt"],
            "한글 설명"
        );
        set_description(&root, "a b.txt", "", true).unwrap();
        assert!(
            !read_descriptions(&root, true)
                .unwrap()
                .contains_key("a b.txt")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn shortcut_delphi_codes_and_header_rules() {
        assert_eq!(text_to_shortcut("Ctrl+Shift+Delete"), 0x602e);
        assert_eq!(text_to_shortcut("Ctrl+Num +"), 0x406b);
        assert_eq!(text_to_shortcut("Ctrl++"), 0x60bb);
        let map = ShortcutMap::parse(Some(
            "NEXUSFILESHORTCUT1.0\nDelete=Shift+Delete\nDelete=Shift+Alt+D\nmiOpen=F3\nmiOpen=F4\n",
        ));
        assert_eq!(map.menu_shortcuts["miOpen"], 0x73);
        assert_eq!(map.lines.len(), 4);
        assert_eq!(map.resolve(0x202e).as_deref(), Some("Delete"));
        assert_eq!(text_to_shortcut("Ctrl+PageUp"), 0x4021);
        assert_eq!(shortcut_to_text(0x4021), "Ctrl+PageUp");
    }
    #[test]
    fn original_rename_pipeline_supports_target_order_and_delete_modes() {
        let rules = vec![
            RenameRule {
                action: "Replace".into(),
                find: "doc".into(),
                replace: "file".into(),
                ..Default::default()
            },
            RenameRule {
                action: "Insert".into(),
                text: "문서_".into(),
                position: "Prefix".into(),
                ..Default::default()
            },
            RenameRule {
                action: "Number".into(),
                start: 7,
                digits: 3,
                position: "Suffix".into(),
                ..Default::default()
            },
            RenameRule {
                action: "Case".into(),
                target: "Extension".into(),
                case_mode: "Lower".into(),
                ..Default::default()
            },
        ];
        assert_eq!(
            rename_with_rules("DOC.TXT", &rules, 1, false).unwrap(),
            "문서_file008.txt"
        );
        let delete = RenameRule {
            action: "Delete".into(),
            delete_from_mode: "FromEnd".into(),
            delete_direction: "RightToLeft".into(),
            count: 2,
            ..Default::default()
        };
        assert_eq!(
            rename_with_rules("한글문서.txt", &[delete], 0, false).unwrap(),
            "한글.txt"
        );
        let delete = RenameRule {
            action: "Delete".into(),
            delete_from_mode: "Delimiter".into(),
            find: "[".into(),
            delete_to_mode: "Delimiter".into(),
            delete_to_delimiter: "]".into(),
            ..Default::default()
        };
        assert_eq!(
            rename_with_rules("a[inside]z.ext", &[delete], 0, false).unwrap(),
            "a[]z.ext"
        );
    }
    #[test]
    fn description_failure_rolls_back_files_and_metadata() {
        let root = sandbox("metadata-rollback");
        let a = root.join("old");
        fs::write(&a, "data").unwrap();
        set_description(&root, "old", "comment", true).unwrap();
        let original = fs::read(root.join("descript.dat")).unwrap();
        assert!(apply_renames(std::slice::from_ref(&a), &["bad=name".into()]).is_err());
        assert_eq!(fs::read(&a).unwrap(), b"data");
        assert_eq!(fs::read(root.join("descript.dat")).unwrap(), original);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn shortcut_editor_preserves_duplicate_internal_bindings() {
        let mut map = ShortcutMap::parse(Some(
            "NEXUSFILESHORTCUT1.0\nDelete=Shift+Delete\nDelete=Shift+Alt+D\n",
        ));
        map.set_binding("miOpen", "Ctrl+Enter").unwrap();
        let reloaded = ShortcutMap::parse(Some(&map.to_text()));
        assert_eq!(reloaded.resolve(0x202e).as_deref(), Some("Delete"));
        assert_eq!(reloaded.menu_shortcuts["miOpen"], 0x400d);
        let mut repeated = ShortcutMap::parse(Some("NEXUSFILESHORTCUT1.0\nmiOpen=F3\nmiOpen=F4\n"));
        repeated.set_binding("miOpen", "F5").unwrap();
        assert_eq!(repeated.menu_shortcuts["miOpen"], 0x74);
    }
    #[test]
    fn destination_history_expires_old_dates_and_converts_current_local_time() {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let entries = import_destination_history(&format!(
            "NEXUSFILEDEST.VER1\r\nC:\\old\t9\t2000-01-01 00:00:00\r\nC:\\new\t3\t{now}\r\n"
        ));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["Path"], r"C:\new");
        assert!(entries[0]["LastUsedUtc"].as_str().unwrap().ends_with('Z'));
        // Original FUN_0046ddf0 reads the date by position, so other locale separators work too.
        let recent = (chrono::Local::now() - chrono::Duration::days(1)).format("%Y/%m/%d %H.%M.%S").to_string();
        let entries = import_destination_history(&format!("NEXUSFILEDEST.VER1\r\nD:\\slash\t2\t{recent}\r\n"));
        assert_eq!(entries.iter().map(|e| e["Path"].as_str().unwrap_or("")).collect::<Vec<_>>(), [r"D:\slash"]);
        assert!(positional_date_time("2026.10.31 22:05:09").is_some() && positional_date_time("2026-10-3").is_none());
    }
    #[test]
    fn description_transfers_preserve_unrelated_entries_and_migrate_legacy() {
        let root = sandbox("transfers");
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("descript.ion"), "one.txt legacy comment\r\n").unwrap();
        migrate_legacy_descriptions(&source, &["one.txt".into()], true).unwrap();
        set_description(&source, "unrelated", "keep", true).unwrap();
        let errors = apply_description_transfers(&[DescriptionTransfer {
            source_directory: source.clone(),
            source_name: "one.txt".into(),
            destination_directory: destination.clone(),
            destination_name: "renamed.txt".into(),
            description: Some("legacy comment".into()),
            move_entry: true,
        }]);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            !read_descriptions(&source, false)
                .unwrap()
                .contains_key("one.txt")
        );
        assert_eq!(
            read_descriptions(&source, false).unwrap()["unrelated"],
            "keep"
        );
        assert_eq!(
            read_descriptions(&destination, false).unwrap()["renamed.txt"],
            "legacy comment"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn imported_legacy_password_is_dpapi_protected_before_serialization() {
        assert_eq!(decode_legacy_password("AAA934535F95"), "secret");
        let site = import_remote_site(
            "[Owned Test]",
            &["host=example.invalid", "pass=AAA934535F95"],
        )
        .unwrap();
        let protected = site["EncryptedPassword"].as_str().unwrap();
        assert!(!protected.is_empty());
        assert_ne!(protected, "secret");
        assert_eq!(
            crate::native::unprotect_secret(protected).unwrap(),
            "secret"
        );
        assert!(!serde_json::to_string(&site).unwrap().contains("secret"));
    }
}

// Original r69 shortcut defaults and DFM menu search order, ported as data.
pub const DEFAULT_SHORTCUT_TEXT: &str = r##"NEXUSFILESHORTCUT1.0
miOpen=Enter
miParameterOpen=Ctrl+Enter
miOpenAllSelectedFiles=Ctrl+Alt+Enter
miCopyTo=Alt+C
miMoveTo=Alt+M
miDelete=Alt+D
miShred=Shift+Ctrl+D
miRename=Alt+R
miAdvRename=Shift+Alt+R
miChangeAttributes=Ctrl+Z
miCalculateSelectedFolderSize=Ctrl+Shift+R
miProperties=F9
miDescription=Alt+Enter
miFileInfo=Shift+Enter
miCreateEmptyFile=Ctrl+N
miShortcutOnDesktop=Ctrl+K
miShortcutOnQuickLaunch=Ctrl+U
miExit=Alt+X
miFolderTree=F10
miFavorites=F11
miNewFolder=Alt+K
miHistoryBack=Alt+Left
miHistoryForward=Alt+Right
miToParent=Backspace
miToRoot=\
miToLastDest=Ctrl+Backspace
miNetwork=Shift+/
miWorkFolder1=Ctrl+1
miWorkFolder2=Ctrl+2
miWorkFolder3=Ctrl+3
miWorkFolder4=Ctrl+4
miWorkFolder5=Ctrl+5
miGoToWorkFolder1=Alt+1
miGoToWorkFolder2=Alt+2
miGoToWorkFolder3=Alt+3
miGoToWorkFolder4=Alt+4
miGoToWorkFolder5=Alt+5
miRefresh=Ctrl+R
miEditCopy=Ctrl+C
miEditCut=Ctrl+X
miEditPaste=Ctrl+V
miPasteAsShortcut=Shift+Ctrl+V
miCopyFullPath=Ctrl+Alt+F
miCopyFileName=Ctrl+Alt+A
miCopyPath=Ctrl+Alt+R
miSelect=Spacebar
miSelectAll=Alt+U
miInvertSelection=Num *
miAdvSelect=Num +
miAdvDeselect=Num -
miRestoreLastSel=Shift+Backspace
miSelectSameExt=Num /
miSelectSameName=Ctrl+Num /
miFind=Ctrl+F
miFindNext=Shift+Ctrl+F
miDualView=Alt+F2
miShowFolders=Alt+F3
miNewTab=Ctrl+T
miListStyleAuto=
miListStyleNames=
miListStyleDetails=
miListStyleThumbs=
miColAuto=Ctrl+Alt+0
miCol1=Ctrl+Alt+1
miCol2=Ctrl+Alt+2
miCol3=Ctrl+Alt+3
miCol4=Ctrl+Alt+4
miCol5=Ctrl+Alt+5
miCol6=Ctrl+Alt+6
miSortByNone=Ctrl+Alt+O
miSortByName=Ctrl+Alt+N
miSortByExt=Ctrl+Alt+E
miSortBySize=Ctrl+Alt+S
miSortByTime=Ctrl+Alt+T
miSortByAttr=Ctrl+Alt+B
miSortDescend=Ctrl+Alt+Num -
miFilterCustom=Ctrl+I
miShowHidden=Alt+Z
miToDesktop=Ctrl+D
miToDocuments=Ctrl+G
miToPictures=Ctrl+Q
miToMusic=Ctrl+S
miToVideos=Ctrl+O
miToDownloads=
miControlPanel=Alt+F12
miAddRemoveProg=Alt+F11
miRecycleBin=Ctrl+B
miEmptyRecycleBin=Shift+Ctrl+B
miToTemp=Ctrl+P
miEmptyTemp=Shift+Ctrl+P
miCommandLine=/
miArcAdd=Ctrl+A
miArcMove=Ctrl+Shift+A
miArcExtract=Ctrl+E
miArcExtractToCurr=Ctrl+Shift+E
miArcExtractToFolder=
miArcExtractAuto=Alt+Q
miFTPQuickConnect=
miFTPManageSites=
miFTPDisconnect=
miMapNetworkDrive=
miUnmapNetworkDrive=
miTextViewer=Alt+V
miTextEditor=Alt+B
miClearDescriptions=
miSaveList=
miSplitFile=Ctrl+Alt+L
miJoinFile=Ctrl+Alt+J
miDiskCopy=
miCleanupDisk=
miCompareFolder=
miOptions=Ctrl+F12
CopyToLeft=Ctrl+Left
CopyToRight=Ctrl+Right
MoveToLeft=Ctrl+Alt+Left
MoveToRight=Ctrl+Alt+Right
CopyTo=Insert
MoveTo=Shift+Insert
SelectAllWithFolder=Ctrl+Num *
FindNext=Ctrl+L
ExchangePath=
ShowPathHistory=Alt+Down
RecycleBin=Delete
Delete=Shift+Delete
Delete=Shift+Alt+D
Shred=Ctrl+Shift+Delete
FocusAddressBar=Alt+`
SelectSameName=Shift+,
SelectSameExt=Shift+.
CloseTab=Ctrl+W
SelectCurrAndMove=Spacebar"##;
const MENU_SHORTCUTS: &[(&str, u16)] = &[
    ("miOpen", 0x0000),
    ("miParameterOpen", 0x400d),
    ("miOpenAllSelectedFiles", 0x0000),
    ("N26", 0x0000),
    ("miCopyTo", 0x8043),
    ("miMoveTo", 0x804d),
    ("N8", 0x0000),
    ("miDelete", 0x8044),
    ("miShred", 0x6044),
    ("N27", 0x0000),
    ("miRename", 0x8052),
    ("miAdvRename", 0xa052),
    ("miChangeAttributes", 0x405a),
    ("N21", 0x0000),
    ("miCalculateSelectedFolderSize", 0x0000),
    ("N31", 0x0000),
    ("miProperties", 0x0078),
    ("miDescription", 0x800d),
    ("miFileInfo", 0x200d),
    ("N1", 0x0000),
    ("miCreateEmptyFile", 0x404e),
    ("miShortcutOnDesktop", 0x404b),
    ("miShortcutOnQuickLaunch", 0x4055),
    ("N6", 0x0000),
    ("miChecksum", 0x0000),
    ("miCRC", 0x0000),
    ("miMD5", 0x0000),
    ("miSHA1", 0x0000),
    ("N33", 0x0000),
    ("miExit", 0x8058),
    ("miFolderTree", 0x0079),
    ("miFavorites", 0x007a),
    ("N37", 0x0000),
    ("miNewFolder", 0x804b),
    ("N39", 0x0000),
    ("miHistoryBack", 0x8025),
    ("miHistoryForward", 0x8027),
    ("N43", 0x0000),
    ("miToParent", 0x0000),
    ("miToRoot", 0x0000),
    ("miToLastDest", 0x4008),
    ("N46", 0x0000),
    ("miNetwork", 0x0000),
    ("N49", 0x0000),
    ("miSetWorkFolder", 0x0000),
    ("miWorkFolder1", 0x4031),
    ("miWorkFolder2", 0x4032),
    ("miWorkFolder3", 0x4033),
    ("miWorkFolder4", 0x4034),
    ("miWorkFolder5", 0x4035),
    ("miGoToWorkFolder", 0x0000),
    ("miGoToWorkFolder1", 0x8031),
    ("miGoToWorkFolder2", 0x8032),
    ("miGoToWorkFolder3", 0x8033),
    ("miGoToWorkFolder4", 0x8034),
    ("miGoToWorkFolder5", 0x8035),
    ("N52", 0x0000),
    ("miRefresh", 0x4052),
    ("miEditCopy", 0x4043),
    ("miEditCut", 0x4058),
    ("miEditPaste", 0x4056),
    ("miPasteAsShortcut", 0x6056),
    ("miPasteFolderStructure", 0x0000),
    ("N62", 0x0000),
    ("miCopyFullPath", 0xc046),
    ("miCopyFileName", 0xc041),
    ("miCopyPath", 0xc052),
    ("N68", 0x0000),
    ("miSelect", 0x0000),
    ("miSelectAll", 0x8055),
    ("miInvertSelection", 0x0000),
    ("miAdvSelect", 0x0000),
    ("miAdvDeselect", 0x0000),
    ("N50", 0x0000),
    ("miRestoreLastSel", 0x2008),
    ("miSelectSameExt", 0x0000),
    ("miSelectSameName", 0x0000),
    ("N77", 0x0000),
    ("miFind", 0x4046),
    ("miFindNext", 0x6046),
    ("N81", 0x0000),
    ("miSearchFile", 0x4048),
    ("miLanguage", 0x0000),
    ("N23", 0x0000),
    ("miMoreLanguages", 0x0000),
    ("miSkin", 0x0000),
    ("N11", 0x0000),
    ("miDualView", 0x0000),
    ("miShowFolders", 0x0000),
    ("miViewWorkWnd", 0x0000),
    ("N4", 0x0000),
    ("miViewToolbar", 0x0000),
    ("miViewFindResult", 0x0000),
    ("miViewVertBar", 0x0000),
    ("miViewButtonBar", 0x0000),
    ("N25", 0x0000),
    ("miShowDriveButtons", 0x0000),
    ("miViewFolderInfoBar", 0x0000),
    ("miViewFileInfoBar", 0x0000),
    ("miViewExtensionInfo", 0x0000),
    ("N20", 0x0000),
    ("miNewTab", 0x4054),
    ("N98", 0x0000),
    ("miListStyle", 0x0000),
    ("miListStyleAuto", 0x0000),
    ("miListStyleNames", 0x0000),
    ("miListStyleDetails", 0x0000),
    ("miListStyleThumbs", 0x0000),
    ("miColumn", 0x0000),
    ("miColAuto", 0xc030),
    ("N17", 0x0000),
    ("miCol1", 0xc031),
    ("miCol2", 0xc032),
    ("miCol3", 0xc033),
    ("miCol4", 0xc034),
    ("miCol5", 0xc035),
    ("miCol6", 0xc036),
    ("N100", 0x0000),
    ("miSortBy", 0x0000),
    ("miSortByNone", 0xc04f),
    ("N3", 0x0000),
    ("miSortByName", 0xc04e),
    ("miSortByExt", 0xc045),
    ("miSortBySize", 0xc053),
    ("miSortByTime", 0xc054),
    ("miSortByAttr", 0xc042),
    ("miSortByColor", 0xc043),
    ("N7", 0x0000),
    ("N106", 0x0000),
    ("miSortDescend", 0xc0bd),
    ("miGroupBy", 0x0000),
    ("miUngroup", 0x0000),
    ("N14", 0x0000),
    ("miGroupByExt", 0x0000),
    ("miGroupBySize", 0x0000),
    ("miGroupByTime", 0x0000),
    ("miGroupByCategory", 0x0000),
    ("N119", 0x0000),
    ("N12", 0x0000),
    ("miGroupByDescend", 0x0000),
    ("miShowGroupName", 0x0000),
    ("miFilter", 0x0000),
    ("N19", 0x0000),
    ("miFilterCustom", 0x4049),
    ("miFolderCategory", 0x0000),
    ("miAutoFolderCategory", 0x0000),
    ("N30", 0x0000),
    ("miCategoryNormal", 0x0000),
    ("N126", 0x0000),
    ("miShowHidden", 0x805a),
    ("miToDesktop", 0x4044),
    ("miToDocuments", 0x4057),
    ("miToPictures", 0x4051),
    ("miToMusic", 0x4053),
    ("miToVideos", 0x404f),
    ("miToDownloads", 0x0000),
    ("N128", 0x0000),
    ("miControlPanel", 0x807b),
    ("miAddRemoveProg", 0x807a),
    ("N131", 0x0000),
    ("miRecycleBin", 0x4042),
    ("miEmptyRecycleBin", 0x6042),
    ("N134", 0x0000),
    ("miToTemp", 0x4050),
    ("miEmptyTemp", 0x6050),
    ("N137", 0x0000),
    ("miCommandLine", 0x0000),
    ("miArcAdd", 0x4041),
    ("miArcMove", 0x6041),
    ("N141", 0x0000),
    ("miArcExtract", 0x4045),
    ("miArcExtractToCurr", 0x6045),
    ("miArcExtractToFolder", 0x0000),
    ("miArcExtractAuto", 0x8051),
    ("N144", 0x0000),
    ("miArcInstall", 0x4049),
    ("N36", 0x0000),
    ("miArcProperties", 0x0000),
    ("miFTPQuickConnect", 0x0000),
    ("N149", 0x0000),
    ("miFTPConnect", 0x0000),
    ("N18", 0x0000),
    ("miFTPManageSites", 0x0000),
    ("miFTPDisconnect", 0x0000),
    ("N01", 0x0000),
    ("N38", 0x0000),
    ("N40", 0x0000),
    ("N44", 0x0000),
    ("N45", 0x0000),
    ("N47", 0x0000),
    ("miFTPGet", 0x0000),
    ("miFTPPut", 0x0000),
    ("N10", 0x0000),
    ("miMapNetworkDrive", 0x0000),
    ("miUnmapNetworkDrive", 0x0000),
    ("miTextViewer", 0x8056),
    ("miTextEditor", 0x8042),
    ("N152", 0x0000),
    ("miClearDescriptions", 0x0000),
    ("N154", 0x0000),
    ("miSaveList", 0x0000),
    ("N156", 0x0000),
    ("miSplitFile", 0xc04c),
    ("miJoinFile", 0xc04a),
    ("N162", 0x0000),
    ("miDiskCopy", 0x0000),
    ("N164", 0x0000),
    ("miCleanupDisk", 0x0000),
    ("N169", 0x0000),
    ("miSynchronizeFolder", 0x0000),
    ("miCompareFolder", 0x0000),
    ("N171", 0x0000),
    ("miOptions", 0x407b),
    ("miHelp", 0x0070),
    ("miShortcut", 0x007b),
    ("N175", 0x0000),
    ("miHomepage", 0x0000),
    ("miEmail", 0x0000),
    ("N178", 0x0000),
    ("miCheckUpdate", 0x0000),
    ("N13", 0x0000),
    ("miAbout", 0x0000),
    ("miTest", 0x0000),
];
const INTERNAL_COMMANDS: &[&str] = &[
    "None",
    "Open",
    "OpenWithParam",
    "CopyTo",
    "MoveTo",
    "CopyToLeft",
    "CopyToRight",
    "MoveToLeft",
    "MoveToRight",
    "Delete",
    "RecycleBin",
    "Shred",
    "Rename",
    "AdvRename",
    "ChangeAttr",
    "Properties",
    "EditDescription",
    "Favorites",
    "NewFolder",
    "Refresh",
    "GoToDesktop",
    "GoToDocuments",
    "GoToMusic",
    "GoToPictures",
    "GoToVideos",
    "GoToDownloads",
    "View",
    "Edit",
    "SelectAll",
    "AddToArchive",
    "Extract",
    "AutoExtract",
    "CloseTab",
    "SelectAllWithFolder",
    "FindNext",
    "ExchangePath",
    "ShowPathHistory",
    "FocusAddressBar",
    "NetworkDialog",
    "SelectSameName",
    "SelectSameExt",
    "SelectCurrAndMove",
    "Select",
    "DeselectAll",
    "FTPSiteManager",
    "Back",
    "Forward",
    "OpenAllSelected",
    "FolderTree",
    "GoToRoot",
    "GoToParent",
    "GoToLastDest",
    "EditCopy",
    "EditCut",
    "EditPaste",
    "InvertSelection",
    "NewTab",
    "CommandLine",
    "SplitFile",
    "JoinFiles",
];
