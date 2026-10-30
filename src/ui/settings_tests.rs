//! C# SettingsWindow(Commit 규칙·확장자 색 가져오기/내보내기)와 FunctionKeyEditorWindow/FunctionKeyEditWindow를 Rust UI에서 확인한다.
//! 한국어와 영어만. 격리 프로필과 격리 폴더만 쓰고, 파일 대화 상자는 가짜 함수다.
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
    // 원본 lbExtColor: 같은 색의 확장자를 한 줄("TXT;DOC")로.
    assert_eq!(groups(&mut app), vec![row("#FF0000", &["TXT", "DOC"]), row("#00FF00", &["ZIP"])]);
    // 추가(0x44f360): PREF_TITLE_ADDEXTS → 색 고르기 → 이미 쓰는 색이면 PREF_SETCOLOR / PREF_COLOREXISTS, 예면 합친다.
    COLOR.with(|c| *c.borrow_mut() = Some(0x0000FF));
    app.settings_action(SettingsAction::AddColor, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, app.language_pack.text("RES.PREF_TITLE_ADDEXTS", "?"));
    app.answer_dialog(DialogResult::Text(Some("log;.ini".into())), &ctx);
    let question = app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone())).expect("the merge question");
    assert_eq!(question, (app.language_pack.text("RES.PREF_SETCOLOR", "?"), app.language_pack.text("RES.PREF_COLOREXISTS", "?")));
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    assert_eq!(groups(&mut app)[0], row("#FF0000", &["TXT", "DOC", "LOG", "INI"]));
    // 편집(0x44f7c0): PREF_TITLE_EDITEXTS, 옛 확장자를 지우고 새 확장자를 같은 색으로.
    app.settings_form().unwrap().color_selected = Some(1);
    app.settings_action(SettingsAction::EditColor, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, app.language_pack.text("RES.PREF_TITLE_EDITEXTS", "?"));
    app.answer_dialog(DialogResult::Text(Some("ZIP;7Z".into())), &ctx);
    assert_eq!(groups(&mut app)[1], row("#00FF00", &["ZIP", "7Z"]));
    // 두 번 누르기(0x44ef68): 쓰지 않는 색이면 묻지 않고 바꾼다.
    COLOR.with(|c| *c.borrow_mut() = Some(0xFF0000));
    app.settings_form().unwrap().color_selected = Some(1);
    app.settings_action(SettingsAction::ChangeColor, &ctx);
    assert!(app.msgboxes.is_empty());
    assert_eq!(groups(&mut app)[1], row("#0000FF", &["ZIP", "7Z"]));
    // 삭제(0x44fb98): 묻지 않고 그 줄의 확장자를 모두 지운다. 확인하면 ExtensionColors로 저장.
    app.settings_form().unwrap().color_selected = Some(0);
    app.settings_action(SettingsAction::DeleteColor, &ctx);
    assert_eq!(groups(&mut app), vec![row("#0000FF", &["ZIP", "7Z"])]);
    draw_settings(&mut app, &ctx);
    // 기본 색(lbBasicColor 0x44ed5c): 줄 1 = 파일 목록 배경(NXBACK), 줄 17 = 폴더 트리 배경(NXTBACK). 0·15·16은 색이 없다.
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
    // 원본 btnApply: 적용해도 창은 그대로.
    app.settings_action(SettingsAction::Apply, &ctx);
    assert!(app.settings_form().is_some());
    // 원본 btnListFont(글꼴 변경): 글꼴 상자에서 고른 이름·크기가 줄에, 모양(TFontStyles 바이트)이 ListFontStyle에 들어간다.
    app.font_picker = |name, _, style| { assert_eq!((name, style), ("Segoe UI", 0), "the current font goes in"); Some(("Tahoma".into(), 10.5, 3)) };
    app.settings_action(SettingsAction::ChooseFont(false), &ctx);
    assert_eq!((text_of(&mut app, "ListFont"), text_of(&mut app, "ListFontSize")), ("Tahoma".to_owned(), "10.5".to_owned()));
    assert_eq!(app.settings_form().unwrap().draft.values.get("ListFontStyle"), Some(&serde_json::json!(3)));
    app.font_picker = |_, _, style| { assert_eq!(style, 0); Some(("Gulim".into(), 9.0, 4)) };
    app.settings_action(SettingsAction::ChooseFont(true), &ctx);
    assert_eq!((text_of(&mut app, "FolderTreeFont"), text_of(&mut app, "FolderTreeFontSize")), ("Gulim".to_owned(), "9".to_owned()));
    app.font_picker = |name, _, style| { assert_eq!((name, style), ("Tahoma", 3), "the chosen style comes back in"); None };
    app.settings_action(SettingsAction::ChooseFont(false), &ctx);
    {
        let form = app.settings_form().unwrap();
        // 원본 TfPreferences 쪽(파일목록 … 기타) + C# 판에만 있는 줄의 "NDR" 쪽.
        assert_eq!(form.tab_titles, ["파일목록", "폴더트리", "색", "옵션", "프로그램", "경로", "기능키", "기타", "NDR"]);
        assert_eq!(form.tabs.len(), 9);
        assert!(matches!(&form.tabs[0][1], Row::Text { key, label, .. } if key == "ListFont" && label == "파일 목록:"));
        assert!(matches!(&form.tabs[3][0], Row::Check { key, label, .. } if key == "SelectFolders" && label == "전체선택 할 때 폴더도 모두 선택하기"));
        assert!(form.tabs[2].iter().any(|row| matches!(row, Row::ColorButtons)));
        assert!(form.tabs[6].iter().any(|row| matches!(row, Row::FunctionKeys)));
        assert!(matches!(&form.tabs[8][0], Row::Check { key, label, .. } if key == "DualPaneMode" && label == "두 패널 표시"));
        // 원본 ComboBox 글(값은 그대로): cbSort1.
        assert!(form.tabs[0].iter().any(|row| matches!(row, Row::Combo { key, choices, .. } if key == "LeftSortMode" && choices[1] == ("Name".to_owned(), "이름순".to_owned()))));
        // 원본 cbColumn1: "자동;1열;…;6열", NDR만의 7~19열은 같은 꼴.
        assert!(form.tabs[0].iter().any(|row| matches!(row, Row::Combo { key, choices, .. } if key == "LeftColumnCount"
            && choices[0] == ("Auto".to_owned(), "자동".to_owned()) && choices[6] == ("6".to_owned(), "6열".to_owned()) && choices[19] == ("19".to_owned(), "19열".to_owned()))));
        // 모든 줄이 한 번씩만 있다.
        let mut keys: Vec<String> = form.tabs.iter().flatten().filter_map(|row| match row {
            Row::Check { key, .. } | Row::Text { key, .. } | Row::Folder { key, .. } | Row::Program { key, .. } | Row::Combo { key, .. } | Row::Slider { key, .. } => Some(key.clone()),
            _ => None,
        }).collect();
        let count = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), count, "no duplicated rows");
    }
    // 활성화 단축키: 한 글자 영문/숫자만.
    set_text(&mut app, "ExplorerShortcutKey", "ab");
    app.settings_action(SettingsAction::Ok, &ctx);
    assert_eq!(app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone())), Some(("환경 설정".to_owned(), "활성화 단축키에는 영문자 또는 숫자 한 글자만 입력하세요.".to_owned())));
    app.answer_message_box(1, &ctx);
    assert!(app.settings_form().is_some(), "an invalid value keeps the window open");
    set_text(&mut app, "ExplorerShortcutKey", "w");
    // 범위·변환 규칙.
    set_text(&mut app, "ListLineSpace", "99");
    set_text(&mut app, "MaxRecentFileCount", "5000");
    // 원본 chkScrollStep1: "페이지 단위 스크롤"을 끄면 1(줄 단위), 켜져 있으면 2.
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
    // 외부 압축 프로그램 줄은 없다(사용자 요청).
    assert!(app.settings_form().unwrap().row_mut("Archive:zip").is_none() && app.settings_form().unwrap().row_mut("UseExternalArchivePrograms").is_none());
    set_text(&mut app, "CleanupLocations", "%TEMP%; ");
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
    let temp = std::path::absolute(native::expand_environment("%TEMP%")).unwrap().to_string_lossy().into_owned();
    assert_eq!(s.strings("UserDefinedCleanupLocations"), [temp]);
    assert_eq!(s.strings("WorkingFolders")[0], "C:\\work");
    assert_eq!((s.values["LeftColumnCount"].as_i64(), s.values["EscapeKeyAction"].as_i64(), s.values["PathClickMode"].as_i64()), (Some(3), Some(3), Some(1)));
    assert_eq!(s.values["NavigateOnPathClick"], true);
    // 잘못된 확장자 색은 경고 상자, 취소는 아무것도 바꾸지 않는다.
    app.dispatch("options", &ctx);
    set_text(&mut app, "ExtensionColors", "zip=red");
    app.settings_action(SettingsAction::Ok, &ctx);
    assert_eq!(app.msgboxes.first().map(|m| m.text.clone()), Some("확장자 색상 규칙은 .확장자=#RRGGBB 형식이어야 합니다: zip=red".to_owned()));
    app.answer_message_box(1, &ctx);
    set_combo(&mut app, "EscapeKeyAction", "Disabled");
    app.settings_action(SettingsAction::Cancel, &ctx);
    assert_eq!(app.settings.values["EscapeKeyAction"].as_i64(), Some(3), "Cancel keeps the settings");
    // 색상표 가져오기(.json)와 내보내기(.col).
    app.dispatch("options", &ctx);
    let json = root.join("colors.json");
    fs::write(&json, r##"{".log": "#00FF00"}"##).unwrap();
    PICK.with(|p| *p.borrow_mut() = Some(json));
    app.settings_action(SettingsAction::ImportColors, &ctx);
    assert_eq!(text_of(&mut app, "ExtensionColors"), ".log=#00FF00");
    let col = root.join("out.col");
    PICK.with(|p| *p.borrow_mut() = Some(col.clone()));
    app.settings_action(SettingsAction::ExportColors, &ctx);
    let written = fs::read_to_string(&col).unwrap();
    assert!(written.starts_with("[NEXUSFILECOLOR5]\r\n") && written.contains("[Ext]\r\nLOG=00FF00"), "{written}");
    app.settings_action(SettingsAction::Cancel, &ctx);
    fs::remove_dir_all(root).unwrap();
}

