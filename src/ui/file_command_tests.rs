//! C# NDR 이름·설명·열기·삭제·완전 삭제 흐름(원본 FUN_00432378, FUN_00434630, FUN_004351cc, FUN_0040c31c, FUN_00432958,
//! FUN_00431dd4, FUN_0042c734)을 Rust UI에서 확인한다. 한국어와 영어만 검수한다.
//! 격리된 임시 폴더만 쓴다. 휴지통으로 보내는 답(휴지통 방식의 예)은 고르지 않는다. 실제 프로그램도 띄우지 않는다(shell_runner 교체).
use super::original_dialogs::{DeleteConfirmChoice, DeleteWarningChoice, DeleteWarningMode, DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{MB_ICONHAND, MB_OK, MB_YESNO};
use std::{cell::RefCell, fs};

thread_local! {
    static LAUNCHED: RefCell<Vec<(PathBuf, String, PathBuf)>> = const { RefCell::new(Vec::new()) };
}
fn capture_launch(path: &Path, parameters: &str, directory: &Path) -> Result<(), String> {
    LAUNCHED.with(|l| l.borrow_mut().push((path.to_path_buf(), parameters.to_owned(), directory.to_path_buf())));
    Ok(())
}

fn fixture(label: &str, language: &str) -> (PathBuf, egui::Context, App) {
    let root = std::env::temp_dir().join(format!(
        "ndr-files-{}-{}-{label}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(left.join("folder1")).unwrap();
    fs::create_dir_all(right.join("FOLDER1")).unwrap();
    for name in ["alpha.txt", "Alphabet.doc", "beta.txt", "gamma.log"] {
        fs::write(left.join(name), name).unwrap();
    }
    let ctx = egui::Context::default();
    let options = Options { profile: Some(root.join("profile")), left: Some(left), right: Some(right), language: Some(language.into()), ..Default::default() };
    let mut app = App::from_context(&ctx, options);
    app.clipboard_writer = |_| Ok(());
    app.shell_runner = capture_launch;
    wait_listing(&mut app, &ctx);
    (root, ctx, app)
}
fn wait_listing(app: &mut App, ctx: &egui::Context) {
    let end = Instant::now() + Duration::from_secs(10);
    while app.panes.iter().any(|p| p.loading) {
        assert!(Instant::now() < end, "listing timeout");
        app.poll(ctx);
        thread::sleep(Duration::from_millis(5));
    }
}
fn wait_job(app: &mut App, ctx: &egui::Context) {
    let end = Instant::now() + Duration::from_secs(30);
    while app.job.as_ref().is_some_and(|j| !j.finished) {
        assert!(Instant::now() < end, "job timeout");
        app.poll(ctx);
        thread::sleep(Duration::from_millis(5));
    }
    wait_listing(app, ctx);
}
/// 보호 항목 검사는 따로 돌므로 대화 상자나 작업이 생길 때까지 기다린다.
fn wait_dialog_or_job(app: &mut App, ctx: &egui::Context) {
    let end = Instant::now() + Duration::from_secs(10);
    while app.original_dialogs.is_empty() && app.job.as_ref().is_none_or(|j| j.finished) {
        assert!(Instant::now() < end, "protected-item scan timeout");
        app.poll(ctx);
        thread::sleep(Duration::from_millis(5));
    }
}
fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        events,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))),
        ..Default::default()
    };
    let _ = ctx.run(raw, |ctx| {
        app.original_dialog_ui(ctx);
        app.msgbox_ui(ctx);
    });
}
fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}
fn row(app: &App, pane: usize, name: &str) -> usize {
    app.panes[pane].entries.iter().position(|e| e.name == name).unwrap_or_else(|| panic!("{name} not listed"))
}
fn cursor_name(app: &App, pane: usize) -> String {
    app.panes[pane].entries[app.panes[pane].cursor].name.clone()
}
fn root_text(app: &App, key: &str) -> String {
    app.language_pack.text(&format!("RES.{key}"), "?")
}
fn prompt_of(app: &App) -> (String, String, String) {
    let dialog = app.original_dialogs.first().expect("a dialog is open");
    match &dialog.kind {
        DialogKind::Prompt { message, value, .. } => (dialog.title.clone(), message.clone(), value.clone()),
        _ => panic!("not a TfGetText dialog"),
    }
}
fn answer_text(app: &mut App, ctx: &egui::Context, value: Option<&str>) {
    app.answer_dialog(DialogResult::Text(value.map(str::to_owned)), ctx);
}
fn answer_warning(app: &mut App, ctx: &egui::Context, choice: DeleteWarningChoice) {
    assert!(matches!(app.original_dialogs.first().map(|d| &d.kind), Some(DialogKind::DeleteWarning { .. })));
    app.answer_dialog(DialogResult::DeleteWarning(choice), ctx);
}
fn system_error_text(error: u32) -> String {
    ndr::names::system_message(error).unwrap()
}

