//! Checks C# SettingsWindow (Commit rules, extension color import/export) and FunctionKeyEditorWindow/FunctionKeyEditWindow in the Rust UI.
//! Korean and English only. Only an isolated profile and isolated folders are used; file dialogs are fakes.
use super::original_dialogs::{DialogKind, DialogResult};
use super::settings_window::{Row, SettingsAction};
use super::*;
use std::{cell::RefCell, fs};

thread_local! {
    static PICK: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static COLOR: RefCell<Option<u32>> = const { RefCell::new(None) };
}
fn pick_color(_: u32) -> Option<u32> {
    COLOR.with(|c| *c.borrow())
}
fn open_pick(_: &str, _: Option<&Path>, _: &str, _: &[(&str, &str)]) -> Option<PathBuf> {
    PICK.with(|p| p.borrow().clone())
}
fn save_pick(_: &str, _: Option<&Path>, _: &str, _: &[(&str, &str)], _: &str, _: bool) -> Option<PathBuf> {
    PICK.with(|p| p.borrow().clone())
}

fn fixture(label: &str, language: &str) -> (PathBuf, egui::Context, App) {
    let root = std::env::temp_dir().join(format!("ndr-settings-{}-{}-{label}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(root.join("left")).unwrap();
    fs::create_dir_all(root.join("right")).unwrap();
    let ctx = egui::Context::default();
    let options = Options { profile: Some(root.join("profile")), left: Some(root.join("left")), right: Some(root.join("right")), language: Some(language.into()), ..Default::default() };
    let mut app = App::from_context(&ctx, options);
    app.open_dialog = open_pick;
    app.save_dialog = save_pick;
    app.folder_picker = |_, _| None;
    (root, ctx, app)
}
fn set_text(app: &mut App, key: &str, text: &str) {
    match app.settings_form().unwrap().row_mut(key) {
        Some(Row::Text { value, .. } | Row::Folder { value, .. } | Row::Program { value, .. }) => *value = text.into(),
        other => panic!("{key}: {other:?}"),
    }
}
fn set_combo(app: &mut App, key: &str, wanted: &str) {
    match app.settings_form().unwrap().row_mut(key) {
        Some(Row::Combo { choices, selected, .. }) => *selected = choices.iter().position(|(v, _)| v == wanted),
        other => panic!("{key}: {other:?}"),
    }
}
fn text_of(app: &mut App, key: &str) -> String {
    match app.settings_form().unwrap().row_mut(key) {
        Some(Row::Text { value, .. }) => value.clone(),
        other => panic!("{key}: {other:?}"),
    }
}

#[test]
fn extension_colors_follow_the_original_color_page() {
    let (root, ctx, mut app) = fixture("colors", "en");
    app.color_picker = pick_color;
    app.dispatch("options", &ctx);
    set_text(&mut app, "ExtensionColors", ".txt=#FF0000; .doc=#FF0000; .zip=#00FF00");
    let groups = |app: &mut App| super::settings_window::color_groups(&text_of(app, "ExtensionColors"));
    let row = |color: &str, names: &[&str]| (color.to_owned(), names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>());
    // Original lbExtColor: extensions of the same color on one row ("TXT;DOC").
    assert_eq!(groups(&mut app), vec![row("#FF0000", &["TXT", "DOC"]), row("#00FF00", &["ZIP"])]);
    // Add (0x44f360): PREF_TITLE_ADDEXTS -> choose color -> for a color already in use PREF_SETCOLOR / PREF_COLOREXISTS; yes merges them.
    COLOR.with(|c| *c.borrow_mut() = Some(0x0000FF));
    app.settings_action(SettingsAction::AddColor, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, app.language_pack.text("RES.PREF_TITLE_ADDEXTS", "?"));
    app.answer_dialog(DialogResult::Text(Some("log;.ini".into())), &ctx);
    let question = app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone())).expect("the merge question");
    assert_eq!(question, (app.language_pack.text("RES.PREF_SETCOLOR", "?"), app.language_pack.text("RES.PREF_COLOREXISTS", "?")));
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    assert_eq!(groups(&mut app)[0], row("#FF0000", &["TXT", "DOC", "LOG", "INI"]));
    // Edit (0x44f7c0): PREF_TITLE_EDITEXTS removes the old extensions and adds the new ones with the same color.
    app.settings_form().unwrap().color_selected = Some(1);
    app.settings_action(SettingsAction::EditColor, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, app.language_pack.text("RES.PREF_TITLE_EDITEXTS", "?"));
    app.answer_dialog(DialogResult::Text(Some("ZIP;7Z".into())), &ctx);
    assert_eq!(groups(&mut app)[1], row("#00FF00", &["ZIP", "7Z"]));
    // Double click (0x44ef68): an unused color changes without asking.
    COLOR.with(|c| *c.borrow_mut() = Some(0xFF0000));
    app.settings_form().unwrap().color_selected = Some(1);
    app.settings_action(SettingsAction::ChangeColor, &ctx);
    assert!(app.msgboxes.is_empty());
    assert_eq!(groups(&mut app)[1], row("#0000FF", &["ZIP", "7Z"]));
    // Delete (0x44fb98): removes all extensions of that row without asking. OK saves to ExtensionColors.
    app.settings_form().unwrap().color_selected = Some(0);
    app.settings_action(SettingsAction::DeleteColor, &ctx);
    assert_eq!(groups(&mut app), vec![row("#0000FF", &["ZIP", "7Z"])]);
    draw_settings(&mut app, &ctx);
    // Base colors (lbBasicColor 0x44ed5c): row 1 = file list background (NXBACK), row 17 = folder tree background (NXTBACK). Rows 0, 15, 16 have no color.
    assert_eq!([0, 1, 14, 15, 16, 17, 22].map(super::settings_window::basic_color_index), [None, Some(0), Some(13), None, None, Some(14), Some(19)]);
    COLOR.with(|c| *c.borrow_mut() = Some(0x00336699));
    app.settings_action(SettingsAction::ChangeBasicColor(14), &ctx);
    assert_eq!(super::settings_window::basic_color(&app.settings_form().unwrap().draft, 14).as_deref(), Some("996633"));
    assert!(app.settings_form().unwrap().draft.flag("UseNexusFileColorMap", false));
    app.settings_action(SettingsAction::Ok, &ctx);
    assert_eq!(app.settings.values["NexusFileColorMapValues"]["NXTBACK"].as_str(), Some("996633"));
    let saved = app.settings.values.get("ExtensionColors").and_then(|v| v.as_object()).cloned().unwrap_or_default();
    assert_eq!(saved.get(".zip").and_then(|v| v.as_str()), Some("#0000FF"));
    assert_eq!(saved.get(".7z").and_then(|v| v.as_str()), Some("#0000FF"));
    assert!(!saved.contains_key(".txt"));
    fs::remove_dir_all(root).unwrap();
}
fn draw_settings(app: &mut App, ctx: &egui::Context) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let form = app.settings_form().unwrap();
            form.tab = 2;
            let _ = super::settings_window::settings_ui(ui, form, false, false);
        });
    });
}