/// 원본 ParameterClick(0x41fed0): 항목 글의 첫 ")"까지를 커서 자리에 넣고 커서를 그 뒤로 옮긴다.
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
        // 원본 tsFunctionKey lvFunctionKey: 환경 설정 창 안의 목록.
        let editor = &mut app.settings_form().unwrap().function_keys;
        assert_eq!(editor.row(4), ["F5".to_owned(), "F5".to_owned(), "Internal".to_owned(), "#Refresh".to_owned(), String::new()], "legacy FunctionKeys are read");
        assert_eq!(editor.row(8)[2], "User command");
        assert_eq!(editor.row(0)[2], "Default");
        // 원본 TfPreferences 목록의 ^F1~^F11(Ctrl+F1~F11)이 F12 뒤에 온다.
        assert_eq!(editor.bindings.len(), 23);
        assert_eq!(editor.row(12)[0], "^F1");
        assert_eq!(editor.row(14)[..4], ["^F3".to_owned(), "^F3".to_owned(), "User command".to_owned(), "calc.exe".to_owned()]);
        assert_eq!(editor.row(22)[0], "^F11");
        editor.grid.click(1, egui::Modifiers::NONE, false);
    }
    // F2 편집: 사용자 명령. 원본 btnOkClick은 이름이 비면 알리지 않고 명령 파일 이름(확장자 뺌)을 쓴다.
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
    if let Some(DialogKind::FunctionKeyEdit(edit)) = app.original_dialogs.last_mut().map(|d| &mut d.kind) {
        (edit.name, edit.internal, edit.command, edit.parameters) = (" Notes ".into(), false, " notepad.exe ".into(), " %P ".into());
    }
    app.function_key_edit_done(true, &ctx);
    // F5 초기화 후 확인 → 설정 사본의 FunctionKeyBindings, 옛 F1~F12는 지운다.
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