#[test]
fn new_folder_and_empty_file_follow_the_original() {
    let (root, ctx, mut app) = fixture("create", "ko");
    let left = root.join("left");
    app.dispatch("newFolder", &ctx);
    let (title, message, value) = prompt_of(&app);
    assert_eq!((title, message), (root_text(&app, "TITLE_NEWFOLDER"), root_text(&app, "MSG_NEWFOLDER")));
    assert_eq!(value, ndr::names::new_folder_default_name(), "short date with '-' for '\\' and '/'");
    answer_text(&mut app, &ctx, Some(""));
    assert!(app.original_dialogs.is_empty() && app.msgboxes.is_empty() && !app.panes[0].loading, "empty input does nothing");
    app.dispatch("newFolder", &ctx);
    answer_text(&mut app, &ctx, Some("새 폴더"));
    wait_listing(&mut app, &ctx);
    assert!(left.join("새 폴더").is_dir());
    assert_eq!(cursor_name(&app, 0), "새 폴더", "the new folder gets the cursor");

    app.dispatch("newFolder", &ctx);
    answer_text(&mut app, &ctx, Some("새 폴더"));
    assert_eq!(app.msgboxes.len(), 1, "CreateDirectoryW fails on an existing name");
    assert!(app.msgboxes[0].system);
    assert_eq!(app.msgboxes[0].title, root_text(&app, "TITLE_ERROR"));
    assert_eq!(app.msgboxes[0].flags, MB_OK | MB_ICONHAND);
    assert_eq!(app.msgboxes[0].text, system_error_text(183));
    assert!(!app.panes[0].loading, "no refresh after a failure");
    app.answer_message_box(1, &ctx);
    app.dispatch("newFolder", &ctx);
    answer_text(&mut app, &ctx, None);
    assert!(app.original_dialogs.is_empty() && app.msgboxes.is_empty());
    app.dispatch("newFolder", &ctx);
    answer_text(&mut app, &ctx, Some("a\\b\\c"));
    assert_eq!(app.msgboxes[0].text, system_error_text(3), "no intermediate folders (ERROR_PATH_NOT_FOUND)");
    app.answer_message_box(1, &ctx);

    app.dispatch("newEmpty", &ctx);
    let (title, message, value) = prompt_of(&app);
    assert_eq!((title, message, value), (root_text(&app, "TITLE_CREATEEMPTYFILE"), root_text(&app, "MSG_CREATEEMPTYFILE"), String::new()));
    answer_text(&mut app, &ctx, Some("빈 파일.txt"));
    wait_listing(&mut app, &ctx);
    assert_eq!(fs::metadata(left.join("빈 파일.txt")).unwrap().len(), 0);
    assert_eq!(cursor_name(&app, 0), "빈 파일.txt");
    fs::write(left.join("빈 파일.txt"), b"keep").unwrap();
    app.panes[0].cursor = 0;
    app.dispatch("newEmpty", &ctx);
    answer_text(&mut app, &ctx, Some("빈 파일.txt"));
    assert_eq!(app.msgboxes[0].text, system_error_text(80), "approved fix: CREATE_NEW failure is reported");
    assert_eq!(fs::read(left.join("빈 파일.txt")).unwrap(), b"keep", "an existing file is never truncated");
    app.answer_message_box(1, &ctx);
    wait_listing(&mut app, &ctx);
    assert_eq!(cursor_name(&app, 0), "빈 파일.txt", "refresh and focus still happen after the error");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rename_asks_once_per_item_and_moves_comments() {
    let (root, ctx, mut app) = fixture("rename", "en");
    let left = root.join("left");
    compat::set_description(&left, "alpha.txt", "comment A", true).unwrap();
    app.panes[0].cursor = row(&app, 0, "alpha.txt");
    app.panes[0].selected = [row(&app, 0, "beta.txt"), row(&app, 0, "alpha.txt")].into_iter().collect();
    app.dispatch("rename", &ctx);
    let dialog = &app.original_dialogs[0];
    assert_eq!(dialog.title, root_text(&app, "RENAME"));
    let template = root_text(&app, "MSG_RENAME");
    match &dialog.kind {
        DialogKind::Rename { message, name, extension } => {
            assert_eq!((name.as_str(), extension.as_str()), ("alpha", "txt"));
            assert_eq!(message, &ndr::markup::substitute(&template, &["alpha.txt", "1", "2"]), "list order, index / count");
        }
        _ => panic!("TfRename expected"),
    }
    app.set_dialog_text("omega", "md");
    app.submit_rename(&ctx);
    match &app.original_dialogs[0].kind {
        DialogKind::Rename { message, name, .. } => {
            assert_eq!(name, "beta");
            assert_eq!(message, &ndr::markup::substitute(&template, &["beta.txt", "2", "2"]));
        }
        _ => panic!("second TfRename expected"),
    }
    app.set_dialog_text("bad:name", "");
    app.submit_rename(&ctx);
    assert_eq!(app.msgboxes.len(), 1, "forbidden characters: MessageBoxW and the dialog stays");
    assert_eq!(app.msgboxes[0].text, root_text(&app, "MSG_INVALID_FILENAME"));
    assert_eq!((app.msgboxes[0].title.as_str(), app.msgboxes[0].flags), ("", MB_OK), "NULL caption, MB_OK");
    assert_eq!(app.original_dialogs.len(), 1);
    app.answer_message_box(1, &ctx);
    app.answer_dialog(DialogResult::Rename(None), &ctx);
    wait_listing(&mut app, &ctx);
    assert!(left.join("omega.md").exists() && left.join("beta.txt").exists(), "Cancel stops the loop");
    assert_eq!(cursor_name(&app, 0), "omega.md", "the focus follows the renamed item");
    let comments = compat::read_descriptions(&left, true).unwrap();
    assert_eq!(comments.get("omega.md").map(String::as_str), Some("comment A"), "the comment follows the item");
    assert!(!comments.contains_key("alpha.txt"));

    // A failed MoveFileW asks whether to continue only while items remain.
    app.panes[0].selected = [row(&app, 0, "Alphabet.doc"), row(&app, 0, "gamma.log")].into_iter().collect();
    app.dispatch("rename", &ctx);
    app.set_dialog_text("beta", "txt");
    app.submit_rename(&ctx);
    let expected = format!("{}\n{}", system_error_text(183), root_text(&app, "MSG_WANTTOCONTINUE2"));
    assert_eq!(app.msgboxes[0].text, expected);
    assert_eq!(app.msgboxes[0].flags, MB_YESNO | MB_ICONHAND);
    assert_eq!(app.msgboxes[0].title, root_text(&app, "TITLE_ERROR"));
    app.answer_message_box(7, &ctx);
    assert!(app.original_dialogs.is_empty(), "No stops the loop");
    wait_listing(&mut app, &ctx);
    app.panes[0].selected = [row(&app, 0, "Alphabet.doc"), row(&app, 0, "gamma.log")].into_iter().collect();
    app.dispatch("rename", &ctx);
    app.set_dialog_text("beta", "txt");
    app.submit_rename(&ctx);
    app.answer_message_box(6, &ctx);
    app.set_dialog_text("beta", "txt");
    app.submit_rename(&ctx);
    assert_eq!(app.msgboxes[0].text, system_error_text(183), "the last item: no continue question");
    assert_eq!(app.msgboxes[0].flags, MB_OK | MB_ICONHAND);
    app.answer_message_box(1, &ctx);
    wait_listing(&mut app, &ctx);
    assert!(left.join("Alphabet.doc").exists() && left.join("gamma.log").exists());

    // Folders keep the whole name; the extension box starts empty. Case-only renames are allowed.
    app.panes[0].selected.clear();
    app.panes[0].cursor = row(&app, 0, "folder1");
    app.dispatch("rename", &ctx);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Rename { name, extension, .. } if name == "folder1" && extension.is_empty()));
    app.set_dialog_text("Folder1", "");
    app.submit_rename(&ctx);
    wait_listing(&mut app, &ctx);
    assert!(app.panes[0].entries.iter().any(|e| e.name == "Folder1"));
    assert_eq!(cursor_name(&app, 0), "Folder1");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copying_marked_items_moves_the_marks_to_restore_selection() {
    // 원본 FUN_0042bdc4: CopySelection(+0xd08) 뒤 ClearSelection. 마지막 선택 복구가 복사한 표시를 되돌린다.
    let (root, ctx, mut app) = fixture("take-marks", "ko");
    assert!(app.current_skin().is_some());
    let (left, right) = (root.join("left"), root.join("right"));
    let marked = [row(&app, 0, "alpha.txt"), row(&app, 0, "beta.txt")];
    app.active = 0;
    app.panes[0].selected = marked.into_iter().collect();
    let sources = app.panes[0].targets();
    app.local_transfer(sources, right.clone(), false, super::file_operation::TransferKind::Folder);
    assert!(app.panes[0].selected.is_empty(), "the source marks are cleared");
    assert_eq!(app.last_selection[0], vec![left.join("alpha.txt"), left.join("beta.txt")]);
    wait_job(&mut app, &ctx);
    assert!(right.join("alpha.txt").exists() && right.join("beta.txt").exists());
    assert!(app.panes[0].selected.is_empty(), "the refresh does not bring them back");
    app.dispatch("restoreSelection", &ctx);
    let names: Vec<_> = app.panes[0].selected.iter().map(|&r| app.panes[0].entries[r].name.clone()).collect();
    assert_eq!(names, vec!["alpha.txt", "beta.txt"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn description_prompt_per_item_and_empty_clears() {
    let (root, ctx, mut app) = fixture("description", "ko");
    let left = root.join("left");
    compat::set_description(&left, "alpha.txt", "기존 설명", true).unwrap();
    app.panes[0].selected = [row(&app, 0, "alpha.txt"), row(&app, 0, "beta.txt")].into_iter().collect();
    app.dispatch("description", &ctx);
    let (title, message, value) = prompt_of(&app);
    assert_eq!(title, root_text(&app, "TITLE_DESCRIPT"));
    assert_eq!(message, root_text(&app, "MSG_DESCRIPT").replace("%s", "alpha.txt"));
    assert_eq!(value, "기존 설명", "the current comment is the default");
    answer_text(&mut app, &ctx, Some("새 설명"));
    let (_, message, value) = prompt_of(&app);
    assert_eq!((message, value), (root_text(&app, "MSG_DESCRIPT").replace("%s", "beta.txt"), String::new()));
    assert!(!app.panes[0].selected.contains(&row(&app, 0, "alpha.txt")), "OK unmarks the item");
    assert!(app.panes[0].selected.contains(&row(&app, 0, "beta.txt")));
    answer_text(&mut app, &ctx, None);
    wait_listing(&mut app, &ctx);
    let comments = compat::read_descriptions(&left, true).unwrap();
    assert_eq!(comments.get("alpha.txt").map(String::as_str), Some("새 설명"));
    assert!(!comments.contains_key("beta.txt"), "Cancel stops before the second item");
    // 원본(스킨 테마): 새로 고침은 표시를 이름으로 되돌리므로 취소한 뒤의 항목은 표시된 채로 남는다.
    assert!(app.panes[0].selected.contains(&row(&app, 0, "beta.txt")));
    app.panes[0].selected.clear();
    app.panes[0].cursor = row(&app, 0, "alpha.txt");
    app.dispatch("description", &ctx);
    answer_text(&mut app, &ctx, Some(""));
    wait_listing(&mut app, &ctx);
    assert!(!compat::read_descriptions(&left, true).unwrap().contains_key("alpha.txt"), "empty input clears the comment");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn open_with_parameters_uses_the_focused_item() {
    let (root, ctx, mut app) = fixture("openparams", "en");
    let left = root.join("left");
    app.panes[0].cursor = row(&app, 0, "alpha.txt");
    app.panes[0].selected = [row(&app, 0, "beta.txt")].into_iter().collect();
    app.dispatch("openWithParameters", &ctx);
    let (title, message, value) = prompt_of(&app);
    assert_eq!(title, root_text(&app, "OPEN_WITH_PARAM"));
    assert_eq!(message, root_text(&app, "MSG_OPEN_WITH_PARAM").replace("%s", "alpha.txt"), "focused item, not the marked one");
    assert!(value.is_empty());
    answer_text(&mut app, &ctx, None);
    assert!(LAUNCHED.with(|l| l.borrow().is_empty()), "Cancel launches nothing");
    app.dispatch("openWithParameters", &ctx);
    let panes = app.macro_panes();
    let expected = compat::expand_command("/x $(SELECTED) $(PATH2)", &panes[0], &panes[0], &panes[1]);
    assert!(expected.contains("beta.txt") && expected.contains("right"), "macros expand: {expected}");
    answer_text(&mut app, &ctx, Some("/x $(SELECTED) $(PATH2)"));
    let launched = LAUNCHED.with(|l| l.borrow().clone());
    assert_eq!(launched, vec![(left.join("alpha.txt"), expected, left.clone())], "file, expanded parameters, listed folder");

    // A folder is entered, and the same-named folder of the other pane too (case-insensitive).
    app.panes[0].selected.clear();
    app.panes[0].cursor = row(&app, 0, "folder1");
    app.dispatch("openWithParameters", &ctx);
    assert!(app.original_dialogs.is_empty(), "no prompt for a folder");
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, left.join("folder1"));
    assert_eq!(app.panes[1].path, root.join("right").join("FOLDER1"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn delete_warning_and_protected_item_confirmation() {
    let (root, ctx, mut app) = fixture("delete", "en");
    let left = root.join("left");
    // Recycle mode: only No and the Delete button are answered so nothing reaches the real Recycle Bin.
    app.panes[0].cursor = row(&app, 0, "alpha.txt");
    app.dispatch("deleteRecycle", &ctx);
    match &app.original_dialogs[0].kind {
        DialogKind::DeleteWarning { message, show_delete, default_button } => {
            assert_eq!(message, &app.delete_warning_message(DeleteWarningMode::Recycle, 1, "alpha.txt"));
            assert!(*show_delete && *default_button == 2, "ShowDeleteButton / DefaultDeleteButton defaults");
        }
        _ => panic!("TfDelWarning expected"),
    }
    assert_eq!(app.original_dialogs[0].title, app.language_pack.text("formDelWarning.Form.Caption", "?"));
    answer_warning(&mut app, &ctx, DeleteWarningChoice::No);
    assert!(app.job.is_none() && left.join("alpha.txt").exists(), "No stops");
    app.dispatch("deleteRecycle", &ctx);
    answer_warning(&mut app, &ctx, DeleteWarningChoice::Delete);
    wait_dialog_or_job(&mut app, &ctx);
    wait_job(&mut app, &ctx);
    assert!(!left.join("alpha.txt").exists(), "the Delete button deletes permanently");
    app.settings.set_flag("ShowDeleteButton", false);
    // 새 목록은 맨 위("..")에서 시작하므로 파일을 고른다(".."은 대상이 아니다).
    app.panes[0].cursor = row(&app, 0, "beta.txt");
    app.dispatch("deleteRecycle", &ctx);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::DeleteWarning { show_delete: false, .. }));
    answer_warning(&mut app, &ctx, DeleteWarningChoice::No);

    // Permanent delete: always warns (even with delconfirm off), no Delete button, then one TfDelConfirm per protected item.
    app.settings.set_flag("ConfirmDelete", false);
    let tree = left.join("tree");
    fs::create_dir_all(tree.join("keep")).unwrap();
    fs::write(tree.join("keep/hidden.txt"), b"h").unwrap();
    fs::write(tree.join("ro.txt"), b"r").unwrap();
    fs::write(tree.join("plain.txt"), b"p").unwrap();
    native::set_attributes(&tree.join("keep/hidden.txt"), false, true).unwrap();
    native::set_attributes(&tree.join("ro.txt"), true, false).unwrap();
    native::set_attributes(&left.join("gamma.log"), true, false).unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.panes[0].selected = [row(&app, 0, "tree"), row(&app, 0, "gamma.log")].into_iter().collect();
    app.dispatch("deletePermanent", &ctx);
    match &app.original_dialogs[0].kind {
        DialogKind::DeleteWarning { message, show_delete, .. } => {
            assert_eq!(message, &app.delete_warning_message(DeleteWarningMode::Delete, 2, "tree"));
            assert!(!show_delete);
        }
        _ => panic!("TfDelWarning expected"),
    }
    answer_warning(&mut app, &ctx, DeleteWarningChoice::Yes);
    let mut asked = Vec::new();
    loop {
        wait_dialog_or_job(&mut app, &ctx);
        let Some(dialog) = app.original_dialogs.first() else { break };
        let DialogKind::DeleteConfirm { message, attributes } = &dialog.kind else { panic!("TfDelConfirm expected") };
        assert_eq!(dialog.title, app.language_pack.text("formDelConfirm.Form.Caption", "?"));
        let (message, attributes) = (message.clone(), attributes.clone());
        let choice = if message.contains("hidden.txt") {
            assert_eq!(attributes, root_text(&app, "HAS_ATTRIBUTES").replace("%s", &root_text(&app, "HIDDEN")));
            DeleteConfirmChoice::No
        } else if message.contains("ro.txt") {
            assert_eq!(attributes, root_text(&app, "HAS_ATTRIBUTES").replace("%s", &root_text(&app, "READONLY")));
            DeleteConfirmChoice::Yes
        } else {
            DeleteConfirmChoice::Yes
        };
        let path = message.lines().next().unwrap().to_owned();
        assert_eq!(message, root_text(&app, "QUE_DELCONFIRM").replace("%s", &path), "QUE_DELCONFIRM with the full path");
        asked.push(path);
        app.answer_dialog(DialogResult::DeleteConfirm(choice), &ctx);
    }
    assert_eq!(asked.len(), 3, "{asked:?}");
    wait_job(&mut app, &ctx);
    assert!(tree.join("keep/hidden.txt").exists() && tree.join("keep").is_dir() && tree.is_dir(), "No keeps the item and its parents");
    assert!(!tree.join("ro.txt").exists() && !tree.join("plain.txt").exists() && !left.join("gamma.log").exists());

    // Yes to All approves the rest without asking again.
    for name in ["a.txt", "b.txt"] {
        fs::write(left.join(name), b"x").unwrap();
        native::set_attributes(&left.join(name), true, false).unwrap();
    }
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.panes[0].selected = [row(&app, 0, "a.txt"), row(&app, 0, "b.txt")].into_iter().collect();
    app.dispatch("deletePermanent", &ctx);
    answer_warning(&mut app, &ctx, DeleteWarningChoice::Yes);
    wait_dialog_or_job(&mut app, &ctx);
    app.answer_dialog(DialogResult::DeleteConfirm(DeleteConfirmChoice::All), &ctx);
    assert!(app.original_dialogs.is_empty(), "no second question after Yes to All");
    wait_job(&mut app, &ctx);
    assert!(!left.join("a.txt").exists() && !left.join("b.txt").exists());
    let _ = native::set_attributes(&tree.join("keep/hidden.txt"), false, false);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shred_always_warns_without_the_delete_button() {
    let (root, ctx, mut app) = fixture("shred", "ko");
    let left = root.join("left");
    app.settings.set_flag("ConfirmDelete", false);
    app.panes[0].cursor = row(&app, 0, "beta.txt");
    app.dispatch("shred", &ctx);
    match &app.original_dialogs[0].kind {
        DialogKind::DeleteWarning { message, show_delete, .. } => {
            assert_eq!(message, &app.delete_warning_message(DeleteWarningMode::Shred, 1, "beta.txt"));
            assert!(!show_delete);
        }
        _ => panic!("TfDelWarning expected"),
    }
    answer_warning(&mut app, &ctx, DeleteWarningChoice::No);
    assert!(left.join("beta.txt").exists());
    app.dispatch("shred", &ctx);
    answer_warning(&mut app, &ctx, DeleteWarningChoice::Yes);
    wait_job(&mut app, &ctx);
    assert!(!left.join("beta.txt").exists());
    assert!(fs::read_dir(&left).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".ndr-shred-")), "no leftover temporary name");
    // The empty right pane has nothing to shred.
    fs::remove_dir(root.join("right/FOLDER1")).unwrap();
    app.refresh(1);
    wait_listing(&mut app, &ctx);
    app.active = 1;
    app.dispatch("shred", &ctx);
    assert_eq!(app.msgboxes[0].text, "완전 삭제할 항목을 선택하세요.");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn original_dialogs_draw_and_answer_from_the_keyboard() {
    let (root, ctx, mut app) = fixture("keyboard", "ko");
    let got = std::rc::Rc::new(RefCell::new(None::<Option<String>>));
    let sink = got.clone();
    app.prompt("제목", "<b>굵게</b> 안내", "기본값", None, false, move |_, value, _| *sink.borrow_mut() = Some(value));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(*got.borrow(), Some(Some("기본값".to_string())), "Enter is OK");
    let sink = got.clone();
    app.prompt("제목", "안내", "", None, false, move |_, value, _| *sink.borrow_mut() = Some(value));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert_eq!(*got.borrow(), Some(None), "Esc is Cancel");

    // TfRename drops typed forbidden characters.
    app.panes[0].selected.clear();
    app.panes[0].cursor = row(&app, 0, "gamma.log");
    app.dispatch("rename", &ctx);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![egui::Event::Text("x:y*z".into())]);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Rename { name, .. } if name == "xyz"), "typed text replaces the selection without \\/:*?\"<>|");
    app.answer_dialog(DialogResult::Rename(None), &ctx);
    wait_listing(&mut app, &ctx);

    // The delete warning and the protected-item confirmation draw with the warning icon.
    app.delete_warning(DeleteWarningMode::Recycle, 3, "x", |_, _, _| {});
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.original_dialogs.is_empty(), "Esc answers No");
    app.delete_confirm("C:\\x\n...", "attributes", |_, _, _| {});
    frame(&mut app, &ctx, vec![]);
    app.answer_dialog(DialogResult::DeleteConfirm(DeleteConfirmChoice::No), &ctx);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn attribute_window_asks_per_item_like_changeattributesasync() {
    use std::os::windows::fs::MetadataExt;
    let (root, ctx, mut app) = fixture("attributes", "ko");
    let left = root.join("left");
    let attributes = |name: &str| fs::metadata(left.join(name)).unwrap().file_attributes();
    let wait_dialog = |app: &mut App, ctx: &egui::Context| {
        let end = Instant::now() + Duration::from_secs(10);
        let generations: Vec<u64> = app.panes.iter().map(|p| p.generation).collect();
        while app.original_dialogs.is_empty() && app.panes.iter().all(|p| !p.loading) && app.panes.iter().map(|p| p.generation).eq(generations.iter().copied()) {
            assert!(Instant::now() < end, "attribute apply timeout");
            app.poll(ctx);
            thread::sleep(Duration::from_millis(5));
        }
    };
    app.panes[0].selected = [row(&app, 0, "alpha.txt"), row(&app, 0, "beta.txt"), row(&app, 0, "gamma.log")].into_iter().collect();
    app.dispatch("attributes", &ctx);
    let dialog = &app.original_dialogs[0];
    assert_eq!(dialog.title, app.language_pack.text("formChangeAttr.Form.Caption", "?"));
    assert!(matches!(&dialog.kind, DialogKind::Attributes(form) if form.path.ends_with("alpha.txt") && form.read_only.is_none() && form.apply_to_folders));
    frame(&mut app, &ctx, vec![]);
    // Invalid time: a warning box with the window title, the window stays.
    app.edit_attribute_form(|form| {
        form.change_time = true;
        form.time = "25:00".into();
    });
    app.submit_attributes(super::original_dialogs::AttributeAction::Apply, &ctx);
    assert_eq!(app.msgboxes[0].text, "날짜 또는 시간이 올바르지 않습니다.");
    assert_eq!(app.msgboxes[0].title, app.original_dialogs[0].title);
    app.answer_message_box(1, &ctx);
    // OK on the first item: read-only on, time 2020-01-02 03:04:05.
    app.edit_attribute_form(|form| {
        form.read_only = Some(true);
        form.date = "2020-01-02".into();
        form.time = "03:04:05".into();
    });
    app.submit_attributes(super::original_dialogs::AttributeAction::Apply, &ctx);
    wait_dialog(&mut app, &ctx);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Attributes(form) if form.path.ends_with("beta.txt")), "next item");
    assert_eq!(attributes("alpha.txt") & 1, 1);
    let expected = chrono::NaiveDate::from_ymd_opt(2020, 1, 2).unwrap().and_hms_opt(3, 4, 5).unwrap().and_local_timezone(chrono::Local).earliest().unwrap();
    assert_eq!(fs::metadata(left.join("alpha.txt")).unwrap().modified().unwrap(), std::time::SystemTime::from(expected));
    // Esc skips the second item; All applies to the rest.
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Attributes(form) if form.path.ends_with("gamma.log")));
    app.edit_attribute_form(|form| form.hidden = Some(true));
    // 원본 TntFormKeyDown(0x41a2f4): End = btnAll.Click(남은 항목 모두).
    frame(&mut app, &ctx, vec![key(egui::Key::End)]);
    wait_dialog(&mut app, &ctx);
    wait_listing(&mut app, &ctx);
    assert!(app.original_dialogs.is_empty());
    assert_eq!(attributes("beta.txt") & 3, 0, "skipped");
    assert_eq!(attributes("gamma.log") & 2, 2);
    assert_eq!(app.status, "2개 항목의 속성을 변경하고 1개 항목을 건너뛰었습니다.");
    let _ = native::set_attributes(&left.join("alpha.txt"), false, false);
    let _ = native::set_attributes(&left.join("gamma.log"), false, false);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn split_and_join_windows_follow_the_csharp_flows() {
    let (root, ctx, mut app) = fixture("splitjoin", "ko");
    let left = root.join("left");
    let payload: Vec<u8> = (0..3000u32).map(|i| (i % 7) as u8).collect();
    fs::write(left.join("big.bin"), &payload).unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    // 원본 miSplitFileClick: 커서 항목이 파일이 아니면 아무것도 하지 않는다(알림 없음).
    app.panes[0].cursor = row(&app, 0, "folder1");
    app.dispatch("split", &ctx);
    assert!(app.msgboxes.is_empty() && app.original_dialogs.is_empty());
    app.panes[0].cursor = row(&app, 0, "big.bin");
    app.dispatch("split", &ctx);
    assert_eq!(app.original_dialogs[0].title, app.language_pack.text("formFileSplit.Form.Caption", "?"));
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Split(form) if form.size == "720 KB" && Path::new(&form.path) == left));
    frame(&mut app, &ctx, vec![]);
    app.edit_dialog_kind(|kind| if let DialogKind::Split(form) = kind { form.size = "0".into() });
    app.submit_split_or_join(&ctx);
    assert_eq!(app.msgboxes[0].text, "분할 크기를 입력하세요. 예: 1440 KB 또는 100 MB.");
    app.answer_message_box(1, &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::Split(form) = kind { form.size = "1 KB".into() });
    app.submit_split_or_join(&ctx);
    wait_job(&mut app, &ctx);
    for part in ["big.001", "big.002", "big.003", "big.crc"] {
        assert!(left.join(part).is_file(), "{part}");
    }
    assert_eq!(app.status, "3개 조각 파일을 만들었습니다.");
    // 원본 miJoinFileClick: 표시한 항목(없으면 커서 항목)이 lbFiles. 이름은 .crc 파일의 원래 이름.
    let mark_parts = |app: &mut App| app.panes[0].selected = ["big.001", "big.002", "big.003"].iter().map(|n| row(app, 0, n)).collect();
    mark_parts(&mut app);
    app.dispatch("join", &ctx);
    match &app.original_dialogs[0].kind {
        DialogKind::Join(form) => {
            assert_eq!(form.files.iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>(), ["big.001", "big.002", "big.003"]);
            assert_eq!(form.name, "big.bin");
        }
        _ => panic!("join window expected"),
    }
    frame(&mut app, &ctx, vec![]);
    app.submit_split_or_join(&ctx);
    assert!(app.msgboxes[0].text.starts_with("같은 이름의 파일이 이미 있어 합치지 않았습니다."), "an existing result is never replaced");
    app.answer_message_box(1, &ctx);
    mark_parts(&mut app);
    app.dispatch("join", &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::Join(form) = kind { form.name = "joined.bin".into() });
    app.submit_split_or_join(&ctx);
    assert!(app.panes[0].selected.is_empty(), "OK takes the marks (CopySelection + ClearSelection)");
    wait_job(&mut app, &ctx);
    assert_eq!(fs::read(left.join("joined.bin")).unwrap(), payload);
    assert_eq!(app.status, "3개 파일을 합쳤습니다.");
    // 표시도 커서 항목도 없으면(폴더) 아무것도 하지 않는다.
    app.panes[0].selected.clear();
    app.panes[0].cursor = row(&app, 0, "folder1");
    app.dispatch("join", &ctx);
    assert!(app.original_dialogs.is_empty() && app.msgboxes.is_empty());
    fs::remove_dir_all(root).unwrap();
}

