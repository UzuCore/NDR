//! Checks the C# NDR name, description, open, delete and shred flows (original FUN_00432378, FUN_00434630, FUN_004351cc, FUN_0040c31c, FUN_00432958,
//! FUN_00431dd4, FUN_0042c734) in the Rust UI. Only Korean and English are checked.
//! Only isolated temp folders are used. Answers that send to the Recycle Bin (Yes in Recycle Bin mode) are never chosen. No real programs start (shell_runner is replaced).
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
/// The protected item check runs separately, so wait until a dialog or job appears.
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
    // Original TfGetText flags 1 (FUN_00432378) -> btnOKClick (0x428c64): forbidden characters on OK show MSG_INVALID_FILENAME and the window stays.
    app.dispatch("newFolder", &ctx);
    if let Some(DialogKind::Prompt { value, .. }) = app.original_dialogs.last_mut().map(|d| &mut d.kind) {
        *value = "a:b".into();
    }
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(app.msgboxes.len(), 1);
    assert_eq!(app.msgboxes[0].text, app.language_pack.text("RES.MSG_INVALID_FILENAME", "?"));
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::Prompt { file_name: true, .. })), "the input box stays open");
    assert!(!left.join("a:b").exists());
    app.answer_message_box(1, &ctx);
    app.answer_dialog(DialogResult::Text(None), &ctx);

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
    // Original FUN_0042bdc4: ClearSelection after CopySelection (+0xd08). Restore Selection brings back the copied marks.
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
    // Original (skin theme): refreshing restores marks by name, so items after a cancel stay marked.
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
    // Initial focus is No (DefaultDeleteButton 2). Even with focus on a button, the letter D alone presses Delete (&D) (VCL CM_DIALOGCHAR).
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::D)]);
    assert!(app.original_dialogs.is_empty(), "plain D presses &Delete while a button has focus");
    wait_dialog_or_job(&mut app, &ctx);
    wait_job(&mut app, &ctx);
    assert!(!left.join("alpha.txt").exists(), "the Delete button deletes permanently");
    fs::write(left.join("alpha.txt"), "alpha.txt").unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.panes[0].cursor = row(&app, 0, "alpha.txt");
    app.dispatch("deleteRecycle", &ctx);
    answer_warning(&mut app, &ctx, DeleteWarningChoice::Delete);
    wait_dialog_or_job(&mut app, &ctx);
    wait_job(&mut app, &ctx);
    assert!(!left.join("alpha.txt").exists(), "the Delete button deletes permanently");
    app.settings.set_flag("ShowDeleteButton", false);
    // The new list starts at the top (".."), so pick the file (".." is not a target).
    app.panes[0].cursor = row(&app, 0, "beta.txt");
    app.dispatch("deleteRecycle", &ctx);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::DeleteWarning { show_delete: false, .. }));
    answer_warning(&mut app, &ctx, DeleteWarningChoice::No);
    // Original miDeleteClick (0x431c04): Shift held while choosing the menu deletes permanently (always warns, even with ConfirmDelete off).
    app.settings.set_flag("ConfirmDelete", false);
    let _ = ctx.run(egui::RawInput { modifiers: egui::Modifiers::SHIFT, ..Default::default() }, |_| {});
    app.dispatch("deleteRecycle", &ctx);
    match app.original_dialogs.first().map(|d| &d.kind) {
        Some(DialogKind::DeleteWarning { message, show_delete: false, .. }) => assert_eq!(message, &app.delete_warning_message(DeleteWarningMode::Delete, 1, "beta.txt")),
        other => panic!("permanent delete warning expected, got {:?}", other.is_some()),
    }
    answer_warning(&mut app, &ctx, DeleteWarningChoice::No);
    let _ = ctx.run(egui::RawInput::default(), |_| {});
    app.settings.set_flag("ConfirmDelete", true);

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
    // Original TfDelConfirm: btnAll with tab order 0 has the initial focus, so Enter = yes to all.
    assert!(matches!(app.original_dialogs.first().map(|d| &d.kind), Some(DialogKind::DeleteConfirm { .. })));
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    frame(&mut app, &ctx, vec![]);
    assert!(app.original_dialogs.is_empty(), "no second question after Yes to All");
    wait_job(&mut app, &ctx);
    assert!(!left.join("a.txt").exists() && !left.join("b.txt").exists());
    // Original TfDelConfirm: Esc does nothing (no Cancel button); closing it (0x10) stops the whole delete.
    for name in ["c.txt", "d.txt"] {
        fs::write(left.join(name), b"x").unwrap();
        native::set_attributes(&left.join(name), true, false).unwrap();
    }
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.panes[0].selected = [row(&app, 0, "c.txt"), row(&app, 0, "d.txt")].into_iter().collect();
    app.dispatch("deletePermanent", &ctx);
    answer_warning(&mut app, &ctx, DeleteWarningChoice::Yes);
    wait_dialog_or_job(&mut app, &ctx);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    frame(&mut app, &ctx, vec![]);
    assert!(matches!(app.original_dialogs.first().map(|d| &d.kind), Some(DialogKind::DeleteConfirm { .. })), "Esc keeps TfDelConfirm open");
    app.answer_dialog(DialogResult::DeleteConfirm(DeleteConfirmChoice::Cancel), &ctx);
    frame(&mut app, &ctx, vec![]);
    assert!(app.original_dialogs.is_empty() && app.job.as_ref().is_none_or(|job| job.finished), "closing the window starts no delete");
    wait_job(&mut app, &ctx);
    assert!(left.join("c.txt").exists() && left.join("d.txt").exists());
    for name in ["c.txt", "d.txt"] {
        let _ = native::set_attributes(&left.join(name), false, false);
    }
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
    native::set_attributes(&left.join("beta.txt"), true, false).unwrap();
    app.panes[0].selected = [row(&app, 0, "alpha.txt"), row(&app, 0, "beta.txt"), row(&app, 0, "gamma.log")].into_iter().collect();
    app.dispatch("attributes", &ctx);
    let dialog = &app.original_dialogs[0];
    assert_eq!(dialog.title, app.language_pack.text("formChangeAttr.Form.Caption", "?"));
    // Original FUN_0041a0e8: checkboxes = that item's attributes (two states), date and time = that item's modification time.
    let alpha_time = chrono::DateTime::<chrono::Local>::from(fs::metadata(left.join("alpha.txt")).unwrap().modified().unwrap());
    let archive = attributes("alpha.txt") & 0x20 != 0;
    assert!(matches!(&dialog.kind, DialogKind::Attributes(form) if form.path.ends_with("alpha.txt") && form.read_only == Some(false) && form.archive == Some(archive) && form.hidden == Some(false) && form.apply_to_folders && form.date == alpha_time.date_naive()));
    frame(&mut app, &ctx, vec![]);
    // OK on the first item: read-only on, time 2020-01-02 03:04:05.
    app.edit_attribute_form(|form| {
        form.change_time = true;
        form.read_only = Some(true);
        form.date = chrono::NaiveDate::from_ymd_opt(2020, 1, 2).unwrap();
        form.time = chrono::NaiveTime::from_hms_opt(3, 4, 5).unwrap();
    });
    app.submit_attributes(super::original_dialogs::AttributeAction::Apply, &ctx);
    wait_dialog(&mut app, &ctx);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Attributes(form) if form.path.ends_with("beta.txt") && form.read_only == Some(true)), "next item shows its own attributes");
    assert_eq!(attributes("alpha.txt") & 1, 1);
    let expected = chrono::NaiveDate::from_ymd_opt(2020, 1, 2).unwrap().and_hms_opt(3, 4, 5).unwrap().and_local_timezone(chrono::Local).earliest().unwrap();
    assert_eq!(fs::metadata(left.join("alpha.txt")).unwrap().modified().unwrap(), std::time::SystemTime::from(expected));
    // Esc skips the second item; All applies to the rest.
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Attributes(form) if form.path.ends_with("gamma.log")));
    app.edit_attribute_form(|form| form.hidden = Some(true));
    // Original TntFormKeyDown (0x41a2f4): End = btnAll.Click (all remaining items).
    frame(&mut app, &ctx, vec![key(egui::Key::End)]);
    wait_dialog(&mut app, &ctx);
    wait_listing(&mut app, &ctx);
    assert!(app.original_dialogs.is_empty());
    assert!(app.panes[0].selected.is_empty(), "applied and skipped items are unmarked (Select(i, false))");
    assert_eq!(attributes("beta.txt") & 3, 1, "skipped: still read-only");
    let _ = native::set_attributes(&left.join("beta.txt"), false, false);
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
    // Original miSplitFileClick: does nothing when the cursor item is not a file (no notice).
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
    // Original miJoinFileClick: marked items (otherwise the cursor item) fill lbFiles. The name is the original name from the .crc file.
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
    // With no marks and no cursor item (a folder), nothing happens.
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
fn list_export_window_follows_the_original_savelist() {
    let (root, ctx, mut app) = fixture("listexport", "en");
    let left = root.join("left");
    compat::set_description(&left, "beta.txt", "comma, \"quoted\"", true).unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.save_dialog = pick_save_target;
    SAVE_TARGET.with(|t| *t.borrow_mut() = Some(root.join("list")));
    // With no marks, SelectAll (including folders): all items.
    app.dispatch("saveList", &ctx);
    assert_eq!(app.original_dialogs[0].title, app.language_pack.text("formSaveList.Form.Caption", "?"));
    frame(&mut app, &ctx, vec![]);
    let count = app.panes[0].entries.iter().filter(|e| !e.drive && !e.is_parent()).count();
    app.edit_dialog_kind(|kind| if let DialogKind::ListExport(form) = kind {
        assert_eq!(form.rows.len(), count);
        form.columns = ndr::list_export::Columns { size: false, date: false, time: false, description: true, ..Default::default() };
        form.delimiter = 1;
        form.rebuild();
    });
    app.submit_list_export(&ctx);
    assert!(app.original_dialogs.is_empty());
    let saved = fs::read(root.join("list.txt")).unwrap();
    assert_eq!(&saved[..2], &[0xFF, 0xFE], "TEncoding.Unicode; .txt added");
    let units: Vec<u16> = saved[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16(&units).unwrap();
    assert!(text.contains("beta.txt,comma, \"quoted\"\r\n"), "no quoting: {text}");
    assert!(text.contains("folder1,\r\n"));
    // Only marked items; confirming takes the marks. Closing the save dialog still closes the window as confirmed.
    app.panes[0].selected = [row(&app, 0, "beta.txt")].into_iter().collect();
    app.dispatch("saveList", &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::ListExport(form) = kind {
        assert_eq!(form.rows.len(), 1);
        form.preview = "edited\n".into();
    });
    SAVE_TARGET.with(|t| *t.borrow_mut() = None);
    app.submit_list_export(&ctx);
    assert!(app.original_dialogs.is_empty() && app.panes[0].selected.is_empty(), "OK takes the marks");
    // The edited preview is used as is. An empty preview does not open the save dialog.
    app.dispatch("saveList", &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::ListExport(form) = kind { form.preview = "edited".into() });
    SAVE_TARGET.with(|t| *t.borrow_mut() = Some(root.join("edited.csv")));
    app.submit_list_export(&ctx);
    assert_eq!(fs::read(root.join("edited.csv")).unwrap(), b"\xFF\xFEe\0d\0i\0t\0e\0d\0\r\0\n\0");
    app.dispatch("saveList", &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::ListExport(form) = kind { form.preview.clear() });
    SAVE_TARGET.with(|t| *t.borrow_mut() = Some(root.join("empty.txt")));
    app.submit_list_export(&ctx);
    assert!(app.original_dialogs.is_empty() && !root.join("empty.txt").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn f12_shortcut_window_closes_on_escape_without_search_focus() {
    // Original TntFormShortCut (0x455a54): while the form is active, Esc (0x1b) closes it regardless of focus.
    let (root, ctx, mut app) = fixture("shortcuts-esc", "en");
    app.dispatch("shortcuts", &ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0));
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let _ = ctx.run(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ctx| app.shortcut_window_ui(ctx));
    };
    run(&mut app, vec![]);
    run(&mut app, vec![]);
    let area = ctx.memory(|m| m.area_rect(egui::Id::new("ndr-shortcut-window"))).expect("window area");
    // Click the lower part of the list, not the search field, to remove the search field focus.
    let point = egui::pos2(area.center().x, area.bottom() - 30.0);
    let click = |pressed| egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    run(&mut app, vec![egui::Event::PointerMoved(point), click(true)]);
    run(&mut app, vec![click(false)]);
    assert!(app.shortcut_window.is_some());
    assert!(ctx.memory(|m| m.focused()).is_none(), "the search box lost focus");
    run(&mut app, vec![key(egui::Key::Escape)]);
    assert!(app.shortcut_window.is_none(), "Esc closes the active F12 window");
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

/// Original FUN_00438034: TfGetText(TITLE_ADVSELECT / TITLE_ADVDESELECT, MSG_ADVSELECT, empty text). PathMatchSpecW per ';'-separated filter,
/// selects or deselects files only (not folders even if the name matches). The previous selection stays; cancel or empty text does nothing.
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
    // Advanced deselect: only files matching the filter are unmarked.
    app.dispatch("advancedDeselect", &ctx);
    assert_eq!(prompt_of(&app).0, "고급 해제", "original TITLE_ADVDESELECT");
    answer_text(&mut app, &ctx, Some("b*"));
    assert_eq!(marked(&app), ["alpha.txt", "note.log"]);
    app.dispatch("restoreSelection", &ctx);
    assert_eq!(marked(&app), ["alpha.txt", "beta.txt", "big.txt", "note.log"], "the previous selection was kept");
    // Cancel and empty text leave everything as is.
    app.dispatch("advancedDeselect", &ctx);
    answer_text(&mut app, &ctx, Some(""));
    app.dispatch("advancedDeselect", &ctx);
    answer_text(&mut app, &ctx, None);
    assert_eq!(marked(&app).len(), 4);
    fs::remove_dir_all(root).unwrap();
}

/// Original TfAdvRename: TntFormKeyDown (0x418bd0) Alt+1-6 = pcAction pages, lvPreviewKeyDown (0x419a4c) Ctrl+C = copy the selected row's original name.
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
    // Click the first preview row (322-340 below the header) and press Ctrl+C.
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
    // Original FUN_00418b04 + FUN_00469b64: a new name with forbidden characters is red in the preview (invalid name).
    let valid = |app: &App| match &app.original_dialogs[0].kind {
        DialogKind::AdvancedRename(form) => (0..form.paths.len()).map(|i| super::rename_dialog::valid_new_name(form, i)).collect::<Vec<_>>(),
        _ => panic!("rename window"),
    };
    assert_eq!(valid(&app), [true, true]);
    edit(&mut app, &|form| form.rules[0].text = "a:".into());
    assert_eq!(valid(&app), [false, false], "':' is one of \\/:*?\"<>|");
    edit(&mut app, &|form| form.rules[0].text = "x".repeat(255));
    assert_eq!(valid(&app), [false, false], "0x105 characters or more");
    edit(&mut app, &|form| form.rules[0].text = "new_".into());
    // Original FUN_0041830c list row: ADVRENAME_ACTIONS name (English description) - name/extension.
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
    // C# SaveSelectedMacro: a value the original format cannot store (';') warns with macroFormatUnsupported(value) and the file stays.
    edit(&mut app, &|form| { let mut semicolon = form.rules[0].clone(); semicolon.text = "a;b".into(); form.rules.push(semicolon); });
    app.save_rename_macro();
    app.answer_dialog(DialogResult::Text(Some("세미콜론".into())), &ctx);
    let expected = app.language_pack.text("formAdvRename.Caption.macroFormatUnsupported", "?").replace("{0}", "a;b");
    assert_eq!((app.msgboxes[0].title.clone(), app.msgboxes[0].text.clone()), (app.language_pack.text("formAdvRename.Caption.macroSaveTitle", "?"), expected));
    assert_eq!(fs::read(root.join("profile").join(ndr::rename_macros::FILE_NAME)).unwrap(), saved, "advrename.save unchanged");
    app.answer_message_box(1, &ctx);
    edit(&mut app, &|form| { form.rules.pop(); });
    // Original FUN_0046d000: with an unusable new name, only REN_INVALIDNAMES (REN_TITLE, information icon) shows and nothing is renamed.
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
/// One frame that also draws the modal progress window.
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
/// Left: folder1, alpha.txt, Alphabet.doc, beta.txt, gamma.log. Right: FOLDER1, beta.txt (same size, different content), omega.txt.
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
    // Original FUN_0041d280: column headers are the two folders (without the trailing "\"). Initially subfolders off, content comparison on, save off, list empty.
    let form = folder_compare(&app);
    assert_eq!((form.left_caption.as_str(), form.right_caption.as_str()), (root.join("left").to_str().unwrap(), root.join("right").to_str().unwrap()));
    assert!(!form.recursive && form.contents && !form.compared && form.rows.is_empty() && form.result.is_none());
    frame(&mut app, &ctx, vec![]);
    // Start (btnOkClick): compares inside the window and the window stays.
    app.folder_compare_action(FolderCompareAction::Start, &ctx);
    assert!(folder_compare(&app).running.is_some(), "btnStop replaces btnOk while comparing");
    frame(&mut app, &ctx, vec![]);
    wait_compare(&mut app, &ctx);
    assert_eq!(app.original_dialogs.len(), 1, "the same window shows the result");
    assert_eq!(app.status, "비교 완료 · 변경 5개");
    let form = folder_compare(&app);
    assert!(form.compared, "btnSave is enabled after a comparison");
    // Original FUN_0041e0b4: FOLDERCOMPARE_RESULT (same 1, different 1, left only 3, right only 1).
    let template = app.language_pack.text("RES.FOLDERCOMPARE_RESULT", "?");
    let expected = ["1", "1", "3", "1"].iter().fold(template, |text, n| text.replacen("%d", n, 1));
    assert_eq!(form.result.as_deref(), Some(expected.as_str()));
    // Original 0x41d464 rows: left name, result ("=", "< >", "<  ", "  >"), right name (empty for the missing side). Same-size files compare content, folders are equal.
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
    // Original lvListCompare: the result column sorts by result code (same 0 < different 1 < left only 2 < right only 3) then left name, always ascending. Column 0 does not sort.
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
    // Save (btnSaveClick): "compare result.html", HTML only. Display order, even rows line1, odd rows line2, UTF-8 BOM, CRLF per line.
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
    // Including subfolders: comparing again in the same window also lists the files inside folder1.
    fs::write(root.join("left").join("folder1").join("inner.txt"), b"x").unwrap();
    app.edit_dialog_kind(|kind| if let DialogKind::FolderCompare(form) = kind {
        form.recursive = true;
        form.contents = false;
    });
    app.folder_compare_action(FolderCompareAction::Start, &ctx);
    wait_compare(&mut app, &ctx);
    assert!(folder_compare(&app).rows.iter().any(|r| r.relative == "folder1\\inner.txt" && r.kind == ndr::folder_compare::DifferenceKind::LeftOnly));
    // Stop (btnStopClick) and close. Closing stops a running comparison too.
    app.folder_compare_action(FolderCompareAction::Start, &ctx);
    app.folder_compare_action(FolderCompareAction::Stop, &ctx);
    wait_compare(&mut app, &ctx);
    assert!(folder_compare(&app).result.is_some() && folder_compare(&app).compared);
    app.folder_compare_action(FolderCompareAction::Close, &ctx);
    assert!(app.original_dialogs.is_empty());
    // Esc closes too (btnCancel Cancel=True).
    app.dispatch("compare", &ctx);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.original_dialogs.is_empty(), "Esc closes the window");
    // A comparison that finishes with no window: cancel shows only the status line. A missing folder error uses the C# RunSafely box.
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
    // The same folder, or folders containing each other, do not start.
    app.navigate(1, root.join("left").join("folder1"), true);
    wait_listing(&mut app, &ctx);
    app.dispatch("compare", &ctx);
    assert!(app.original_dialogs.is_empty());
    assert_eq!(app.msgboxes[0].text, "비교하거나 동기화할 두 폴더는 서로 같거나 포함 관계일 수 없습니다.");
    app.answer_message_box(1, &ctx);
    // Korean texts of synchronization (cancelling the direction choice does nothing).
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
    assert!(app.job.as_ref().is_some_and(|j| j.title == "Synchronize folders" && j.modal.is_none() && j.progress.as_ref().is_some_and(|p| p.total_files == Some(3))), "progress window");
    frame_with_progress(&mut app, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(app.status, "Synchronization complete · 3 files copied · 0 folders created · 0 skipped · 0 errors");
    assert_eq!(fs::read(right.join("beta.txt")).unwrap(), b"beta.txt", "Overwrite replaced the different file");
    assert!(right.join("alpha.txt").is_file() && !right.join("gamma.log").exists(), "only the checked items");
    assert!(right.join("omega.txt").is_file(), "extra destination files are kept");
    assert!(app.panes[1].entries.iter().any(|e| e.name == "alpha.txt"), "both panes are refreshed");
    // Right -> left, size and modification date: copied files kept their times, so only omega.txt, present only on the right, remains.
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
    // The same flow answered with yes.
    app.dispatch("synchronize", &ctx);
    app.answer_dialog(DialogResult::Choice(Some(1)), &ctx);
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    wait_job(&mut app, &ctx);
    app.comparison_action(super::compare_dialog::ComparisonAction::Commit, &ctx);
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    app.answer_message_box(6, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(fs::read(left.join("omega.txt")).unwrap(), b"omega");
    // With no differences it only notifies.
    app.dispatch("synchronize", &ctx);
    app.answer_dialog(DialogResult::Choice(Some(1)), &ctx);
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    wait_job(&mut app, &ctx);
    assert!(app.original_dialogs.is_empty());
    assert_eq!(app.status, "There are no differences to synchronize.");
    // Stops when some items could not be read; errors of the copy result go to a text window.
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
fn favorites_window_follows_the_original_orgfavorites() {
    let (root, ctx, mut app) = fixture("favorites", "ko");
    let (left, right) = (root.join("left"), root.join("right"));
    let gone = root.join("gone").to_string_lossy().into_owned();
    app.settings.set_strings("Favorites", vec![right.to_string_lossy().into_owned(), gone.clone(), gone.clone()]);
    app.settings.set_strings("FavoriteNames", vec!["오른쪽".into(), String::new(), "없음".into()]);
    app.dispatch("favorites", &ctx);
    assert_eq!(app.original_dialogs[0].title, "즐겨찾기 구성", "[formOrgFavorites] Form.Caption");
    let form = favorites(&app);
    assert_eq!(form.rows.len(), 20, "FUN_0044bed8 makes 20 rows");
    assert_eq!(form.rows[0], ["오른쪽".to_owned(), right.to_string_lossy().into_owned()]);
    assert!(form.rows[1].is_empty(), "a slot without a name has no SubItems");
    assert_eq!(form.rows[2], ["없음".to_owned(), gone.clone()]);
    frame(&mut app, &ctx, vec![]);
    let act = |app: &mut App, action, ctx: &egui::Context| app.favorites_action(action, ctx);
    use super::favorites_dialog::FavoritesAction as A;
    // With no selected row, rename and change path do nothing.
    act(&mut app, A::Rename, &ctx);
    act(&mut app, A::ModifyPath, &ctx);
    assert_eq!(app.original_dialogs.len(), 1);
    // Change path on an empty row: name (PathFindFileNameW) + path.
    app.edit_dialog_kind(|kind| if let DialogKind::Favorites(form) = kind { form.selected = Some(1) });
    let folder = left.join("folder1");
    act(&mut app, A::ModifyPath, &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::FolderTree(_))), "FUN_0040ca90 folder tree");
    app.answer_dialog(DialogResult::FolderTree(Some(folder.clone())), &ctx);
    assert_eq!(favorites(&app).rows[1], ["folder1".to_owned(), folder.to_string_lossy().into_owned()]);
    // Rename: TfGetText(title RES.RENAME, initial value = name).
    act(&mut app, A::Rename, &ctx);
    assert_eq!(top_prompt(&app), ("이름 바꾸기".to_owned(), String::new(), "folder1".to_owned()));
    app.answer_dialog(DialogResult::Text(Some("작업".into())), &ctx);
    assert_eq!(favorites(&app).rows[1][0], "작업");
    // Change path keeps the name.
    act(&mut app, A::ModifyPath, &ctx);
    app.answer_dialog(DialogResult::FolderTree(Some(right.clone())), &ctx);
    assert_eq!(favorites(&app).rows[1], ["작업".to_owned(), right.to_string_lossy().into_owned()]);
    // Up/Down swap SubItems and move the selection too. Nothing changes at the top or bottom.
    act(&mut app, A::Up, &ctx);
    assert_eq!((favorites(&app).rows[0][0].as_str(), favorites(&app).rows[1][0].as_str(), favorites(&app).selected), ("작업", "오른쪽", Some(0)));
    act(&mut app, A::Up, &ctx);
    assert_eq!(favorites(&app).selected, Some(0));
    app.edit_dialog_kind(|kind| if let DialogKind::Favorites(form) = kind { form.selected = Some(19) });
    act(&mut app, A::Down, &ctx);
    assert_eq!(favorites(&app).selected, Some(19));
    // A row with only a name is not saved (it needs two SubItems).
    app.edit_dialog_kind(|kind| if let DialogKind::Favorites(form) = kind { form.selected = Some(5) });
    act(&mut app, A::Rename, &ctx);
    app.answer_dialog(DialogResult::Text(Some("이름만".into())), &ctx);
    assert_eq!(favorites(&app).rows[5], ["이름만".to_owned()]);
    // Delete only clears SubItems (the row stays).
    app.edit_dialog_kind(|kind| if let DialogKind::Favorites(form) = kind { form.selected = Some(2) });
    act(&mut app, A::Delete, &ctx);
    assert!(favorites(&app).rows[2].is_empty() && favorites(&app).rows.len() == 20);
    // Enter does not save (no Default button).
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(app.original_dialogs.len(), 1);
    act(&mut app, A::Save, &ctx);
    assert!(app.original_dialogs.is_empty());
    let saved = app.settings.strings("Favorites");
    assert_eq!(&saved[..3], [right.to_string_lossy().into_owned(), right.to_string_lossy().into_owned(), String::new()]);
    assert_eq!(&app.settings.strings("FavoriteNames")[..6], ["작업".to_owned(), "오른쪽".to_owned(), String::new(), String::new(), String::new(), String::new()]);
    // Cancel (Esc, btnCancel Cancel) changes nothing.
    app.dispatch("favorites", &ctx);
    app.edit_dialog_kind(|kind| if let DialogKind::Favorites(form) = kind { form.selected = Some(0) });
    // Access keys: Down (&W) is pressed by typing W alone (VCL CM_DIALOGCHAR).
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::W)]);
    assert_eq!(favorites(&app).selected, Some(1), "W = Down");
    act(&mut app, A::Delete, &ctx);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.original_dialogs.is_empty());
    assert_eq!(app.settings.strings("FavoriteNames")[0], "작업");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn favorites_menu_and_path_history_follow_the_context_menus() {
    let (root, ctx, mut app) = fixture("favmenu", "en");
    let (left, right) = (root.join("left"), root.join("right"));
    app.settings.set_strings("Favorites", vec![right.to_string_lossy().into_owned(), String::new(), left.join("folder1").to_string_lossy().into_owned()]);
    app.settings.set_strings("FavoriteNames", vec!["R&ight".into(), String::new(), "&Sub".into()]);
    // F11 = favorites drop-down (not the organize window).
    frame_keys(&mut app, &ctx, vec![key(egui::Key::F11)]);
    let popup = app.popup.as_ref().expect("F11 opens the favorites menu");
    let labels: Vec<(String, Option<char>, String)> = popup.items.iter().filter_map(|item| match item {
        super::favorites_dialog::PopupItem::Entry { label, access, gesture, .. } => Some((label.clone(), *access, gesture.clone())),
        _ => None,
    }).collect();
    // Original pmFavorites: favorite rows, separator, miAddToFavorites, miOrgFavorites.
    let add = app.language_pack.text("Menu.miAddToFavorites", "?");
    let organize = app.language_pack.text("Menu.miOrgFavorites", "?");
    assert_eq!(labels, [("Right".to_owned(), Some('i'), "Shift+F1".to_owned()), ("Sub".to_owned(), Some('S'), "Shift+F3".to_owned()), (add, None, String::new()), (organize, None, String::new())], "FUN_0042eedc: Caption = the name as stored");
    // Original miAddToFavoritesClick: TfGetText(TITLE_ADDFAV, MSG_ADDFAV (%s = folder), initial value = folder name) -> on OK adds at the end (first empty slot).
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
    // Access key s -> that slot's folder (FUN_0048d4b4).
    popup_frame(&mut app, &ctx, vec![egui::Event::Text("s".into())]);
    assert!(app.popup.is_none());
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, left.join("folder1"));
    // History: go back.
    app.dispatch("favoritesMenu", &ctx);
    popup_frame(&mut app, &ctx, vec![egui::Event::Text("i".into())]);
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, right);
    assert_eq!(app.panes[0].back, [left.clone(), left.join("folder1")]);
    app.show_history_menu(0, None);
    // Original FUN_0048ced4: the whole history list (oldest first), "&N. path", the current position checked.
    let entries: Vec<(String, Option<char>, bool)> = app.popup.as_ref().unwrap().items.iter().filter_map(|item| match item {
        super::favorites_dialog::PopupItem::Entry { label, access, checked, .. } => Some((label.clone(), *access, *checked)),
        _ => None,
    }).collect();
    let line = |n: char, path: &std::path::Path, checked: bool| (format!("{n}. {}", path.to_string_lossy()), Some(n), checked);
    assert_eq!(entries, [line('0', &left, false), line('1', &left.join("folder1"), false), line('2', &right, true)], "oldest first, the current one checked");
    for _ in 0..2 {
        popup_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    }
    popup_frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, left.join("folder1"), "the second entry");
    // Original FUN_0048d05c -> FUN_00458348: added as a new visit, not a history move (appended after the current position).
    assert_eq!(app.panes[0].back, [left.clone(), left.join("folder1"), right.clone()]);
    assert!(app.panes[0].forward.is_empty());
    // With no favorites (original) only the separator and the two fixed items, without a hint row. Esc closes it.
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
    // Original FUN_0048d10c: copy address, edit address, "-", favorites with name and path (no numbers), "-", add and organize favorites. Opens at the mouse position.
    app.show_address_bar_menu(1, egui::pos2(30.0, 40.0));
    let popup = app.popup.as_ref().expect("the address bar menu");
    assert_eq!(popup.position, Some(egui::pos2(30.0, 40.0)));
    let rows: Vec<String> = popup.items.iter().map(|item| match item {
        super::favorites_dialog::PopupItem::Entry { label, .. } => label.clone(),
        super::favorites_dialog::PopupItem::Separator => "-".into(),
    }).collect();
    let text = |key: &str| app.language_pack.text(key, "?");
    assert_eq!(rows, [text("Menu.mipAddressCopy"), text("Menu.mipAddressEdit"), "-".into(), "Right".into(), "-".into(), text("Menu.miAddToFavorites"), text("Menu.miOrgFavorites")]);
    assert_eq!(app.active, 1);
    // Copy address = that pane's path.
    app.run_popup_action(super::favorites_dialog::PopupAction::CopyAddress(1));
    assert_eq!(CLIPBOARD.with(|c| c.borrow().clone()), Some(app.panes[1].path.to_string_lossy().into_owned()));
    // Edit address = Down twice (once per frame) + Enter.
    popup_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    popup_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    popup_frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert!(app.popup.is_none());
    assert!(app.panes[1].address_editing, "Edit Address starts editing that pane's address");
    assert_eq!(app.panes[1].address, app.panes[1].path.to_string_lossy());
    fs::remove_dir_all(root).unwrap();
}
/// One frame that goes through pane key handling (App::keyboard).
fn frame_keys(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
}
fn popup_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.popup_menu_ui(ctx));
}