/// 원본 tsFunctionKey lvFunctionKey OnKeyDown: Enter = 편집(확인이 아니다), Delete = 삭제.
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

/// 원본 [FuncKey] "^F1"~"^F11"(TfPreferences 목록 0x44cd58의 "^F%d"): Ctrl+F3이 "^F3" 지정을 실행한다. F3만 누르면 실행하지 않는다.
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
    // C# 판 grid.FontSize = ListFontSize: 목록에만. 메뉴·창 등 나머지 글은 12화소 그대로.
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

/// 원본 ListFont "\"이름\",크기,모양": 모양(TFontStyles 바이트)의 굵게·기울임은 그 꼴의 글꼴 파일, 밑줄·취소선은 목록 글에 입힌다.
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
    // 밑줄만 바뀌면 글꼴 파일은 다시 찾지 않는다.
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
    // 원본 SetLineSpace(+0x364): 0~32화소.
    assert_eq!(app.list_line_space(), 0.0);
    app.settings.values.insert("ListLineSpace".into(), serde_json::json!(6));
    assert_eq!(app.list_line_space(), 6.0);
    app.settings.values.insert("ListLineSpace".into(), serde_json::json!(99));
    assert_eq!(app.list_line_space(), 32.0);
    // 선 색은 GetBasic(13) NXLINE(색상표를 쓸 때).
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
    let _ = fs::remove_dir_all(root);
}


#[test]
fn applying_options_on_a_fresh_profile_keeps_default_on_settings() {
    let (root, ctx, mut app) = fixture("fresh-defaults", "ko");
    // C# NdrSettings·원본 ini 처음 값이 켬인 설정은 창에서 그대로 적용해도 켬으로 남는다(전에는 창의 처음 값이 꺼짐이라 적용하면 꺼졌다).
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