thread_local! {
    static SAVE_TARGET: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}
fn pick_save_target(_: &str, _: Option<&Path>, _: &str, _: &[(&str, &str)], _: &str, _: bool) -> Option<PathBuf> {
    SAVE_TARGET.with(|t| t.borrow().clone())
}

#[test]
fn list_export_window_saves_like_savelistasync() {
    let (root, ctx, mut app) = fixture("listexport", "en");
    let left = root.join("left");
    compat::set_description(&left, "beta.txt", "comma, \"quoted\"", true).unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.save_dialog = pick_save_target;
    SAVE_TARGET.with(|t| *t.borrow_mut() = Some(root.join("list")));
    app.dispatch("saveList", &ctx);
    assert_eq!(app.original_dialogs[0].title, app.language_pack.text("formSaveList.Form.Caption", "?"));
    frame(&mut app, &ctx, vec![]);
    // No columns: an information box, the window stays.
    app.edit_dialog_kind(|kind| if let DialogKind::ListExport(form) = kind { form.columns = [false; 7]; form.rebuild(); });
    app.submit_list_export(&ctx);
    assert_eq!(app.msgboxes[0].text, "Select at least one column to save.");
    app.answer_message_box(1, &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::ListExport(form) = kind {
        form.columns = [true, true, false, false, false, false, true];
        form.delimiter = 1;
        form.rebuild();
    });
    app.submit_list_export(&ctx);
    assert!(app.original_dialogs.is_empty());
    let saved = fs::read(root.join("list.txt")).unwrap();
    assert_eq!(&saved[..3], &[0xEF, 0xBB, 0xBF], "UTF-8 with BOM; .txt added");
    let text = String::from_utf8(saved[3..].to_vec()).unwrap();
    assert!(text.contains("beta,txt,\"comma, \"\"quoted\"\"\"\r\n"), "{text}");
    assert!(text.contains("folder1,,\r\n"));
    assert_eq!(app.status, format!("Saved {} items.", app.panes[0].entries.len()));
    // An edited preview is saved as typed.
    app.dispatch("saveList", &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::ListExport(form) = kind { form.preview = "edited\r\n".into(); form.edited = true; });
    SAVE_TARGET.with(|t| *t.borrow_mut() = Some(root.join("edited.csv")));
    app.submit_list_export(&ctx);
    assert_eq!(fs::read(root.join("edited.csv")).unwrap(), b"\xEF\xBB\xBFedited\r\n");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn f12_shortcut_window_edits_the_double_clicked_line() {
    let (root, ctx, mut app) = fixture("shortcuts", "en");
    app.dispatch("shortcuts", &ctx);
    app.dispatch("shortcuts", &ctx);
    assert!(app.shortcut_window.is_some(), "one window");
    let rows = app.shortcut_rows("");
    assert_eq!(rows.len(), app.shortcut_lines().len(), "every shortcut.ini line");
    let row = app.shortcut_rows("findnext").into_iter().next().expect("FindNext line");
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.shortcut_window_ui(ctx));
    app.edit_shortcut(row.clone());
    assert_eq!(app.original_dialogs[0].title, app.language_pack.text("formEditShortCut.Form.Caption", "?"));
    frame(&mut app, &ctx, vec![]);
    let ctrl_shift = egui::Modifiers { ctrl: true, shift: true, ..egui::Modifiers::NONE };
    let press = egui::Event::Key { key: egui::Key::K, physical_key: None, pressed: true, repeat: false, modifiers: ctrl_shift };
    frame(&mut app, &ctx, vec![press]);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::EditShortcut(form) if form.text == "Ctrl+Shift+K"), "the pressed key becomes the text");
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::EditShortcut(form) if form.text == "Escape"), "Esc is captured, it does not close");
    app.edit_dialog_kind(|kind| if let DialogKind::EditShortcut(form) = kind { form.text = "Ctrl+Shift+K".into() });
    app.answer_dialog(DialogResult::EditShortcut(Some("Ctrl+Shift+K".into())), &ctx);
    let stored = app.settings.text("OriginalShortcutMapText", "");
    assert!(stored.starts_with("NEXUSFILESHORTCUT1.0\r\n"), "saved with the header");
    let lines = app.shortcut_lines();
    assert_eq!(lines[row.index], format!("{}=Ctrl+Shift+K", row.name));
    assert_eq!(app.shortcut_rows("").len(), rows.len());
    assert!(root.join("profile").join("settings.json").is_file(), "saved to the isolated profile");
    fs::remove_dir_all(root).unwrap();
}