#[test]
fn settings_window_commits_like_csharp() {
    let (root, ctx, mut app) = fixture("ko", "ko");
    app.dispatch("options", &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "옵션", "original [formPreferences] Form.Caption");
    // Original btnApply: applying keeps the window open.
    app.settings_action(SettingsAction::Apply, &ctx);
    assert!(app.settings_form().is_some());
    // Original btnListFont (change font): the name and size chosen in the font box go to the row, the style (TFontStyles byte) to ListFontStyle.
    // The list size field is pixels (initially 14 = 10.5 points), the font box points: choosing 10.5 points gives 14 pixels.
    app.font_picker = |name, size, style| { assert_eq!((name, size, style), ("Segoe UI", 10.5, 0), "the current font goes in as points"); Some(("Tahoma".into(), 12.0, 3)) };
    app.settings_action(SettingsAction::ChooseFont(false), &ctx);
    assert_eq!((text_of(&mut app, "ListFont"), text_of(&mut app, "ListFontSize")), ("Tahoma".to_owned(), "16".to_owned()), "12 points = 16 pixels");
    assert_eq!(app.settings_form().unwrap().draft.values.get("ListFontStyle"), Some(&serde_json::json!(3)));
    app.font_picker = |_, size, style| { assert_eq!((size, style), (9.0, 0), "the folder tree size stays in points"); Some(("Gulim".into(), 9.0, 4)) };
    app.settings_action(SettingsAction::ChooseFont(true), &ctx);
    assert_eq!((text_of(&mut app, "FolderTreeFont"), text_of(&mut app, "FolderTreeFontSize")), ("Gulim".to_owned(), "9".to_owned()));
    app.font_picker = |name, size, style| { assert_eq!((name, size, style), ("Tahoma", 12.0, 3), "the chosen font comes back in"); None };
    app.settings_action(SettingsAction::ChooseFont(false), &ctx);
    {
        let form = app.settings_form().unwrap();
        // Original TfPreferences pages (file list ... other) + the "NDR" page for rows only in the C# version.
        assert_eq!(form.tab_titles, ["파일목록", "폴더트리", "색", "옵션", "프로그램", "경로", "기능키", "기타", "NDR"]);
        assert_eq!(form.tabs.len(), 9);
        assert!(matches!(&form.tabs[0][1], Row::Text { key, label, .. } if key == "ListFont" && label == "파일 목록:"));
        assert!(matches!(&form.tabs[3][0], Row::Check { key, label, .. } if key == "SelectFolders" && label == "전체선택 할 때 폴더도 모두 선택하기"));
        assert!(form.tabs[2].iter().any(|row| matches!(row, Row::ColorButtons)));
        assert!(form.tabs[6].iter().any(|row| matches!(row, Row::FunctionKeys)));
        assert!(matches!(&form.tabs[8][0], Row::Check { key, label, .. } if key == "DualPaneMode" && label == "두 패널 표시"));
        // Original ComboBox text (values unchanged): cbSort1.
        assert!(form.tabs[0].iter().any(|row| matches!(row, Row::Combo { key, choices, .. } if key == "LeftSortMode" && choices[1] == ("Name".to_owned(), "이름순".to_owned()))));
        // Original cbColumn1: "자동;1열;...;6열", and the NDR-only 7-19 columns use the same form.
        assert!(form.tabs[0].iter().any(|row| matches!(row, Row::Combo { key, choices, .. } if key == "LeftColumnCount"
            && choices[0] == ("Auto".to_owned(), "자동".to_owned()) && choices[6] == ("6".to_owned(), "6열".to_owned()) && choices[19] == ("19".to_owned(), "19열".to_owned()))));
        // Every row appears exactly once.
        let mut keys: Vec<String> = form.tabs.iter().flatten().filter_map(|row| match row {
            Row::Check { key, .. } | Row::Text { key, .. } | Row::Folder { key, .. } | Row::Program { key, .. } | Row::Combo { key, .. } | Row::Slider { key, .. } => Some(key.clone()),
            _ => None,
        }).collect();
        let count = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), count, "no duplicated rows");
    }
    // Activation hotkey: one English letter or digit only.
    set_text(&mut app, "ExplorerShortcutKey", "ab");
    app.settings_action(SettingsAction::Ok, &ctx);
    assert_eq!(app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone())), Some(("환경 설정".to_owned(), "활성화 단축키에는 영문자 또는 숫자 한 글자만 입력하세요.".to_owned())));
    app.answer_message_box(1, &ctx);
    assert!(app.settings_form().is_some(), "an invalid value keeps the window open");
    set_text(&mut app, "ExplorerShortcutKey", "w");
    // Range and conversion rules.
    set_text(&mut app, "ListLineSpace", "99");
    set_text(&mut app, "MaxRecentFileCount", "5000");
    // Original chkScrollStep1: turning "scroll by page" off gives 1 (by line), on gives 2.
    let scroll_label = app.language_pack.text("formPreferences.Caption.chkScrollStep1", "?");
    match app.settings_form().unwrap().row_mut("LeftScrollStep") {
        Some(Row::Check { value, label, .. }) => {
            assert!(*value, "the original default 2 (page) shows checked");
            assert_eq!(*label, scroll_label);
            *value = false;
        }
        other => panic!("LeftScrollStep: {other:?}"),
    }
    set_text(&mut app, "FolderTreeDepth", "abc");
    set_text(&mut app, "ExtensionColors", "zip=#abcdef; *.TXT=#112233");
    set_text(&mut app, "SpecialFolder:Music", "D:\\Music");
    // There is no external archiver row (user request).
    assert!(app.settings_form().unwrap().row_mut("Archive:zip").is_none() && app.settings_form().unwrap().row_mut("UseExternalArchivePrograms").is_none());
    // There is no Disk Cleanup field either (Disk Cleanup was removed per user request).
    assert!(app.settings_form().unwrap().row_mut("CleanupLocations").is_none() && app.settings_form().unwrap().row_mut("UserDefinedCleanupExtensions").is_none());
    set_text(&mut app, "WF0", "  C:\\work  ");
    set_combo(&mut app, "LeftColumnCount", "3");
    set_combo(&mut app, "EscapeKeyAction", "3");
    set_combo(&mut app, "PathClickMode", "1");
    let depth_before = app.settings.values.get("FolderTreeDepth").cloned();
    app.settings_action(SettingsAction::Ok, &ctx);
    assert!(app.original_dialogs.is_empty(), "OK closes the window");
    let s = &app.settings;
    assert_eq!(s.text("ExplorerShortcutKey", ""), "W");
    assert_eq!((s.values["ListLineSpace"].as_i64(), s.values["MaxRecentFileCount"].as_i64(), s.values["LeftScrollStep"].as_i64()), (Some(32), Some(1000), Some(1)));
    assert_eq!(s.values.get("FolderTreeDepth").cloned(), depth_before, "an unparsable number keeps the old value");
    assert_eq!(s.values["ExtensionColors"], serde_json::json!({".zip": "#ABCDEF", ".txt": "#112233"}));
    assert_eq!(s.values["SpecialFolderPaths"]["Music"], "D:\\Music");
    assert_eq!(s.strings("WorkingFolders")[0], "C:\\work");
    assert_eq!((s.values["LeftColumnCount"].as_i64(), s.values["EscapeKeyAction"].as_i64(), s.values["PathClickMode"].as_i64()), (Some(3), Some(3), Some(1)));
    assert_eq!(s.values["NavigateOnPathClick"], true);
    // An invalid extension color shows a warning box; cancel changes nothing.
    app.dispatch("options", &ctx);
    set_text(&mut app, "ExtensionColors", "zip=red");
    app.settings_action(SettingsAction::Ok, &ctx);
    assert_eq!(app.msgboxes.first().map(|m| m.text.clone()), Some("확장자 색상 규칙은 .확장자=#RRGGBB 형식이어야 합니다: zip=red".to_owned()));
    app.answer_message_box(1, &ctx);
    set_combo(&mut app, "EscapeKeyAction", "Disabled");
    app.settings_action(SettingsAction::Cancel, &ctx);
    assert_eq!(app.settings.values["EscapeKeyAction"].as_i64(), Some(3), "Cancel keeps the settings");
    // Import a color table (.json) and export it (.col).
    app.dispatch("options", &ctx);
    let json = root.join("colors.json");
    fs::write(&json, r##"{".log": "#00FF00"}"##).unwrap();
    PICK.with(|p| *p.borrow_mut() = Some(json));
    app.settings_action(SettingsAction::ImportColors, &ctx);
    assert_eq!(text_of(&mut app, "ExtensionColors"), ".log=#00FF00");
    let col = root.join("out.col");
    PICK.with(|p| *p.borrow_mut() = Some(col.clone()));
    app.settings_action(SettingsAction::ExportColors, &ctx);
    let bytes = fs::read(&col).unwrap();
    assert!(bytes.starts_with(&[0xFF, 0xFE]), "Unicode with a BOM like the original SaveToFile");
    let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let written = String::from_utf16(&units).unwrap();
    assert!(written.starts_with("[NEXUSFILECOLOR5]\r\nNXBACK=") && written.contains("[Ext]\r\nLOG=00FF00"), "{written}");
    app.settings_action(SettingsAction::Cancel, &ctx);
    fs::remove_dir_all(root).unwrap();
}