/// Original Recycle Bin button FUN_0042f13c: no key = Recycle Bin (regardless of settings), Shift = delete, Shift+Ctrl = shred, Ctrl+Alt = open the Recycle Bin, other combinations = nothing.
#[test]
fn recycle_button_follows_the_original_keys() {
    use egui::Modifiers as M;
    let target = super::commands::recycle_button_target;
    assert_eq!(target(M::NONE), Some("deleteRecycle"));
    assert_eq!(target(M::SHIFT), Some("deletePermanent"));
    assert_eq!(target(M::SHIFT | M::CTRL), Some("shred"));
    assert_eq!(target(M::CTRL | M::ALT), Some("openRecycle"));
    assert_eq!(target(M::CTRL), None);
    assert_eq!(target(M::ALT), None);
    assert_eq!(target(M::SHIFT | M::ALT), None);
    assert_eq!(target(M::SHIFT | M::CTRL | M::ALT), None);
}

thread_local! {
    static TREE_ROOT: RefCell<PathBuf> = const { RefCell::new(PathBuf::new()) };
}
/// Test tree root: only paths under the fake drive (test folder) use that root.
fn fake_tree_root(path: &str) -> String {
    let root = TREE_ROOT.with(|r| r.borrow().clone());
    let text = root.to_string_lossy().into_owned();
    if path.to_lowercase().starts_with(&text.to_lowercase()) { format!("{text}\\") } else { String::new() }
}