/// 원본 FUN_00438034: TfGetText(TITLE_ADVSELECT / TITLE_ADVDESELECT, MSG_ADVSELECT, 빈 글). ';'로 나눈 필터마다 PathMatchSpecW,
/// 파일만 고르거나 푼다(폴더는 이름이 맞아도 아님). 이전 선택은 남고, 취소나 빈 글은 아무것도 하지 않는다.
#[test]
fn advanced_select_follows_advancedselectasync() {
    let (root, ctx, mut app) = fixture("advselect", "ko");
    let left = root.join("left");
    fs::write(left.join("big.txt"), vec![0u8; 16]).unwrap();
    fs::write(left.join("note.log"), b"x").unwrap();
    fs::create_dir_all(left.join("dir.txt")).unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    let marked = |app: &App| { let mut v: Vec<String> = app.panes[0].selected.iter().map(|i| app.panes[0].entries[*i].name.clone()).collect(); v.sort(); v };
    app.dispatch("advancedSelect", &ctx);
    assert_eq!(prompt_of(&app), ("고급 선택".to_owned(), app.language_pack.text("RES.MSG_ADVSELECT", "?"), String::new()), "original TITLE_ADVSELECT / MSG_ADVSELECT, empty value");
    answer_text(&mut app, &ctx, Some("*.TXT;note.*"));
    assert_eq!(marked(&app), ["alpha.txt", "beta.txt", "big.txt", "note.log"], "PathMatchSpecW per filter, files only (dir.txt is a folder)");
    // 고급 해제: 필터에 맞는 파일만 푼다.
    app.dispatch("advancedDeselect", &ctx);
    assert_eq!(prompt_of(&app).0, "고급 해제", "original TITLE_ADVDESELECT");
    answer_text(&mut app, &ctx, Some("b*"));
    assert_eq!(marked(&app), ["alpha.txt", "note.log"]);
    app.dispatch("restoreSelection", &ctx);
    assert_eq!(marked(&app), ["alpha.txt", "beta.txt", "big.txt", "note.log"], "the previous selection was kept");
    // 취소·빈 글은 그대로.
    app.dispatch("advancedDeselect", &ctx);
    answer_text(&mut app, &ctx, Some(""));
    app.dispatch("advancedDeselect", &ctx);
    answer_text(&mut app, &ctx, None);
    assert_eq!(marked(&app).len(), 4);
    fs::remove_dir_all(root).unwrap();
}

/// 원본 TfAdvRename: TntFormKeyDown(0x418bd0) Alt+1~6 = pcAction 쪽, lvPreviewKeyDown(0x419a4c) Ctrl+C = 고른 줄의 원래 이름 복사.
#[test]
fn advanced_rename_keys_follow_the_original() {
    let (root, ctx, mut app) = fixture("advrename-keys", "en");
    app.panes[0].selected = [row(&app, 0, "alpha.txt"), row(&app, 0, "beta.txt")].into_iter().collect();
    app.dispatch("advancedRename", &ctx);
    let mut form = match &app.original_dialogs[0].kind {
        DialogKind::AdvancedRename(form) => (**form).clone(),
        _ => panic!("rename window"),
    };
    let language = app.language_pack.clone();
    let run = |form: &mut super::rename_dialog::RenameForm, events: Vec<egui::Event>, modifiers: egui::Modifiers| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 700.0))), events, modifiers, ..Default::default() };
        ctx.run(raw, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                let _ = super::rename_dialog::rename_ui(ui, form, &language, false);
            });
        })
    };
    run(&mut form, vec![], egui::Modifiers::NONE);
    let alt = egui::Modifiers::ALT;
    run(&mut form, vec![egui::Event::Key { key: egui::Key::Num3, physical_key: None, pressed: true, repeat: false, modifiers: alt }], alt);
    assert_eq!(form.editor.action, "Delete", "Alt+3 = the third page (tsDelete)");
    // 미리 보기 첫 줄(머리 아래 322~340)을 누르고 Ctrl+C.
    let at = egui::pos2(100.0, 330.0);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    run(&mut form, vec![egui::Event::PointerMoved(at)], egui::Modifiers::NONE);
    run(&mut form, vec![button(true)], egui::Modifiers::NONE);
    run(&mut form, vec![button(false)], egui::Modifiers::NONE);
    assert_eq!(form.preview_selected, Some(0));
    let output = run(&mut form, vec![egui::Event::Copy], egui::Modifiers::COMMAND);
    let copied: Vec<String> = output.platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }).collect();
    let first = form.paths[0].file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(copied, [first]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn advanced_rename_window_builds_rules_macros_and_renames() {
    let (root, ctx, mut app) = fixture("advrename", "ko");
    let left = root.join("left");
    app.panes[0].selected = [row(&app, 0, "alpha.txt"), row(&app, 0, "beta.txt")].into_iter().collect();
    app.dispatch("advancedRename", &ctx);
    assert_eq!(app.original_dialogs[0].title, app.language_pack.text("formAdvRename.Form.Caption", "?"));
    frame(&mut app, &ctx, vec![]);
    app.commit_advanced_rename();
    assert_eq!(app.msgboxes[0].text, "적용할 이름 변경 작업을 추가하세요.");
    app.answer_message_box(1, &ctx);
    let edit = |app: &mut App, change: &dyn Fn(&mut super::rename_dialog::RenameForm)| app.edit_dialog_kind(|kind| if let DialogKind::AdvancedRename(form) = kind { change(form) });
    edit(&mut app, &|form| { form.editor.action = "Insert".into(); form.editor.position = "Prefix".into(); form.editor.text = "new_".into(); });
    app.add_rename_rule();
    edit(&mut app, &|form| { form.editor.action = "Number".into(); form.editor.position = "Suffix".into(); form.numbers = ["1".into(), "0".into(), "1".into(), "1".into(), "x".into()]; });
    app.add_rename_rule();
    assert_eq!(app.msgboxes[0].text, "위치와 번호 값은 정수로 입력하세요.");
    app.answer_message_box(1, &ctx);
    edit(&mut app, &|form| form.numbers[4] = "2".into());
    app.add_rename_rule();
    let preview = |app: &App| match &app.original_dialogs[0].kind {
        DialogKind::AdvancedRename(form) => (0..form.paths.len()).map(|i| super::rename_dialog::preview_name(form, i)).collect::<Vec<_>>(),
        _ => panic!("rename window"),
    };
    assert_eq!(preview(&app), ["new_alpha01.txt", "new_beta02.txt"]);
    // 원본 FUN_0041830c 목록 줄: ADVRENAME_ACTIONS 이름 (영어 설명) - name/extension.
    let actions = app.language_pack.text("RES.ADVRENAME_ACTIONS", "?");
    let actions: Vec<&str> = actions.split(';').collect();
    let lines = match &app.original_dialogs[0].kind {
        DialogKind::AdvancedRename(form) => form.rules.iter().map(|r| super::rename_dialog::rule_display(r, &app.language_pack)).collect::<Vec<_>>(),
        _ => panic!("rename window"),
    };
    assert_eq!(lines, [format!("{} (new_, First) - name", actions[0]), format!("{} (Start: 1, Step: 1, Digit: 2, Last) - name", actions[4])]);
    let mut rule = ndr::compat::RenameRule { action: "Delete".into(), delete_from_mode: "Delimiter".into(), find: "-".into(), count: 3, delete_direction: "RightToLeft".into(), target: "Extension".into(), ..Default::default() };
    assert_eq!(super::rename_dialog::rule_display(&rule, &app.language_pack), format!("{} (From: \"-\", Count: 3, Backward) - extension", actions[2]));
    rule = ndr::compat::RenameRule { action: "Replace".into(), find: "a".repeat(101), replace: "b".into(), position: "Last".into(), ..Default::default() };
    assert_eq!(super::rename_dialog::rule_display(&rule, &app.language_pack), format!("{} ({}... -> b, Last) - name", actions[1], "a".repeat(95)));
    frame(&mut app, &ctx, vec![]);
    // Save the rules as a macro, then refuse to replace it.
    app.save_rename_macro();
    app.answer_dialog(DialogResult::Text(Some("사진 정리".into())), &ctx);
    let saved = fs::read(root.join("profile").join(ndr::rename_macros::FILE_NAME)).unwrap();
    assert_eq!(ndr::rename_macros::read(&root.join("profile").join(ndr::rename_macros::FILE_NAME)).unwrap()[0].rules.len(), 2, "{saved:?}");
    app.save_rename_macro();
    app.answer_dialog(DialogResult::Text(Some("사진 정리".into())), &ctx);
    assert_eq!(app.msgboxes[0].text, app.language_pack.text("RES.MSG_SAMENAME", "?"), "same name: the original TfMsgBox");
    assert_eq!(app.msgboxes[0].flags, MB_YESNO | ndr::msgbox::MB_ICONQUESTION);
    app.answer_message_box(7, &ctx);
    // C# SaveSelectedMacro: 원본 형식에 쓸 수 없는 값(';')이면 macroFormatUnsupported(값) 경고, 파일은 그대로.
    edit(&mut app, &|form| { let mut semicolon = form.rules[0].clone(); semicolon.text = "a;b".into(); form.rules.push(semicolon); });
    app.save_rename_macro();
    app.answer_dialog(DialogResult::Text(Some("세미콜론".into())), &ctx);
    let expected = app.language_pack.text("formAdvRename.Caption.macroFormatUnsupported", "?").replace("{0}", "a;b");
    assert_eq!((app.msgboxes[0].title.clone(), app.msgboxes[0].text.clone()), (app.language_pack.text("formAdvRename.Caption.macroSaveTitle", "?"), expected));
    assert_eq!(fs::read(root.join("profile").join(ndr::rename_macros::FILE_NAME)).unwrap(), saved, "advrename.save unchanged");
    app.answer_message_box(1, &ctx);
    edit(&mut app, &|form| { form.rules.pop(); });
    // 원본 FUN_0046d000: 쓸 수 없는 새 이름이 있으면 REN_INVALIDNAMES(REN_TITLE, 정보 아이콘)만 보이고 바꾸지 않는다.
    edit(&mut app, &|form| { let mut bad = form.rules[0].clone(); bad.text = "x?".into(); form.rules.push(bad); });
    app.commit_advanced_rename();
    assert_eq!((app.msgboxes[0].title.clone(), app.msgboxes[0].text.clone(), app.msgboxes[0].flags), (app.language_pack.text("RES.REN_TITLE", "?"), app.language_pack.text("RES.REN_INVALIDNAMES", "?"), MB_OK | ndr::msgbox::MB_ICONASTERISK));
    app.answer_message_box(1, &ctx);
    assert!(left.join("alpha.txt").exists());
    edit(&mut app, &|form| { form.rules.pop(); });
    // Rename: the confirmation lists the plan.
    app.commit_advanced_rename();
    assert!(app.msgboxes[0].text.contains("alpha.txt  →  new_alpha01.txt"));
    app.answer_message_box(6, &ctx);
    assert!(app.original_dialogs.is_empty(), "closed after renaming");
    wait_listing(&mut app, &ctx);
    assert!(left.join("new_alpha01.txt").is_file() && left.join("new_beta02.txt").is_file());
    assert!(!left.join("alpha.txt").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn search_window_puts_results_in_the_pane() {
    let (root, ctx, mut app) = fixture("search", "ko");
    let left = root.join("left");
    fs::write(left.join("folder1").join("inner-alpha.txt"), b"x").unwrap();
    app.dispatch("search", &ctx);
    assert_eq!(app.original_dialogs[0].title, "파일 및 폴더 검색");
    let search = |app: &mut App, ctx: &egui::Context, action| app.search_action(action, ctx);
    frame(&mut app, &ctx, vec![]);
    app.edit_dialog_kind(|kind| if let DialogKind::Search(form) = kind {
        assert_eq!(form.locations, [left.to_string_lossy().into_owned()], "the current folder is the first location");
        form.files = false;
    });
    search(&mut app, &ctx, super::search_dialog::SearchAction::Search);
    assert_eq!(app.msgboxes[0].text, "파일 또는 폴더를 하나 이상 선택하세요.");
    app.answer_message_box(1, &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::Search(form) = kind {
        form.files = true;
        form.date_from = "2020-13-40".into();
    });
    search(&mut app, &ctx, super::search_dialog::SearchAction::Search);
    assert!(app.msgboxes[0].text.starts_with("날짜를 선택하고"));
    app.answer_message_box(1, &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::Search(form) = kind {
        form.date_from.clear();
        form.root = root.join("missing").to_string_lossy().into_owned();
    });
    search(&mut app, &ctx, super::search_dialog::SearchAction::Add);
    assert_eq!(app.msgboxes[0].text, "추가할 폴더가 존재하지 않습니다.");
    app.answer_message_box(1, &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::Search(form) = kind { form.patterns = "*alpha*".into(); });
    search(&mut app, &ctx, super::search_dialog::SearchAction::Search);
    wait_job(&mut app, &ctx);
    let mut names: Vec<String> = app.panes[0].entries.iter().map(|e| e.name.clone()).collect();
    names.sort();
    assert_eq!(names, ["Alphabet.doc", "alpha.txt", "inner-alpha.txt"], "recursive name search, results become the list");
    assert!(app.status.contains('3'), "{}", app.status);
    assert_eq!(app.settings.strings("SavedSearchPatterns")[0], "*alpha*", "the pattern history");
    app.dispatch("refresh", &ctx);
    wait_listing(&mut app, &ctx);
    assert!(app.panes[0].entries.iter().any(|e| e.name == "beta.txt"), "F5 brings back the folder");
    fs::remove_dir_all(root).unwrap();
}

thread_local! {
    static CLIPBOARD: RefCell<Option<String>> = const { RefCell::new(None) };
}
fn capture_clipboard(text: &str) -> Result<(), String> {
    CLIPBOARD.with(|c| *c.borrow_mut() = Some(text.to_owned()));
    Ok(())
}
/// 모달 진행 창도 그리는 한 장면.
fn frame_with_progress(app: &mut App, ctx: &egui::Context) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        app.original_dialog_ui(ctx);
        app.modal_progress_ui(ctx);
        app.msgbox_ui(ctx);
    });
}
fn comparison(app: &App) -> &super::compare_dialog::ComparisonForm {
    match &app.original_dialogs.last().expect("a dialog is open").kind {
        DialogKind::Comparison(form) => form,
        _ => panic!("the comparison window is not on top"),
    }
}
fn choice(app: &App) -> (String, String, Vec<String>, usize) {
    let dialog = app.original_dialogs.last().expect("a dialog is open");
    match &dialog.kind {
        DialogKind::Choice(form) => (dialog.title.clone(), form.message.clone(), form.options.clone(), form.selected),
        _ => panic!("no choice dialog"),
    }
}
/// 왼쪽: folder1, alpha.txt, Alphabet.doc, beta.txt, gamma.log. 오른쪽: FOLDER1, beta.txt(같은 크기, 다른 내용), omega.txt.
fn compare_fixture(label: &str, language: &str) -> (PathBuf, egui::Context, App) {
    let (root, ctx, mut app) = fixture(label, language);
    fs::write(root.join("right").join("beta.txt"), b"BETA.TXT").unwrap();
    fs::write(root.join("right").join("omega.txt"), b"omega").unwrap();
    app.refresh(1);
    wait_listing(&mut app, &ctx);
    (root, ctx, app)
}