/// Original ParameterClick (0x41fed0): inserts the item text up to its first ")" at the cursor and moves the cursor after it.
#[test]
fn function_key_parameter_menu_inserts_macros_like_the_original() {
    use super::function_key_editor::{PARAMETER_MENU, insert_parameter};
    assert_eq!(PARAMETER_MENU.len(), 18);
    let mut text = String::from("/a 끝");
    let caret = insert_parameter(&mut text, 3, PARAMETER_MENU[0]);
    assert_eq!((text.as_str(), caret), ("/a $(SELECTED)끝", 14));
    let caret = insert_parameter(&mut text, caret, PARAMETER_MENU[4]);
    assert_eq!((text.as_str(), caret), ("/a $(SELECTED)$(PATH1)끝", 22));
    let caret = insert_parameter(&mut text, 999, PARAMETER_MENU[15]);
    assert_eq!((text.as_str(), caret), ("/a $(SELECTED)$(PATH1)끝$(EXTENSION)", 35));
}

#[test]
fn function_key_editor_follows_csharp_in_english() {
    let (root, ctx, mut app) = fixture("en", "en");
    app.settings.values.insert("FunctionKeys".into(), serde_json::json!({"F5": "#Refresh", "F9": "notepad.exe", "^F3": "calc.exe"}));
    app.dispatch("options", &ctx);
    {
        // Original tsFunctionKey lvFunctionKey: the list inside the preferences window.
        let editor = &mut app.settings_form().unwrap().function_keys;
        assert_eq!(editor.row(4), ["F5".to_owned(), "F5".to_owned(), "Internal".to_owned(), "#Refresh".to_owned(), String::new()], "legacy FunctionKeys are read");
        assert_eq!(editor.row(8)[2], "User command");
        assert_eq!(editor.row(0)[2], "Default");
        // ^F1-^F11 (Ctrl+F1-F11) of the original TfPreferences list come after F12.
        assert_eq!(editor.bindings.len(), 23);
        assert_eq!(editor.row(12)[0], "^F1");
        assert_eq!(editor.row(14)[..4], ["^F3".to_owned(), "^F3".to_owned(), "User command".to_owned(), "calc.exe".to_owned()]);
        assert_eq!(editor.row(22)[0], "^F11");
        editor.grid.click(1, egui::Modifiers::NONE, false);
    }
    // Edit F2: user command. Original btnOkClick silently uses the command file name (without extension) when the name is empty.
    app.settings_action(SettingsAction::EditFunctionKey, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "Edit Function Key — F2");
    if let Some(DialogKind::FunctionKeyEdit(edit)) = app.original_dialogs.last_mut().map(|d| &mut d.kind) {
        assert_eq!(edit.name, "F2");
        edit.set_internal(true);
        assert!(edit.name.is_empty() && edit.command.is_empty(), "rbCommandTypeClick clears the fields");
        edit.set_internal(false);
        (edit.name, edit.command, edit.parameters) = ("  ".into(), r"C:\Tools\Notes.exe".into(), " %P ".into());
        assert_eq!(edit.result().unwrap().name, "Notes");
    }
    assert!(app.msgboxes.is_empty());
    // Original cbCommandChange (0x41fc30): for a user command, text typed in the command field goes into the name field as is.
    {
        let mut form = super::function_key_editor::FunctionKeyEditForm::new("F9", &Default::default(), false);
        form.internal = false;
        let id = egui::Id::new("ndr-function-key-command");
        let run = |events: Vec<egui::Event>, form: &mut super::function_key_editor::FunctionKeyEditForm| {
            let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| super::function_key_editor::function_key_edit_ui(ui, form, &app.language_pack, false));
            });
        };
        run(vec![], &mut form);
        ctx.memory_mut(|m| m.request_focus(id));
        run(vec![], &mut form);
        run(vec![egui::Event::Text("calc.exe".into())], &mut form);
        assert_eq!((form.command.as_str(), form.name.as_str()), ("calc.exe", "calc.exe"));
    }
    if let Some(DialogKind::FunctionKeyEdit(edit)) = app.original_dialogs.last_mut().map(|d| &mut d.kind) {
        (edit.name, edit.internal, edit.command, edit.parameters) = (" Notes ".into(), false, " notepad.exe ".into(), " %P ".into());
    }
    app.function_key_edit_done(true, &ctx);
    // After resetting F5, OK -> FunctionKeyBindings of the settings copy; the old F1-F12 are removed.
    app.settings_form().unwrap().function_keys.grid.click(4, egui::Modifiers::NONE, false);
    app.settings_action(SettingsAction::ResetFunctionKey, &ctx);
    app.settings_action(SettingsAction::Ok, &ctx);
    assert_eq!(app.settings.values["FunctionKeyBindings"], serde_json::json!({
        "F2": {"Name": "Notes", "Command": "notepad.exe", "Parameters": "%P", "IsInternalCommand": false},
        "F9": {"Name": "F9", "Command": "notepad.exe", "Parameters": "", "IsInternalCommand": false},
        "^F3": {"Name": "^F3", "Command": "calc.exe", "Parameters": "", "IsInternalCommand": false}
    }));
    assert_eq!(app.settings.values["FunctionKeys"], serde_json::json!({}));
    fs::remove_dir_all(root).unwrap();
}