/// Original miFolderTreeClick 0x43c81a -> FUN_0040ca90 -> TfFolderTree: selects the initial path (scans drives when there is no tree file),
/// moves with the arrows (ArrowKey 0x7e7c3c: Down = next item on the same level), Enter (btnOk Default) = move the current pane to that path. Saves the tree file on close.
#[test]
fn folder_tree_window_follows_the_original_tffoldertree() {
    let (root, ctx, mut app) = fixture("foldertree", "ko");
    let drive = root.join("drive");
    for dir in ["x/a/a1", "x/a/a2", "x/b", "x/c", "y"] {
        fs::create_dir_all(drive.join(dir)).unwrap();
    }
    TREE_ROOT.with(|r| *r.borrow_mut() = drive.clone());
    app.system.tree_root = fake_tree_root;
    let start = drive.join("x").join("a");
    app.settings.set_strings("Favorites", vec![drive.join("y").to_string_lossy().into_owned()]);
    app.settings.set_strings("FavoriteNames", vec!["Y".into()]);
    app.navigate(0, start.clone(), false);
    wait_listing(&mut app, &ctx);
    app.active = 0;
    app.dispatch("folderTree", &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::FolderTree(_))));
    assert_eq!(app.original_dialogs.last().unwrap().title, app.language_pack.text("formFolderTree.Form.Caption", "?"));
    let end = Instant::now() + Duration::from_secs(20);
    while matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::FolderTree(form)) if form.scan.is_some()) {
        assert!(Instant::now() < end, "scan timeout");
        frame(&mut app, &ctx, vec![]);
        thread::sleep(Duration::from_millis(10));
    }
    let form = |app: &App| match app.original_dialogs.last().map(|d| &d.kind) {
        Some(DialogKind::FolderTree(form)) => (form.address.clone(), form.focused.map(|i| form.tree.items[i].name.clone())),
        _ => panic!("no folder tree"),
    };
    assert_eq!(form(&app), (start.to_string_lossy().into_owned(), Some("a".to_owned())), "SetPath(처음 경로)");
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    assert_eq!(form(&app).1.as_deref(), Some("b"), "↓ = next item on the same level");
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowLeft)]);
    assert_eq!(form(&app).1.as_deref(), Some("x"), "← = parent");
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowRight)]);
    assert_eq!(form(&app).1.as_deref(), Some("a"), "→ = next visible (first child, same row)");
    // miCreateNewFolderClick 0x425b68 (F7): TfGetText(TITLE_NEWFOLDER, MSG_NEWFOLDER, today's date) -> creates it, adds it to the tree and selects it.
    frame(&mut app, &ctx, vec![key(egui::Key::F7)]);
    let (title, message, value) = top_prompt(&app);
    assert_eq!((title, message, value), (root_text(&app, "TITLE_NEWFOLDER"), root_text(&app, "MSG_NEWFOLDER"), ndr::names::new_folder_default_name()));
    answer_text(&mut app, &ctx, Some("new"));
    assert!(start.join("new").is_dir());
    assert_eq!(form(&app), (start.join("new").to_string_lossy().into_owned(), Some("new".to_owned())));
    // miRenameClick 0x425e6c (F2): TfGetText(RENAME, MSG_RENAME(name, 1, 1), name) -> MoveFileW.
    frame(&mut app, &ctx, vec![key(egui::Key::F2)]);
    let (title, message, value) = top_prompt(&app);
    assert_eq!((title, value), (root_text(&app, "RENAME"), "new".to_owned()));
    assert_eq!(message, root_text(&app, "MSG_RENAME").replacen("%s", "new", 1).replacen("%d", "1", 2));
    answer_text(&mut app, &ctx, Some("renamed"));
    assert!(start.join("renamed").is_dir() && !start.join("new").exists());
    assert_eq!(form(&app), (start.join("renamed").to_string_lossy().into_owned(), Some("renamed".to_owned())));
    // FUN_004266ac (Del): deletes, removes it from the tree and selects the parent.
    frame(&mut app, &ctx, vec![key(egui::Key::Delete)]);
    assert!(!start.join("renamed").exists());
    assert_eq!(form(&app).1.as_deref(), Some("a"));
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowRight), key(egui::Key::ArrowLeft)]);
    assert_eq!(form(&app).1.as_deref(), Some("a"));
    let with = |key: egui::Key, modifiers: egui::Modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    // pmFavorites (FUN_004259e0): Shift+F1 = SetPath to favorite 0.
    frame(&mut app, &ctx, vec![with(egui::Key::F1, egui::Modifiers::SHIFT)]);
    assert_eq!(form(&app).1.as_deref(), Some("y"), "Shift+F1 = favorite 0");
    // tbSearchClick (Ctrl+F) -> TfSearchFolder: Enter = StrStrIW search (all items), Enter in the list = SetPath to that path.
    frame(&mut app, &ctx, vec![with(egui::Key::F, egui::Modifiers::CTRL)]);
    frame(&mut app, &ctx, vec![]);
    app.edit_dialog_kind(|kind| if let DialogKind::FolderTree(form) = kind { form.search.as_mut().unwrap().query = "A2".into() });
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    let results = match app.original_dialogs.last().map(|d| &d.kind) {
        Some(DialogKind::FolderTree(form)) => form.search.as_ref().map(|s| (s.results.clone(), s.selected)),
        _ => None,
    };
    assert_eq!(results, Some((vec![start.join("a2").to_string_lossy().into_owned()], Some(0))));
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(form(&app).1.as_deref(), Some("a2"), "lbResult Enter = SetPath");
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowLeft)]);
    assert_eq!(form(&app).1.as_deref(), Some("a"));
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown), key(egui::Key::Enter)]);
    assert!(app.original_dialogs.is_empty(), "Enter = btnOk(Default)");
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, drive.join("x").join("b"));
    let saved: Vec<_> = fs::read_dir(root.join("profile").join("treedata")).unwrap().flatten().collect();
    assert_eq!(saved.len(), 1, "SaveTree on close");
    fs::remove_dir_all(root).unwrap();
}