fn folder_compare(app: &App) -> &super::compare_dialog::FolderCompareForm {
    app.original_dialogs.iter().rev().find_map(|d| if let DialogKind::FolderCompare(form) = &d.kind { Some(form.as_ref()) } else { None }).expect("no TfCompareFolder window")
}
fn wait_compare(app: &mut App, ctx: &egui::Context) {
    let end = Instant::now() + Duration::from_secs(20);
    while folder_compare(app).running.is_some() {
        assert!(Instant::now() < end, "compare timeout");
        app.poll(ctx);
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn folder_compare_follows_original_tfcomparefolder() {
    use super::compare_dialog::FolderCompareAction;
    let (root, ctx, mut app) = compare_fixture("compare", "ko");
    app.dispatch("compare", &ctx);
    assert_eq!(app.original_dialogs[0].title, "폴더 비교", "[formCompareFolder] Form.Caption");
    // 원본 FUN_0041d280: 열 머리글은 두 폴더(끝 "\" 뺌). 처음에는 하위 폴더 꺼짐·내용 비교 켬, 결과 저장 꺼짐, 목록 비어 있음.
    let form = folder_compare(&app);
    assert_eq!((form.left_caption.as_str(), form.right_caption.as_str()), (root.join("left").to_str().unwrap(), root.join("right").to_str().unwrap()));
    assert!(!form.recursive && form.contents && !form.compared && form.rows.is_empty() && form.result.is_none());
    frame(&mut app, &ctx, vec![]);
    // 비교시작(btnOkClick): 창 안에서 비교하고 창은 그대로 있다.
    app.folder_compare_action(FolderCompareAction::Start, &ctx);
    assert!(folder_compare(&app).running.is_some(), "btnStop replaces btnOk while comparing");
    frame(&mut app, &ctx, vec![]);
    wait_compare(&mut app, &ctx);
    assert_eq!(app.original_dialogs.len(), 1, "the same window shows the result");
    assert_eq!(app.status, "비교 완료 · 변경 5개");
    let form = folder_compare(&app);
    assert!(form.compared, "btnSave is enabled after a comparison");
    // 원본 FUN_0041e0b4: FOLDERCOMPARE_RESULT(같음 1, 다름 1, 왼쪽에만 3, 오른쪽에만 1).
    let template = app.language_pack.text("RES.FOLDERCOMPARE_RESULT", "?");
    let expected = ["1", "1", "3", "1"].iter().fold(template, |text, n| text.replacen("%d", n, 1));
    assert_eq!(form.result.as_deref(), Some(expected.as_str()));
    // 원본 0x41d464 줄: 왼쪽 이름, 비교결과("=", "< >", "<  ", "  >"), 오른쪽 이름(없는 쪽은 빈 칸). 같은 크기 파일은 내용, 폴더는 같음.
    let cells: Vec<[String; 3]> = (0..form.rows.len()).map(|r| [form.cell(r, 1), form.cell(r, 2), form.cell(r, 3)]).collect();
    let row = |l: &str, m: &str, r: &str| [l.to_owned(), m.to_owned(), r.to_owned()];
    assert_eq!(
        cells,
        [
            row("alpha.txt", "<  ", ""),
            row("Alphabet.doc", "<  ", ""),
            row("beta.txt", "< >", "beta.txt"),
            row("folder1", "=", "FOLDER1"),
            row("gamma.log", "<  ", ""),
            row("", "  >", "omega.txt"),
        ]
    );
    // 원본 lvListCompare: 비교결과 열은 결과 번호(같음 0 < 다름 1 < 왼쪽에만 2 < 오른쪽에만 3) 다음 왼쪽 이름, 늘 오름차순. 0열은 정렬하지 않음.
    app.edit_dialog_kind(|kind| if let DialogKind::FolderCompare(form) = kind {
        form.sort(0);
        assert_eq!(form.grid.sort, None);
        form.sort(2);
        let order: Vec<String> = form.grid.order.iter().map(|&r| form.rows[r].relative.clone()).collect();
        assert_eq!(order, ["folder1", "beta.txt", "alpha.txt", "Alphabet.doc", "gamma.log", "omega.txt"]);
        form.sort(2);
        assert_eq!(form.grid.sort, Some((2, false)), "clicking again keeps ascending order");
        form.sort(3);
        assert_eq!(form.rows[form.grid.order[0]].relative, "alpha.txt", "left-only rows have an empty right name");
        form.sort(2);
    });
    // 결과저장(btnSaveClick): "compare result.html", HTML 한 가지. 보이는 순서, 짝수 줄 line1·홀수 줄 line2, UTF-8 BOM, 줄마다 CRLF.
    app.save_dialog = pick_save_target;
    SAVE_TARGET.with(|t| *t.borrow_mut() = Some(root.join("report.html")));
    app.folder_compare_action(FolderCompareAction::Save, &ctx);
    let page = fs::read(root.join("report.html")).unwrap();
    assert!(page.starts_with(&[0xEF, 0xBB, 0xBF]));
    let page = String::from_utf8(page[3..].to_vec()).unwrap();
    let lines: Vec<&str> = page.split("\r\n").collect();
    assert!(lines[0].starts_with("<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//ENhttp://") && lines[0].contains("\t<title>NexusFile Folder Compare</title>\t"));
    assert_eq!(lines[1], format!("<tr class=\"header\"><td align=\"right\" class=\"headleft\">{}</td><td>&nbsp;</td><td class=\"headright\">{}</td></tr>", root.join("left").display(), root.join("right").display()));
    assert_eq!(lines[2..7], ["<tr class=\"line1\">", "<td align=\"right\" class=\"same\">folder1</td>", "<td width=\"50\" align=\"center\" class=\"same\">=</td>", "<td class=\"same\">FOLDER1</td>", "</tr>"]);
    assert_eq!(lines[7..12], ["<tr class=\"line2\">", "<td align=\"right\" class=\"diff\">beta.txt</td>", "<td width=\"50\" align=\"center\" class=\"diff\">&lt; &gt;</td>", "<td class=\"diff\">beta.txt</td>", "</tr>"]);
    assert_eq!(lines[lines.len() - 3..], ["</table></div></body>", "</html>", ""]);
    // 하위 폴더 포함: 같은 창에서 다시 비교하면 folder1 안의 파일도 나온다.
    fs::write(root.join("left").join("folder1").join("inner.txt"), b"x").unwrap();
    app.edit_dialog_kind(|kind| if let DialogKind::FolderCompare(form) = kind {
        form.recursive = true;
        form.contents = false;
    });
    app.folder_compare_action(FolderCompareAction::Start, &ctx);
    wait_compare(&mut app, &ctx);
    assert!(folder_compare(&app).rows.iter().any(|r| r.relative == "folder1\\inner.txt" && r.kind == ndr::folder_compare::DifferenceKind::LeftOnly));
    // 중단(btnStopClick)과 닫기. 닫으면 비교 중이어도 멈춘다.
    app.folder_compare_action(FolderCompareAction::Start, &ctx);
    app.folder_compare_action(FolderCompareAction::Stop, &ctx);
    wait_compare(&mut app, &ctx);
    assert!(folder_compare(&app).result.is_some() && folder_compare(&app).compared);
    app.folder_compare_action(FolderCompareAction::Close, &ctx);
    assert!(app.original_dialogs.is_empty());
    // Esc도 닫기(btnCancel Cancel=True).
    app.dispatch("compare", &ctx);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.original_dialogs.is_empty(), "Esc closes the window");
    // 창이 없을 때 끝난 비교: 취소면 상태 줄만. 없는 폴더 오류는 C# RunSafely 상자.
    let run = |source_is_left| super::compare_dialog::CompareRun { left: root.join("left"), right: root.join("right"), source_is_left };
    app.compare_done(run(None), Err("Cancelled".into()), true);
    assert_eq!(app.status, "폴더 비교를 취소했습니다.");
    app.compare_done(run(Some(true)), Err("Cancelled".into()), true);
    assert_eq!(app.status, "폴더 동기화를 취소했습니다.");
    app.dispatch("compare", &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::FolderCompare(form) = kind {
        form.left = root.join("gone");
    });
    app.folder_compare_action(FolderCompareAction::Start, &ctx);
    wait_compare(&mut app, &ctx);
    assert_eq!((app.msgboxes[0].title.as_str(), app.msgboxes[0].flags), ("NDR", MB_OK | ndr::msgbox::MB_ICONEXCLAMATION), "RunSafely");
    assert!(app.msgboxes[0].text.starts_with("폴더를 찾을 수 없습니다: "));
    app.answer_message_box(1, &ctx);
    app.folder_compare_action(FolderCompareAction::Close, &ctx);
    // 같은 폴더이거나 서로 포함하면 시작하지 않는다.
    app.navigate(1, root.join("left").join("folder1"), true);
    wait_listing(&mut app, &ctx);
    app.dispatch("compare", &ctx);
    assert!(app.original_dialogs.is_empty());
    assert_eq!(app.msgboxes[0].text, "비교하거나 동기화할 두 폴더는 서로 같거나 포함 관계일 수 없습니다.");
    app.answer_message_box(1, &ctx);
    // 동기화의 한국어 글(방향 고르기에서 취소하면 아무것도 하지 않는다).
    app.navigate(1, root.join("right"), true);
    wait_listing(&mut app, &ctx);
    app.dispatch("synchronize", &ctx);
    let (title, message, options, selected) = choice(&app);
    assert_eq!((title.as_str(), options, selected), ("폴더 동기화 방향", vec!["왼쪽 → 오른쪽".to_owned(), "오른쪽 → 왼쪽".to_owned()], 0));
    assert_eq!(message, "어느 쪽 파일로 다른 폴더를 갱신할까요? 동기화는 파일을 복사하고 대상에서 추가 파일을 삭제하지 않습니다.");
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.original_dialogs.is_empty() && app.job.is_none(), "Esc cancels the choice");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_sync_follows_synchronizefoldersasync() {
    let (root, ctx, mut app) = compare_fixture("sync", "en");
    let (left, right) = (root.join("left"), root.join("right"));
    app.dispatch("synchronize", &ctx);
    let (title, _, options, selected) = choice(&app);
    assert_eq!((title.as_str(), options, selected), ("Synchronization direction", vec!["Left → Right".to_owned(), "Right → Left".to_owned()], 0));
    frame(&mut app, &ctx, vec![]);
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    let (title, _, options, selected) = choice(&app);
    assert_eq!((title.as_str(), options[selected].as_str()), ("Compare file contents", "Include file contents"), "UseContentComparison");
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert!(app.job.as_ref().is_some_and(|j| j.title == "Synchronization compare"), "Enter accepts the choice");
    wait_job(&mut app, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "Synchronization preview");
    let form = comparison(&app);
    let plan: Vec<&str> = form.rows.iter().map(|r| r.difference.relative.as_str()).collect();
    assert_eq!(plan, ["alpha.txt", "Alphabet.doc", "beta.txt", "gamma.log"], "differences and left-only items; the right-only file stays out");
    assert_eq!(form.status(false), "4 different · 4 / 4 selected · 0 unreadable");
    frame(&mut app, &ctx, vec![]);
    app.comparison_action(super::compare_dialog::ComparisonAction::SelectNone, &ctx);
    app.comparison_action(super::compare_dialog::ComparisonAction::Commit, &ctx);
    assert_eq!((app.msgboxes[0].title.as_str(), app.msgboxes[0].text.as_str()), ("Synchronization preview", "Select at least one file to synchronize."));
    app.answer_message_box(1, &ctx);
    app.comparison_action(super::compare_dialog::ComparisonAction::SelectAll, &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::Comparison(form) = kind { form.rows[3].include = false; });
    assert_eq!(comparison(&app).status(false), "4 different · 3 / 4 selected · 0 unreadable");
    app.comparison_action(super::compare_dialog::ComparisonAction::Commit, &ctx);
    let (title, _, options, selected) = choice(&app);
    assert_eq!((title.as_str(), options.len(), selected), ("Existing items", 4, 0));
    app.answer_dialog(DialogResult::Choice(Some(1)), &ctx);
    assert_eq!(app.msgboxes[0].title, "Run synchronization");
    assert_eq!(app.msgboxes[0].text, format!("Synchronize the 3 selected files from\n{}\nto\n{}\n? Extra files in the destination are not deleted.", left.display(), right.display()));
    assert_eq!(app.msgboxes[0].flags, MB_YESNO | ndr::msgbox::MB_ICONQUESTION);
    app.answer_message_box(6, &ctx);
    assert!(app.job.as_ref().is_some_and(|j| j.title == "Synchronize folders" && j.modal.is_none() && j.progress.as_ref().is_some_and(|p| p.allow_pause && p.total_files == Some(3))), "FileOperationProgressWindow with pause");
    frame_with_progress(&mut app, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(app.status, "Synchronization complete · 3 files copied · 0 folders created · 0 skipped · 0 errors");
    assert_eq!(fs::read(right.join("beta.txt")).unwrap(), b"beta.txt", "Overwrite replaced the different file");
    assert!(right.join("alpha.txt").is_file() && !right.join("gamma.log").exists(), "only the checked items");
    assert!(right.join("omega.txt").is_file(), "extra destination files are kept");
    assert!(app.panes[1].entries.iter().any(|e| e.name == "alpha.txt"), "both panes are refreshed");
    // 오른쪽 → 왼쪽, 크기·수정 날짜: 복사한 파일은 시각을 지켰으므로 오른쪽에만 있는 omega.txt만 남는다.
    app.dispatch("synchronize", &ctx);
    app.answer_dialog(DialogResult::Choice(Some(1)), &ctx);
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    wait_job(&mut app, &ctx);
    let plan: Vec<String> = comparison(&app).rows.iter().map(|r| r.difference.relative.clone()).collect();
    assert_eq!(plan, ["omega.txt"]);
    app.comparison_action(super::compare_dialog::ComparisonAction::Commit, &ctx);
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    assert!(app.msgboxes[0].text.starts_with(&format!("Synchronize the 1 selected files from\n{}\nto\n{}\n", right.display(), left.display())));
    app.answer_message_box(7, &ctx);
    assert!(app.job.is_none() && !left.join("omega.txt").exists(), "No does nothing");
    // 같은 흐름을 예로.
    app.dispatch("synchronize", &ctx);
    app.answer_dialog(DialogResult::Choice(Some(1)), &ctx);
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    wait_job(&mut app, &ctx);
    app.comparison_action(super::compare_dialog::ComparisonAction::Commit, &ctx);
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    app.answer_message_box(6, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(fs::read(left.join("omega.txt")).unwrap(), b"omega");
    // 차이가 없으면 알리기만 한다.
    app.dispatch("synchronize", &ctx);
    app.answer_dialog(DialogResult::Choice(Some(1)), &ctx);
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    wait_job(&mut app, &ctx);
    assert!(app.original_dialogs.is_empty());
    assert_eq!(app.status, "There are no differences to synchronize.");
    // 읽지 못한 항목이 있으면 멈추고, 복사 결과의 오류는 글 창으로.
    let run = super::compare_dialog::CompareRun { left: left.clone(), right: right.clone(), source_is_left: Some(true) };
    app.compare_done(run, Ok((vec![], vec!["X: denied".into()])), false);
    assert_eq!(app.text.as_ref().map(|t| t.title.as_str()), Some("Folder synchronization stopped"));
    assert!(app.text.as_ref().unwrap().text.ends_with("\n\nX: denied"));
    assert_eq!(app.status, "Synchronization stopped · 1 unreadable items");
    app.text = None;
    let failed = ndr::folder_compare::SyncResult { copied: 1, created_directories: 0, skipped_existing: 2, errors: vec!["a: denied".into()] };
    app.sync_done(Ok(failed), false);
    assert_eq!(app.status, "Synchronization complete · 1 files copied · 0 folders created · 2 skipped · 1 errors");
    assert_eq!(app.text.as_ref().map(|t| (t.title.as_str(), t.text.as_str())), Some(("Synchronization errors", "a: denied")));
    app.sync_done(Err("disk gone".into()), false);
    assert_eq!((app.msgboxes[0].title.as_str(), app.msgboxes[0].text.as_str()), ("Synchronization error", "disk gone"));
    assert_eq!(app.status, "The synchronization could not be completed.");
    app.answer_message_box(1, &ctx);
    app.sync_done(Ok(ndr::folder_compare::SyncResult::default()), true);
    assert_eq!(app.status, "Synchronization canceled. Items already completed are kept.");
    wait_listing(&mut app, &ctx);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn modal_progress_cancels_and_closes_itself() {
    let (root, ctx, mut app) = fixture("modal", "ko");
    let modal = ModalProgress { stop: "중단".into(), stopping: Some("중단 중…".into()), stopping_text: "현재 작업을 안전하게 멈추는 중…".into(), kind: ModalKind::Message };
    let (release_tx, release_rx) = mpsc::channel::<()>();
    assert!(app.start_modal_task("폴더 비교".into(), modal, move |_, cancel| {
        let _ = release_rx.recv_timeout(Duration::from_secs(10));
        let _ = cancel;
        Ok(())
    }));
    assert!(!app.start_task("other".into(), |_, _| Ok(())), "one operation at a time");
    frame_with_progress(&mut app, &ctx);
    frame_with_progress_events(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.job.as_ref().unwrap().cancel.load(Ordering::Relaxed), "Esc asks the task to stop");
    let status = app.status.clone();
    release_tx.send(()).unwrap();
    wait_job(&mut app, &ctx);
    assert!(app.job.is_none(), "closed without a Close button");
    assert_eq!(app.status, status, "the status line is left to the result handler");
    fs::remove_dir_all(root).unwrap();
}
fn frame_with_progress_events(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.modal_progress_ui(ctx));
}

static KO_EMPTIED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static SHELL_EMPTIED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn fake_shell_empty() -> Result<(), String> {
    SHELL_EMPTIED.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
static EN_EMPTIED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn fake_empty_ko() -> Result<(), String> {
    KO_EMPTIED.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
fn fake_empty_en() -> Result<(), String> {
    EN_EMPTIED.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
fn failing_empty() -> Result<(), String> {
    Err("0x8000ffff".into())
}
fn cleanup_options(app: &App) -> &super::cleanup_dialog::CleanupOptionsForm {
    match &app.original_dialogs.last().expect("a dialog is open").kind {
        DialogKind::CleanupOptions(form) => form,
        _ => panic!("the cleanup options window is not on top"),
    }
}
fn cleanup_results(app: &App) -> &super::cleanup_dialog::CleanupResultsForm {
    match &app.original_dialogs.last().expect("a dialog is open").kind {
        DialogKind::CleanupResults(form) => form,
        _ => panic!("the cleanup results window is not on top"),
    }
}
/// 정리 대상만 담은 격리 폴더: a.tmp, b.bak, c.tmp(숨김+읽기 전용), keep.txt, sub/d.tmp(각 5바이트).
fn junk_folder(root: &Path) -> PathBuf {
    let junk = root.join("junk");
    fs::create_dir_all(junk.join("sub")).unwrap();
    for name in ["a.tmp", "b.bak", "c.tmp", "keep.txt", "sub/d.tmp"] {
        fs::write(junk.join(name), b"12345").unwrap();
    }
    native::set_attributes(&junk.join("c.tmp"), true, true).unwrap();
    junk
}

#[test]
fn computer_cleanup_follows_rundiskcleanupasync() {
    let (root, ctx, mut app) = fixture("cleanup", "ko");
    app.recycle_emptier = fake_empty_ko;
    let junk = junk_folder(&root);
    app.dispatch("cleanup", &ctx);
    assert_eq!(app.original_dialogs[0].title, "컴퓨터 청소", "[formCleanup] Form.Caption");
    let form = cleanup_options(&app);
    assert!(form.system_temp && form.internet_temp && !form.cookies && form.recycle && !form.user_defined, "CleanupOptionsWindow defaults");
    assert_eq!(form.extensions, ndr::cleanup::DEFAULT_EXTENSIONS);
    frame(&mut app, &ctx, vec![]);
    let accept = |app: &mut App, ctx: &egui::Context| app.cleanup_action(super::cleanup_dialog::CleanupAction::Accept, ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::CleanupOptions(form) = kind { form.system_temp = false; form.internet_temp = false; form.recycle = false; });
    accept(&mut app, &ctx);
    assert_eq!((app.msgboxes[0].title.as_str(), app.msgboxes[0].text.as_str()), ("컴퓨터 청소", "정리할 항목을 하나 이상 선택하세요."));
    app.answer_message_box(1, &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::CleanupOptions(form) = kind { form.user_defined = true; });
    accept(&mut app, &ctx);
    assert_eq!(app.msgboxes[0].text, "사용자 지정 위치에서 먼저 검색하세요.");
    app.answer_message_box(1, &ctx);
    // 사용자 지정 검색: 격리 폴더만, tmp;bak.
    let junk_text = junk.to_string_lossy().into_owned();
    app.edit_dialog_kind(|kind| if let DialogKind::CleanupOptions(form) = kind {
        form.locations = vec![junk_text.clone()];
        form.extensions = "tmp;bak".into();
    });
    app.cleanup_action(super::cleanup_dialog::CleanupAction::Search, &ctx);
    assert!(app.job.as_ref().is_some_and(|j| j.title == "정리 위치 검색" && j.modal.is_some()), "CleanupScanProgressWindow");
    frame_with_progress(&mut app, &ctx);
    wait_job(&mut app, &ctx);
    let mut found: Vec<String> = cleanup_options(&app).results.iter().map(|(c, checked)| { assert!(checked); c.name.clone() }).collect();
    found.sort();
    assert_eq!(found, ["a.tmp", "b.bak", "c.tmp", "d.tmp"]);
    assert!(cleanup_options(&app).searched);
    frame(&mut app, &ctx, vec![]);
    // 위치가 바뀌면 다시 검색해야 한다.
    // 원본 btnAdd = 폴더 고르기 창(Browse). 취소하면 아무것도 바뀌지 않는다.
    app.folder_picker = |_, _| None;
    app.cleanup_action(super::cleanup_dialog::CleanupAction::Browse, &ctx);
    assert!(app.msgboxes.is_empty());
    assert!(cleanup_options(&app).searched, "a cancelled picker changes nothing");
    app.edit_dialog_kind(|kind| if let DialogKind::CleanupOptions(form) = kind { form.extensions = "tmp".into(); form.invalidate(); });
    assert!(!cleanup_options(&app).searched && cleanup_options(&app).results.is_empty(), "InvalidateUserDefinedSearch");
    app.edit_dialog_kind(|kind| if let DialogKind::CleanupOptions(form) = kind { form.extensions = "tmp;bak".into(); });
    app.cleanup_action(super::cleanup_dialog::CleanupAction::Search, &ctx);
    wait_job(&mut app, &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::CleanupOptions(form) = kind {
        for row in form.results.iter_mut() {
            row.1 = row.0.name != "b.bak";
        }
    });
    accept(&mut app, &ctx);
    assert_eq!(app.settings.strings("UserDefinedCleanupLocations"), [junk_text.clone()]);
    assert_eq!(app.settings.text("UserDefinedCleanupExtensions", "?"), "tmp;bak");
    // 결과 창: 고른 사용자 지정 파일 3개, 모두 선택.
    assert_eq!(app.original_dialogs.last().unwrap().title, "검색 결과(V)");
    assert_eq!(cleanup_results(&app).status(true), "선택한 파일 3 / 3개 · 15 B");
    frame(&mut app, &ctx, vec![]);
    app.cleanup_action(super::cleanup_dialog::CleanupAction::Accept, &ctx);
    assert_eq!((app.msgboxes[0].title.as_str(), app.msgboxes[0].text.as_str()), ("검색 결과(V)", "정말로 선택된 항목들의 파일을 삭제하시겠습니까?"), "RES.MSG_CLEANUP");
    app.answer_message_box(7, &ctx);
    assert!(junk.join("a.tmp").exists() && app.original_dialogs.len() == 1, "No keeps the window");
    app.cleanup_action(super::cleanup_dialog::CleanupAction::Accept, &ctx);
    app.answer_message_box(6, &ctx);
    // 보호 속성 파일은 TfDelConfirm: 아니요면 남긴다.
    wait_dialog_or_job(&mut app, &ctx);
    assert!(matches!(app.original_dialogs.last().unwrap().kind, DialogKind::DeleteConfirm { .. }));
    app.answer_dialog(DialogResult::DeleteConfirm(DeleteConfirmChoice::No), &ctx);
    assert!(app.job.as_ref().is_some_and(|j| j.title == "선택 파일 정리"));
    wait_job(&mut app, &ctx);
    assert_eq!(app.status, "2개 파일을 정리했습니다. 휴지통 비우기 안 함. 실패 0개, 건너뜀 1개");
    // 원본 FUN_0041c77c: MSG_CLEANUP_DONE(정리 창 제목, 정보 아이콘).
    assert_eq!((app.msgboxes[0].title.clone(), app.msgboxes[0].text.clone(), app.msgboxes[0].flags), (app.language_pack.text("formCleanup.Form.Caption", "?"), app.language_pack.text("RES.MSG_CLEANUP_DONE", "?"), MB_OK | ndr::msgbox::MB_ICONASTERISK));
    app.answer_message_box(1, &ctx);
    assert!(!junk.join("a.tmp").exists() && !junk.join("sub/d.tmp").exists());
    assert!(junk.join("c.tmp").exists() && junk.join("b.bak").exists() && junk.join("keep.txt").exists());
    assert_eq!(KO_EMPTIED.load(Ordering::SeqCst), 0, "the Recycle Bin was not selected");
    // 휴지통 비우기 명령(가짜 비우기).
    // 원본 miEmptyRecycleBinClick(스킨 테마): NDR 상자 없이 SHEmptyRecycleBinW(flags 0, 셸 확인 창).
    app.shell_recycle_emptier = fake_shell_empty;
    app.dispatch("emptyRecycle", &ctx);
    assert!(app.msgboxes.is_empty());
    let end = Instant::now() + Duration::from_secs(10);
    while SHELL_EMPTIED.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < end, "shell recycle timeout");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(KO_EMPTIED.load(Ordering::SeqCst), 0);
    app.answer_message_box(1, &ctx);
    wait_listing(&mut app, &ctx);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_results_and_recycle_bin_follow_cleanupresultswindow() {
    let (root, ctx, mut app) = fixture("cleanup-en", "en");
    app.recycle_emptier = fake_empty_en;
    let junk = junk_folder(&root);
    let cancel = AtomicBool::new(false);
    let found = ndr::cleanup::scan(&[(junk.clone(), ndr::cleanup::parse_patterns("bak"))], &cancel, &mut |_, _, _| {}).unwrap();
    let mut extra = found.clone();
    extra.push(ndr::cleanup::Candidate { name: "keep.txt".into(), full_path: junk.join("keep.txt"), size: 5 });
    let options = super::cleanup_dialog::CleanupOptions { recycle: true, user_defined: true, user_results: extra, ..Default::default() };
    app.run_cleanup(options.clone());
    assert_eq!(app.original_dialogs.last().unwrap().title, "Search Result(V)");
    let form = cleanup_results(&app);
    assert_eq!(form.status(false), "Selected files 2 / 2 · 10 B · Empty Recycle Bin is also selected");
    frame(&mut app, &ctx, vec![]);
    // 아무것도 고르지 않고 휴지통만: MSG_CLEANUP 없이 비우기 확인.
    app.cleanup_action(super::cleanup_dialog::CleanupAction::SelectNone, &ctx);
    app.cleanup_action(super::cleanup_dialog::CleanupAction::Accept, &ctx);
    assert_eq!(app.msgboxes[0].title, "Confirm Empty Recycle Bin");
    assert_eq!(app.msgboxes[0].text, "This empties all Recycle Bins on all drives.\n\n\nEmptying the Recycle Bin cannot be undone. Continue?");
    app.answer_message_box(7, &ctx);
    app.cleanup_action(super::cleanup_dialog::CleanupAction::SelectAll, &ctx);
    app.cleanup_action(super::cleanup_dialog::CleanupAction::Accept, &ctx);
    assert_eq!(app.msgboxes[0].text, "Are you sure you want to delete all selected files?");
    app.answer_message_box(6, &ctx);
    assert_eq!(app.msgboxes[0].text, "This empties all Recycle Bins on all drives.\n\nThen it will permanently delete 2 selected files.\n\nEmptying the Recycle Bin cannot be undone. Continue?");
    app.answer_message_box(6, &ctx);
    assert!(app.original_dialogs.is_empty());
    assert_eq!(app.status, "Emptying the Recycle Bin…");
    let end = Instant::now() + Duration::from_secs(10);
    while !app.status.starts_with("Cleaned up") {
        assert!(Instant::now() < end, "cleanup timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.status, "Cleaned up 2 files. Recycle Bin emptying: complete. Failed: 0, skipped: 0");
    assert_eq!(app.msgboxes[0].text, "Cleanup completed.", "original MSG_CLEANUP_DONE");
    app.answer_message_box(1, &ctx);
    assert_eq!(EN_EMPTIED.load(Ordering::SeqCst), 1);
    assert!(!junk.join("b.bak").exists() && !junk.join("keep.txt").exists() && junk.join("a.tmp").exists());
    wait_listing(&mut app, &ctx);
    // 휴지통 비우기가 실패하면 파일 정리도 하지 않는다.
    app.recycle_emptier = failing_empty;
    let only = vec![ndr::cleanup::Candidate { name: "a.tmp".into(), full_path: junk.join("a.tmp"), size: 5 }];
    app.run_cleanup(super::cleanup_dialog::CleanupOptions { user_results: only, ..options.clone() });
    app.cleanup_action(super::cleanup_dialog::CleanupAction::Accept, &ctx);
    app.answer_message_box(6, &ctx);
    app.answer_message_box(6, &ctx);
    while app.msgboxes.is_empty() {
        assert!(Instant::now() < end, "recycle timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!((app.msgboxes[0].title.as_str(), app.msgboxes[0].text.as_str()), ("File cleanup", "Could not empty the Recycle Bin. File cleanup was canceled.\n\n0x8000ffff"));
    app.answer_message_box(1, &ctx);
    assert!(junk.join("a.tmp").exists());
    // 후보 없이 휴지통만이면 단추 글이 바뀐다(창을 그려 본다).
    app.run_cleanup(super::cleanup_dialog::CleanupOptions { user_results: vec![], ..options.clone() });
    assert!(cleanup_results(&app).candidates.is_empty());
    frame(&mut app, &ctx, vec![]);
    app.answer_dialog(DialogResult::CleanupResults(None), &ctx);
    // 위치·알림 글.
    let temp = super::cleanup_dialog::cleanup_roots(&super::cleanup_dialog::CleanupOptions { system_temp: true, ..Default::default() });
    assert_eq!(temp, [(std::env::temp_dir(), vec![])], "Empty Temporary Folder scans only Path.GetTempPath()");
    app.run_cleanup(super::cleanup_dialog::CleanupOptions::default());
    assert_eq!(app.status, "No cleanup folders are available to scan.");
    let plain = super::cleanup_dialog::CleanupOptions { system_temp: true, ..Default::default() };
    app.cleanup_scanned(super::cleanup_dialog::ScanPurpose::Cleanup(plain.clone()), Ok(vec![]), false);
    assert_eq!(app.status, "No files matched the cleanup filters.");
    app.cleanup_scanned(super::cleanup_dialog::ScanPurpose::Cleanup(plain), Err("Cancelled".into()), true);
    assert_eq!(app.status, "Cleanup scan canceled.");
    let done = super::cleanup_dialog::CleanupDone { requested: 1, to_delete: 1, skipped: 0, recycle: false };
    app.cleanup_finished(done, ndr::cleanup::DeleteResult { errors: vec!["x: denied".into()], successful: 0, cancelled: false }, false);
    assert_eq!(app.status, "Cleaned up 0 files. Recycle Bin emptying: not selected. Failed: 1, skipped: 0");
    assert_eq!(app.text.as_ref().map(|t| (t.title.as_str(), t.text.as_str())), Some(("Files that could not be cleaned up", "x: denied")));
    wait_listing(&mut app, &ctx);
    fs::remove_dir_all(root).unwrap();
}

thread_local! {
    static PICKED_FOLDER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}
fn pick_folder(_: &str, _: Option<&Path>) -> Option<PathBuf> {
    PICKED_FOLDER.with(|p| p.borrow().clone())
}
fn top_prompt(app: &App) -> (String, String, String) {
    let dialog = app.original_dialogs.last().expect("a dialog is open");
    match &dialog.kind {
        DialogKind::Prompt { message, value, .. } => (dialog.title.clone(), message.clone(), value.clone()),
        _ => panic!("not a TfGetText dialog"),
    }
}
fn favorites(app: &App) -> &super::favorites_dialog::FavoritesForm {
    match &app.original_dialogs.last().expect("a dialog is open").kind {
        DialogKind::Favorites(form) => form,
        _ => panic!("the favorites window is not on top"),
    }
}

#[test]
fn favorites_window_follows_managefavoritesasync() {
    let (root, ctx, mut app) = fixture("favorites", "ko");
    app.folder_picker = pick_folder;
    let (left, right) = (root.join("left"), root.join("right"));
    app.settings.set_strings("Favorites", vec![right.to_string_lossy().into_owned(), root.join("gone").to_string_lossy().into_owned()]);
    app.settings.set_strings("FavoriteNames", vec!["오른쪽".into(), String::new()]);
    app.dispatch("favorites", &ctx);
    assert_eq!(app.original_dialogs[0].title, "즐겨찾기 구성", "[formOrgFavorites] Form.Caption");
    let form = favorites(&app);
    assert_eq!(form.items.len(), 20, "20 slots");
    assert_eq!(form.items[0], ("오른쪽".to_owned(), right.to_string_lossy().into_owned()));
    assert_eq!(form.items[1], (String::new(), String::new()), "an entry without a name is an empty slot");
    assert_eq!((form.gestures[0].as_str(), form.gestures[10].as_str()), ("Shift+F1", "Ctrl+Shift+F1"), "GetFavoriteGesture defaults");
    frame(&mut app, &ctx, vec![]);
    let act = |app: &mut App, action, ctx: &egui::Context| app.favorites_action(action, ctx);
    // 현재 폴더 추가: 첫 빈 칸, 이름은 폴더 이름. 같은 경로는 한 번.
    act(&mut app, super::favorites_dialog::FavoritesAction::AddCurrent, &ctx);
    act(&mut app, super::favorites_dialog::FavoritesAction::AddCurrent, &ctx);
    let form = favorites(&app);
    assert_eq!((form.items[1].0.as_str(), form.selected), ("left", Some(1)));
    assert_eq!(form.items.iter().filter(|(_, p)| !p.is_empty()).count(), 2);
    // 경로 추가(입력 상자, 없는 폴더는 무시).
    act(&mut app, super::favorites_dialog::FavoritesAction::AddPath, &ctx);
    assert_eq!(top_prompt(&app).0, "즐겨찾기 추가");
    app.answer_dialog(DialogResult::Text(Some(root.join("nope").to_string_lossy().into_owned())), &ctx);
    assert_eq!(favorites(&app).items.iter().filter(|(_, p)| !p.is_empty()).count(), 2);
    let folder = left.join("folder1");
    act(&mut app, super::favorites_dialog::FavoritesAction::AddPath, &ctx);
    app.answer_dialog(DialogResult::Text(Some(folder.to_string_lossy().into_owned())), &ctx);
    assert_eq!(favorites(&app).items[2], ("folder1".to_owned(), folder.to_string_lossy().into_owned()));
    // 이름 바꾸기(빈 이름도 받는다), 경로 바꾸기(폴더 선택), 위/아래, 삭제.
    act(&mut app, super::favorites_dialog::FavoritesAction::Rename, &ctx);
    assert_eq!(top_prompt(&app), ("즐겨찾기 이름".to_owned(), "표시 이름".to_owned(), "folder1".to_owned()));
    app.answer_dialog(DialogResult::Text(Some("작업".into())), &ctx);
    PICKED_FOLDER.with(|p| *p.borrow_mut() = Some(right.join("FOLDER1")));
    act(&mut app, super::favorites_dialog::FavoritesAction::ModifyPath, &ctx);
    assert_eq!(favorites(&app).items[2], ("작업".to_owned(), right.join("FOLDER1").to_string_lossy().into_owned()), "the name stays");
    act(&mut app, super::favorites_dialog::FavoritesAction::Up, &ctx);
    assert_eq!((favorites(&app).items[1].0.as_str(), favorites(&app).selected), ("작업", Some(1)));
    act(&mut app, super::favorites_dialog::FavoritesAction::Down, &ctx);
    act(&mut app, super::favorites_dialog::FavoritesAction::Delete, &ctx);
    assert_eq!(favorites(&app).items[2], (String::new(), String::new()), "the first 20 slots are emptied, not removed");
    assert_eq!(favorites(&app).items.len(), 20);
    frame(&mut app, &ctx, vec![]);
    act(&mut app, super::favorites_dialog::FavoritesAction::Save, &ctx);
    assert!(app.original_dialogs.is_empty());
    let saved = app.settings.strings("Favorites");
    assert_eq!(saved.len(), 20);
    assert_eq!(&saved[..2], [right.to_string_lossy().into_owned(), left.to_string_lossy().into_owned()]);
    assert_eq!(&app.settings.strings("FavoriteNames")[..3], ["오른쪽".to_owned(), "left".to_owned(), String::new()]);
    // 취소는 바꾸지 않는다.
    app.dispatch("favorites", &ctx);
    act(&mut app, super::favorites_dialog::FavoritesAction::AddCurrent, &ctx);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.original_dialogs.is_empty());
    assert_eq!(app.settings.strings("Favorites")[2], "");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn favorites_menu_and_path_history_follow_the_context_menus() {
    let (root, ctx, mut app) = fixture("favmenu", "en");
    let (left, right) = (root.join("left"), root.join("right"));
    app.settings.set_strings("Favorites", vec![right.to_string_lossy().into_owned(), String::new(), "folder1".into()]);
    app.settings.set_strings("FavoriteNames", vec!["R&ight".into(), String::new(), "Sub".into()]);
    app.settings.values.insert("KeyboardShortcuts".into(), serde_json::json!({"favoriteOpen3": "Ctrl+Alt+3"}));
    // F11 = 즐겨찾기 드롭다운(관리 창이 아니다).
    frame_keys(&mut app, &ctx, vec![key(egui::Key::F11)]);
    let popup = app.popup.as_ref().expect("F11 opens the favorites menu");
    let labels: Vec<(String, Option<char>, String)> = popup.items.iter().filter_map(|item| match item {
        super::favorites_dialog::PopupItem::Entry { label, access, gesture, .. } => Some((label.clone(), *access, gesture.clone())),
        _ => None,
    }).collect();
    // 원본 pmFavorites: 즐겨찾기 줄, 구분선, miAddToFavorites, miOrgFavorites.
    let add = app.language_pack.text("Menu.miAddToFavorites", "?");
    let organize = app.language_pack.text("Menu.miOrgFavorites", "?");
    assert_eq!(labels, [("Right".to_owned(), Some('i'), "Shift+F1".to_owned()), ("3. Sub".to_owned(), Some('3'), "Ctrl+Alt+3".to_owned()), (add, None, String::new()), (organize, None, String::new())], "a name with & keeps its own access key");
    // 원본 miAddToFavoritesClick: TfGetText(TITLE_ADDFAV, MSG_ADDFAV(%s = 폴더), 처음 값 = 폴더 이름) → 확인이면 끝(빈 칸)에 더한다.
    let saved_popup = app.popup.take();
    app.dispatch("addToFavorites", &ctx);
    let (title, message, value) = prompt_of(&app);
    assert_eq!(title, root_text(&app, "TITLE_ADDFAV"));
    assert!(message.contains(&left.to_string_lossy().into_owned()), "{message}");
    assert_eq!(value, left.file_name().unwrap().to_string_lossy());
    app.answer_dialog(DialogResult::Text(Some("Mine".into())), &ctx);
    assert_eq!(app.settings.strings("Favorites")[1], left.to_string_lossy(), "the first empty slot");
    assert_eq!(app.settings.strings("FavoriteNames")[1], "Mine");
    app.popup = saved_popup;
    // 단축 문자 3 → 상대 경로 "folder1"은 현재 폴더 기준.
    popup_frame(&mut app, &ctx, vec![egui::Event::Text("3".into())]);
    assert!(app.popup.is_none());
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, left.join("folder1"));
    // 이동 기록: 뒤로 이동.
    app.dispatch("favoritesMenu", &ctx);
    popup_frame(&mut app, &ctx, vec![egui::Event::Text("i".into())]);
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, right);
    assert_eq!(app.panes[0].back, [left.clone(), left.join("folder1")]);
    app.show_history_menu(0, None);
    let entries: Vec<String> = app.popup.as_ref().unwrap().items.iter().filter_map(|item| match item {
        super::favorites_dialog::PopupItem::Entry { label, .. } => Some(label.clone()),
        super::favorites_dialog::PopupItem::Header(title) => Some(format!("[{title}]")),
        _ => None,
    }).collect();
    assert_eq!(entries, ["[Back]".to_owned(), left.join("folder1").to_string_lossy().into_owned(), left.to_string_lossy().into_owned()], "nearest first");
    for _ in 0..2 {
        popup_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    }
    popup_frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, left, "the second entry");
    assert!(app.panes[0].back.is_empty());
    assert_eq!(app.panes[0].forward, [right.clone(), left.join("folder1")], "skipped entries move to the other side");
    // 즐겨찾기가 없으면(원본) 안내 줄 없이 구분선과 고정 항목 둘만. Esc로 닫힌다.
    app.settings.set_strings("FavoriteNames", vec![]);
    app.dispatch("favoritesMenu", &ctx);
    let items = &app.popup.as_ref().unwrap().items;
    assert!(matches!(items[0], super::favorites_dialog::PopupItem::Separator));
    assert_eq!(items.len(), 3);
    popup_frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.popup.is_none());
    app.dispatch("favoritesMenu", &ctx);
    app.run_popup_action(super::favorites_dialog::PopupAction::ManageFavorites);
    assert_eq!(app.original_dialogs[0].title, "Organize Favorites");
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn address_bar_menu_follows_the_original_pm_address_bar() {
    let (root, ctx, mut app) = fixture("addrmenu", "en");
    let (left, right) = (root.join("left"), root.join("right"));
    app.clipboard_writer = capture_clipboard;
    app.settings.set_strings("Favorites", vec![right.to_string_lossy().into_owned(), String::new(), left.to_string_lossy().into_owned()]);
    app.settings.set_strings("FavoriteNames", vec!["R&ight".into(), "Empty".into(), String::new()]);
    // 원본 FUN_0048d10c: 주소 복사, 주소 편집, "-", 이름과 경로가 있는 즐겨찾기(번호 없이), "-", 즐겨찾기 추가·편집. 마우스 위치에 연다.
    app.show_address_bar_menu(1, egui::pos2(30.0, 40.0));
    let popup = app.popup.as_ref().expect("the address bar menu");
    assert_eq!(popup.position, Some(egui::pos2(30.0, 40.0)));
    let rows: Vec<String> = popup.items.iter().map(|item| match item {
        super::favorites_dialog::PopupItem::Entry { label, .. } => label.clone(),
        super::favorites_dialog::PopupItem::Separator => "-".into(),
        super::favorites_dialog::PopupItem::Header(title) => format!("[{title}]"),
    }).collect();
    let text = |key: &str| app.language_pack.text(key, "?");
    assert_eq!(rows, [text("Menu.mipAddressCopy"), text("Menu.mipAddressEdit"), "-".into(), "Right".into(), "-".into(), text("Menu.miAddToFavorites"), text("Menu.miOrgFavorites")]);
    assert_eq!(app.active, 1);
    // 주소 복사 = 그 패널의 경로.
    app.run_popup_action(super::favorites_dialog::PopupAction::CopyAddress(1));
    assert_eq!(CLIPBOARD.with(|c| c.borrow().clone()), Some(app.panes[1].path.to_string_lossy().into_owned()));
    // 주소 편집 = 아래 화살표 두 번(장면마다 한 번) + Enter.
    popup_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    popup_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    popup_frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert!(app.popup.is_none());
    assert!(app.panes[1].address_editing, "Edit Address starts editing that pane's address");
    assert_eq!(app.panes[1].address, app.panes[1].path.to_string_lossy());
    fs::remove_dir_all(root).unwrap();
}
/// 패널 글쇠 처리(App::keyboard)를 거치는 한 장면.
fn frame_keys(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
}
fn popup_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.popup_menu_ui(ctx));
}


/// 원본 lbLocationKeyDown(0x41cac0): 위치 목록에서 Delete = 지우기(btnDeleteClick), Insert = 더하기(btnAddClick).
#[test]
fn cleanup_location_keys_follow_the_original() {
    let (root, ctx, app) = fixture("cleanup-keys", "en");
    let mut form = super::cleanup_dialog::CleanupOptionsForm::new("tmp".into(), &["C:\\junk".into()]);
    form.user_defined = true;
    form.selected_location = Some(0);
    let press = |form: &mut super::cleanup_dialog::CleanupOptionsForm, key: egui::Key| {
        let events = vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }];
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 700.0))), events, ..Default::default() };
        let mut action = None;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                action = super::cleanup_dialog::options_ui(ui, form, &app.language_pack, false, false);
            });
        });
        action
    };
    assert!(matches!(press(&mut form, egui::Key::Delete), Some(super::cleanup_dialog::CleanupAction::Remove)));
    assert!(matches!(press(&mut form, egui::Key::Insert), Some(super::cleanup_dialog::CleanupAction::Browse)));
    form.selected_location = None;
    assert!(press(&mut form, egui::Key::Delete).is_none(), "no selected location: nothing");
    let _ = fs::remove_dir_all(root);
}