/// Original tsFunctionKey lvFunctionKey OnKeyDown: Enter = edit (not OK), Delete = delete.
#[test]
fn function_key_tab_keys_follow_the_original_list() {
    let (root, ctx, mut app) = fixture("ko", "ko");
    app.dispatch("options", &ctx);
    let mut actions = Vec::new();
    for (enter, delete) in [(true, false), (false, true)] {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let form = app.settings_form().unwrap();
                form.tab = 6;
                actions.push(super::settings_window::settings_ui(ui, form, enter, delete));
            });
        });
    }
    assert_eq!(actions, [Some(SettingsAction::EditFunctionKey), Some(SettingsAction::ResetFunctionKey)]);
    fs::remove_dir_all(root).unwrap();
}

thread_local! {
    static CARET_RUNS: std::cell::RefCell<Vec<(PathBuf, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn record_caret_run(path: &Path, arguments: &str, _: &Path) -> Result<(), String> {
    CARET_RUNS.with(|r| r.borrow_mut().push((path.to_path_buf(), arguments.to_owned())));
    Ok(())
}

/// Original [FuncKey] "^F1"-"^F11" ("^F%d" of the TfPreferences list 0x44cd58): Ctrl+F3 runs the "^F3" assignment. F3 alone does not.
#[test]
fn ctrl_function_keys_run_the_original_caret_bindings() {
    let (root, ctx, mut app) = fixture("ctrl-f", "ko");
    app.shell_runner = record_caret_run;
    app.settings.values.insert("FunctionKeys".into(), serde_json::json!({"^F3": "calc.exe"}));
    let ctrl = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
    assert!(app.shortcut_key(egui::Key::F3, ctrl, &ctx));
    assert_eq!(CARET_RUNS.with(|r| r.borrow().clone()), vec![(PathBuf::from("calc.exe"), String::new())]);
    CARET_RUNS.with(|r| r.borrow_mut().clear());
    let _ = app.shortcut_key(egui::Key::F3, egui::Modifiers::NONE, &ctx);
    assert!(CARET_RUNS.with(|r| r.borrow().is_empty()), "plain F3 keeps its own meaning");
    let _ = fs::remove_dir_all(root);
}


#[test]
fn list_font_size_only_sizes_the_file_list_like_csharp() {
    let (root, ctx, mut app) = fixture("list-font-size", "en");
    // C# version grid.FontSize = ListFontSize: lists only. Other text such as menus and windows stays at 12 px.
    app.settings.values.insert("ListFontSize".into(), serde_json::json!(20));
    app.apply_preferences(&ctx);
    assert_eq!(app.list_font_size, 20.0);
    let body = ctx.style().text_styles[&egui::TextStyle::Body].size;
    assert_eq!(body, super::BASE_TEXT_SIZE);
    app.settings.values.insert("ListFontSize".into(), serde_json::json!(99));
    app.apply_preferences(&ctx);
    assert_eq!(app.list_font_size, 32.0);
    let _ = fs::remove_dir_all(root);
}

thread_local! {
    static FONT_LOOKUPS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Original ListFont "\"name\",size,style": bold and italic of the style (TFontStyles byte) pick that face's font file, underline and strikeout are applied to list text.
#[test]
fn list_font_style_picks_the_styled_font_file_like_the_original() {
    let (root, ctx, mut app) = fixture("list-font-style", "en");
    app.system.font_file = |name| {
        FONT_LOOKUPS.with(|l| l.borrow_mut().push(name.to_owned()));
        None
    };
    app.settings.set_text("ListFont", "Tahoma");
    app.settings.values.insert("ListFontStyle".into(), serde_json::json!(1 | 2 | 4));
    app.apply_preferences(&ctx);
    assert_eq!(FONT_LOOKUPS.with(|l| l.take()), ["Tahoma Bold Italic", "Tahoma Bold", "Tahoma Italic", "Tahoma"]);
    assert_eq!(app.list_font_style, 7);
    // When only the underline changes, the font file is not looked up again.
    app.settings.values.insert("ListFontStyle".into(), serde_json::json!(3 | 8));
    app.apply_preferences(&ctx);
    assert!(FONT_LOOKUPS.with(|l| l.take()).is_empty());
    assert_eq!(app.list_font_style, 11);
    let mut job = egui::text::LayoutJob::default();
    super::list_text("a", 4 | 8).append_to(&mut job, &ctx.style(), egui::FontSelection::Default, egui::Align::LEFT);
    assert!(job.sections[0].format.underline.width > 0.0 && job.sections[0].format.strikethrough.width > 0.0);
    assert_eq!(ndr::native::styled_font_names("Segoe UI", 0), ["Segoe UI"]);
    assert_eq!(ndr::native::styled_font_names("Segoe UI", 2), ["Segoe UI Italic", "Segoe UI"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn line_space_and_grid_lines_follow_the_original_file_list() {
    let (root, ctx, mut app) = fixture("grid-lines", "en");
    // Original SetLineSpace (+0x364): 0-32 px.
    assert_eq!(app.list_line_space(), 0.0);
    app.settings.values.insert("ListLineSpace".into(), serde_json::json!(6));
    assert_eq!(app.list_line_space(), 6.0);
    app.settings.values.insert("ListLineSpace".into(), serde_json::json!(99));
    assert_eq!(app.list_line_space(), 32.0);
    // The line color is GetBasic(13) NXLINE (when the color table is used).
    let mut color = None;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let theme = app.grid_line_color(ui);
            app.settings.set_flag("UseNexusFileColorMap", true);
            app.settings.values.insert("NexusFileColorMapValues".into(), serde_json::json!({ "NXLINE": "123456" }));
            color = Some((theme, app.grid_line_color(ui)));
        });
    });
    let (theme, mapped) = color.unwrap();
    assert_eq!(mapped, Color32::from_rgb(0x12, 0x34, 0x56));
    assert_ne!(theme, mapped);
    // Row line position: the center of the original bottom + LineSpace / 2 - 1 pixel row (integer division), always aligned to a screen pixel center (so a 1 px line never smears over two rows
    // with odd line spacing or fractional list positions and scales).
    for ppp in [1.0f32, 1.25, 1.5, 2.0] {
        let raw = egui::RawInput {
            viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(ppp), ..Default::default() })).collect(),
            ..Default::default()
        };
        let mut checked = 0;
        let _ = ctx.run(raw, |ctx| {
            let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
            assert_eq!(painter.pixels_per_point(), ppp);
            for (bottom, space) in [(20.0f32, 0.0f32), (20.0, 1.0), (20.0, 5.0), (20.0, 6.0), (20.3, 5.0), (37.75, 3.0)] {
                let y = App::row_line_y(&painter, bottom, space);
                let physical = y * ppp;
                assert!((physical - (physical.floor() + 0.5)).abs() < 1e-3, "ppp {ppp}, bottom {bottom}, space {space}: y {y} is not a pixel center");
                let expected = bottom + (space / 2.0).floor() - 0.5;
                assert!((y - expected).abs() <= 0.5 / ppp + 1e-3, "ppp {ppp}: y {y} stays at the original row ({expected})");
                checked += 1;
            }
        });
        assert_eq!(checked, 6);
    }
    let raw = egui::RawInput {
        viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(1.0), ..Default::default() })).collect(),
        ..Default::default()
    };
    let mut ys = None;
    let _ = ctx.run(raw, |ctx| {
        let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), egui::Rect::EVERYTHING);
        ys = Some((App::row_line_y(&painter, 20.0, 5.0), App::row_line_y(&painter, 20.0, 6.0), App::row_line_y(&painter, 20.0, 0.0)));
    });
    assert_eq!(ys, Some((21.5, 22.5, 19.5)));
    let _ = fs::remove_dir_all(root);
}


#[test]
fn applying_options_on_a_fresh_profile_keeps_default_on_settings() {
    let (root, ctx, mut app) = fixture("fresh-defaults", "ko");
    // Settings whose C# NdrSettings and original ini default is on stay on when the window is applied unchanged (previously the window default was off, so applying turned them off).
    let keys = ["UseTabs", "ShowFolderDiskInfoBar", "ShowFileInfoBar", "ShowFavoritesBar", "RemoveReadOnlyWhenCopying", "ShowColumnLines", "ShowDeleteButton"];
    for key in keys {
        app.settings.values.remove(key);
    }
    app.dispatch("options", &ctx);
    app.settings_action(SettingsAction::Apply, &ctx);
    for key in keys {
        assert!(app.settings.flag(key, false), "{key} stays on");
    }
    let _ = fs::remove_dir_all(root);
}