/// User request: inside an archive new folder, empty file, rename and delete; copy and move into the archive; move out of the archive; copy from a subfolder.
#[test]
fn archive_items_can_be_created_renamed_deleted_added_and_moved() {
    let (root, ctx, mut app) = fixture("archive-edit", "ko");
    let (left, right) = (root.join("left"), root.join("right"));
    let source = root.join("src");
    fs::create_dir_all(source.join("dir/sub")).unwrap();
    fs::write(source.join("a.txt"), b"a").unwrap();
    fs::write(source.join("dir/sub/c.txt"), b"c").unwrap();
    let zip = left.join("pack.zip");
    ndr::archive::create(&[source.join("a.txt"), source.join("dir")], &zip, None, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.active = 0;
    app.panes[0].cursor = row(&app, 0, "pack.zip");
    app.open_current();
    wait_listing(&mut app, &ctx);
    assert!(app.panes[0].archive.is_some());
    let names = |app: &App| app.panes[0].entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    // New folder.
    app.new_folder();
    answer_text(&mut app, &ctx, Some("made"));
    wait_job(&mut app, &ctx);
    assert!(names(&app).contains(&"made".to_owned()), "{:?} / {}", names(&app), app.status);
    // Empty file.
    app.create_empty_file();
    answer_text(&mut app, &ctx, Some("empty.txt"));
    wait_job(&mut app, &ctx);
    assert!(app.panes[0].entries.iter().any(|e| e.name == "empty.txt" && e.size == 0), "{:?}", names(&app));
    // Rename.
    app.panes[0].selected.clear();
    app.panes[0].cursor = row(&app, 0, "a.txt");
    app.rename_items();
    app.answer_dialog(DialogResult::Rename(Some(("b".into(), "txt".into()))), &ctx);
    wait_job(&mut app, &ctx);
    assert!(names(&app).contains(&"b.txt".to_owned()) && !names(&app).contains(&"a.txt".to_owned()), "{:?}", names(&app));
    // Delete (yes to the delete warning).
    app.panes[0].cursor = row(&app, 0, "made");
    app.delete_items(true);
    answer_warning(&mut app, &ctx, DeleteWarningChoice::Yes);
    wait_job(&mut app, &ctx);
    assert!(!names(&app).contains(&"made".to_owned()), "{:?}", names(&app));
    // Local -> archive copy (source stays) and move (source removed).
    fs::write(right.join("new.txt"), b"new").unwrap();
    fs::write(right.join("moved.txt"), b"moved").unwrap();
    app.refresh(1);
    wait_listing(&mut app, &ctx);
    app.active = 1;
    app.panes[1].cursor = row(&app, 1, "new.txt");
    app.copy_to_other(1, false, &ctx);
    wait_job(&mut app, &ctx);
    app.panes[1].cursor = row(&app, 1, "moved.txt");
    app.copy_to_other(1, true, &ctx);
    wait_job(&mut app, &ctx);
    assert!(names(&app).contains(&"new.txt".to_owned()) && names(&app).contains(&"moved.txt".to_owned()), "{:?}", names(&app));
    assert!(right.join("new.txt").is_file() && !right.join("moved.txt").exists());
    // Archive -> local move: extract and delete from the archive.
    app.active = 0;
    app.panes[0].cursor = row(&app, 0, "b.txt");
    app.copy_to_other(0, true, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(fs::read(right.join("b.txt")).unwrap(), b"a");
    assert!(!names(&app).contains(&"b.txt".to_owned()), "{:?}", names(&app));
    // Copying from a subfolder places items without that sub path.
    app.panes[0].cursor = row(&app, 0, "dir");
    app.open_current();
    wait_listing(&mut app, &ctx);
    assert!(app.panes[0].entries.iter().any(|e| e.name == "sub"), "{:?} {:?} {:?}", names(&app), app.panes[0].archive.as_ref().map(|a| a.prefix.clone()), ndr::archive::list(&zip, None).unwrap().into_iter().map(|e| (e.key, e.is_dir)).collect::<Vec<_>>());
    app.panes[0].cursor = row(&app, 0, "sub");
    app.copy_to_other(0, false, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(fs::read(right.join("sub").join("c.txt")).unwrap(), b"c");
    assert!(!right.join("dir").exists(), "the archive folder path is not recreated");
    // Files added into the archive go under the current archive folder.
    app.active = 1;
    app.refresh(1);
    wait_listing(&mut app, &ctx);
    app.panes[1].cursor = row(&app, 1, "new.txt");
    app.copy_to_other(1, false, &ctx);
    wait_job(&mut app, &ctx);
    let keys: Vec<String> = ndr::archive::list(&zip, None).unwrap().into_iter().map(|e| e.key.replace('\\', "/")).collect();
    assert!(keys.iter().any(|k| k == "dir/new.txt") && keys.iter().any(|k| k == "dir/sub/c.txt"), "{keys:?}");
    let _ = fs::remove_dir_all(root);
}

/// Adding into an archive asks with the original TfFileExists on a name conflict (destination = the archive item's size), and skipped items keep their source even on a move.
/// The right-drag menu of an archive pane enables only copy and move, and dropping on a folder row goes into that archive folder.
#[test]
fn archive_conflicts_ask_and_drops_into_archive_folders() {
    let (root, ctx, mut app) = fixture("archive-conflict", "ko");
    let (left, right) = (root.join("left"), root.join("right"));
    let source = root.join("src");
    fs::create_dir_all(source.join("dir")).unwrap();
    fs::write(source.join("a.txt"), b"a").unwrap();
    fs::write(source.join("keep.txt"), b"inside").unwrap();
    fs::write(source.join("dir/x.txt"), b"x").unwrap();
    let zip = left.join("pack.zip");
    ndr::archive::create(&[source.join("a.txt"), source.join("keep.txt"), source.join("dir")], &zip, None, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    for (name, body) in [("a.txt", "newer"), ("b.txt", "b"), ("c.txt", "c"), ("keep.txt", "outside")] {
        fs::write(right.join(name), body).unwrap();
    }
    app.refresh(0);
    app.refresh(1);
    wait_listing(&mut app, &ctx);
    app.active = 0;
    app.panes[0].cursor = row(&app, 0, "pack.zip");
    app.open_current();
    wait_listing(&mut app, &ctx);
    let read = |key: &str| {
        let out = root.join(format!("read-{}-{}", key.replace('/', "-"), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        ndr::archive::extract_entries_with_program(&zip, &out, &[key.into()], None, &std::sync::atomic::AtomicBool::new(false), &mut |_, _| ndr::fsops::ConflictChoice::Overwrite, &mut |_, _| None, None, &mut |_, _, _| {}).unwrap();
        fs::read_to_string(out.join(key.rsplit('/').next().unwrap())).unwrap()
    };
    let wait_conflict = |app: &mut App| -> (PathBuf, bool, Option<u64>, bool) {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            assert!(Instant::now() < end, "no conflict dialog (status: {})", app.status);
            app.poll(&ctx);
            if let Some(DialogKind::FileConflict(form)) = app.original_dialogs.first().map(|d| &d.kind) {
                return (form.destination.clone(), form.destination_dir, form.destination_size, form.destination_modified.is_some());
            }
            thread::sleep(Duration::from_millis(5));
        }
    };
    let answer = |app: &mut App, choice: ndr::fsops::ConflictChoice| app.answer_dialog(DialogResult::FileConflict(Some((choice, false))), &ctx);
    // Copy: a name conflict asks. The destination side is the archive item (1 byte).
    app.active = 1;
    app.panes[1].cursor = row(&app, 1, "a.txt");
    app.copy_to_other(1, false, &ctx);
    let (destination, destination_dir, destination_size, has_time) = wait_conflict(&mut app);
    assert_eq!(destination, zip.join("a.txt"));
    assert_eq!((destination_dir, destination_size), (false, Some(1)));
    assert!(has_time, "the archive entry time is shown");
    answer(&mut app, ndr::fsops::ConflictChoice::Skip);
    wait_job(&mut app, &ctx);
    assert_eq!(read("a.txt"), "a", "skipped");
    // Move + skip: the source stays.
    app.panes[1].cursor = row(&app, 1, "a.txt");
    app.copy_to_other(1, true, &ctx);
    wait_conflict(&mut app);
    answer(&mut app, ndr::fsops::ConflictChoice::Skip);
    wait_job(&mut app, &ctx);
    assert!(right.join("a.txt").is_file(), "a skipped move keeps the source");
    // Overwrite.
    app.panes[1].cursor = row(&app, 1, "a.txt");
    app.copy_to_other(1, false, &ctx);
    wait_conflict(&mut app);
    answer(&mut app, ndr::fsops::ConflictChoice::Overwrite);
    wait_job(&mut app, &ctx);
    assert_eq!(read("a.txt"), "newer");
    // Archive -> local move + skip: the archive item stays.
    app.active = 0;
    app.panes[0].cursor = row(&app, 0, "keep.txt");
    app.copy_to_other(0, true, &ctx);
    wait_conflict(&mut app);
    answer(&mut app, ndr::fsops::ConflictChoice::Skip);
    wait_job(&mut app, &ctx);
    assert_eq!(fs::read_to_string(right.join("keep.txt")).unwrap(), "outside");
    assert_eq!(read("keep.txt"), "inside", "a skipped move out keeps the archive entry");
    // Right-drag menu: copy and move enabled, shortcut gray.
    let zip_folder = app.panes[0].path.clone();
    app.show_drop_menu(0, vec![right.join("b.txt")], zip_folder.clone(), egui::pos2(10.0, 10.0));
    let states: Vec<bool> = app
        .popup
        .as_ref()
        .expect("drop menu")
        .items
        .iter()
        .filter_map(|item| match item {
            super::favorites_dialog::PopupItem::Entry { enabled, .. } => Some(*enabled),
            _ => None,
        })
        .collect();
    assert_eq!(states, vec![true, true, false, true]);
    app.popup = None;
    app.drop_from_menu(0, super::drag_drop::DropEffect::Move, vec![right.join("b.txt")], zip_folder);
    wait_job(&mut app, &ctx);
    assert_eq!(read("b.txt"), "b");
    assert!(!right.join("b.txt").exists(), "moved into the archive");
    // Dropping on an archive folder row = into that folder (yes to the drag-and-drop confirmation).
    app.handle_drop(0, vec![right.join("c.txt")], PathBuf::from("dir"), egui::Modifiers::NONE);
    assert!(!app.msgboxes.is_empty(), "drag and drop confirmation");
    app.answer_message_box(6, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(read("dir/c.txt"), "c");
    assert!(right.join("c.txt").is_file(), "a plain drop copies");
    let _ = fs::remove_dir_all(root);
}

/// FUN_00405e60: an archive without folder entries still lists the implied folder, dated by the entry that made it.
#[test]
fn implied_archive_folders_take_the_date_of_their_first_entry() {
    let (root, ctx, mut app) = fixture("archive-implied-folder", "en");
    let left = root.join("left");
    let source = root.join("src");
    fs::create_dir_all(source.join("dir/sub")).unwrap();
    fs::write(source.join("dir/sub/c.txt"), b"c").unwrap();
    let zip = left.join("pack.zip");
    // Relative file path only: 7-Zip stores "dir\sub\c.txt" with no folder entries.
    let status = std::process::Command::new(ndr::archive::seven_zip_path().unwrap())
        .current_dir(&source)
        .args(["a", "-tzip", "-bso0", "-bsp0"])
        .arg(&zip)
        .arg("dir\\sub\\c.txt")
        .status()
        .unwrap();
    assert!(status.success());
    let listed = ndr::archive::list(&zip, None).unwrap();
    assert!(listed.iter().all(|e| !e.is_dir), "no folder entries: {:?}", listed.iter().map(|e| e.key.clone()).collect::<Vec<_>>());
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.active = 0;
    app.panes[0].cursor = row(&app, 0, "pack.zip");
    app.open_current();
    wait_listing(&mut app, &ctx);
    let dir = app.panes[0].entries.iter().find(|e| e.name == "dir").cloned().expect("implied folder");
    assert!(dir.is_dir && dir.modified.is_some() && dir.modified == listed[0].modified, "{:?} vs {:?}", dir.modified, listed[0].modified);
    let _ = fs::remove_dir_all(root);
}