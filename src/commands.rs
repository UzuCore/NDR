#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compat::MacroPane, settings::Settings};
    use std::path::{Path, PathBuf};

    #[test]
    fn legacy_function_key_label_and_parameter_prompt() {
        let mut settings = Settings::default();
        settings.values.insert(
            "FunctionKeys".into(),
            serde_json::json!({"F2":"[Run]tool.exe one %P two"}),
        );
        let binding = resolve_function_key(&settings, "f2").unwrap();
        let pane = MacroPane {
            path: PathBuf::from(r"C:\한 글"),
            focused: Some("a b.txt".into()),
            selected: vec![],
        };
        let result = prepare_function_key(&binding, &pane, &pane, &pane, Path::new(r"C:\NDIR"));
        assert_eq!(
            result,
            PreparedCommand::External {
                program: "tool.exe".into(),
                arguments: "one  two".into(),
                prompt: true
            }
        );
    }

    #[test]
    fn explicit_function_binding_preserves_internal_flag() {
        let mut settings = Settings::default();
        settings.values.insert("FunctionKeyBindings".into(), serde_json::json!({"F3":{"Name":"  View  ","Command":" #Open:arg ","Parameters":" ignored ","IsInternalCommand":true}}));
        let binding = resolve_function_key(&settings, "F3").unwrap();
        let pane = MacroPane::default();
        assert_eq!(
            prepare_function_key(&binding, &pane, &pane, &pane, Path::new(".")),
            PreparedCommand::Internal("Open".into())
        );
        assert_eq!(binding.name, "View");
    }

    #[test]
    fn windows_args_preserve_unicode_spaces_quotes_and_backslashes() {
        assert_eq!(
            parse_windows_args(r#""한 글" "a\"b" "C:\dir\\" """#).unwrap(),
            vec!["한 글", "a\"b", "C:\\dir\\", ""]
        );
        assert!(parse_windows_args("abc \"unfinished").is_err());
    }

    #[test]
    fn archive_source_name_cannot_introduce_an_option() {
        let options = ArchiveOptions {
            extension: "7z".into(),
            level: 9,
            password: String::new(),
            volume_bytes: None,
            recursive: false,
            append: false,
        };
        let args = build_archive_create_args(
            &options,
            &[PathBuf::from("-evil")],
            Path::new("out.7z"),
            Path::new("."),
            Some("a {archive} {sources}"),
        )
        .unwrap();
        assert_eq!(args[2], ".\\-evil");
        assert_eq!(args.len(), 3);
        assert!(
            build_archive_create_args(
                &options,
                &[],
                Path::new("out.7z"),
                Path::new("."),
                Some("a {archive} {sources} {unknown}")
            )
            .is_err()
        );
    }

    #[test]
    fn toolbar_parses_the_original_user_command_and_container_items() {
        // 원본 FUN_0047d98c 형식: InternalCommand 0, UserCommand 1, Separator 2, Container 3(<subitems>, FUN_0047da78).
        let xml = br#"<toolbar>
<item type="UserCommand"><name>Note</name><icon>$(SKINDIR)\tb_x.png</icon><command>notepad.exe %F</command></item>
<item type="Container"><name>More</name><icon>tb_more.png</icon>
  <subitems>
    <item type="InternalCommand"><name>Up</name><command>GoToParent</command></item>
    <item type="Separator" />
    <item type="UserCommand"><command>[Calc]calc.exe</command></item>
    <item type="Container"><name>nested</name></item>
  </subitems>
</item>
</toolbar>"#;
        let config = parse_toolbar(xml).unwrap();
        assert_eq!(config.items.len(), 2);
        assert_eq!((config.items[0].action.as_str(), config.items[0].caption.as_str(), config.items[0].icon.as_str()), ("user:notepad.exe %F", "Note", "tb_x.png"));
        let more = &config.items[1];
        assert_eq!((more.action.as_str(), more.caption.as_str()), ("container:1", "More"));
        let kinds: Vec<(ToolbarKind, &str)> = more.children.iter().map(|c| (c.kind.clone(), c.action.as_str())).collect();
        assert_eq!(kinds, [(ToolbarKind::Command, "function:GoToParent"), (ToolbarKind::Separator, ""), (ToolbarKind::Command, "user:[Calc]calc.exe")]);
        assert!(config.warning.is_some(), "a nested container is skipped");
    }

    #[test]
    fn toolbar_recycle_bin_sends_to_the_recycle_bin_like_the_original() {
        // 원본 FUN_00444710: RecycleBin 단추의 클릭은 FUN_0042f13c(수식 키에 따라 휴지통·영구·완전 삭제, Ctrl+Alt면 휴지통 열기).
        assert_eq!(toolbar_action("RecycleBin").as_deref(), Some("deleteToolbar"));
        assert_ne!(toolbar_action("OpenRecycleBin").as_deref(), Some("deleteToolbar"));
    }
    #[test]
    fn toolbar_preserves_icons_and_skips_unknown_commands() {
        let xml = br#"<toolbar><item type="InternalCommand"><icon>$(SKINDIR)\tb_back.png</icon><command>Back</command></item><item type="InternalCommand"><command>Unknown</command></item><item type="QuickSearch"><width>500</width></item></toolbar>"#;
        let parsed = parse_toolbar(xml).unwrap();
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].icon, "tb_back.png");
        assert_eq!(parsed.items[0].action, "historyBack");
        assert_eq!(parsed.items[1].width, 400.0);
        assert!(parsed.warning.is_some());
        assert!(parse_toolbar(b"<!DOCTYPE toolbar><toolbar><item/></toolbar>").is_err());
        assert!(parse_toolbar(b"<toolbar><item type=\"Separator\"/>").is_err());
        let oversized = format!(
            "<toolbar>{}</toolbar>",
            "<item type=\"Unknown\"/>".repeat(257)
        );
        assert!(parse_toolbar(oversized.as_bytes()).is_err());
        let entities = parse_toolbar(br#"<toolbar><item type="InternalCommand"><command>Back</command><caption>A &amp; <![CDATA[B]]></caption></item></toolbar>"#).unwrap();
        assert_eq!(entities.items[0].caption, "A & B");
    }

    #[test]
    fn quoted_program_unquotes_and_arguments_expand_as_one_string() {
        let binding = FunctionBinding {
            name: "F4".into(),
            command: r#""C:\Program Files\tool.exe" /open"#.into(),
            parameters: "$(FILENAME)".into(),
            internal: false,
            legacy: false,
        };
        let pane = MacroPane {
            path: PathBuf::from(r"C:\자료"),
            focused: Some("a b.txt".into()),
            selected: vec![],
        };
        assert_eq!(
            prepare_function_key(&binding, &pane, &pane, &pane, Path::new(".")),
            PreparedCommand::External {
                program: r"C:\Program Files\tool.exe".into(),
                arguments: "/open a b.txt".into(),
                prompt: false
            }
        );
    }

    #[test]
    fn archive_defaults_and_extract_templates_keep_each_path_separate() {
        let options = ArchiveOptions {
            extension: "zip".into(),
            level: 12,
            password: "secret".into(),
            volume_bytes: Some(4096),
            recursive: false,
            append: false,
        };
        assert_eq!(
            build_archive_create_args(
                &options,
                &[PathBuf::from("한 글.txt")],
                Path::new("out file.zip"),
                Path::new("."),
                None
            )
            .unwrap(),
            vec![
                "a",
                "-tzip",
                "-r-",
                "-mx=9",
                "-psecret",
                "-mem=AES256",
                "-v4096b",
                "out file.zip",
                "한 글.txt"
            ]
        );
        assert_eq!(
            build_archive_extract_args(
                Path::new("a file.zip"),
                Path::new("dest folder"),
                "",
                &["-evil".into()],
                Some("x {archive} -o{destination} {keys}"),
                true
            )
            .unwrap(),
            vec!["x", "a file.zip", "-odest folder", "--", "-evil"]
        );
        assert!(
            build_archive_extract_args(
                Path::new("a.zip"),
                Path::new("dest"),
                "",
                &["-evil".into()],
                None,
                false
            )
            .is_err()
        );
    }
}
use crate::{compat, settings::Settings};
use quick_xml::{Reader, events::Event};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionBinding {
    pub name: String,
    pub command: String,
    pub parameters: String,
    pub internal: bool,
    pub legacy: bool,
}

pub fn resolve_function_key(settings: &Settings, key: &str) -> Option<FunctionBinding> {
    fn entry<'a>(settings: &'a Settings, field: &str, key: &str) -> Option<&'a Value> {
        settings
            .values
            .get(field)?
            .as_object()?
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }
    if let Some(value) = entry(settings, "FunctionKeyBindings", key).and_then(Value::as_object) {
        let get = |field: &str| {
            value
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        return Some(FunctionBinding {
            name: get("Name"),
            command: get("Command"),
            parameters: get("Parameters"),
            internal: value
                .get("IsInternalCommand")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            legacy: false,
        });
    }
    let command = entry(settings, "FunctionKeys", key)?.as_str()?.trim();
    if command.is_empty() {
        return None;
    }
    Some(FunctionBinding {
        name: key.to_owned(),
        command: command.to_owned(),
        parameters: String::new(),
        internal: command.starts_with('#'),
        legacy: true,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedCommand {
    Disabled,
    Internal(String),
    /// C# ExecuteFunctionKeyMacroAsync의 바깥 명령: 프로그램, 매크로를 푼 인수 글, `%p`/`%P`가 있었는지(있으면 인수 편집 창을 먼저 연다).
    /// 빈 프로그램·폴더 열기·ShellExecuteEx는 부르는 쪽이 인수 편집 뒤에 C# 순서대로 처리한다.
    External {
        program: String,
        arguments: String,
        prompt: bool,
    },
}

pub fn prepare_function_key(
    binding: &FunctionBinding,
    active: &compat::MacroPane,
    left: &compat::MacroPane,
    right: &compat::MacroPane,
    program_dir: &Path,
) -> PreparedCommand {
    let mut command = binding.command.trim();
    if binding.legacy {
        command = compat::strip_function_key_label(command);
    }
    if command.is_empty() || (!binding.internal && command.starts_with('*')) {
        return PreparedCommand::Disabled;
    }
    if binding.internal || command.starts_with('#') {
        let name = command
            .trim_start_matches('#')
            .split(':')
            .next()
            .unwrap_or("")
            .trim();
        return if name.is_empty() || name.eq_ignore_ascii_case("None") {
            PreparedCommand::Disabled
        } else {
            PreparedCommand::Internal(name.to_owned())
        };
    }
    // 있는 프로그램 경로면 그대로, 아니면 원본처럼 PathRemoveArgsW/PathGetArgsW로 나눈다.
    let (program, mut arguments) = if !binding.legacy && Path::new(command).exists() {
        (command.to_owned(), binding.parameters.clone())
    } else {
        let line = if binding.legacy || binding.parameters.is_empty() {
            command.to_owned()
        } else {
            format!("{command} {}", binding.parameters)
        };
        split_program(&line)
    };
    // FUN_0040df28(프로그램·인수) → 인수에만 패널 매크로(FUN_00442dd4) → TakeParameterPrompt.
    let program = compat::expand_program_macros(program.trim_matches('"'), program_dir);
    let mut prompt = false;
    if !arguments.is_empty() {
        arguments = compat::expand_program_macros(&arguments, program_dir);
        if !arguments.is_empty() {
            arguments = compat::expand_command(&arguments, active, left, right);
        }
        prompt = compat::take_parameter_prompt(&mut arguments);
    }
    PreparedCommand::External { program, arguments, prompt }
}

fn split_program(line: &str) -> (String, String) {
    let mut quoted = false;
    for (i, ch) in line.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        }
        if ch == ' ' && !quoted {
            return (
                line[..i].to_owned(),
                line[i..].trim_start_matches(' ').to_owned(),
            );
        }
    }
    (line.to_owned(), String::new())
}

/// Windows command-line parsing, including backslash-before-quote rules.
pub fn parse_windows_args(input: &str) -> Result<Vec<String>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut args = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i == chars.len() {
            break;
        }
        let mut out = String::new();
        let mut quoted = false;
        while i < chars.len() && (quoted || !chars[i].is_whitespace()) {
            if chars[i] == '\\' {
                let start = i;
                while i < chars.len() && chars[i] == '\\' {
                    i += 1;
                }
                let count = i - start;
                out.extend(std::iter::repeat_n(
                    '\\',
                    count / if chars.get(i) == Some(&'"') { 2 } else { 1 },
                ));
                if chars.get(i) == Some(&'"') {
                    if count % 2 == 1 {
                        out.push('"');
                        i += 1;
                    } else {
                        quoted = !quoted;
                        i += 1;
                    }
                }
                continue;
            }
            if chars[i] == '"' {
                quoted = !quoted;
            } else {
                out.push(chars[i]);
            }
            i += 1;
        }
        if quoted {
            return Err("Unclosed argument quote".into());
        }
        args.push(out);
    }
    Ok(args)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolbarKind {
    Command,
    Separator,
    QuickSearch,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarItem {
    pub kind: ToolbarKind,
    pub action: String,
    pub icon: String,
    pub caption: String,
    pub align_right: bool,
    pub width: f64,
    /// 원본 Container(형식 3)의 하위 항목(<subitems>): InternalCommand·UserCommand·Separator. 다른 항목은 비어 있다.
    pub children: Vec<ToolbarItem>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarConfig {
    pub items: Vec<ToolbarItem>,
    pub warning: Option<String>,
}

pub fn load_toolbar(profile_dir: &Path, default_xml: &[u8]) -> Result<ToolbarConfig, String> {
    let path = profile_dir.join("toolbar.xml");
    match fs::read(&path).and_then(|bytes| parse_toolbar(&bytes).map_err(std::io::Error::other)) {
        Ok(config) => Ok(config),
        Err(error) => {
            let mut fallback = parse_toolbar(default_xml)?;
            if error.kind() != std::io::ErrorKind::NotFound {
                fallback.warning = Some(format!("{}: {error}", path.display()));
            }
            Ok(fallback)
        }
    }
}

fn toolbar_action(command: &str) -> Option<String> {
    const ACTIONS: &[(&str, &str)] = &[
        ("Back", "historyBack"),
        ("Forward", "historyForward"),
        ("Up", "goParent"),
        ("NewFolder", "newFolder"),
        ("Rename", "rename"),
        ("Copy", "copyToOther"),
        ("Move", "moveToOther"),
        ("Delete", "deleteToolbar"),
        ("Extract", "extractToolbar"),
        ("CreateZip", "createZip"),
        ("BatchRename", "batchRename"),
        ("SaveList", "saveList"),
        ("Split", "split"),
        ("JoinParts", "join"),
        ("GoToDesktop", "desktop"),
        ("GoToDocuments", "documents"),
        ("GoToPictures", "pictures"),
        ("GoToMusic", "music"),
        ("GoToVideos", "videos"),
        ("Favorites", "favorites"),
        // 원본 FUN_00444710: command가 RecycleBin인 단추는 클릭 처리기를 FUN_0042f13c로 바꾼다(삭제 단추):
        // 수식 키 없음 = 휴지통으로, Shift = 영구 삭제, Shift+Ctrl = 완전 삭제, Ctrl+Alt = 휴지통 열기. NDIR deleteToolbar와 같다.
        // C# 판은 휴지통을 열었다.
        ("RecycleBin", "deleteToolbar"),
        ("Search", "search"),
        ("BrowseFolder", "browseFolder"),
        ("ToggleHidden", "toggleHidden"),
        ("NewTab", "newTab"),
        ("CloseTab", "closeTab"),
    ];
    let command = command.trim().trim_start_matches('#');
    ACTIONS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(command))
        .map(|(_, action)| (*action).to_owned())
        .or_else(|| {
            const INTERNAL: &[&str] = &[
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
                "OpenRecycleBin",
            ];
            INTERNAL
                .iter()
                .find(|name| name.eq_ignore_ascii_case(command))
                .map(|name| format!("function:{name}"))
        })
}

pub fn parse_toolbar(xml: &[u8]) -> Result<ToolbarConfig, String> {
    if xml.len() > 128 * 1024 {
        return Err("toolbar.xml exceeds 128 KiB".into());
    }
    let mut reader = Reader::from_reader(xml);
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut current: Option<(String, ToolbarItem, String)> = None;
    let mut child: Option<(String, ToolbarItem, String)> = None;
    let mut warning = false;
    let mut source_items = 0usize;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::DocType(_) => return Err("DTD is not allowed in toolbar.xml".into()),
            event @ (Event::Start(_) | Event::Empty(_)) => {
                let empty = matches!(event, Event::Empty(_));
                let tag = match event {
                    Event::Start(tag) | Event::Empty(tag) => tag,
                    _ => unreachable!(),
                };
                let name = String::from_utf8_lossy(tag.local_name().as_ref()).to_ascii_lowercase();
                if depth == 0 && name != "toolbar" {
                    return Err("Expected <toolbar>".into());
                }
                // 원본 FUN_0047ef08: <item>마다 FUN_0047e4e0, Container(형식 3)면 <subitems>의 <item>마다 FUN_0047da78.
                let child_item = depth == 3 && name == "item" && current.as_ref().is_some_and(|c| c.2 == "subitems");
                if (depth == 1 && name == "item") || child_item {
                    source_items += 1;
                    if source_items > 256 {
                        return Err("Too many toolbar items".into());
                    }
                    let mut kind = String::new();
                    let mut align_right = false;
                    for attr in tag.attributes() {
                        let attr = attr.map_err(|e| e.to_string())?;
                        let key = String::from_utf8_lossy(attr.key.as_ref()).to_ascii_lowercase();
                        let value = attr
                            .decoded_and_normalized_value(
                                quick_xml::XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )
                            .map_err(|e| e.to_string())?
                            .into_owned();
                        if key == "type" {
                            kind = value.trim().to_owned();
                        } else if key == "align" {
                            align_right = value.trim().eq_ignore_ascii_case("right");
                        }
                    }
                    let item = (
                        kind,
                        ToolbarItem {
                            kind: ToolbarKind::Command,
                            action: String::new(),
                            icon: String::new(),
                            caption: String::new(),
                            align_right: align_right && !child_item,
                            width: 1.0,
                            children: Vec::new(),
                        },
                        String::new(),
                    );
                    if child_item {
                        child = Some(item);
                    } else {
                        current = Some(item);
                    }
                } else if depth == 2 && current.is_some() {
                    current.as_mut().unwrap().2 = name.clone();
                } else if depth == 4 && child.is_some() {
                    child.as_mut().unwrap().2 = name.clone();
                }
                if empty {
                    if depth == 1 && name == "item" {
                        finish_toolbar_item(&mut current, &mut items, &mut warning);
                    } else if child_item {
                        finish_toolbar_child(&mut child, &mut current, &mut warning);
                    }
                } else {
                    depth += 1;
                }
            }
            Event::Text(text) => {
                let target = if child.is_some() { &mut child } else { &mut current };
                append_toolbar_text(target, &text.decode().map_err(|e| e.to_string())?);
            }
            Event::CData(text) => {
                let target = if child.is_some() { &mut child } else { &mut current };
                append_toolbar_text(target, &text.decode().map_err(|e| e.to_string())?);
            }
            Event::GeneralRef(reference) => {
                let value =
                    if let Some(c) = reference.resolve_char_ref().map_err(|e| e.to_string())? {
                        c.to_string()
                    } else {
                        match reference.decode().map_err(|e| e.to_string())?.as_ref() {
                            "amp" => "&",
                            "lt" => "<",
                            "gt" => ">",
                            "quot" => "\"",
                            "apos" => "'",
                            other => return Err(format!("Unknown XML entity: {other}")),
                        }
                        .to_owned()
                    };
                let target = if child.is_some() { &mut child } else { &mut current };
                append_toolbar_text(target, &value);
            }
            Event::End(tag) => {
                let name = String::from_utf8_lossy(tag.local_name().as_ref()).to_ascii_lowercase();
                if depth == 2 && name == "item" {
                    finish_toolbar_item(&mut current, &mut items, &mut warning);
                } else if depth == 4 && name == "item" && child.is_some() {
                    finish_toolbar_child(&mut child, &mut current, &mut warning);
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => {
                if depth != 0 {
                    return Err("Unclosed toolbar.xml element".into());
                }
                break;
            }
            _ => {}
        }
    }
    if items.is_empty() {
        return Err("No usable toolbar items".into());
    }
    // Container 단추의 명령 = "container:<차례>"(누르면 그 하위 항목 메뉴).
    for (index, item) in items.iter_mut().enumerate() {
        if item.action == "container:" {
            item.action = format!("container:{index}");
        }
    }
    Ok(ToolbarConfig {
        items,
        warning: warning.then(|| "Some toolbar items were skipped".into()),
    })
}

/// Container의 하위 <item>을 끝낸다: InternalCommand·UserCommand·Separator만 받는다(원본 FUN_0047da78).
fn finish_toolbar_child(child: &mut Option<(String, ToolbarItem, String)>, current: &mut Option<(String, ToolbarItem, String)>, warning: &mut bool) {
    let kind = child.as_ref().map(|c| c.0.to_ascii_lowercase()).unwrap_or_default();
    if !matches!(kind.as_str(), "internalcommand" | "usercommand" | "separator") {
        *child = None;
        *warning = true;
        return;
    }
    let mut finished = Vec::new();
    finish_toolbar_item(child, &mut finished, warning);
    if let Some((_, parent, _)) = current.as_mut() {
        parent.children.extend(finished);
    }
}

fn finish_toolbar_item(
    current: &mut Option<(String, ToolbarItem, String)>,
    items: &mut Vec<ToolbarItem>,
    warning: &mut bool,
) {
    let Some((kind, mut item, _)) = current.take() else {
        return;
    };
    if kind.eq_ignore_ascii_case("separator") {
        item.kind = ToolbarKind::Separator;
        item.action.clear();
        item.icon.clear();
        item.caption.clear();
        items.push(item);
    } else if kind.eq_ignore_ascii_case("quicksearch") {
        item.kind = ToolbarKind::QuickSearch;
        item.width = item
            .caption
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .unwrap_or(180.0)
            .clamp(80.0, 400.0);
        item.caption.clear();
        item.action.clear();
        items.push(item);
    } else if kind.eq_ignore_ascii_case("internalcommand") {
        if let Some(action) = toolbar_action(&item.action) {
            item.action = action;
            item.icon = normalize_icon(&item.icon);
            item.caption = item.caption.trim().to_owned();
            items.push(item);
        } else {
            *warning = true;
        }
    } else if kind.eq_ignore_ascii_case("usercommand") {
        // 원본 형식 1(UserCommand): 클릭은 FUN_0044381c(폼, 명령, 내부 명령 아님) = 기능키의 사용자 명령과 같은 실행.
        let command = item.action.trim().to_owned();
        if command.is_empty() {
            *warning = true;
        } else {
            item.action = format!("user:{command}");
            item.icon = normalize_icon(&item.icon);
            item.caption = item.caption.trim().to_owned();
            items.push(item);
        }
    } else if kind.eq_ignore_ascii_case("container") {
        // 원본 형식 3(Container): DropDownMenu = pmToolBar(+0x874), 펼칠 때 pmToolBarPopup이 하위 항목으로 메뉴를 만든다.
        item.action = "container:".into();
        item.icon = normalize_icon(&item.icon);
        item.caption = item.caption.trim().to_owned();
        items.push(item);
    } else {
        *warning = true;
    }
}
fn append_toolbar_text(current: &mut Option<(String, ToolbarItem, String)>, value: &str) {
    if let Some((_, item, field)) = current.as_mut() {
        match field.as_str() {
            "icon" => item.icon.push_str(value),
            // 원본 FUN_0047e4e0·FUN_0047da78의 글 요소는 <name>, C# 판은 <caption>.
            "caption" | "name" | "width" => item.caption.push_str(value),
            "command" => item.action.push_str(value),
            _ => {}
        }
    }
}

fn normalize_icon(icon: &str) -> String {
    let filename = icon.trim().rsplit(['/', '\\']).next().unwrap_or("");
    if filename.len() <= 128 && !filename.contains([':', '\0']) && !filename.contains("..") {
        filename.to_owned()
    } else {
        String::new()
    }
}

#[derive(Clone, Debug)]
pub struct ArchiveOptions {
    pub extension: String,
    pub level: i32,
    pub password: String,
    pub volume_bytes: Option<u64>,
    pub recursive: bool,
    /// 있는 압축 파일(한 파일)에 더한다(C# AddToArchiveAsync). 아니면 새로 만들어 교체한다.
    pub append: bool,
}

fn count_token(tokens: &[String], needle: &str) -> usize {
    tokens.iter().filter(|s| s.as_str() == needle).count()
}
fn known_placeholders(token: &str, allowed: &[&str]) -> Result<(), String> {
    let re = regex::Regex::new(r"\{[A-Za-z][A-Za-z0-9]*\}").expect("constant regex");
    for found in re.find_iter(token) {
        if !allowed.contains(&found.as_str()) {
            return Err(format!("Unknown archive placeholder: {}", found.as_str()));
        }
    }
    Ok(())
}
fn source_relative(path: &Path, working_directory: &Path) -> String {
    let source = path.strip_prefix(working_directory).unwrap_or(path);
    let value = source.to_string_lossy().into_owned();
    if value.starts_with('-') {
        format!(".\\{value}")
    } else {
        value
    }
}
pub fn build_archive_create_args(
    options: &ArchiveOptions,
    sources: &[PathBuf],
    archive: &Path,
    working_directory: &Path,
    template: Option<&str>,
) -> Result<Vec<String>, String> {
    let ext = options.extension.to_ascii_lowercase();
    let source_args: Vec<String> = sources
        .iter()
        .map(|p| source_relative(p, working_directory))
        .collect();
    let archive = archive.to_string_lossy().into_owned();
    if let Some(template) = template.filter(|s| !s.trim().is_empty()) {
        let tokens = parse_windows_args(template)?;
        if count_token(&tokens, "{archive}") != 1 || count_token(&tokens, "{sources}") != 1 {
            return Err("Use {archive} and {sources} once as separate arguments".into());
        }
        let mut out: Vec<String> = Vec::new();
        for token in tokens {
            if token == "{sources}" {
                out.extend(source_args.iter().cloned());
                continue;
            }
            known_placeholders(
                &token,
                &[
                    "{archive}",
                    "{extension}",
                    "{level}",
                    "{password}",
                    "{recursive}",
                    "{volumeBytes}",
                    "{volumeMiB}",
                    "{volumeKiB}",
                ],
            )?;
            if token.contains("{sources}") || (token.contains("{archive}") && token != "{archive}")
            {
                return Err("Path placeholders must be separate arguments".into());
            }
            if token.contains("{password}") && options.password.is_empty() {
                if token == "{password}"
                    && out
                        .last()
                        .is_some_and(|s| ["-p", "-g"].contains(&s.as_str()))
                {
                    out.pop();
                }
                continue;
            }
            if ["{volumeBytes}", "{volumeMiB}", "{volumeKiB}"]
                .iter()
                .any(|p| token.contains(p))
                && options.volume_bytes.is_none()
            {
                if token.starts_with("{volume") && out.last().is_some_and(|s| s == "-v") {
                    out.pop();
                }
                continue;
            }
            let mut value = token
                .replace("{archive}", &archive)
                .replace("{extension}", &ext)
                .replace("{level}", &options.level.to_string())
                .replace("{password}", &options.password)
                .replace("{recursive}", if options.recursive { "-r" } else { "" });
            if let Some(bytes) = options.volume_bytes {
                value = value
                    .replace("{volumeBytes}", &bytes.to_string())
                    .replace(
                        "{volumeMiB}",
                        &bytes.div_ceil(1024 * 1024).max(1).to_string(),
                    )
                    .replace("{volumeKiB}", &bytes.div_ceil(1024).max(1).to_string());
            }
            if !value.is_empty() {
                out.push(value);
            }
        }
        return Ok(out);
    }
    let mut out = match ext.as_str() {
        "rar" => {
            let mut v = vec!["a".into()];
            if options.recursive {
                v.push("-r".into());
            }
            v.push(format!("-m{}", options.level.clamp(0, 5)));
            if !options.password.is_empty() {
                v.push(format!("-p{}", options.password));
            }
            if let Some(bytes) = options.volume_bytes {
                v.push(format!("-v{bytes}b"));
            }
            v
        }
        "arj" => {
            let mut v = vec!["a".into()];
            if options.recursive {
                v.push("-r".into());
            }
            v.push(format!("-m{}", options.level.clamp(0, 4)));
            if !options.password.is_empty() {
                v.push(format!("-g{}", options.password));
            }
            if let Some(bytes) = options.volume_bytes {
                v.push(format!("-v{bytes}"));
            }
            v
        }
        "zip" | "7z" => {
            let mut v = vec![
                "a".into(),
                format!("-t{ext}"),
                if options.recursive {
                    "-r".into()
                } else {
                    "-r-".into()
                },
                format!("-mx={}", options.level.clamp(0, 9)),
            ];
            if !options.password.is_empty() {
                v.push(format!("-p{}", options.password));
                if ext == "zip" {
                    v.push("-mem=AES256".into());
                }
            }
            if let Some(bytes) = options.volume_bytes {
                v.push(format!("-v{bytes}b"));
            }
            v
        }
        "wim" => {
            if !options.password.is_empty() || options.volume_bytes.is_some() {
                return Err("WIM does not support password or split volumes".into());
            }
            vec![
                "a".into(),
                "-twim".into(),
                if options.recursive {
                    "-r".into()
                } else {
                    "-r-".into()
                },
                format!("-mx={}", options.level.clamp(0, 9)),
            ]
        }
        _ => {
            return Err(format!(
                "External archive format .{ext} requires a custom template"
            ));
        }
    };
    out.push(archive);
    out.extend(source_args);
    Ok(out)
}

pub fn build_archive_extract_args(
    archive: &Path,
    destination: &Path,
    password: &str,
    keys: &[String],
    template: Option<&str>,
    stop_switch_parsing: bool,
) -> Result<Vec<String>, String> {
    if !stop_switch_parsing && keys.iter().any(|k| k.starts_with('-')) {
        return Err("Selected archive key resembles an option; enable switch termination".into());
    }
    let archive = archive.to_string_lossy().into_owned();
    let destination = destination.to_string_lossy().into_owned();
    if let Some(template) = template.filter(|s| !s.trim().is_empty()) {
        let tokens = parse_windows_args(template)?;
        if tokens
            .iter()
            .map(|s| s.matches("{archive}").count())
            .sum::<usize>()
            != 1
            || tokens
                .iter()
                .map(|s| s.matches("{destination}").count())
                .sum::<usize>()
                != 1
            || (!keys.is_empty() && count_token(&tokens, "{keys}") != 1)
        {
            return Err("Use {archive}, {destination}, and {keys} once".into());
        }
        let mut out: Vec<String> = Vec::new();
        for token in tokens {
            if token.contains("{keys}") && token != "{keys}" {
                return Err("{keys} must be a separate argument".into());
            }
            if token == "{keys}" {
                if !keys.is_empty() {
                    if stop_switch_parsing {
                        out.push("--".into());
                    }
                    out.extend(keys.iter().cloned());
                }
                continue;
            }
            known_placeholders(&token, &["{archive}", "{destination}", "{password}"])?;
            if token.contains("{password}") && password.is_empty() {
                if token == "{password}"
                    && out
                        .last()
                        .is_some_and(|s| ["-p", "-g"].contains(&s.as_str()))
                {
                    out.pop();
                }
                continue;
            }
            let value = token
                .replace("{archive}", &archive)
                .replace("{destination}", &destination)
                .replace("{password}", password);
            if !value.is_empty() {
                out.push(value);
            }
        }
        return Ok(out);
    }
    let mut out = vec!["x".into(), archive, format!("-o{destination}"), "-y".into()];
    if !password.is_empty() {
        out.push(format!("-p{password}"));
    }
    if !keys.is_empty() {
        if stop_switch_parsing {
            out.push("--".into());
        }
        out.extend(keys.iter().cloned());
    }
    Ok(out)
}
