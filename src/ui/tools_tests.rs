//! C# ChecksumAsync, MergeTextFilesAsync, OpenWithAsync, RunCommandAsync, OriginalFindAsync, Map/UnmapNetworkDriveAsync,
//! ToggleExplorerIntegrationAsync, ImportLegacySettingsAsync, InstallSelectedArchive and SelectDestinationFolder/RememberDestination
//! in the Rust UI. Korean and English only. Only isolated folders; running programs, network drives, the registry and folder/file dialogs are fakes.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use std::{cell::RefCell, fs};

thread_local! {
    static COPIED: RefCell<String> = const { RefCell::new(String::new()) };
    static LAUNCHED: RefCell<Vec<(String, String, PathBuf)>> = const { RefCell::new(Vec::new()) };
    static CONNECTED: RefCell<Vec<(String, String, Option<String>, Option<String>)>> = const { RefCell::new(Vec::new()) };
    static CANCELLED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static EXPLORER: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    static VERBS: RefCell<Vec<(PathBuf, String)>> = const { RefCell::new(Vec::new()) };
    static SAVE_TARGET: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static OPEN_TARGET: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static PICKED: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static TEMP_FOLDER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static DRIVES: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    static LABELS: RefCell<Vec<(PathBuf, String)>> = const { RefCell::new(Vec::new()) };
}
fn fake_drives() -> Vec<PathBuf> {
    DRIVES.with(|d| d.borrow().clone())
}
fn fake_volume_label(root: &Path) -> Option<String> {
    LABELS.with(|l| l.borrow().iter().rev().find(|(r, _)| r == root).map(|(_, label)| label.clone()))
}
/// Names starting with "deny" give ERROR_ACCESS_DENIED (5), "fail" another error (1).
fn fake_set_volume_label(root: &Path, label: &str) -> Result<(), u32> {
    if label.starts_with("deny") {
        return Err(5);
    }
    if label.starts_with("fail") {
        return Err(1);
    }
    LABELS.with(|l| l.borrow_mut().push((root.to_path_buf(), label.to_owned())));
    Ok(())
}
fn fake_temp_folder() -> Option<PathBuf> {
    TEMP_FOLDER.with(|t| t.borrow().clone())
}
fn launch(path: &Path, parameters: &str, directory: &Path) -> Result<(), String> {
    LAUNCHED.with(|l| l.borrow_mut().push((path.to_string_lossy().into_owned(), parameters.to_owned(), directory.to_path_buf())));
    Ok(())
}
fn launched() -> Vec<(String, String, PathBuf)> {
    LAUNCHED.with(|l| std::mem::take(&mut *l.borrow_mut()))
}
fn connect(local: &str, remote: &str, user: Option<&str>, password: Option<&str>) -> Result<(), (u32, String)> {
    if user == Some("bad") {
        return Err((86, "The specified network password is not correct.".into()));
    }
    CONNECTED.with(|c| c.borrow_mut().push((local.into(), remote.into(), user.map(str::to_owned), password.map(str::to_owned))));
    Ok(())
}
fn disconnect(name: &str) -> Result<(), (u32, String)> {
    CANCELLED.with(|c| c.borrow_mut().push(name.into()));
    Ok(())
}
fn one_network_drive() -> Vec<String> {
    vec!["Z:".into()]
}
fn no_network_drives() -> Vec<String> {
    vec![]
}
fn explorer_off() -> bool {
    false
}
fn explorer_on() -> bool {
    true
}
fn set_explorer(enable: bool) -> Result<(), String> {
    EXPLORER.with(|e| e.borrow_mut().push(enable));
    Ok(())
}
fn run_verb(path: &Path, verb: &str) -> Result<(), String> {
    VERBS.with(|v| v.borrow_mut().push((path.to_path_buf(), verb.into())));
    Ok(())
}
/// Multi-item properties window (original FUN_00458da8): records "properties*" per path.
fn record_properties(paths: &[PathBuf]) -> Result<(), String> {
    VERBS.with(|v| v.borrow_mut().extend(paths.iter().map(|p| (p.clone(), "properties*".to_owned()))));
    Ok(())
}
fn save_target(_: &str, _: Option<&Path>, _: &str, _: &[(&str, &str)], _: &str, _: bool) -> Option<PathBuf> {
    SAVE_TARGET.with(|t| t.borrow().clone())
}
fn open_target(_: &str, _: Option<&Path>, _: &str, _: &[(&str, &str)]) -> Option<PathBuf> {
    OPEN_TARGET.with(|t| t.borrow().clone())
}
fn picked(_: &str, _: Option<&Path>) -> Option<PathBuf> {
    PICKED.with(|t| t.borrow().clone())
}

pub(super) fn fixture(label: &str, language: &str) -> (PathBuf, egui::Context, App) {
    let root = std::env::temp_dir().join(format!("ndr-tools-{}-{}-{label}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let (left, right) = (root.join("left"), root.join("right"));
    fs::create_dir_all(left.join("folder1")).unwrap();
    fs::create_dir_all(&right).unwrap();
    for name in ["alpha.txt", "beta.txt", "gamma.log"] {
        fs::write(left.join(name), name).unwrap();
    }
    let ctx = egui::Context::default();
    let options = Options { profile: Some(root.join("profile")), left: Some(left), right: Some(right), language: Some(language.into()), ..Default::default() };
    let mut app = App::from_context(&ctx, options);
    app.clipboard_writer = |_| Ok(());
    app.shell_runner = launch;
    app.save_dialog = save_target;
    app.open_dialog = open_target;
    app.folder_picker = picked;
    app.system = super::tools_dialog::SystemHooks { network_drives: one_network_drive, add_connection: connect, cancel_connection: disconnect, explorer_enabled: explorer_off, set_explorer, run_verb, properties: record_properties, network_path_error: |_| 0, connect_share: |_| 1223, shortcut_folder: || None, create_shortcut: |_, _| Ok(()), clipboard_files: Vec::new, temp_folder: fake_temp_folder, ready_drives: fake_drives, volume_label: fake_volume_label, set_volume_label: fake_set_volume_label, disk_info: |_| None, drive_item: fake_drive_item, file_icon: |_, _| Some((1, 1, vec![255, 255, 255, 255])), overlay_icon: |_| None, path_icon: |_| None, per_file_icon: |_| false, font_file: |_| None, large_icon: |_| None, compact_path: |p, _| p.to_owned(), wheel_lines: || 3, play_wave: super::tools_dialog::record_wave, tree_root: |_| String::new(), shell_delete: |_, _| 2, shortcut_target: |_| None };
    wait_until(&mut app, &ctx, |app| app.panes.iter().all(|p| !p.loading));
    (root, ctx, app)
}
/// On timeout reports the calling line (#[track_caller]) so it is clear which wait stalled.
#[track_caller]
fn wait_until(app: &mut App, ctx: &egui::Context, done: impl Fn(&App) -> bool) {
    let caller = std::panic::Location::caller();
    let end = Instant::now() + Duration::from_secs(20);
    while !done(app) {
        assert!(Instant::now() < end, "timeout waiting at {caller}");
        app.poll(ctx);
        thread::sleep(Duration::from_millis(5));
    }
}
fn wait_job(app: &mut App, ctx: &egui::Context) {
    wait_until(app, ctx, |app| app.job.as_ref().is_none_or(|j| j.finished));
    wait_until(app, ctx, |app| app.panes.iter().all(|p| !p.loading));
}
fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        app.original_dialog_ui(ctx);
        app.msgbox_ui(ctx);
    });
}
fn enter() -> egui::Event {
    egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}
fn row(app: &App, name: &str) -> usize {
    app.panes[0].entries.iter().position(|e| e.name == name).unwrap_or_else(|| panic!("{name} not listed"))
}
fn prompt(app: &App) -> (String, String, String, Option<Vec<String>>) {
    let dialog = app.original_dialogs.last().expect("a dialog is open");
    match &dialog.kind {
        DialogKind::Prompt { message, value, history, .. } => (dialog.title.clone(), message.clone(), value.clone(), history.clone()),
        _ => panic!("not a TfGetText dialog"),
    }
}
fn choice(app: &App) -> (String, Vec<String>) {
    let dialog = app.original_dialogs.last().expect("a dialog is open");
    match &dialog.kind {
        DialogKind::Choice(form) => (dialog.title.clone(), form.options.clone()),
        _ => panic!("no choice dialog"),
    }
}
fn boxed(app: &App) -> (String, String) {
    let first = app.msgboxes.first().expect("a message box");
    (first.title.clone(), first.text.clone())
}

#[test]
fn command_line_find_and_open_with_follow_the_original() {
    let (root, ctx, mut app) = fixture("command", "ko");
    let left = root.join("left");
    // Find: TfGetText(TITLE_FIND, MSG_FIND, empty text).
    app.dispatch("find", &ctx);
    let (title, message, value, _) = prompt(&app);
    assert_eq!((title, message, value), (app.language_pack.text("RES.TITLE_FIND", "?"), app.language_pack.text("RES.MSG_FIND", "?"), String::new()));
    app.answer_dialog(DialogResult::Text(Some("gam".into())), &ctx);
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "gamma.log");
    // Run command: an input box with history.
    app.settings.set_strings("CommandHistory", vec!["old".into()]);
    app.dispatch("command", &ctx);
    let (title, message, _, history) = prompt(&app);
    assert_eq!((title, message, history), (app.language_pack.text("RES.TITLE_CMD", "?"), app.language_pack.text("RES.MSG_CMD", "?"), Some(vec!["old".to_owned()])), "original miCommandLineClick");
    app.answer_dialog(DialogResult::Text(Some("someprog.exe a b".into())), &ctx);
    assert_eq!(launched(), [("someprog.exe".to_owned(), "a b".to_owned(), left.clone())], "ShellExecuteEx(name, arguments, current folder)");
    assert_eq!(app.settings.strings("CommandHistory"), ["someprog.exe a b", "old"]);
    // Original shortcut.ini miCommandLine=/: the same input box with the / key (VK_OEM_2) from the list.
    let slash = vec![
        egui::Event::Key { key: egui::Key::Slash, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
        egui::Event::Text("/".into()),
    ];
    let raw = egui::RawInput { events: slash, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
    let (title, _, _, _) = prompt(&app);
    assert_eq!(title, app.language_pack.text("RES.TITLE_CMD", "?"), "the / key opens the command line");
    app.answer_dialog(DialogResult::Text(None), &ctx);
    // Executable history: one candidate with the same name runs that path without arguments.
    let tool = left.join("folder1").join("tool.exe");
    fs::write(&tool, b"").unwrap();
    app.settings.set_strings("ExecutableHistory", vec![format!("tool={}", tool.display()), "ghost=C:\\nowhere\\ghost.exe".into()]);
    app.run_command("TOOL --ignored".into());
    assert_eq!(launched(), [(tool.to_string_lossy().into_owned(), String::new(), left.clone())]);
    app.run_command("ghost".into());
    assert_eq!(launched(), [("ghost".to_owned(), String::new(), left.clone())], "a missing cached path is no candidate");
    // Several candidates show a choice menu.
    let second = left.join("tool.exe");
    fs::write(&second, b"").unwrap();
    app.settings.set_strings("ExecutableHistory", vec![format!("tool={}", tool.display()), format!("tool={}", second.display())]);
    app.run_command("tool".into());
    assert_eq!(app.popup.as_ref().map(|p| p.items.len()), Some(2));
    assert_eq!(app.popup.as_ref().and_then(|p| p.highlight), Some(0), "original pmTempPopup: the first row is pre-selected");
    app.run_popup_action(super::favorites_dialog::PopupAction::RunExecutable { path: second.to_string_lossy().into_owned(), directory: left.clone() });
    app.popup = None;
    assert_eq!(launched()[0].0, second.to_string_lossy());
    // cd, http://, ftp://.
    app.run_command("cd folder1".into());
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].path, left.join("folder1"));
    app.run_command("cd missing".into());
    assert_eq!(app.panes[0].path, left.join("folder1"), "a missing folder changes nothing");
    app.run_command("http://example.invalid/x".into());
    assert_eq!(launched()[0].0, "http://example.invalid/x");
    app.run_command("ftp://host.invalid/dir".into());
    assert!(
        matches!(&app.original_dialogs.last().unwrap().kind, DialogKind::FtpQuickConnect(f) if f.host == "ftp://host.invalid/dir" && f.pane == app.active),
        "original miCommandLineClick: ftp:// opens TfFTPQuickConnect with the raw command as the host"
    );
    app.original_dialogs.clear();
    let history = app.settings.strings("CommandHistory");
    assert_eq!(history[0], "ftp://host.invalid/dir");
    assert!(history.len() <= 20);
    // Open with another program: program (initially TextViewerPath) -> argument template.
    app.navigate(0, left.clone(), true);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    // Original mipOpenWithClick (0x43da54, skin theme): the cursor file goes to rundll32 shell32.dll,OpenAs_RunDLL.
    app.panes[0].cursor = row(&app, "alpha.txt");
    app.dispatch("openWith", &ctx);
    assert_eq!(launched(), [("rundll32.exe".to_owned(), format!("shell32.dll,OpenAs_RunDLL {}", left.join("alpha.txt").display()), left.clone())]);
    fs::remove_dir_all(root).unwrap();
}

/// Called from the listing thread, so it does not use thread-local values: always "Test", fixed disk, 2 KB free / 8 KB total.
fn fake_drive_item(_: &Path) -> Option<(String, u32, Option<(u64, u64)>)> {
    Some(("Test".into(), 3, Some((2048, 8192))))
}

fn copied_checksum(text: &str) -> Result<(), String> {
    COPIED.with(|c| *c.borrow_mut() = text.to_owned());
    Ok(())
}

#[test]
fn checksum_merge_and_destination_follow_the_csharp_flows() {
    let (root, ctx, mut app) = fixture("checksum", "en");
    let (left, right) = (root.join("left"), root.join("right"));
    // Original TfChecksum (submenu MD5): one focused file, title "MD5", CALCULATING at first, then a lower-case hash and an enabled copy button.
    app.panes[0].cursor = row(&app, "alpha.txt");
    app.dispatch("checksum:Md5", &ctx);
    assert_eq!(app.original_dialogs[0].title, "MD5");
    let calculating = app.language_pack.text("RES.CALCULATING", "?");
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Checksum { value, ready: false } if *value == calculating));
    frame(&mut app, &ctx, vec![]);
    wait_until(&mut app, &ctx, |app| matches!(&app.original_dialogs[0].kind, DialogKind::Checksum { ready: true, .. }));
    let md5 = ndr::tools::checksum(&left.join("alpha.txt"), "md5").unwrap().to_ascii_lowercase();
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Checksum { value, .. } if *value == md5));
    app.clipboard_writer = copied_checksum;
    frame(&mut app, &ctx, vec![]);
    // Enter = the focused copy button.
    frame(&mut app, &ctx, vec![enter()]);
    assert_eq!(COPIED.with(|c| c.borrow().clone()), md5);
    assert_eq!(app.original_dialogs.len(), 1, "copying keeps the window open");
    COPIED.with(|c| c.borrow_mut().clear());
    app.copy_checksum();
    assert_eq!(COPIED.with(|c| c.borrow().clone()), md5);
    app.clipboard_writer = |_| Ok(());
    app.answer_dialog(DialogResult::Checksum, &ctx);
    assert!(app.original_dialogs.is_empty() && app.text.is_none());
    // CRC is IntToHex(8 chars, upper case). With focus on a folder nothing happens.
    app.dispatch("checksum:Crc32", &ctx);
    wait_until(&mut app, &ctx, |app| matches!(&app.original_dialogs[0].kind, DialogKind::Checksum { ready: true, .. }));
    let crc32 = ndr::tools::checksum(&left.join("alpha.txt"), "crc32").unwrap();
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Checksum { value, .. } if *value == crc32 && value.len() == 8));
    app.answer_dialog(DialogResult::Checksum, &ctx);
    app.panes[0].cursor = row(&app, "folder1");
    app.dispatch("checksum:Sha1", &ctx);
    assert!(app.original_dialogs.is_empty());
    // SHA-256 of the C# version keeps the text window.
    app.panes[0].cursor = row(&app, "alpha.txt");
    app.dispatch("checksum:Sha256", &ctx);
    wait_until(&mut app, &ctx, |app| app.text.is_some());
    let sha256 = ndr::tools::checksum(&left.join("alpha.txt"), "sha256").unwrap();
    assert_eq!(app.text.as_ref().map(|t| (t.title.clone(), t.text.clone())), Some(("SHA-256 Checksum".to_owned(), format!("alpha.txt\r\nSHA-256 {sha256}"))));
    app.text = None;
    app.panes[0].selected = [row(&app, "alpha.txt"), row(&app, "folder1"), row(&app, "beta.txt")].into_iter().collect();
    app.dispatch("checksum", &ctx);
    wait_until(&mut app, &ctx, |app| app.text.is_some());
    let text = app.text.take().unwrap();
    assert_eq!(text.title, "Checksum");
    let crc = ndr::tools::checksum(&left.join("beta.txt"), "crc32").unwrap();
    assert!(text.text.starts_with("alpha.txt\r\nCRC32   ") && text.text.contains(&format!("\r\n\r\nbeta.txt\r\nCRC32   {crc}\r\nMD5     ")), "{}", text.text);
    assert!(!text.text.contains("folder1"), "folders are left out");
    app.panes[0].selected.clear();
    app.panes[0].cursor = row(&app, "folder1");
    app.dispatch("checksum", &ctx);
    assert_eq!(boxed(&app).1, "Select the files to checksum.");
    app.answer_message_box(1, &ctx);
    // Merge text files: two or more, save path, same path check, choose a separator.
    app.panes[0].selected = [row(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("merge", &ctx);
    assert_eq!(boxed(&app), ("NDR".to_owned(), "Select at least two text files to merge.".to_owned()));
    app.answer_message_box(1, &ctx);
    app.panes[0].selected = [row(&app, "alpha.txt"), row(&app, "beta.txt")].into_iter().collect();
    SAVE_TARGET.with(|t| *t.borrow_mut() = Some(left.join("ALPHA.TXT")));
    app.dispatch("merge", &ctx);
    assert_eq!(boxed(&app).1, "The result cannot be saved over one of the input files.");
    app.answer_message_box(1, &ctx);
    SAVE_TARGET.with(|t| *t.borrow_mut() = Some(root.join("merged.txt")));
    app.dispatch("merge", &ctx);
    assert_eq!(choice(&app), ("Line breaks between files".to_owned(), vec!["Join without separators".to_owned(), "Add two line breaks".to_owned()]));
    app.answer_dialog(DialogResult::Choice(Some(1)), &ctx);
    wait_until(&mut app, &ctx, |app| app.status.starts_with("Merged"));
    assert_eq!(app.status, "Merged 2 text files.");
    assert_eq!(fs::read(root.join("merged.txt")).unwrap(), b"alpha.txt\r\n\r\nbeta.txt\r\n\r\n");
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    // Copy to another folder: the destination window (work folders, most used destinations), RememberDestination.
    let mut work = vec![right.to_string_lossy().into_owned()];
    work.resize(5, String::new());
    app.settings.set_strings("WorkingFolders", work);
    app.panes[0].selected = [row(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("copyToFolder", &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, format!("{}{}", app.language_pack.text("RES.COPY", "?"), app.language_pack.text("RES.SELECTDEST", "?")), "original COPY + SELECTDEST");
    frame(&mut app, &ctx, vec![]);
    app.destination_action(super::destination_dialog::DestinationAction::Work(1), &ctx);
    assert_eq!(boxed(&app), ("Copy - Select a destination".to_owned(), "Work folder 2 is not set or is unavailable.".to_owned()));
    // Even with a box on top, the destination window keeps being drawn behind it (it used to disappear).
    let serial = app.original_dialogs.last().unwrap().serial;
    frame(&mut app, &ctx, vec![]);
    let dialog_layer = egui::Id::new(("ndr-original-dialog", serial));
    assert!(ctx.memory(|m| m.areas().visible_layer_ids().iter().any(|layer| layer.id == dialog_layer)), "the dialog is drawn under the box");
    assert_eq!(app.original_dialogs.len(), 1);
    app.answer_message_box(1, &ctx);
    frame(&mut app, &ctx, vec![egui::Event::Key { key: egui::Key::Num1, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
    assert!(app.original_dialogs.is_empty(), "key 1 picks work folder 1");
    wait_job(&mut app, &ctx);
    assert!(right.join("alpha.txt").is_file());
    assert_eq!(app.settings.text("LastDestination", ""), right.to_string_lossy());
    let used = &app.settings.values["MostUsedDestinations"][0];
    assert_eq!((used["Path"].as_str().unwrap(), used["UseCount"].as_u64()), (right.to_string_lossy().as_ref(), Some(1)));
    app.panes[0].selected = [row(&app, "beta.txt")].into_iter().collect();
    app.dispatch("moveToFolder", &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "Move - Select a destination");
    let form = match &app.original_dialogs.last().unwrap().kind {
        DialogKind::Destination(form) => form.clone(),
        _ => panic!("no destination window"),
    };
    assert_eq!(form.most_used, [(right.to_string_lossy().into_owned(), 1)]);
    app.destination_action(super::destination_dialog::DestinationAction::MostUsed(right.to_string_lossy().into_owned()), &ctx);
    wait_job(&mut app, &ctx);
    assert!(right.join("beta.txt").is_file() && !left.join("beta.txt").exists(), "moved");
    assert_eq!(app.settings.values["MostUsedDestinations"][0]["UseCount"].as_u64(), Some(2));
    // Browse and remove from history.
    app.panes[0].cursor = row(&app, "gamma.log");
    app.dispatch("copyToFolder", &ctx);
    app.destination_action(super::destination_dialog::DestinationAction::Remove(right.to_string_lossy().into_owned()), &ctx);
    assert_eq!(boxed(&app).1, app.language_pack.text("RES.SELDEST_DELMUFOLDER", "?"));
    app.answer_message_box(6, &ctx);
    assert!(app.settings.values["MostUsedDestinations"].as_array().unwrap().is_empty());
    app.destination_action(super::destination_dialog::DestinationAction::Browse, &ctx);
    app.answer_dialog(super::original_dialogs::DialogResult::FolderTree(Some(root.join("left").join("folder1"))), &ctx);
    wait_job(&mut app, &ctx);
    assert!(left.join("folder1").join("gamma.log").is_file());
    // Original btnOkClick (0x453c4c): with a drive row (kind 8) chosen and the path field at that root, OK = folder tree (btnTreeClick).
    DRIVES.with(|d| *d.borrow_mut() = vec![right.clone()]);
    app.panes[0].cursor = row(&app, "gamma.log");
    app.dispatch("copyToFolder", &ctx);
    let drive_row = match &app.original_dialogs.last().unwrap().kind {
        DialogKind::Destination(form) => super::destination_dialog::original_items(form).iter().position(|(item, _)| matches!(item, super::destination_dialog::DestinationItem::Drive(_))).expect("a drive row"),
        _ => panic!("no destination window"),
    };
    app.edit_dialog_kind(|kind| if let DialogKind::Destination(form) = kind { form.selected = drive_row; form.path = right.to_string_lossy().into_owned() });
    app.destination_action(super::destination_dialog::DestinationAction::Accept, &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::FolderTree(_))), "the drive row opens the folder tree");
    app.answer_dialog(super::original_dialogs::DialogResult::FolderTree(None), &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::Destination(_))), "cancelling the tree keeps the destination window");
    DRIVES.with(|d| d.borrow_mut().clear());
    fs::remove_dir_all(root).unwrap();
}

thread_local! {
    static NETWORK_PICKS: std::cell::RefCell<Vec<(String, Option<PathBuf>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// In place of the original miNetwork folder picker: records the title and initial folder and picks "net" in the initial folder.
fn fake_network_picker(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
    NETWORK_PICKS.with(|p| p.borrow_mut().push((title.to_owned(), initial.map(Path::to_path_buf))));
    initial.map(|p| if p.ends_with("net") { p.to_path_buf() } else { p.join("net") })
}

#[test]
fn network_drives_explorer_import_and_install_follow_the_csharp_flows() {
    let (root, ctx, mut app) = fixture("system", "ko");
    let used: Vec<String> = native::drives().iter().map(|d| d.to_string_lossy().to_lowercase()).collect();
    let letter = ('m'..='y').find(|c| !used.iter().any(|d| d.starts_with(&format!("{c}:")))).expect("a free drive letter");
    // Map network drive: letter -> share path -> yes/no/cancel -> (no) account input window.
    app.dispatch("mapDrive", &ctx);
    assert_eq!(prompt(&app).0, "네트워크 드라이브 연결");
    app.answer_dialog(DialogResult::Text(Some("12".into())), &ctx);
    assert_eq!(boxed(&app), ("NDR".to_owned(), "드라이브 문자는 Z: 형식으로 입력하세요.".to_owned()));
    app.answer_message_box(1, &ctx);
    app.dispatch("mapDrive", &ctx);
    app.answer_dialog(DialogResult::Text(Some(letter.to_string())), &ctx);
    assert_eq!(prompt(&app).1, "공유 경로 (예: \\\\server\\share)");
    app.answer_dialog(DialogResult::Text(Some("\\\\server\\share".into())), &ctx);
    assert_eq!(boxed(&app), ("네트워크 인증".to_owned(), "현재 Windows 계정으로 연결할까요?\n아니요를 선택하면 별도 계정 정보를 입력합니다.".to_owned()));
    app.answer_message_box(7, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "네트워크 드라이브 인증");
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![enter()]);
    assert!(!app.original_dialogs.is_empty(), "OK needs a user name");
    app.edit_dialog_kind(|kind| if let DialogKind::Credentials { user, password } = kind { *user = "  me ".into(); *password = "pw".into(); });
    frame(&mut app, &ctx, vec![enter()]);
    let mapped = format!("{letter}:");
    assert_eq!(CONNECTED.with(|c| c.borrow().clone()), [(mapped.clone(), "\\\\server\\share".to_owned(), Some("me".to_owned()), Some("pw".to_owned()))]);
    assert_eq!(app.status, format!("{mapped} → \\\\server\\share 연결됨"));
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.dispatch("mapDrive", &ctx);
    app.answer_dialog(DialogResult::Text(Some(letter.to_string())), &ctx);
    app.answer_dialog(DialogResult::Text(Some("\\\\server\\share".into())), &ctx);
    app.answer_message_box(7, &ctx);
    app.answer_dialog(DialogResult::Credentials(Some(("bad".into(), String::new()))), &ctx);
    assert_eq!(boxed(&app).1, "네트워크 드라이브 연결에 실패했습니다. Windows 오류 86: The specified network password is not correct.");
    app.answer_message_box(1, &ctx);
    // Disconnect.
    app.dispatch("unmapDrive", &ctx);
    assert_eq!(choice(&app), ("네트워크 드라이브 연결 해제".to_owned(), vec!["Z:".to_owned()]));
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    assert_eq!(CANCELLED.with(|c| c.borrow().clone()), ["Z:"]);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.system.network_drives = no_network_drives;
    app.dispatch("unmapDrive", &ctx);
    assert_eq!(boxed(&app).1, "연결된 네트워크 드라이브가 없습니다.");
    app.answer_message_box(1, &ctx);
    // Original miNetwork (SHBrowseForFolderW, root = network, title [RES] NETWORK): opens with the current folder selected, and a choice goes to that folder.
    app.active = 0;
    let start = app.panes[0].path.clone();
    fs::create_dir_all(start.join("net")).unwrap();
    app.network_picker = fake_network_picker;
    app.dispatch("browseNetwork", &ctx);
    assert_eq!(NETWORK_PICKS.with(|p| p.borrow().clone()), [(app.language_pack.text("RES.NETWORK", "?"), Some(start.clone()))]);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].path, start.join("net"));
    assert!(app.original_internal("NetworkDialog", &ctx), "internal command 0x26 opens the same browser");
    assert_eq!(NETWORK_PICKS.with(|p| p.borrow().len()), 2);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    fs::remove_dir(start.join("net")).unwrap();
    app.navigate(0, start.clone(), true);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    // Explorer menu: asks to add when absent, to remove when present.
    app.dispatch("shellIntegration", &ctx);
    assert_eq!(boxed(&app), ("탐색기 통합".to_owned(), "현재 계정의 Windows 탐색기 폴더 메뉴에 NDR을 추가할까요?".to_owned()));
    app.answer_message_box(6, &ctx);
    assert_eq!(app.status, "탐색기 메뉴에 NDR을 추가했습니다.");
    app.system.explorer_enabled = explorer_on;
    app.dispatch("shellIntegration", &ctx);
    assert_eq!(boxed(&app).1, "NDR의 탐색기 폴더 메뉴 등록을 제거할까요?");
    app.answer_message_box(7, &ctx);
    assert_eq!(EXPLORER.with(|e| e.borrow().clone()), [true], "No changes nothing");
    // Install archive (shell verb install).
    app.panes[0].cursor = row(&app, "alpha.txt");
    app.dispatch("archiveInstall", &ctx);
    assert_eq!(boxed(&app).1, "설치할 압축 파일을 선택하세요.");
    app.answer_message_box(1, &ctx);
    let package = root.join("left").join("setup.zip");
    fs::write(&package, b"").unwrap();
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.panes[0].cursor = row(&app, "setup.zip");
    app.dispatch("archiveInstall", &ctx);
    assert_eq!(VERBS.with(|v| v.borrow().clone()), [(package, "install".to_owned())]);
    // Import legacy INI: replace settings, the toolbar.xml beside it (an existing one kept), start locations.
    let legacy = root.join("legacy");
    fs::create_dir_all(&legacy).unwrap();
    let ini = legacy.join("nexusfile.ini");
    fs::write(&ini, format!("[Window]\ndualview=0\n[View1]\nlastpath1={}\n", root.join("right").display())).unwrap();
    fs::write(legacy.join("toolbar.xml"), include_bytes!("../../Resources/toolbar.xml")).unwrap();
    fs::create_dir_all(root.join("profile")).unwrap();
    fs::write(root.join("profile").join("toolbar.xml"), include_bytes!("../../Resources/toolbar.xml")).unwrap();
    OPEN_TARGET.with(|t| *t.borrow_mut() = Some(ini));
    app.dispatch("importLegacy", &ctx);
    wait_until(&mut app, &ctx, |app| app.panes.iter().all(|p| !p.loading));
    assert_eq!(app.status, "레거시 설정을 가져왔습니다: nexusfile.ini");
    assert!(!app.dual && !app.settings.flag("DualPaneMode", true));
    assert_eq!(app.panes[0].path, root.join("right"));
    let backups = fs::read_dir(root.join("profile")).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with("toolbar.xml.backup-")).count();
    assert_eq!(backups, 1, "the previous toolbar.xml is kept");
    fs::remove_dir_all(root).unwrap();
}

fn archive_keys(path: &Path) -> Vec<String> {
    let mut keys: Vec<String> = ndr::archive::list(path, None).unwrap().into_iter().filter(|e| !e.is_dir).map(|e| e.key.replace('\\', "/")).collect();
    keys.sort();
    keys
}
fn archive_form(app: &App) -> &super::archive_dialog::ArchiveCreateForm {
    match &app.original_dialogs.last().expect("a dialog is open").kind {
        DialogKind::ArchiveCreate(form) => form,
        _ => panic!("no archive window"),
    }
}

/// Original rbFormatClick (PathRenameExtensionW). The archive always goes to the current folder (fixed: a typed path is not used, only the name).
#[test]
fn archive_name_field_follows_the_original_tfaddtoarc() {
    use super::archive_dialog::{archive_output_path, rename_extension};
    assert_eq!(rename_extension("left.zip", "7z"), "left.7z");
    assert_eq!(rename_extension(r"D:\a.b\left", "zip"), r"D:\a.b\left.zip");
    assert_eq!(rename_extension("set.tar.gz", "zip"), "set.zip");
    assert_eq!(rename_extension(r"C:\x\pack.zip", "tar.bz2"), r"C:\x\pack.tar.bz2");
    assert_eq!(rename_extension(".hidden", "zip"), ".hidden.zip");
    let here = Path::new(r"C:\work");
    assert_eq!(archive_output_path("pack.zip", here), Some(PathBuf::from(r"C:\work\pack.zip")));
    assert_eq!(archive_output_path(r"sub\pack.zip", here), Some(PathBuf::from(r"C:\work\pack.zip")));
    assert_eq!(archive_output_path(r"D:\out\pack.7z", here), Some(PathBuf::from(r"C:\work\pack.7z")));
    assert_eq!(archive_output_path("  ", here), None);
    assert_eq!(archive_output_path(r"D:\out\", here), None);
}

/// VCL TLabel FocusControl: "비밀번호(&P):" of the create archive window focuses the password field with Alt+P.
#[test]
fn archive_window_labels_focus_their_fields() {
    let (root, ctx, mut app) = fixture("archivelabel", "ko");
    app.panes[0].selected = [row(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("createArchive", &ctx);
    frame(&mut app, &ctx, vec![]);
    let alt_p = egui::Event::Key { key: egui::Key::P, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
    frame(&mut app, &ctx, vec![alt_p]);
    frame(&mut app, &ctx, vec![]);
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("archive-password"))), "Alt+P focuses the password field");
    let _ = fs::remove_dir_all(root);
}

/// External archivers are not used: RAR and ARJ (the bundled 7-Zip cannot create them) cannot be chosen.
#[test]
fn archive_window_has_no_external_program_formats() {
    let (root, ctx, mut app) = fixture("archiveprog", "ko");
    app.settings.values.insert("ExternalPrograms".into(), serde_json::json!({ "rar": ndr::archive::seven_zip_path().unwrap().to_string_lossy() }));
    app.panes[0].selected = [row(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("createArchive", &ctx);
    let formats: Vec<String> = archive_form(&app).formats.iter().map(|f| f.extension.clone()).collect();
    assert!(!formats.iter().any(|f| f == "rar" || f == "arj"), "{formats:?}");
    assert!(archive_form(&app).formats.iter().all(|f| f.program.is_none() && f.builtin));
    let _ = fs::remove_dir_all(root);
}

/// Puts a name in the original cbFileName.
fn name_archive(app: &mut App, name: &str) {
    let name = name.to_owned();
    app.edit_dialog_kind(move |kind| if let DialogKind::ArchiveCreate(form) = kind { form.file_name = name.clone(); });
}

#[test]
fn archive_create_add_move_and_extract_follow_the_csharp_flows() {
    let (root, ctx, mut app) = fixture("archive", "ko");
    let (left, right) = (root.join("left"), root.join("right"));
    app.settings.set_flag("UseRecycleBin", false);
    fs::write(left.join("folder1").join("inner.txt"), b"inner").unwrap();
    app.panes[0].selected = [row(&app, "alpha.txt"), row(&app, "folder1")].into_iter().collect();
    // Create archive window: include subfolders (initially on when there are folders), split size check.
    app.dispatch("createArchive", &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "압축하기");
    let form = archive_form(&app);
    assert!(form.recursive);
    assert_eq!(form.formats.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), ["zip", "7z", "tar", "tar.gz", "tar.bz2"], "plain names, no ALZ/WIM");
    assert_eq!(form.preset_labels, ["최고", "보통", "빠르게"]);
    assert_eq!(form.volume, "분할 안함");
    frame(&mut app, &ctx, vec![]);
    // Original FUN_0040d86c: unreadable volume text such as "열 MB" reads as 0 = one archive (archive_dialog parse_volume); there is no warning.
    let pack = left.join("pack.zip");
    // Original TfAddToArc cbFileName: default name (folder name + extension). Created with that name in the current folder without a save box.
    assert_eq!(archive_form(&app).file_name, "left.zip");
    let _ = &right;
    SAVE_TARGET.with(|t| *t.borrow_mut() = None);
    let named = pack.to_string_lossy().into_owned();
    app.edit_dialog_kind(|kind| if let DialogKind::ArchiveCreate(form) = kind { form.volume = "분할 안함".into(); form.set_format(0); form.file_name = "pack.zip".into(); });
    // Even if another pane becomes current while the window is open, the archive is created in the folder of the pane that opened the window.
    app.active = 1;
    app.submit_archive_create(0, &ctx);
    app.active = 0;
    wait_job(&mut app, &ctx);
    assert_eq!(app.status, "ZIP 압축 파일을 만들었습니다.");
    assert_eq!(archive_keys(&pack), ["alpha.txt", "folder1/inner.txt"]);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "pack.zip", "the cursor goes to the new archive");
    assert_eq!(app.settings.text("DefaultArchiveFormatExtension", ""), "zip", "RememberArchiveDialogChoices");
    // Creating again with the same name asks to replace the set (no = unchanged). The new list starts at "..", so put the cursor on folder1 as before.
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "folder1").unwrap();
    app.dispatch("createArchive", &ctx);
    name_archive(&mut app, &named);
    app.submit_archive_create(0, &ctx);
    assert_eq!(boxed(&app).0, "압축 파일 교체");
    assert!(boxed(&app).1.starts_with("기존 압축 파일 묶음 1개를 새 결과로 교체할까요?\r\n\r\npack.zip"));
    app.answer_message_box(7, &ctx);
    assert!(app.job.as_ref().is_none_or(|j| j.finished));
    // Add to an archive: existing items stay. A different format is reported.
    app.panes[0].selected = [row(&app, "beta.txt")].into_iter().collect();
    app.dispatch("archiveAdd", &ctx);
    assert_eq!(archive_form(&app).file_name, "beta.zip", "one item: its own name");
    name_archive(&mut app, &named);
    app.submit_archive_create(0, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(app.status, "1개 항목을 pack.zip에 추가했습니다.");
    assert_eq!(archive_keys(&pack), ["alpha.txt", "beta.txt", "folder1/inner.txt"]);
    app.panes[0].cursor = row(&app, "folder1");
    app.dispatch("archiveAdd", &ctx);
    name_archive(&mut app, &left.join("other.7z").to_string_lossy());
    app.submit_archive_create(0, &ctx);
    assert_eq!(boxed(&app), ("NDR".to_owned(), "대상 파일의 형식(7z)과 선택한 형식(.zip)이 다릅니다.".to_owned()));
    app.answer_message_box(1, &ctx);
    // Move into an archive: sources are deleted after confirmation (this test does not use the Recycle Bin).
    app.panes[0].selected = [row(&app, "gamma.log")].into_iter().collect();
    app.dispatch("archiveMove", &ctx);
    name_archive(&mut app, &named);
    app.submit_archive_create(0, &ctx);
    assert_eq!(boxed(&app), ("압축 후 이동".to_owned(), "압축 파일을 만든 뒤 원본을 휴지통으로 이동할까요?".to_owned()));
    app.answer_message_box(6, &ctx);
    wait_job(&mut app, &ctx);
    assert!(!left.join("gamma.log").exists());
    assert_eq!(archive_keys(&pack), ["alpha.txt", "beta.txt", "folder1/inner.txt", "gamma.log"]);
    // Extract into a new folder: pack, pack(2).
    app.panes[0].selected.clear();
    app.panes[0].cursor = row(&app, "pack.zip");
    app.dispatch("extractNewFolder", &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(app.status, format!("압축을 풀었습니다: {}", left.display()));
    // Progress window bar: reports the total item count after reading the listing (Event::Totals).
    assert!(app.job.as_ref().and_then(|j| j.progress.as_ref()).and_then(|p| p.total_files).is_some_and(|n| n >= 4), "extraction reports its total");
    assert!(left.join("pack").join("folder1").join("inner.txt").is_file());
    app.panes[0].cursor = row(&app, "pack.zip");
    app.dispatch("extractNewFolder", &ctx);
    wait_job(&mut app, &ctx);
    assert!(PathBuf::from(format!("{}\\pack(2)", left.display())).join("gamma.log").is_file());
    // Auto: as is when the top level is one folder, otherwise a new folder.
    fs::create_dir_all(left.join("solo")).unwrap();
    fs::write(left.join("solo").join("x.txt"), b"x").unwrap();
    let cancel = AtomicBool::new(false);
    ndr::archive::create(&[left.join("solo")], &left.join("solo.zip"), None, &cancel).unwrap();
    fs::remove_dir_all(left.join("solo")).unwrap();
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.panes[0].cursor = row(&app, "solo.zip");
    app.dispatch("extractAuto", &ctx);
    wait_job(&mut app, &ctx);
    assert!(left.join("solo").join("x.txt").is_file() && !left.join("solo").join("solo").exists(), "a single root folder goes straight in");
    app.panes[0].cursor = row(&app, "pack.zip");
    app.dispatch("extractAuto", &ctx);
    wait_job(&mut app, &ctx);
    assert!(PathBuf::from(format!("{}\\pack(3)", left.display())).join("alpha.txt").is_file());
    // Inside an archive: original miCopyToClick = FUN_00439bac (mode 0), the extraction destination window (initially the other pane); OK extracts those items.
    // Move is below (changed per user request).
    app.panes[0].archive = Some(ArchiveLocation { file: pack.clone(), prefix: String::new(), password: String::new() });
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.panes[0].cursor = row(&app, "folder1");
    app.dispatch("copyToOther", &ctx);
    let title = app.original_dialogs.last().map(|d| d.title.clone()).unwrap_or_default();
    assert!(title.starts_with(&app.language_pack.text("RES.EXTRACT", "?")), "{title}");
    app.destination_action(super::destination_dialog::DestinationAction::Accept, &ctx);
    if !app.msgboxes.is_empty() {
        app.answer_message_box(6, &ctx);
    }
    wait_job(&mut app, &ctx);
    assert!(right.join("folder1").join("inner.txt").is_file() && !right.join("alpha.txt").exists());
    app.panes[0].cursor = row(&app, "folder1");
    // In the original, moving inside an archive (miMoveToClick) did nothing. Per user request a modifiable zip opens the destination window (cancelled here).
    app.dispatch("moveToOther", &ctx);
    assert!(app.msgboxes.is_empty() && app.original_dialogs.len() == 1 && app.job.as_ref().is_none_or(|j| j.finished), "move inside a zip asks for a destination");
    app.destination_action(super::destination_dialog::DestinationAction::Cancel, &ctx);
    assert!(app.original_dialogs.is_empty() && app.job.as_ref().is_none_or(|j| j.finished), "cancelling the destination does nothing");
    app.dispatch("archiveProperties", &ctx);
    wait_until(&mut app, &ctx, |app| app.text.is_some());
    let text = app.text.take().unwrap();
    assert_eq!(text.title, "압축 파일 속성");
    assert!(text.text.starts_with("파일: pack.zip\r\n형식: ZIP\r\n") && text.text.contains("파일 항목: 4\r\n폴더 항목: 1\r\n"), "{}", text.text);
    app.dispatch("extractCurrent", &ctx);
    assert!(app.original_dialogs.is_empty() && app.job.as_ref().is_none_or(|j| j.finished), "inside an archive the current-folder mode does nothing");
    fs::remove_dir_all(root).unwrap();
}

// ---- Disk copy (C# DiskCopyWindow). All device functions are fakes and image files go only to isolated folders. ----
use super::disk_copy_dialog::{DiskAction, DiskHooks, DiskMode, DiskStage, physical_text};
use ndr::disk::{PhysicalDisk, RemovableDrive};

/// The fake device functions are called on the worker thread, so the records are global. Disk tests take turns with this lock.
static DISK_LOCK: Mutex<()> = Mutex::new(());
static DISK_CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static DISK_HOLD: AtomicBool = AtomicBool::new(false);
static DISK_FAIL_WRITE: AtomicBool = AtomicBool::new(false);
thread_local! {
    static DISK_TEMP: RefCell<PathBuf> = RefCell::new(PathBuf::new());
    static DISK_DIALOGS: RefCell<Vec<(String, String, Vec<(String, String)>, String, bool)>> = const { RefCell::new(Vec::new()) };
}
fn two_drives() -> Vec<RemovableDrive> {
    vec![
        RemovableDrive { root: "E:\\".into(), label: "USB-A".into(), bytes: 1_048_576 },
        RemovableDrive { root: "F:\\".into(), label: String::new(), bytes: 1_048_576 },
    ]
}
fn two_disks() -> Vec<PhysicalDisk> {
    vec![PhysicalDisk { number: 3, bytes: 4096 }, PhysicalDisk { number: 4, bytes: 4096 }]
}
fn no_disks() -> Vec<PhysicalDisk> {
    vec![]
}
fn disk_record(text: String) {
    DISK_CALLS.lock().unwrap_or_else(|e| e.into_inner()).push(text);
}
fn disk_calls() -> Vec<String> {
    std::mem::take(&mut *DISK_CALLS.lock().unwrap_or_else(|e| e.into_inner()))
}
fn fake_read(device: &str, output: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, bool)) -> Result<(), String> {
    disk_record(format!("read {device} {}", output.display()));
    progress(2048, 4096, false);
    if DISK_HOLD.load(Ordering::SeqCst) {
        let end = Instant::now() + Duration::from_secs(10);
        while !cancel.load(Ordering::SeqCst) {
            if Instant::now() > end {
                return Err("held too long".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
        return Err("Cancelled".into());
    }
    if output.to_string_lossy().contains("fail") {
        return Err("Access is denied.".into());
    }
    fs::write(output, b"image").map_err(|e| e.to_string())?;
    progress(4096, 4096, false);
    Ok(())
}
fn fake_write(image: &Path, device: &str, verify: bool, _: &AtomicBool, progress: &mut dyn FnMut(u64, u64, bool)) -> Result<(), String> {
    disk_record(format!("write {} {device} {verify}", image.display()));
    if DISK_FAIL_WRITE.load(Ordering::SeqCst) {
        return Err("Device write failed".into());
    }
    progress(4096, 4096, false);
    if verify {
        progress(4096, 4096, true);
    }
    Ok(())
}
fn fake_copy(source: &str, destination: &str, verify: bool, _: &AtomicBool, progress: &mut dyn FnMut(u64, u64, bool)) -> Result<(), String> {
    disk_record(format!("copy {source} {destination} {verify}"));
    progress(1_048_576, 1_048_576, false);
    Ok(())
}
fn disk_temp() -> PathBuf {
    DISK_TEMP.with(|t| t.borrow().clone())
}
fn disk_save(title: &str, initial: Option<&Path>, name: &str, filters: &[(&str, &str)], extension: &str, overwrite: bool) -> Option<PathBuf> {
    let initial = initial.map(|p| p.display().to_string()).unwrap_or_default();
    let filters = filters.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    DISK_DIALOGS.with(|d| d.borrow_mut().push((format!("{title}|{initial}"), name.into(), filters, extension.into(), overwrite)));
    SAVE_TARGET.with(|t| t.borrow().clone())
}
fn disk_open(title: &str, initial: Option<&Path>, name: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    let initial = initial.map(|p| p.display().to_string()).unwrap_or_default();
    let filters = filters.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    DISK_DIALOGS.with(|d| d.borrow_mut().push((format!("{title}|{initial}"), name.into(), filters, String::new(), false)));
    OPEN_TARGET.with(|t| t.borrow().clone())
}
fn disk_fixture(label: &str, language: &str, physical_disks: fn() -> Vec<PhysicalDisk>) -> (PathBuf, egui::Context, App) {
    let (root, ctx, mut app) = fixture(label, language);
    app.disk = DiskHooks { drives: two_drives, physical_disks, read_image: fake_read, write_image: fake_write, copy_device: fake_copy, temp_dir: disk_temp };
    app.save_dialog = disk_save;
    app.open_dialog = disk_open;
    DISK_TEMP.with(|t| *t.borrow_mut() = root.clone());
    DISK_DIALOGS.with(|d| d.borrow_mut().clear());
    DISK_HOLD.store(false, Ordering::SeqCst);
    DISK_FAIL_WRITE.store(false, Ordering::SeqCst);
    disk_calls();
    (root, ctx, app)
}
fn disk_open_window(app: &mut App, ctx: &egui::Context) {
    app.dispatch("diskCopy", ctx);
    assert!(app.disk_copy_form().is_some(), "the disk copy window opens");
}
fn disk_wait_idle(app: &mut App, ctx: &egui::Context) {
    wait_until(app, ctx, |app| {
        !app.msgboxes.is_empty() || app.original_dialogs.iter().all(|d| !matches!(&d.kind, DialogKind::DiskCopy(f) if f.run.as_ref().is_some_and(|r| r.stage != DiskStage::Swap)))
    });
}

#[test]
fn disk_copy_window_follows_the_csharp_flows() {
    let _serial = DISK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (root, ctx, mut app) = disk_fixture("disk-ko", "ko", two_disks);
    disk_open_window(&mut app, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "디스크 복사");
    frame(&mut app, &ctx, vec![]);
    {
        let form = app.disk_copy_form().unwrap();
        // Initially: direct copy, source = first drive, destination = second.
        assert_eq!((form.mode, form.drive, form.destination_drive, form.physical_disk, form.destination_physical), (DiskMode::Copy, Some(0), Some(1), Some(0), Some(1)));
        assert_eq!((form.drive_label(), form.start_label()), ("원본 드라이브", "드라이브 복사"));
        assert_eq!(form.status, "이동식 드라이브와 이미지 파일을 선택하세요.");
        assert_eq!(form.warning(), "대상 드라이브의 모든 기존 데이터를 덮어씁니다. 원본과 대상의 용량이 같아야 하며, 복사 중에는 취소할 수 없습니다.");
        assert_eq!(form.drive_text(&form.drives[0]), "E:\\ USB-A · 1.0 MB");
        assert_eq!(form.drive_text(&form.drives[1]), "F:\\ 이동식 디스크 · 1.0 MB", "an empty volume label shows 이동식 디스크");
        assert!(form.can_start());
        form.destination_drive = Some(0);
        assert!(!form.can_start(), "source and destination must differ");
        form.destination_drive = Some(1);
        form.verify = true;
    }
    // Direct copy: warning confirmation (no does nothing, yes copies -> the window closes).
    app.disk_copy_action(DiskAction::Start, &ctx);
    assert_eq!(boxed(&app), ("이동식 드라이브 직접 복사".to_owned(), "F:\\ (이동식 디스크)의 기존 볼륨 데이터가 E:\\ (USB-A) 내용으로 덮어씁니다.\n\n작업 중에는 드라이브를 분리하거나 NDR을 종료하지 마세요. 계속할까요?".to_owned()));
    assert_eq!(app.msgboxes[0].flags, ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONEXCLAMATION);
    app.answer_message_box(7, &ctx);
    assert!(app.disk_copy_form().unwrap().run.is_none() && disk_calls().is_empty(), "No starts nothing");
    app.disk_copy_action(DiskAction::Start, &ctx);
    app.answer_message_box(6, &ctx);
    assert_eq!(app.disk_copy_form().unwrap().status, "드라이브를 잠그고 있습니다…");
    wait_until(&mut app, &ctx, |app| app.original_dialogs.is_empty());
    assert_eq!(disk_calls(), ["copy E:\\ F:\\ true"], "verify follows the check box; success closes the window");
    // Create image: browse (save dialog: Documents folder, E-.img, .img type, overwrite prompt) -> starts without confirmation.
    disk_open_window(&mut app, &ctx);
    let image = root.join("E-.img");
    SAVE_TARGET.with(|t| *t.borrow_mut() = Some(image.clone()));
    {
        let form = app.disk_copy_form().unwrap();
        form.mode = DiskMode::Create;
        assert_eq!((form.drive_label(), form.path_label(), form.start_label()), ("이동식 드라이브", "저장할 이미지", "이미지 만들기"));
        assert_eq!(form.warning(), "이미지 파일은 원본 드라이브 바깥에 저장하세요.");
        assert!(!form.can_start(), "an image path is required");
    }
    frame(&mut app, &ctx, vec![]);
    app.disk_copy_action(DiskAction::Browse, &ctx);
    let (title, name, filters, extension, overwrite) = DISK_DIALOGS.with(|d| d.borrow_mut().remove(0));
    let documents = native::documents_folder().map(|p| p.display().to_string()).unwrap_or_default();
    assert_eq!((title, name.as_str(), extension.as_str(), overwrite), (format!("디스크 이미지 저장|{documents}"), "E-.img", "img", true));
    assert_eq!(filters, [("Raw disk image (*.img)".to_owned(), "*.img".to_owned()), ("모든 파일 (*.*)".to_owned(), "*.*".to_owned())]);
    assert_eq!(app.disk_copy_form().unwrap().image_path, image.to_string_lossy());
    app.disk_copy_action(DiskAction::Start, &ctx);
    assert!(app.msgboxes.is_empty(), "creating an image asks nothing");
    assert_eq!(app.disk_copy_form().unwrap().status, "드라이브에서 데이터를 읽고 있습니다…");
    wait_until(&mut app, &ctx, |app| app.original_dialogs.is_empty());
    assert_eq!(disk_calls(), [format!("read E:\\ {}", image.display())]);
    assert!(image.is_file());
    // Cancel: during creation the cancel button and closing the window (cancel request) work. The window stays and the status text changes.
    for action in [DiskAction::Cancel, DiskAction::Close] {
        disk_open_window(&mut app, &ctx);
        DISK_HOLD.store(true, Ordering::SeqCst);
        {
            let form = app.disk_copy_form().unwrap();
            form.mode = DiskMode::Create;
            form.image_path = image.to_string_lossy().into_owned();
        }
        app.disk_copy_action(DiskAction::Start, &ctx);
        wait_until(&mut app, &ctx, |app| app.original_dialogs.last().is_some_and(|d| matches!(&d.kind, DialogKind::DiskCopy(f) if f.status == "2,048 / 4,096 바이트 (50%)")));
        assert!(app.disk_copy_form().unwrap().run.as_ref().is_some_and(|r| r.cancellable()));
        frame(&mut app, &ctx, vec![]);
        app.disk_copy_action(action, &ctx);
        assert!(app.disk_copy_form().is_some(), "a running copy keeps the window open");
        disk_wait_idle(&mut app, &ctx);
        DISK_HOLD.store(false, Ordering::SeqCst);
        let form = app.disk_copy_form().unwrap();
        assert!(form.run.is_none());
        assert_eq!(form.status, "이미지 만들기를 취소했습니다. 불완전한 이미지 파일은 삭제했습니다.");
        disk_calls();
        app.disk_copy_action(DiskAction::Close, &ctx);
        assert!(app.original_dialogs.is_empty(), "닫기 closes an idle window");
    }
    // One-drive copy (media swap): temp image -> swap confirmation. No writes nothing and deletes the temp image.
    disk_open_window(&mut app, &ctx);
    {
        let form = app.disk_copy_form().unwrap();
        form.whole_disk = true;
        assert_eq!(form.start_label(), "전체 디스크 복제");
        assert!(form.warning().starts_with("대상 물리 디스크의 파티션 테이블과 모든 데이터를 덮어씁니다."));
        form.destination_physical = Some(0);
        assert!(!form.can_start(), "the target disk must differ");
        form.physical[1].bytes = 8192;
        form.destination_physical = Some(1);
        assert!(!form.can_start(), "disk capacities must match");
        form.physical[1].bytes = 4096;
        assert!(form.can_start());
        form.same_drive = true;
        form.verify = true;
        assert!(form.warning().starts_with("원본을 Windows 임시 폴더의 이미지 파일로 저장한 뒤 미디어를 바꿉니다."));
        assert!(form.can_start());
    }
    frame(&mut app, &ctx, vec![]);
    let disk3 = physical_text(&PhysicalDisk { number: 3, bytes: 4096 });
    assert_eq!(disk3, "PhysicalDrive3 · 4,096 bytes");
    for (attempt, swap) in [(0, 7), (1, 6)] {
        app.disk_copy_action(DiskAction::Start, &ctx);
        assert_eq!(boxed(&app), ("한 드라이브 디스크 복사".to_owned(), format!("{disk3} (미디어 교체 후)의 파티션 테이블과 모든 파티션을 포함한 물리 디스크 전체가 {disk3} 내용으로 덮어써집니다.\n\n작업 중에는 디스크를 분리하거나 NDR을 종료하지 마세요. 계속할까요?")));
        app.answer_message_box(6, &ctx);
        assert_eq!(app.disk_copy_form().unwrap().status, "원본 디스크를 임시 이미지로 저장하고 있습니다…");
        wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
        let calls = disk_calls();
        assert_eq!(calls.len(), 1);
        let temp = PathBuf::from(calls[0].strip_prefix("read \\\\.\\PhysicalDrive3 ").expect("reads the source disk"));
        let name = temp.file_name().unwrap().to_string_lossy().into_owned();
        assert!(temp.starts_with(&root) && name.starts_with("NDR-DiskCopy-") && name.ends_with(".img") && name.len() == "NDR-DiskCopy-.img".len() + 32, "{name}");
        assert!(temp.is_file());
        assert_eq!(boxed(&app), ("디스크 복사".to_owned(), "원본을 읽었습니다. 원본 미디어를 꺼내고 같은 용량의 대상 미디어를 넣은 뒤 계속을 누르세요.\n\n대상 미디어의 모든 데이터를 덮어씁니다.".to_owned()));
        let form = app.disk_copy_form().unwrap();
        assert!(form.run.as_ref().is_some_and(|r| r.stage == DiskStage::Swap && !r.cancellable()), "cancel is off after reading");
        if attempt == 0 {
            app.answer_message_box(swap, &ctx);
            let form = app.disk_copy_form().unwrap();
            assert_eq!(form.status, "복사를 취소했습니다. 대상 미디어에는 기록하지 않았습니다.");
            assert!(form.run.is_none() && !temp.exists() && disk_calls().is_empty());
            continue;
        }
        // Yes -> write to the same disk number (following the verify checkbox). On failure keep the recovery image and switch to restore mode.
        DISK_FAIL_WRITE.store(true, Ordering::SeqCst);
        app.answer_message_box(swap, &ctx);
        assert_eq!(app.disk_copy_form().unwrap().status, "대상 미디어에 이미지를 기록하고 있습니다…");
        wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
        DISK_FAIL_WRITE.store(false, Ordering::SeqCst);
        assert_eq!(disk_calls(), [format!("write {} \\\\.\\PhysicalDrive3 true", temp.display())]);
        assert_eq!(boxed(&app), ("디스크 복사".to_owned(), format!("복원 또는 드라이브 복사 중 오류가 발생했습니다. 대상 디스크의 데이터가 일부 변경되었을 수 있습니다.\n\nDevice write failed\n\n복구용 이미지 파일을 보존했습니다: {}", temp.display())));
        assert_eq!(app.msgboxes[0].flags, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION);
        app.answer_message_box(1, &ctx);
        let form = app.disk_copy_form().unwrap();
        assert!(temp.is_file(), "the recovery image is kept");
        assert_eq!((form.mode, form.image_path.as_str(), form.whole_disk), (DiskMode::Restore, &*temp.to_string_lossy(), true));
        assert_eq!((form.path_label(), form.start_label()), ("원본 전체 디스크 이미지", "전체 디스크 복원"));
        // Whole-disk restore from the remaining image: confirm -> write without verify -> the window closes.
        app.disk_copy_action(DiskAction::Start, &ctx);
        assert_eq!(boxed(&app), ("전체 물리 디스크 복원".to_owned(), format!("{disk3}의 파티션 테이블과 모든 파티션을 포함한 물리 디스크 전체가 {name} 내용으로 덮어써집니다.\n\n작업 중에는 디스크를 분리하거나 NDR을 종료하지 마세요. 계속할까요?")));
        app.answer_message_box(6, &ctx);
        assert_eq!(app.disk_copy_form().unwrap().status, "물리 디스크 볼륨을 잠그고 있습니다…");
        wait_until(&mut app, &ctx, |app| app.original_dialogs.is_empty());
        assert_eq!(disk_calls(), [format!("write {} \\\\.\\PhysicalDrive3 false", temp.display())]);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn disk_copy_window_in_english_reports_errors_like_csharp() {
    let _serial = DISK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (root, ctx, mut app) = disk_fixture("disk-en", "en", no_disks);
    disk_open_window(&mut app, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "Disk Copy");
    {
        let form = app.disk_copy_form().unwrap();
        assert_eq!((form.drive_label(), form.start_label(), form.status.as_str()), ("Source drive", "Copy drive", "Select a removable drive and an image file."));
        assert_eq!(form.drive_text(&form.drives[1]), "F:\\ Removable Disk · 1.0 MB");
        form.whole_disk = true;
        assert_eq!(form.warning(), "This overwrites the target disk's partition table and every byte. Disk capacities must match exactly. The operation cannot be cancelled. No removable physical disks were found. Disk access may require administrator privileges.");
        assert!(!form.can_start(), "no physical disk, no start");
        form.show_progress(524_288, 1_048_576, true);
        assert_eq!((form.status.as_str(), form.progress), ("Verifying · 524,288 / 1,048,576 bytes (50%)", 0.5));
    }
    frame(&mut app, &ctx, vec![]);
    app.disk_copy_action(DiskAction::Close, &ctx);
    assert!(app.original_dialogs.is_empty());
    // Whole-disk restore: browse (open dialog) -> confirm -> write fails -> "restore or drive copy" error, the window stays.
    app.disk.physical_disks = two_disks;
    disk_open_window(&mut app, &ctx);
    let image = root.join("backup.img");
    OPEN_TARGET.with(|t| *t.borrow_mut() = Some(image.clone()));
    {
        let form = app.disk_copy_form().unwrap();
        form.mode = DiskMode::Restore;
        form.whole_disk = true;
        assert_eq!((form.path_label(), form.start_label()), ("Source image", "Restore full disk"));
        assert_eq!(form.warning(), "The target disk's partition table and every byte will be replaced. Image and disk capacities must match exactly.");
    }
    app.disk_copy_action(DiskAction::Browse, &ctx);
    let (title, name, filters, _, _) = DISK_DIALOGS.with(|d| d.borrow_mut().remove(0));
    assert_eq!((title.as_str(), name.as_str()), ("Select Disk Image|", ""));
    assert_eq!(filters[1], ("All files (*.*)".to_owned(), "*.*".to_owned()));
    DISK_FAIL_WRITE.store(true, Ordering::SeqCst);
    app.disk_copy_action(DiskAction::Start, &ctx);
    assert_eq!(boxed(&app), ("Restore full physical disk".to_owned(), "The entire physical disk, including its partition table and every partition, PhysicalDrive3 · 4,096 bytes will be overwritten with backup.img.\n\nDo not disconnect the disk or close NDR during the operation. Continue?".to_owned()));
    app.answer_message_box(6, &ctx);
    assert!(app.disk_copy_blocks_close(), "the main window cannot close while restoring");
    assert!(app.disk_copy_form().unwrap().run.as_ref().is_some_and(|r| !r.cancellable()), "restoring cannot be cancelled");
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    DISK_FAIL_WRITE.store(false, Ordering::SeqCst);
    assert_eq!(disk_calls(), [format!("write {} \\\\.\\PhysicalDrive3 false", image.display())]);
    assert_eq!(boxed(&app), ("Disk Copy".to_owned(), "An error occurred while restoring or copying the drive. Data on the target disk may have been partly changed.\n\nDevice write failed".to_owned()));
    app.answer_message_box(1, &ctx);
    assert!(app.disk_copy_form().is_some_and(|f| f.run.is_none()) && !app.disk_copy_blocks_close());
    // Volume restore confirmation text, volume image creation failure ("The image could not be created").
    {
        let form = app.disk_copy_form().unwrap();
        form.whole_disk = false;
        form.drive = Some(1);
        assert_eq!(form.start_label(), "Restore drive");
    }
    app.disk_copy_action(DiskAction::Start, &ctx);
    assert_eq!(boxed(&app), ("Restore removable drive".to_owned(), "The existing volume data on F:\\ (Removable Disk) will be overwritten with backup.img.\n\nDo not disconnect the drive or close NDR during the operation. Continue?".to_owned()));
    app.answer_message_box(7, &ctx);
    {
        let form = app.disk_copy_form().unwrap();
        form.mode = DiskMode::Create;
        form.image_path = root.join("fail.img").to_string_lossy().into_owned();
    }
    app.disk_copy_action(DiskAction::Start, &ctx);
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    assert_eq!(boxed(&app), ("Disk Copy".to_owned(), "The image could not be created.\n\nAccess is denied.".to_owned()));
    app.answer_message_box(1, &ctx);
    assert_eq!(disk_calls(), [format!("read F:\\ {}", root.join("fail.img").display())]);
    app.disk_copy_action(DiskAction::Close, &ctx);
    assert!(app.original_dialogs.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn function_key_parameter_prompt_follows_csharp() {
    // C# ExecuteFunctionKeyMacroAsync: program macros -> pane macros -> for %p InputDialog(TITLE_EDITPARAM, MSG_EDITPARAM, arguments) ->
    // empty program ignored, an existing folder opens, otherwise ShellExecuteEx(program, argument text, active pane folder).
    let (root, ctx, mut app) = fixture("fkey-prompt", "en");
    let left = root.join("left");
    let alpha = row(&app, "alpha.txt");
    app.panes[0].cursor = alpha;
    let folder = left.join("folder1");
    app.settings.values.insert(
        "FunctionKeyBindings".into(),
        serde_json::json!({
            "F9": {"Name": "Run", "Command": "tool.exe", "Parameters": "-x %p !.!", "IsInternalCommand": false},
            "F11": {"Name": "Folder", "Command": folder.to_string_lossy(), "Parameters": "", "IsInternalCommand": false}
        }),
    );
    app.settings.values.insert("FunctionKeys".into(), serde_json::json!({"F7": "[Label]viewer.exe \"!.!\" $(PATH)"}));
    app.run_function_key_button("F9", &ctx);
    let (title, message, value, history) = prompt(&app);
    assert_eq!((title, message, value, history), (app.language_pack.text("TITLE_EDITPARAM", "?"), app.language_pack.text("MSG_EDITPARAM", "?"), "-x  alpha.txt".to_owned(), None));
    assert!(launched().is_empty(), "nothing runs before the answer");
    app.answer_dialog(DialogResult::Text(Some("edited \"a b\"".into())), &ctx);
    assert_eq!(launched(), [("tool.exe".to_owned(), "edited \"a b\"".to_owned(), left.clone())], "the edited text is the whole argument string");
    // Cancel does not run.
    app.run_function_key_button("F9", &ctx);
    app.answer_dialog(DialogResult::Text(None), &ctx);
    assert!(launched().is_empty(), "Cancel runs nothing");
    // Old FunctionKeys: [name] removed, arguments are the single line with pane macros expanded.
    app.run_function_key_button("F7", &ctx);
    assert!(app.original_dialogs.is_empty(), "no %p, no prompt");
    let path = format!("{}\\", left.display());
    assert_eq!(launched(), [("viewer.exe".to_owned(), format!("\"alpha.txt\" {path}"), left.clone())]);
    // An existing folder opens in the active pane.
    app.run_function_key_button("F11", &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].path, folder);
    assert!(launched().is_empty());
    fs::remove_dir_all(root).unwrap();
}


thread_local! {
    static SHORTCUT_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static LINKS: RefCell<Vec<(PathBuf, PathBuf)>> = const { RefCell::new(Vec::new()) };
    static CLIP: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}
fn shortcut_dir() -> Option<PathBuf> {
    SHORTCUT_DIR.with(|d| d.borrow().clone())
}
fn record_link(target: &Path, folder: &Path) -> Result<(), String> {
    LINKS.with(|l| l.borrow_mut().push((target.to_path_buf(), folder.to_path_buf())));
    Ok(())
}
fn reject_link(_: &Path, _: &Path) -> Result<(), String> {
    Err("test access denied".to_owned())
}
fn links() -> Vec<(PathBuf, PathBuf)> {
    LINKS.with(|l| std::mem::take(&mut *l.borrow_mut()))
}
fn clip() -> Vec<PathBuf> {
    CLIP.with(|c| c.borrow().clone())
}

#[test]
fn desktop_shortcut_reports_failure_and_creates_real_link() {
    let (root, _ctx, mut app) = fixture("desktop-real-link", "ko");
    let folder = root.join("desktop");
    fs::create_dir_all(&folder).unwrap();
    SHORTCUT_DIR.with(|d| *d.borrow_mut() = Some(folder.clone()));
    app.system.shortcut_folder = shortcut_dir;
    let entry = app.panes[0].entries.iter().position(|e| e.name == "alpha.txt").unwrap();
    app.panes[0].cursor = entry;
    app.panes[0].selected.insert(entry);

    app.system.create_shortcut = reject_link;
    app.shortcut_command();
    assert!(!app.msgboxes.is_empty(), "creation failure must be reported");
    assert!(app.msgboxes.last().unwrap().text.contains("test access denied"));
    assert!(app.panes[0].selected.contains(&entry), "failed creation must preserve selection");
    app.msgboxes.clear();

    app.system.create_shortcut = ndr::native::create_original_shortcut;
    app.shortcut_command();
    let link = folder.join("alpha.txt.lnk");
    assert!(link.is_file(), "a real Windows Shell Link must be written to the destination");
    assert!(ndr::native::shortcut_target(&link).is_some(), "the link must be readable by the shell");
    assert!(app.msgboxes.is_empty(), "desktop shortcut should succeed without a popup");
    assert!(app.panes[0].selected.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn system_commands_follow_csharp() {
    let (root, ctx, mut app) = fixture("system", "en");
    let (left, right) = (root.join("left"), root.join("right"));
    app.system.shortcut_folder = shortcut_dir;
    app.system.create_shortcut = record_link;
    app.system.clipboard_files = clip;
    let index = |app: &App, name: &str| app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    // Missing destination reports an error and retains the marked items;
    // a valid Desktop folder creates the links.
    app.panes[0].selected = [index(&app, "beta.txt"), index(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("shortcut", &ctx);
    assert!(links().is_empty());
    assert!(!app.msgboxes.is_empty(), "missing destination must be reported");
    app.msgboxes.clear();
    let desk = root.join("desk");
    fs::create_dir_all(&desk).unwrap();
    SHORTCUT_DIR.with(|d| *d.borrow_mut() = Some(desk.clone()));
    app.dispatch("shortcut", &ctx);
    assert_eq!(links(), vec![(left.join("alpha.txt"), desk.clone()), (left.join("beta.txt"), desk.clone())]);
    assert!(app.panes[0].selected.is_empty());
    assert!(app.msgboxes.is_empty(), "successful desktop shortcut needs no popup");
    app.msgboxes.clear();
    // PasteShortcutAsync: clipboard order, into the current folder.
    CLIP.with(|c| *c.borrow_mut() = vec![right.clone(), left.join("gamma.log")]);
    app.dispatch("pasteShortcut", &ctx);
    assert_eq!(links(), vec![(right.clone(), left.clone()), (left.join("gamma.log"), left.clone())]);
    // OpenAllSelectedFilesAsync: every marked item through the shell in the current folder; .exe goes to the history.
    fs::write(left.join("tool.exe"), b"MZ").unwrap();
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.panes[0].selected = [index(&app, "tool.exe"), index(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("openAll", &ctx);
    let opened: Vec<_> = launched().into_iter().map(|(p, a, d)| (PathBuf::from(p), a, d)).collect();
    assert_eq!(opened, vec![(left.join("alpha.txt"), String::new(), left.clone()), (left.join("tool.exe"), String::new(), left.clone())]);
    assert!(app.settings.strings("ExecutableHistory").iter().any(|row| row.eq_ignore_ascii_case(&format!("tool={}", left.join("tool.exe").display()))));
    app.panes[0].selected.clear();
    // GoToSystemFolderAsync: SpecialFolderPaths first, only existing folders.
    app.settings.values.insert("SpecialFolderPaths".into(), serde_json::json!({"Music": right.to_string_lossy(), "Videos": root.join("missing").to_string_lossy()}));
    app.dispatch("videos", &ctx);
    assert_eq!(app.panes[0].path, left);
    app.dispatch("music", &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].path, right);
    app.navigate(0, left.clone(), true);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    // ShowProperties: the focused item's shell properties.
    app.panes[0].cursor = index(&app, "alpha.txt");
    VERBS.with(|v| v.borrow_mut().clear());
    // Original miPropertiesClick (skin theme): the multi-file properties window of all marked items.
    app.panes[0].selected = [index(&app, "alpha.txt"), index(&app, "beta.txt")].into_iter().collect();
    app.dispatch("properties", &ctx);
    let mut recorded = VERBS.with(|v| v.borrow().clone());
    recorded.sort();
    assert_eq!(recorded, vec![(left.join("alpha.txt"), "properties*".to_owned()), (left.join("beta.txt"), "properties*".to_owned())]);
    app.panes[0].selected.clear();
    VERBS.with(|v| v.borrow_mut().clear());
    // ViewTextAsync: the configured viewer gets the quoted file path in the file's folder; with none, the NDR text window.
    app.settings.set_text("TextViewerPath", "viewer.exe");
    app.dispatch("textView", &ctx);
    assert_eq!(launched(), vec![("viewer.exe".to_owned(), format!("\"{}\"", left.join("alpha.txt").display()), left.clone())]);
    // Original FUN_00430304 (skin theme): notepad.exe when the setting is empty; nothing when the cursor is a folder.
    app.settings.set_text("TextEditorPath", "");
    app.dispatch("textEdit", &ctx);
    assert_eq!(launched(), vec![("notepad.exe".to_owned(), format!("\"{}\"", left.join("alpha.txt").display()), left.clone())]);
    let file_cursor = app.panes[0].cursor;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.is_dir).unwrap();
    app.dispatch("textView", &ctx);
    assert!(launched().is_empty() && app.msgboxes.is_empty());
    app.panes[0].cursor = file_cursor;
    // ShowAbout: information box with the profile folder.
    app.dispatch("about", &ctx);
    let (_, about) = boxed(&app);
    assert!(about.starts_with(&format!("NDR {}", env!("CARGO_PKG_VERSION"))) && about.contains(&app.profile.display().to_string()), "{about}");
    // User request: version 1.0.0, homepage = the NDR repository.
    assert_eq!(env!("CARGO_PKG_VERSION"), "1.0.0");
    assert_eq!(app.menu_label("homepage", "Homepage"), "W. GitHub Repository");
    let korean = ndr::compat::Language::load(&app.resources.join("Languages"), "ko");
    assert_eq!(korean.text("Menu.miHomepage", ""), "W. GitHub 저장소");
    app.dispatch("homepage", &ctx);
    assert_eq!(launched().first().map(|(program, _, _)| program.clone()), Some("https://github.com/UzuCore/NDR".to_owned()));
    // User request: Help also opens the NDR repository.
    LAUNCHED.with(|l| l.borrow_mut().clear());
    app.dispatch("help", &ctx);
    assert_eq!(launched().first().map(|(program, _, _)| program.clone()), Some("https://github.com/UzuCore/NDR".to_owned()));
    let _ = fs::remove_dir_all(&root);
}

/// C# OpenItemAsync (archive pane) + ExtractExternalWithPasswordPromptAsync: extracts the item temporarily to open it, and asks when the password is wrong.
#[test]
fn archive_entry_opening_asks_for_the_password_like_csharp() {
    let (root, ctx, mut app) = fixture("entry-open", "en");
    let left = root.join("left");
    let secret = left.join("secret.zip");
    ndr::archive::create(&[left.join("alpha.txt")], &secret, Some("s3cret"), &AtomicBool::new(false)).unwrap();
    app.panes[0].archive = Some(ArchiveLocation { file: secret.clone(), prefix: String::new(), password: "s3cret".into() });
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    // the remembered password is wrong now: the prompt appears, the right answer opens the temporary copy
    app.panes[0].archive.as_mut().unwrap().password = "wrong".into();
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "alpha.txt").unwrap();
    app.open_current();
    wait_until(&mut app, &ctx, |app| app.password_request.is_some());
    let request = app.password_request.take().unwrap();
    assert_eq!(request.archive, secret);
    // Original FUN_004399b4: TfGetText(TITLE_PASSWORD, MSG_PASSWORD, empty text), masked input.
    app.open_password_prompt(request);
    let (title, message, value, history) = prompt(&app);
    assert_eq!((title, message, value, history), (app.language_pack.text("RES.TITLE_PASSWORD", "?"), app.language_pack.text("RES.MSG_PASSWORD", "?"), String::new(), None));
    assert!(matches!(app.original_dialogs.last().unwrap().kind, super::original_dialogs::DialogKind::Prompt { password: true, .. }));
    app.answer_dialog(DialogResult::Text(Some("s3cret".into())), &ctx);
    wait_until(&mut app, &ctx, |app| app.panes[0].archive.as_ref().is_some_and(|l| l.password == "s3cret"));
    let opened = launched();
    assert_eq!(opened.len(), 1, "{opened:?}");
    let path = PathBuf::from(&opened[0].0);
    assert!(path.starts_with(std::env::temp_dir().join("NDR")) && path.ends_with("alpha.txt"), "{path:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "alpha.txt");
    // cancelling the prompt opens nothing and shows nothing
    app.panes[0].archive.as_mut().unwrap().password = "wrong".into();
    app.open_current();
    wait_until(&mut app, &ctx, |app| app.password_request.is_some());
    app.password_request.take().unwrap().reply.send(None).unwrap();
    let end = Instant::now() + Duration::from_millis(500);
    while Instant::now() < end {
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(launched().is_empty() && app.msgboxes.is_empty());
    let _ = fs::remove_dir_all(path.parent().unwrap());
    let _ = fs::remove_dir_all(&root);
}


/// The create archive formats have no ALZ or EGG (user request; extract only). Formats other than zip and 7z cannot have passwords.
#[test]
fn alz_and_egg_are_not_creation_formats() {
    let (root, _ctx, app) = fixture("alzegg-formats", "ko");
    let formats = app.archive_formats(true);
    assert!(!formats.iter().any(|f| f.extension == "egg" || f.extension == "alz"), "no ALZ/EGG creation");
    let tar = formats.iter().find(|f| f.extension == "tar").cloned().expect("tar");
    let choice = |password: &str| super::archive_dialog::ArchiveChoice { format: tar.clone(), format_index: 2, preset: 0, level: 0, password: password.into(), volume: None, recursive: true, output: None };
    assert!(matches!(app.archive_program(&choice("")), Ok(None)));
    assert_eq!(app.archive_program(&choice("secret")).unwrap_err(), ".tar 형식은 암호를 넣을 수 없습니다.");
    let _ = fs::remove_dir_all(root);
}

/// Original 0x483c79: with playwav (default on), a .wav is not opened in the shell but played with PlaySoundW(NULL) then PlaySoundW(file).
/// Original 0x43d9fc: the completion sound is complete.wav in the program folder.
#[test]
fn wave_files_play_inside_ndr_and_the_completion_sound_is_complete_wav() {
    let (root, ctx, mut app) = fixture("playwav", "en");
    let wave = root.join("left").join("Ding.WAV");
    fs::write(&wave, b"RIFF").unwrap();
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.path == wave).expect("the wav row");
    super::tools_dialog::PLAYED.with(|p| p.borrow_mut().clear());
    app.open_current();
    assert_eq!(super::tools_dialog::PLAYED.with(|p| p.borrow().clone()), vec![None, Some(wave.clone())]);
    assert!(launched().is_empty(), "not handed to the shell");
    assert!(app.panes[0].archive.is_none());
    // Completion sound.
    super::tools_dialog::PLAYED.with(|p| p.borrow_mut().clear());
    app.settings.set_flag("PlayCompletionSound", true);
    assert!(app.start_task("Copy".into(), |_, _| Ok(())));
    if let Some(job) = app.job.as_mut() {
        job.chime = true;
    }
    wait_until(&mut app, &ctx, |_| super::tools_dialog::PLAYED.with(|p| !p.borrow().is_empty()));
    assert_eq!(super::tools_dialog::PLAYED.with(|p| p.borrow().clone()), vec![Some(app.resources.join("complete.wav"))]);
    let _ = fs::remove_dir_all(root);
}

/// Original TNXScrollStep: 2 (default) = by page (one step = visible rows), 1 = by line (Mouse.WheelScrollLines rows).
#[test]
fn wheel_steps_follow_the_original_scroll_step() {
    let (root, _ctx, mut app) = fixture("scrollstep", "en");
    assert!(app.scroll_by_page(0) && app.scroll_by_page(1), "the original default is 2 (page)");
    app.settings.values.insert("LeftScrollStep".into(), serde_json::json!(1));
    assert!(!app.scroll_by_page(0) && app.scroll_by_page(1));
    assert_eq!(App::wheel_step(true, 110.0, 25.0, 3), 100.0, "a page is the whole rows that fit");
    assert_eq!(App::wheel_step(true, 10.0, 25.0, 3), 25.0, "at least one row");
    assert_eq!(App::wheel_step(false, 110.0, 25.0, 3), 75.0, "WheelScrollLines rows");
    assert_eq!(App::wheel_step(false, 600.0, 150.0, 1), 150.0, "thumbnails: one row");
    // Original SetScrollTop (by page): positions at page boundaries, 0..pages - 1.
    assert_eq!(App::page_snap(90.0, 100.0, 5), 100.0, "nearest page boundary");
    assert_eq!(App::page_snap(-50.0, 100.0, 5), 0.0);
    assert_eq!(App::page_snap(900.0, 100.0, 5), 400.0, "the last page starts on a boundary");
    let _ = fs::remove_dir_all(root);
}

/// C# OpenItemAsync: an item opened inside an archive that is an archive format the bundled 7-Zip reads (e.g. .iso) is entered instead of opened in the shell.
#[test]
fn nested_archives_of_any_known_format_open_as_panes_like_csharp() {
    let (root, ctx, mut app) = fixture("nested-iso", "en");
    let iso = root.join("left").join("disc.iso");
    fs::write(&iso, b"not really an image").unwrap();
    app.archive_entries_opened(0, root.join("outer.zip"), None, vec![iso.clone()], vec![], true);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert!(app.panes[0].archive.is_none(), "invalid nested ISO must return to its containing folder instead of trapping the pane");
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "disc.iso");
    assert!(app.status.contains("Could not open archive"));
    assert!(launched().is_empty(), "not handed to the shell");
    // ALZ and EGG are read by bundled 7zip+. Create *real* archives so the
    // test checks successful pane navigation rather than only file extensions.
    let payload = root.join("left/nested-payload.txt");
    fs::write(&payload, b"nested valid content").unwrap();
    for name in ["old.alz", "old.egg"] {
        let nested = root.join("left").join(name);
        let extension = name.rsplit_once('.').unwrap().1;
        let options = ndr::commands::ArchiveOptions {
            extension: extension.into(), level: 5, password: String::new(),
            volume_bytes: None, recursive: true, append: false,
        };
        ndr::archive::create_configured(&[payload.clone()], &nested, &options, &AtomicBool::new(false)).unwrap();
        app.archive_entries_opened(1, root.join("outer.zip"), None, vec![nested.clone()], vec![], true);
        wait_until(&mut app, &ctx, |app| !app.panes[1].loading);
        assert_eq!(app.panes[1].archive.as_ref().map(|l| l.file.clone()), Some(nested));
        assert!(app.panes[1].entries.iter().any(|e| e.name == "nested-payload.txt"), "valid {extension} contents must appear in the pane");
        app.panes[1].archive = None;
    }
    assert!(launched().is_empty(), "ALZ/EGG not handed to the shell");
    // ACE, which the bundled 7-Zip cannot read: the C# TryGetExternalArchiveProgram path. Without an external program it is not opened in the shell; the C# error text (NDR warning box).
    let ace = root.join("left").join("old.ace");
    fs::write(&ace, b"x").unwrap();
    app.archive_entries_opened(1, root.join("outer.zip"), None, vec![ace.clone()], vec![], true);
    assert!(app.panes[1].archive.is_none());
    assert!(launched().is_empty(), "ACE is not handed to the shell");
    assert!(app.msgboxes.last().is_some_and(|m| m.text.contains(".ace")), "{:?}", app.msgboxes.last().map(|m| m.text.clone()));
    let _ = fs::remove_dir_all(root);
}

/// Archives the bundled 7-Zip cannot read: external archivers are not used, so even with a leftover setting it is not opened and a notice shows.
#[test]
fn unreadable_archives_are_not_handed_to_an_external_program() {
    let (root, _ctx, mut app) = fixture("ace-open", "en");
    let ace = root.join("left").join("old.ace");
    fs::write(&ace, b"not an archive").unwrap();
    let seven = ndr::archive::seven_zip_path().unwrap();
    app.settings.values.insert("ExternalPrograms".into(), serde_json::json!({ "ace": seven.to_string_lossy() }));
    app.open_with_external_archive_program(0, ace, true);
    assert!(app.job.is_none(), "no extraction job");
    assert!(app.msgboxes.last().is_some_and(|m| m.text == "The .ace format cannot be opened."), "{:?}", app.msgboxes.last().map(|m| m.text.clone()));
    let _ = fs::remove_dir_all(root);
}

/// Original 0x439593: when the next volume is missing, TfMsgBox(TITLE_ERROR, ARC_CHANGEVOL %s, MB_OKCANCEL | MB_ICONEXCLAMATION). OK = retry, cancel = end.
#[test]
fn missing_next_volume_asks_arc_changevol_like_the_original() {
    let (root, ctx, mut app) = fixture("changevol", "en");
    let answers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = answers.clone();
    assert!(app.start_task("Extract".into(), move |events, cancel| {
        let first = super::commands::ask_change_volume(&events, &cancel, "C:\\a\\x.7z.002");
        let second = super::commands::ask_change_volume(&events, &cancel, "C:\\a\\x.7z.002");
        seen.lock().unwrap().extend([first, second]);
        Ok(())
    }));
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    let expected = app.language_pack.text("RES.ARC_CHANGEVOL", "?").replacen("%s", "C:\\a\\x.7z.002", 1);
    assert_eq!(app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone(), m.flags)), Some((app.language_pack.text("RES.TITLE_ERROR", "?"), expected, ndr::msgbox::MB_OKCANCEL | ndr::msgbox::MB_ICONEXCLAMATION)));
    app.answer_message_box(1, &ctx);
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    app.answer_message_box(2, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(*answers.lock().unwrap(), [true, false], "OK retries, Cancel stops");
    fs::remove_dir_all(root).unwrap();
}

/// C# ApplySingleInstanceModeAsync: "run only one instance" in preferences applies at once. When another NDR already holds it, revert to the previous value and notify.
#[test]
fn single_instance_mode_applies_without_restart_like_csharp() {
    let (root, ctx, mut app) = fixture("single", "en");
    let mut settings = app.settings.clone();
    settings.set_flag("UseSingleInstance", true);
    app.apply_settings(settings);
    assert!(app.instance_guard.is_some() && app.navigation.is_some());
    assert!(app.settings.flag("UseSingleInstance", false));
    // another NDR on the same profile cannot take it now (and nothing is forwarded to us)
    let profile = app.profile.clone();
    let other = std::thread::spawn(move || ndr::lifecycle::InstanceGuard::try_acquire(&profile).map(|g| g.is_some()));
    assert_eq!(other.join().unwrap(), Ok(false));
    // turning it off releases the guard
    let mut settings = app.settings.clone();
    settings.set_flag("UseSingleInstance", false);
    app.apply_settings(settings);
    assert!(app.instance_guard.is_none());
    // a guard held by another NDR: enabling fails, the setting goes back and a message box explains it
    let profile = app.profile.clone();
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let guard = ndr::lifecycle::InstanceGuard::try_acquire(&profile).unwrap();
        held_tx.send(guard.is_some()).unwrap();
        let _ = release_rx.recv();
        drop(guard);
    });
    assert!(held_rx.recv().unwrap());
    let mut settings = app.settings.clone();
    settings.set_flag("UseSingleInstance", true);
    app.apply_settings(settings);
    assert!(app.instance_guard.is_none());
    assert!(!app.settings.flag("UseSingleInstance", true), "the previous value is kept");
    assert_eq!(boxed(&app).0, "NDR");
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    let _ = ctx;
    let _ = fs::remove_dir_all(&root);
}


/// Original address bar (TNXPathLabel.SetPath) segments and pane frame colors (black skin: lframelocal 305B91, lframearc 505000, lframeinactive 3C3C3C).
#[test]
fn skin_address_segments_and_frame_colors_follow_the_original() {
    let (root, _ctx, mut app) = fixture("skin-crumbs", "en");
    app.settings.set_text("Theme", "Black");
    let skin = app.current_skin().expect("the Black skin");
    let folder = root.join("left");
    app.panes[0].path = folder.clone();
    let segments = app.crumb_segments(0);
    let full = std::path::absolute(&folder).unwrap();
    assert_eq!(segments.len(), full.ancestors().count());
    assert_eq!(segments.last().map(|s| s.label.as_str()), Some("left"));
    // Original TNXPathLabel: the root segment is "C:" (without the trailing '\').
    assert!(segments[0].label.ends_with(':') && !segments[0].label.ends_with('\\') && segments.iter().all(|s| s.folder.is_some()), "{}", segments[0].label);
    app.active = 0;
    assert_eq!(app.skin_frame_stroke(&skin, 0), egui::Stroke::new(1.0_f32, Color32::from_rgb(0x30, 0x5B, 0x91)));
    assert_eq!(app.skin_frame_stroke(&skin, 1), egui::Stroke::new(1.0_f32, Color32::from_rgb(0x1C, 0x36, 0x57)), "inactive: 60% lframelocal + 40% lback (000000), FUN_00483300");
    app.panes[0].archive = Some(ArchiveLocation { file: folder.join("x.zip"), prefix: "inner/".into(), password: String::new() });
    assert_eq!(app.skin_frame_stroke(&skin, 0).color, Color32::from_rgb(0x50, 0x50, 0x00));
    let labels: Vec<String> = app.crumb_segments(0).into_iter().map(|s| s.label).collect();
    assert_eq!(&labels[labels.len() - 3..], ["left", "x.zip", "inner"], "archive folders, the archive name, the inner folders");
    let _ = fs::remove_dir_all(root);
}

/// Original TNXTabSet (skin theme): CalcTabWidth width rules, first tab alignment, no row with one tab, tab image and text colors.
#[test]
fn skin_tab_set_follows_the_original_tnxtabset() {
    use super::skin_pane::{tab_first_index, tab_set_layout, TabSetLayout};
    assert_eq!(tab_set_layout(&[40, 50], 400), TabSetLayout { tab_width: 78, visible: 4, scroll: false }, "widest + 28");
    assert_eq!(tab_set_layout(&[50; 6], 400), TabSetLayout { tab_width: 64, visible: 6, scroll: false }, "widest + 14");
    assert_eq!(tab_set_layout(&[50; 5], 500).tab_width, 78);
    assert_eq!(tab_set_layout(&[200; 5], 416), TabSetLayout { tab_width: 80, visible: 5, scroll: false }, "evenly when wider than 60");
    assert_eq!(tab_set_layout(&[50; 10], 400), TabSetLayout { tab_width: 70, visible: 5, scroll: true }, "(384 − 32) / (352 / 60)");
    assert_eq!((tab_first_index(0, 7, 5, 10), tab_first_index(6, 2, 5, 10), tab_first_index(8, 9, 5, 10)), (3, 2, 5));
    let (root, ctx, mut app) = fixture("skin-tabs", "en");
    app.settings.set_text("Theme", "Black");
    let skin = app.current_skin().expect("the Black skin");
    let tab = app.toolbar_icon(&ctx, "tab.png").expect("tab.png");
    let run = |app: &mut App| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0))), ..Default::default() };
        ctx.run(raw, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                app.skin_tab_set(ui, 0, ctx, &skin);
                ui.label("after");
            });
        })
    };
    let after = |output: &egui::FullOutput| output.shapes.iter().find_map(|c| match &c.shape { egui::Shape::Text(t) if t.galley.text() == "after" => Some(t.pos.y), _ => None });
    let single = run(&mut app);
    assert!(after(&single).is_some_and(|y| y < 24.0), "one tab: no tab set (Visible = count > 1)");
    app.panes[0].tabs = vec![root.join("alpha"), root.join("beta")];
    app.panes[0].tab = 1;
    let output = run(&mut app);
    assert!(after(&output).is_some_and(|y| y >= 24.0), "the tab set takes 24");
    let pieces: Vec<egui::Rect> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Mesh(m) if m.texture_id == tab.id() => Some(m.calc_bounds()), _ => None }).collect();
    assert!(pieces.first().is_some_and(|r| r.left() == 16.0 && r.width() == 5.0 && r.height() == 24.0), "left part at 16: {pieces:?}");
    let texts: Vec<(String, Color32)> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Text(t) if t.galley.text() != "after" => Some((t.galley.text().to_owned(), t.fallback_color)), _ => None }).collect();
    assert_eq!(texts, [("alpha".to_owned(), Color32::from_gray(0x66)), ("beta".to_owned(), Color32::from_gray(0xC0))], "tabtext, tabactive");
    // The tab list button (left end 12 x 24) shows "1. alpha" and "2. beta". Typing "1" while the menu is open goes to the first tab (original "&1. ").
    let run_events = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0))), events, ..Default::default() };
        ctx.run(raw, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| app.skin_tab_set(ui, 0, ctx, &skin));
        })
    };
    let at = egui::pos2(6.0, 12.0);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    run_events(&mut app, vec![egui::Event::PointerMoved(at)]);
    run_events(&mut app, vec![button(true)]);
    run_events(&mut app, vec![button(false)]);
    let menu = run_events(&mut app, vec![]);
    let items: Vec<String> = menu.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Text(t) if t.galley.text().contains(". ") => Some(t.galley.text().to_owned()), _ => None }).collect();
    assert_eq!(items, ["1. alpha", "2. beta"]);
    app.menu_letter = Some('1');
    run_events(&mut app, vec![]);
    assert_eq!(app.panes[0].tab, 0, "the access key picks the first tab");
    let _ = fs::remove_dir_all(root);
}

/// Original +0x344: a pane showing hidden files has a red "H" after the folder info.
#[test]
fn skin_folder_info_shows_the_original_hidden_mark() {
    let (root, ctx, mut app) = fixture("skin-hidden-mark", "en");
    app.settings.set_text("Theme", "Black");
    let skin = app.current_skin().expect("the Black skin");
    let run = |app: &mut App| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0))), ..Default::default() };
        ctx.run(raw, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| app.skin_folder_info_bar(ui, 0, &skin));
        })
    };
    let mark = |output: &egui::FullOutput| output.shapes.iter().find_map(|c| match &c.shape { egui::Shape::Text(t) if t.galley.text() == "H" => Some(t.fallback_color), _ => None });
    app.panes[0].show_hidden = false;
    assert_eq!(mark(&run(&mut app)), None, "hidden files not shown: empty caption");
    app.panes[0].show_hidden = true;
    assert_eq!(mark(&run(&mut app)), Some(Color32::from_rgb(0xFF, 0, 0)), "hidden files shown: red H");
    let _ = fs::remove_dir_all(root);
}

/// The old extension info row must no longer open an external site when clicked.
#[test]
fn skin_extension_info_line_is_not_an_external_link() {
    let (root, ctx, mut app) = fixture("skin-extinfo", "en");
    app.settings.set_text("Theme", "Black");
    app.settings.set_flag("ShowExtensionInfoBar", true);
    let skin = app.current_skin().expect("the Black skin");
    thread_local!(static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) });
    fn runner(path: &Path, _: &str, _: &Path) -> Result<(), String> {
        OPENED.with(|o| o.borrow_mut().push(path.to_string_lossy().into_owned()));
        Ok(())
    }
    app.shell_runner = runner;
    let file = app.panes[0].entries.iter().position(|e| !e.is_dir && !e.drive).expect("a file");
    app.panes[0].cursor = file;
    let at = egui::pos2(30.0, 28.0);
    let frame = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0))), events, ..Default::default() };
        ctx.run(raw, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| app.skin_file_info_bar(ui, 0, &skin));
        })
    };
    let output = frame(&mut app, vec![egui::Event::PointerMoved(at)]);
    let texts: Vec<String> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Text(t) => Some(t.galley.text().to_owned()), _ => None }).collect();
    assert!(texts.iter().filter(|t| !t.is_empty()).count() <= 1, "only the first line has text: {texts:?}");
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&mut app, vec![button(true)]);
    frame(&mut app, vec![button(false)]);
    assert!(OPENED.with(|o| o.borrow().is_empty()), "removed file-info feature must not open a website");
    let _ = fs::remove_dir_all(root);
}

/// Original folder info bar (FUN_00482700, 20, lfolderback, lfoldertext, lfoldersubtext1) and file info bar (FUN_00482d44, 20/40, lfileback, lfiletext).
#[test]
fn skin_info_bars_follow_the_original() {
    let (root, ctx, mut app) = fixture("skin-infobars", "en");
    app.settings.set_text("Theme", "Black");
    let skin = app.current_skin().expect("the Black skin");
    assert_eq!(app.skin_file_info_height(), 20.0);
    app.settings.set_flag("ShowExtensionInfoBar", true);
    assert_eq!(app.skin_file_info_height(), 40.0, "the second line (sextinfobar)");
    app.settings.set_flag("ShowFileInfoBar", false);
    assert_eq!(app.skin_file_info_height(), 0.0);
    app.settings.set_flag("ShowFileInfoBar", true);
    app.settings.set_flag("ShowExtensionInfoBar", false);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0))), ..Default::default() };
    let output = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            app.skin_folder_info_bar(ui, 0, &skin);
            app.skin_file_info_bar(ui, 0, &skin);
        });
    });
    let fills: Vec<(egui::Rect, Color32)> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Rect(r) => Some((r.rect, r.fill)), _ => None }).collect();
    assert!(fills.contains(&(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 20.0)), Color32::BLACK)), "lfolderback: {fills:?}");
    assert!(fills.contains(&(egui::Rect::from_min_size(egui::pos2(0.0, 20.0), egui::vec2(600.0, 20.0)), Color32::from_gray(0x3C))), "lfileback: {fills:?}");
    let galleys: Vec<std::sync::Arc<egui::Galley>> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Text(t) => Some(t.galley.clone()), _ => None }).collect();
    let counts = galleys.iter().find(|g| g.text().contains("Folder")).expect("FOLDERINFO");
    let colors: Vec<Color32> = counts.job.sections.iter().map(|s| s.format.color).collect();
    assert!(colors.contains(&Color32::from_rgb(0x4C, 0x82, 0xDC)) && colors.contains(&Color32::from_gray(0x85)), "lfoldersubtext1 numbers, lfoldertext words: {colors:?}");
    let _ = fs::remove_dir_all(root);
}

/// Original DrawListHeader (0x7e2274): column names NXFORE*0.4+NXBACK*0.6, the sort column *0.6 with ▲ at the top (ascending), vertical lines NXLINE*0.3.
#[test]
fn skin_list_header_follows_the_original_draw_list_header() {
    use super::skin_pane::mix_color;
    let (fore, back) = (Color32::from_gray(0xC0), Color32::BLACK);
    assert_eq!([0.4, 0.6, 0.3].map(|t| mix_color(fore, back, t)), [Color32::from_gray(0x4D), Color32::from_gray(0x73), Color32::from_gray(0x3A)]);
    assert_eq!(mix_color(Color32::from_gray(0x22), back, 0.3), Color32::from_gray(0x0A));
    let (root, ctx, app) = fixture("skin-header", "en");
    let columns = app.detail_columns(false);
    let widths: Vec<Option<f32>> = columns.iter().map(|_| Some(80.0)).collect();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 300.0))), ..Default::default() };
    let output = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let _ = app.skin_list_header(ui, 0, &columns, &widths, egui::FontId::proportional(12.0), 0.0);
        });
    });
    let titles: Vec<(String, f32, Color32)> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.pos.x, t.fallback_color)), _ => None }).collect();
    // The original writes column text at + 5 from GetListHeaderWidth (30 in icon view): the name cell text is at 35 (same spot as the name text).
    assert_eq!(titles.first().map(|t| (t.1, t.2)), Some((35.0, Color32::from_gray(0x73))), "the Name column sorts by default: {titles:?}");
    assert!(titles.iter().skip(1).all(|t| t.2 == Color32::from_gray(0x4D)), "{titles:?}");
    let marks: Vec<egui::Rect> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Rect(r) if r.fill == Color32::from_gray(0x3A) => Some(r.rect), _ => None }).collect();
    assert_eq!(marks.len(), 3, "a 5-3-1 triangle");
    assert_eq!(marks.iter().map(|r| (r.top(), r.width())).collect::<Vec<_>>(), [(2.0, 5.0), (1.0, 3.0), (0.0, 1.0)], "ascending: wide row at the bottom");
    let _ = fs::remove_dir_all(root);
}

/// Original drive button row (FUN_004825d8, FUN_00483108): height 22, ldrivesback background, buttons every 36, text ldrivestext.
#[test]
fn skin_drive_bar_follows_the_original() {
    let (root, ctx, mut app) = fixture("skin-drives", "en");
    app.settings.set_text("Theme", "Black");
    app.settings.set_flag("ShowIcons", false);
    let skin = app.current_skin().expect("the Black skin");
    let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 300.0))), ..Default::default() };
    let output = ctx.run(raw(), |ctx| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| app.skin_drive_bar(ui, 0, ctx, &skin));
    });
    assert!(!app.settings.flag("ShowDriveButtons", false), "off by default (sdrivebtns=0)");
    assert!(!output.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == Color32::from_gray(0x20))), "nothing drawn while off");
    app.settings.set_flag("ShowDriveButtons", true);
    app.drive_cache = Some((std::time::Instant::now(), vec![(PathBuf::from("C:\\"), "C:\\ · Fixed".into()), (PathBuf::from("D:\\"), "D:\\ · Fixed".into())]));
    let output = ctx.run(raw(), |ctx| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| app.skin_drive_bar(ui, 0, ctx, &skin));
    });
    let fills: Vec<(egui::Rect, Color32)> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Rect(r) => Some((r.rect, r.fill)), _ => None }).collect();
    assert!(fills.contains(&(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 22.0)), Color32::from_gray(0x20))), "ldrivesback: {fills:?}");
    let labels: Vec<(String, f32, f32, Color32)> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.pos.x, t.galley.size().x, t.fallback_color)), _ => None }).collect();
    assert_eq!(labels.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["C:", "D:"], "one button per drive");
    // Original TNXImageButton text layout 1: image x = (36 - (spacing 2 + text width + image width)) div 2, text x = image x + image width + 2 (image width 0 with icons off).
    for (i, l) in labels.iter().enumerate() {
        assert_eq!(l.1, 36.0 * i as f32 + ((36.0 - (2.0 + l.2.round())) / 2.0).trunc() + 2.0, "{l:?}");
    }
    assert!(labels.iter().all(|l| l.3 == Color32::from_gray(0x99)), "ldrivestext (Black 999999): {labels:?}");
    let _ = fs::remove_dir_all(root);
}

/// Original TNXScrollBar (UpdateThumbWidth, RecalcThumbPos): thumb length = usable length x visible / total (at least the bar thickness, 20), position = button + proportion.
#[test]
fn skin_scrollbar_thumb_follows_the_original_tnxscrollbar() {
    use super::skin_pane::scrollbar_thumb;
    assert_eq!(scrollbar_thumb(300.0, 12.0, 16.0, 0.0, 1000.0, 250.0), (16.0, 67.0), "268 × 250 / 1000, just under the top button");
    assert_eq!(scrollbar_thumb(300.0, 12.0, 16.0, 750.0, 1000.0, 250.0), (217.0, 67.0), "the end: 16 + 268 − 67");
    assert_eq!(scrollbar_thumb(300.0, 12.0, 16.0, 375.0, 1000.0, 250.0).0, 116.0, "half way: 16 + floor(201 / 2)");
    assert_eq!(scrollbar_thumb(300.0, 12.0, 16.0, 0.0, 100_000.0, 250.0).1, 20.0, "never shorter than 20");
    assert_eq!(scrollbar_thumb(40.0, 12.0, 16.0, 0.0, 1000.0, 250.0), (16.0, 8.0), "never longer than the track between the buttons");
}

/// Original TNXPngButton DropDownMenu (MouseDown 0x7cc31c → DropDown 0x7ccc38 → Click 0x7cca74): the menu opens on the press, stays open
/// after the release, and a second press on the button closes it.
#[test]
fn dropdown_button_menu_opens_on_press() {
    let ctx = egui::Context::default();
    let button = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(16.0, 16.0));
    let frame = |events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0))), events, ..Default::default() };
        let mut open = false;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.interact(button, egui::Id::new("dropdown-test"), egui::Sense::click());
                open = super::skin_pane::menu_popup(&response).show(|ui| ui.label("item")).is_some();
            });
        });
        open
    };
    let at = button.center();
    let pointer = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    assert!(!frame(vec![egui::Event::PointerMoved(at)]));
    assert!(frame(vec![pointer(true)]), "the press opens the menu");
    assert!(frame(vec![]), "still open while held");
    assert!(frame(vec![pointer(false)]), "the release of the opening press keeps it open");
    assert!(frame(vec![]));
    assert!(!frame(vec![pointer(true)]), "a second press closes it");
    assert!(!frame(vec![pointer(false)]));
    assert!(!frame(vec![]));
    // Press and release in one frame also leaves it open.
    assert!(frame(vec![pointer(true), pointer(false)]));
    assert!(frame(vec![]));
}

/// Original TNXScrollBar MouseMove (0x7cea58) / DoScrollTimer (0x7ce734): a held arrow pauses while the pointer is off it and resumes on
/// return; a held track stops for the rest of the press once the pointer reaches the thumb or leaves the track.
#[test]
fn skin_scrollbar_hold_follows_the_original_timer_rules() {
    use super::skin_pane::{scroll_hold, HoldStep};
    let ctx = egui::Context::default();
    let bar = egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(12.0, 300.0));
    let thumb = egui::Rect::from_min_size(egui::pos2(100.0, 16.0), egui::vec2(12.0, 60.0));
    let frame = |events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0))), events, ..Default::default() };
        let mut step = HoldStep::None;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let id = egui::Id::new("bar");
                let track = ui.interact(bar, id.with("track"), egui::Sense::click_and_drag());
                let up = ui.interact(egui::Rect::from_min_size(bar.min, egui::vec2(12.0, 16.0)), id.with("up"), egui::Sense::click_and_drag());
                let down = ui.interact(egui::Rect::from_min_size(egui::pos2(100.0, 284.0), egui::vec2(12.0, 16.0)), id.with("down"), egui::Sense::click_and_drag());
                let _grip = ui.interact(thumb, id.with("thumb"), egui::Sense::drag());
                step = scroll_hold(ui, id, [(&up, -1.0), (&down, 1.0)], &track, thumb, ui.input(|i| i.pointer.interact_pos()));
            });
        });
        step
    };
    let at = |x: f32, y: f32| egui::Event::PointerMoved(egui::pos2(x, y));
    let button = |x: f32, y: f32, pressed: bool| egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    // Arrow: steps while over it, pauses off it, resumes on return.
    frame(vec![at(106.0, 290.0)]);
    assert_eq!(frame(vec![button(106.0, 290.0, true)]), HoldStep::Step(1.0));
    assert_eq!(frame(vec![at(200.0, 290.0)]), HoldStep::Paused);
    assert_eq!(frame(vec![at(106.0, 290.0)]), HoldStep::Step(1.0));
    assert_eq!(frame(vec![button(106.0, 290.0, false)]), HoldStep::None);
    // Track: pages toward the pointer; once the pointer is on the thumb, it stays stopped for this press.
    frame(vec![at(106.0, 200.0)]);
    assert_eq!(frame(vec![button(106.0, 200.0, true)]), HoldStep::Track);
    assert_eq!(frame(vec![at(106.0, 40.0)]), HoldStep::Paused, "the thumb is under the pointer");
    assert_eq!(frame(vec![at(106.0, 200.0)]), HoldStep::Paused, "not restarted (only arrows are)");
    assert_eq!(frame(vec![button(106.0, 200.0, false)]), HoldStep::None);
    assert_eq!(frame(vec![button(106.0, 200.0, true)]), HoldStep::Track, "a new press pages again");
}

/// Original DrawFocus (0x7d7f54), GraphUtil HLS, InitSelMarkBitmap and size cell division.
/// Original horizontal TNXScrollBar: draws the vertical images rotated 90 degrees (FUN_007cdfc0 clockwise, FUN_007ce0e4 counterclockwise).
#[test]
fn skin_horizontal_scrollbar_rotates_the_vertical_pieces() {
    use super::skin_pane::rotated_image;
    let ctx = egui::Context::default();
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(0.5, 1.0));
    let dest = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(16.0, 8.0));
    let corners = |clockwise: bool| {
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| rotated_image(ui.painter(), egui::TextureId::Managed(7), dest, uv, clockwise));
        });
        output.shapes.iter().find_map(|c| match &c.shape { egui::Shape::Mesh(m) if m.texture_id == egui::TextureId::Managed(7) => Some(m.vertices.iter().map(|v| (v.pos, v.uv)).collect::<Vec<_>>()), _ => None }).expect("the rotated mesh")
    };
    // Clockwise: image bottom left -> top left, top left -> top right (the top edge goes right). Counterclockwise: top right -> top left.
    let cw = corners(true);
    assert_eq!(cw[0], (dest.left_top(), egui::pos2(0.0, 1.0)));
    assert_eq!(cw[1], (dest.right_top(), egui::pos2(0.0, 0.0)));
    let ccw = corners(false);
    assert_eq!(ccw[0], (dest.left_top(), egui::pos2(0.5, 0.0)));
    assert_eq!(ccw[3], (dest.left_bottom(), egui::pos2(0.0, 0.0)));
}

#[test]
fn skin_address_segments_shrink_like_the_original_update_display_info() {
    use super::skin_pane::path_label_widths;
    // Label 200 (usable 184, first share = first text + 20), "..." 10.
    assert_eq!(path_label_widths(&[50], 10, 300), [54], "fits: text + 4");
    assert_eq!(path_label_widths(&[300], 10, 200), [168], "one segment: label − 32");
    assert_eq!(path_label_widths(&[20, 300], 10, 200), [24, 128], "two: last = 184 − (20 + 36)");
    assert_eq!(path_label_widths(&[20, 100, 30, 50], 10, 200), [24, 21, 21, 54], "middles share (184 − 110 − 32) / 2");
    assert_eq!(path_label_widths(&[20, 100, 5, 50], 10, 200), [24, 33, 9, 54], "a narrow middle keeps its width, the rest share the remainder");
    assert_eq!(path_label_widths(&[20, 100, 300], 10, 200), [24, 30, 98], "even '...' middles do not fit: the last shrinks too");
    assert!(path_label_widths(&[], 10, 200).is_empty());
}

#[test]
fn skin_thumbnail_focus_follows_the_original_draw_focus_style_3() {
    use super::skin_pane::{rounded_gradient_shapes, thumbnail_focus, thumbnail_focus_rect};
    let gray = Color32::from_gray;
    // NXSELECT FFFFFF (lightness 240 -> lower 190 = CA), NXBACK 000000. Blend values are original single constants (0.9 = 0.89999998 -> 229).
    let (select, back) = (gray(0xFF), gray(0x00));
    assert_eq!(thumbnail_focus(select, back, true, false, false, true), (gray(0x66), gray(0x51), gray(0x99)), "cursor, focused: 0.4 and line 0.6");
    assert_eq!(thumbnail_focus(select, back, true, true, false, true).2, gray(0xE5), "marked cursor line 0.9");
    assert_eq!(thumbnail_focus(select, back, false, true, false, true), (gray(0x33), gray(0x28), gray(0x66)), "marked only: 0.2 and line 0.4");
    assert_eq!(thumbnail_focus(select, back, true, true, false, false).2, gray(0xB2), "unfocused marked cursor line 0.7");
    assert_eq!(thumbnail_focus(select, back, false, true, false, false), (gray(26), gray(20), gray(77)), "unfocused marked: 0.1 and line 0.3");
    assert_eq!(thumbnail_focus(select, back, false, false, true, true), (gray(128), gray(101), gray(255)), "drop target: 0.5 and line 1.0");
    // A dark NXSELECT lightens the top (lightness < 50 -> +50).
    let (top, bottom, _) = thumbnail_focus(gray(0x10), back, true, false, false, true);
    assert!(top.r() > bottom.r(), "{top:?} {bottom:?}");
    // Position: cell 150 x 200, size 120, text 20 -> left (150 - 132) div 2 + 1 = 10, top 1, 130 x 150.
    let cell = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(150.0, 200.0));
    assert_eq!(thumbnail_focus_rect(cell, 120.0, 20.0), egui::Rect::from_min_size(egui::pos2(10.0, 1.0), egui::vec2(130.0, 150.0)));
    // Rounded rectangle (radius 4): gradient fill and a 1 px border.
    let shapes = rounded_gradient_shapes(egui::Rect::from_min_size(egui::pos2(10.0, 1.0), egui::vec2(130.0, 150.0)), 4.0, gray(0x66), gray(0x51), gray(0x99));
    let egui::Shape::Mesh(mesh) = &shapes[0] else { panic!("fill mesh") };
    assert_eq!(mesh.calc_bounds(), egui::Rect::from_min_size(egui::pos2(10.0, 1.0), egui::vec2(130.0, 150.0)));
    assert!(mesh.vertices.iter().any(|v| v.color == gray(0x66)) && mesh.vertices.iter().any(|v| v.color == gray(0x51)), "top and bottom colours");
    assert!(matches!(&shapes[1], egui::Shape::Rect(r) if r.stroke.color == gray(0x99) && r.corner_radius == egui::CornerRadius::same(4)));
}

#[test]
fn skin_focus_bar_follows_the_original_draw_focus() {
    use super::skin_pane::{focus_bar, hls_to_rgb, rgb_to_hls, selection_mark_shapes, split_size_text, FocusBar};
    let gray = |v: u8| Color32::from_gray(v);
    assert_eq!(rgb_to_hls(Color32::WHITE), (160, 240, 0));
    assert_eq!(rgb_to_hls(Color32::from_rgb(255, 0, 0)), (0, 120, 240));
    assert_eq!(hls_to_rgb(0, 120, 240), Color32::from_rgb(255, 0, 0), "pure red survives the round trip");
    let (file, back, select) = (gray(0xC0), Color32::BLACK, Color32::WHITE);
    assert_eq!(focus_bar(file, back, select, true, false, true), FocusBar { top: gray(0xC0), bottom: gray(0xAB), lines: Some(gray(0xA0)) }, "cursor, focused: the item colour, L − 20 at the bottom, L − 30 lines");
    assert_eq!(focus_bar(file, back, select, false, true, true), FocusBar { top: gray(0x26), bottom: gray(0x23), lines: None }, "marked: NXSELECT mixed 0.15 with NXBACK");
    assert_eq!(focus_bar(file, back, select, true, false, false), FocusBar { top: gray(0x0C), bottom: gray(0x0C), lines: Some(gray(0x30)) }, "cursor, unfocused: flat 0.06, lines 0.25");
    assert_eq!(focus_bar(file, back, select, false, true, false).top, gray(0x1A), "marked, unfocused: 0.1");
    assert_eq!(focus_bar(gray(0x08), back, select, true, false, true).top, hls_to_rgb(160, 28, 0), "dark items get lighter at the top instead");
    assert_eq!((split_size_text("1,234 KB"), split_size_text("12"), split_size_text("3.5 GB")), (("1,234", " KB"), ("12", ""), ("3.5", " GB")));
    let rect = egui::Rect::from_min_size(egui::pos2(10.0, 100.0), egui::vec2(200.0, 20.0));
    let marks: Vec<egui::Rect> = selection_mark_shapes(rect, select).into_iter().filter_map(|s| match s { egui::Shape::Rect(r) => Some(r.rect), _ => None }).collect();
    assert_eq!(marks, [
        egui::Rect::from_min_max(egui::pos2(13.0, 106.0), egui::pos2(14.0, 113.0)),
        egui::Rect::from_min_max(egui::pos2(14.0, 107.0), egui::pos2(15.0, 112.0)),
        egui::Rect::from_min_max(egui::pos2(15.0, 108.0), egui::pos2(16.0, 111.0)),
        egui::Rect::from_min_max(egui::pos2(16.0, 109.0), egui::pos2(17.0, 110.0)),
    ], "a 7-5-3-1 ▶ at left + 2, centred on the row");
}

/// Original bbFunctions (skin theme): seven equal cells F2-F8, key text bbkeytext (black E2A31D), background btnbar_back.bmp repeated.
#[test]
fn skin_function_key_bar_follows_the_original_button_bar() {
    let (root, ctx, mut app) = fixture("skin-fkeys", "ko");
    app.settings.set_text("Theme", "Black");
    let skin = app.current_skin().expect("the Black skin");
    let tile = app.skin_bitmap(&ctx, "btnbar_back.bmp").expect("btnbar_back.bmp");
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 400.0))), ..Default::default() };
    let output = ctx.run(raw, |ctx| {
        egui::TopBottomPanel::bottom("functionbar").exact_height(20.0).frame(egui::Frame::NONE).show(ctx, |ui| app.skin_function_key_bar(ui, ctx, &skin));
    });
    let keys: Vec<(String, egui::Pos2, Color32)> = output
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text().starts_with('F') && t.galley.text().len() == 2 => Some((t.galley.text().to_owned(), t.pos, t.fallback_color)),
            _ => None,
        })
        .collect();
    assert_eq!(keys.iter().map(|k| k.0.as_str()).collect::<Vec<_>>(), ["F2", "F3", "F4", "F5", "F6", "F7", "F8"]);
    assert!(keys.iter().enumerate().all(|(i, k)| (k.1.x - (97.0 * i as f32 + 5.0)).abs() < 0.01), "cells of (700 - 18 size grip) / 7 = 97, text at + 5: {keys:?}");
    assert!(keys.iter().all(|k| k.2 == Color32::from_rgb(0xE2, 0xA3, 0x1D)), "bbkeytext");
    let tiles = output.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Mesh(m) if m.texture_id == tile.id())).count();
    assert!(tiles >= 700 / 8, "{tiles}");
    // Dividers at ItemRect.right - 2 (the last item ends at 700 - 18), y = (20 - 2 - 14) / 2.
    let divider = app.skin_bitmap(&ctx, "btnbar_divider.bmp").expect("btnbar_divider.bmp");
    let dividers: Vec<egui::Pos2> = output
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Mesh(m) if m.texture_id == divider.id() => Some(m.calc_bounds().min),
            _ => None,
        })
        .collect();
    let expected: Vec<egui::Pos2> = (0..7).map(|i| egui::pos2(if i == 6 { 680.0 } else { 97.0 * (i + 1) as f32 - 4.0 }, 382.0)).collect();
    assert_eq!(dividers, expected);
    // The size grip past Width - 20 shows the resize cursor.
    let at = egui::pos2(690.0, 390.0);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 400.0))), events: vec![egui::Event::PointerMoved(at)], ..Default::default() };
    let output = ctx.run(raw, |ctx| {
        egui::TopBottomPanel::bottom("functionbar").exact_height(20.0).frame(egui::Frame::NONE).show(ctx, |ui| app.skin_function_key_bar(ui, ctx, &skin));
    });
    assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::ResizeNwSe);
    // Original bbFunctionsContextPopup: right-clicking a cell also runs that function key (F7 new folder -> name input window).
    let at = egui::pos2(550.0, 390.0);
    let button = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
    for events in [vec![egui::Event::PointerMoved(at), button(true)], vec![button(false)]] {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 400.0))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::TopBottomPanel::bottom("functionbar").exact_height(20.0).frame(egui::Frame::NONE).show(ctx, |ui| app.skin_function_key_bar(ui, ctx, &skin));
        });
    }
    assert!(!app.original_dialogs.is_empty() || !app.msgboxes.is_empty(), "a right click on F7 runs F7");
    let _ = fs::remove_dir_all(root);
}

/// Original FUN_00479970: "[name]command" uses the text inside [ ], "#internal command" RES FUNC_<UPPER>, programs the file name without extension.
#[test]
fn function_key_captions_follow_the_original_button_bar() {
    use super::user_commands::original_function_key_caption;
    let resource = |key: &str| match key {
        "FUNC_RECYCLEBIN" => Some("휴지통에 버리기".to_owned()),
        "FUNC_REFRESH" => Some("최신 정보로 갱신".to_owned()),
        _ => None,
    };
    assert_eq!(original_function_key_caption("#RecycleBin", &resource), "휴지통에 버리기");
    assert_eq!(original_function_key_caption("#refresh", &resource), "최신 정보로 갱신", "names match without case");
    assert_eq!(original_function_key_caption("#Rename", &resource), "Rename", "no FUNC_ text: the name after #");
    assert_eq!(original_function_key_caption("#Unknown", &resource), "Unknown", "not an internal command");
    assert_eq!(original_function_key_caption("[메모장]C:\\Windows\\notepad.exe", &resource), "메모장");
    assert_eq!(original_function_key_caption("[]x", &resource), "]x", "empty label: everything after [");
    // FUN_0040d494: only the program part counts; an unquoted space starts the arguments.
    assert_eq!(original_function_key_caption("C:\\Tools\\my tool.exe", &resource), "my");
    assert_eq!(original_function_key_caption("\"C:\\Tools\\my tool.exe\" /a %s", &resource), "my tool");
    assert_eq!(original_function_key_caption("notepad.exe %s", &resource), "notepad");
    assert_eq!(original_function_key_caption("\"C:\\v1.0 beta\\run", &resource), "run", "no closing quote: to the end");
    assert_eq!(original_function_key_caption("C:\\Tools\\v1.0 beta\\run", &resource), "v1");
    assert_eq!(original_function_key_caption("  ", &resource), "");
}

/// Original FUN_00444710 (skin theme): 35x35 buttons at top tbtopoffset (black 7), from left 4 every width + 4, separator cells 15,
/// align=right items from (width - 6 - sum(width + 4)). The background tiles toolbar_back.bmp horizontally.
#[test]
fn skin_toolbar_follows_the_original_layout() {
    let (left, right) = super::toolbar::toolbar_slots(&[35.0, 35.0, 15.0, 35.0], &[35.0], 800.0);
    assert_eq!((left, right), (vec![4.0, 43.0, 82.0, 101.0], vec![800.0 - 6.0 - 39.0]));
    let (root, ctx, mut app) = fixture("skin-toolbar", "en");
    app.settings.set_text("Theme", "Black");
    let skin = app.current_skin().expect("the Black skin");
    assert_eq!((skin.positive_integer("Toolbar", "tbheight", 0), skin.positive_integer("Toolbar", "tbtopoffset", 0)), (51, 7));
    let back = app.toolbar_icon(&ctx, "tb_back.png").expect("tb_back.png");
    let tile = app.skin_bitmap(&ctx, "toolbar_back.bmp").expect("toolbar_back.bmp");
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))), ..Default::default() };
    let output = ctx.run(raw, |ctx| {
        egui::TopBottomPanel::top("toolbar").exact_height(51.0).frame(egui::Frame::NONE).show(ctx, |ui| app.skin_toolbar(ui, ctx, &skin));
    });
    let bounds = |id: egui::TextureId| output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Mesh(m) if m.texture_id == id => Some(m.calc_bounds()), _ => None }).collect::<Vec<_>>();
    let first = app.toolbar.items.iter().position(|i| i.kind == ndr::commands::ToolbarKind::Command && i.icon.ends_with("tb_back.png")).expect("a Back item");
    assert_eq!(first, 0, "Back comes first in the shipped toolbar.xml");
    // Original default\toolbar.xml: back, forward | desktop, documents, pictures, music, videos, favorites | Recycle Bin (right).
    let commands: Vec<(&str, bool)> = app.toolbar.items.iter().filter(|i| i.kind == ndr::commands::ToolbarKind::Command).map(|i| (i.icon.rsplit('\\').next().unwrap_or(""), i.align_right)).collect();
    assert_eq!(commands, [("tb_back.png", false), ("tb_forward.png", false), ("tb_desktop.png", false), ("tb_documents.png", false), ("tb_pictures.png", false), ("tb_music.png", false), ("tb_videos.png", false), ("tb_favorites.png", false), ("tb_recyclebin.png", true)]);
    assert_eq!(bounds(back.id()).first().copied(), Some(egui::Rect::from_min_size(egui::pos2(4.0, 7.0), egui::vec2(35.0, 35.0))));
    let tiles = bounds(tile.id());
    assert!(tiles.len() >= 1000 / 8 && tiles.iter().all(|r| r.height() == 51.0), "{}", tiles.len());
    // Original TNXImageButton (one image cell): hovering changes nothing, pressing makes colors x 6 / 10 (FUN_007c39e4).
    let at = egui::pos2(20.0, 20.0);
    let frame = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))), events, ..Default::default() };
        ctx.run(raw, |ctx| {
            egui::TopBottomPanel::top("toolbar").exact_height(51.0).frame(egui::Frame::NONE).show(ctx, |ui| app.skin_toolbar(ui, ctx, &skin));
        })
    };
    let back_color = |output: &egui::FullOutput| output.shapes.iter().find_map(|c| match &c.shape { egui::Shape::Mesh(m) if m.texture_id == back.id() => m.vertices.first().map(|v| v.color), _ => None });
    // Original Back button (+0xd70): without history Enabled = False -> constant alpha 0x82.
    app.panes[app.active].back.clear();
    let disabled = frame(&mut app, vec![egui::Event::PointerMoved(at)]);
    assert_eq!(back_color(&disabled), Some(Color32::WHITE.gamma_multiply(130.0 / 255.0)), "no history: disabled");
    app.panes[app.active].back.push(root.clone());
    let hovered = frame(&mut app, vec![egui::Event::PointerMoved(at)]);
    assert_eq!(back_color(&hovered), Some(Color32::WHITE), "hover: unchanged");
    assert!(!hovered.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.contains(at))), "no hover overlay");
    frame(&mut app, vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }]);
    let pressed = frame(&mut app, vec![]);
    assert_eq!(back_color(&pressed), Some(Color32::from_gray(153)), "pressed (still held): × 0.6");
    let _ = fs::remove_dir_all(root);
}

/// Original FUN_00444710: the Favorites button has DropDownMenu = pmFavorites. Clicking opens the favorites menu at the button's bottom left.
#[test]
fn skin_toolbar_favorites_drops_down_the_favorites_menu() {
    let (root, ctx, mut app) = fixture("skin-favorites", "en");
    app.settings.set_text("Theme", "Black");
    let skin = app.current_skin().expect("the Black skin");
    let frame = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))), events, ..Default::default() };
        ctx.run(raw, |ctx| {
            egui::TopBottomPanel::top("toolbar").exact_height(51.0).frame(egui::Frame::NONE).show(ctx, |ui| app.skin_toolbar(ui, ctx, &skin));
        })
    };
    // Left group: back 4, forward 43, separator 82 (15), desktop 101 ... favorites 296 (width 35, top 7).
    let at = egui::pos2(310.0, 20.0);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&mut app, vec![egui::Event::PointerMoved(at)]);
    frame(&mut app, vec![button(true)]);
    // TNXImageButton MouseDown → DropDown: the menu is already open on the press.
    assert_eq!(app.popup.as_ref().and_then(|p| p.position), Some(egui::pos2(296.0, 42.0)), "opened on the press");
    let popup_frame = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))), events, ..Default::default() };
        ctx.run(raw, |ctx| {
            egui::TopBottomPanel::top("toolbar").exact_height(51.0).frame(egui::Frame::NONE).show(ctx, |ui| app.skin_toolbar(ui, ctx, &skin));
            app.popup_menu_ui(ctx);
        })
    };
    popup_frame(&mut app, vec![]);
    popup_frame(&mut app, vec![button(false)]);
    popup_frame(&mut app, vec![]);
    let popup = app.popup.as_ref().expect("the release of the opening press keeps the favorites menu");
    assert_eq!(popup.position, Some(egui::pos2(296.0, 42.0)), "under the button");
    assert!(app.original_dialogs.is_empty(), "not the organize dialog");
    let _ = fs::remove_dir_all(root);
}

/// Original TNXLabel markup &lt;b&gt;: the "bold" family (Segoe UI Bold) is wider than the normal face.
#[test]
fn bold_markup_uses_a_bold_face() {
    let (root, ctx, _app) = fixture("bold-face", "en");
    let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0))), ..Default::default() };
    let _ = ctx.run(raw(), |_| {});
    let mut widths = (0.0, 0.0);
    let _ = ctx.run(raw(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let width = |ui: &egui::Ui, family: egui::FontFamily| ui.fonts_mut(|f| f.layout_no_wrap("Hidden WWW".into(), egui::FontId::new(12.0, family), Color32::WHITE).size().x);
            widths = (width(ui, egui::FontFamily::Proportional), width(ui, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())));
        });
    });
    if std::path::Path::new("C:/Windows/Fonts/segoeuib.ttf").exists() {
        assert!(widths.1 > widths.0, "{widths:?}");
    }
    let _ = fs::remove_dir_all(root);
}

/// Original pmMain (DFM): the list right-click menu of the skin theme has only original items. The C# theme keeps the C# open, to other pane and copy path rows.
/// Original pVertBar buttons (TNXPngButton, vertbarbtns.png 160x16 = ten 16x16 cells): cells are drawn at 16x16 (not squashed).
#[test]
fn vertical_bar_cells_are_square() {
    let (root, ctx, mut app) = fixture("vertbar-cells", "en");
    app.settings.set_text("Theme", "Black");
    let strip = app.toolbar_icon(&ctx, "vertbarbtns.png").expect("vertbarbtns.png");
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(20.0, 500.0))), ..Default::default() };
    let output = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| app.vertical_bar(ui, ctx));
    });
    let sizes: Vec<egui::Vec2> = output.shapes.iter().filter_map(|c| match &c.shape {
        egui::Shape::Mesh(m) if m.texture_id == strip.id() => {
            let (min, max) = m.vertices.iter().fold((egui::pos2(f32::MAX, f32::MAX), egui::pos2(f32::MIN, f32::MIN)), |(a, b), v| (a.min(v.pos), b.max(v.pos)));
            Some(max - min)
        }
        _ => None,
    }).collect();
    assert_eq!(sizes.len(), 8, "eight strip buttons: {sizes:?}");
    assert!(sizes.iter().all(|s| (s.x - 16.0).abs() < 0.01 && (s.y - 16.0).abs() < 0.01), "{sizes:?}");
    // FUN_0042cc2c cell table [0,1,2,3,4,5,6,8] in name order: the sync buttons at the top use cells 0 and 1, the copy buttons 4 and 5.
    let mut cells: Vec<(i32, u32)> = output.shapes.iter().filter_map(|c| match &c.shape {
        egui::Shape::Mesh(m) if m.texture_id == strip.id() => {
            let top = m.vertices.iter().map(|v| v.pos.y).fold(f32::MAX, f32::min);
            let u = m.vertices.iter().map(|v| v.uv.x).fold(f32::MAX, f32::min);
            Some((top.round() as i32, (u * 10.0).round() as u32))
        }
        _ => None,
    }).collect();
    cells.sort();
    assert_eq!(cells.iter().map(|c| c.1).collect::<Vec<_>>(), [0, 1, 2, 3, 4, 5, 6, 8], "{cells:?}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn skin_menus_use_the_original_menu_colours() {
    // Original MenuDrawItem (0x42e518) and menu bar (0x430e74): skin [Menu] colors (black: mback 141414, mtext B4B4B4, mhighlight E6E6E6, mover 000000,
    // mdivider 303030, mborder 5F5F5F, mrtext C8C8C8, mmainback 2A2A2A, mmaintext 9F9F9F, mmainoverback 333333, mmainover FFFFFF).
    let (root, _ctx, mut app) = fixture("skin-menu-colours", "en");
    app.settings.set_text("Theme", "Black");
    let skin = app.current_skin().expect("the Black skin");
    let mut style = egui::Style::default();
    super::skin_pane::skin_menu_style(&skin).apply(&mut style);
    let v = &style.visuals;
    assert_eq!((v.window_fill, v.window_stroke.color), (Color32::from_rgb(0x14, 0x14, 0x14), Color32::from_rgb(0x5F, 0x5F, 0x5F)));
    assert_eq!(v.widgets.inactive.fg_stroke.color, Color32::from_rgb(0xB4, 0xB4, 0xB4));
    assert_eq!((v.widgets.hovered.weak_bg_fill, v.widgets.hovered.fg_stroke.color), (Color32::from_rgb(0xE6, 0xE6, 0xE6), Color32::BLACK));
    assert_eq!((v.widgets.noninteractive.bg_stroke.color, v.weak_text_color), (Color32::from_rgb(0x30, 0x30, 0x30), Some(Color32::from_rgb(0xC8, 0xC8, 0xC8))));
    assert_eq!(v.override_text_color, None);
    // Original MenuDrawItem: text at row left + 0x10 (the check image slot before it), MenuMeasureItem width = text + 0x20.
    assert_eq!(style.spacing.button_padding.x, super::skin_pane::MENU_GUTTER);
    assert_eq!(super::skin_pane::MENU_GUTTER, 16.0);
    let (bar_back, bar) = super::skin_pane::skin_menu_bar_style(&skin);
    let mut style = egui::Style::default();
    bar.apply(&mut style);
    assert_eq!(bar_back, Color32::from_rgb(0x2A, 0x2A, 0x2A));
    assert_eq!((style.visuals.widgets.inactive.fg_stroke.color, style.visuals.widgets.hovered.weak_bg_fill, style.visuals.widgets.hovered.fg_stroke.color), (Color32::from_rgb(0x9F, 0x9F, 0x9F), Color32::from_rgb(0x33, 0x33, 0x33), Color32::WHITE));
    // Shortcut column: mrback 000000, mrtext C8C8C8, mrover 5577EE.
    assert_eq!(super::skin_pane::skin_menu_shortcut_colors(&skin), (Color32::BLACK, Color32::from_rgb(0xC8, 0xC8, 0xC8), Color32::from_rgb(0x55, 0x77, 0xEE)));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn checked_menu_items_draw_the_original_check_bitmaps() {
    // Original MenuDrawItem: checkable items draw CHECK (21 points), radio items RCHECK (12 points) at the row left, nothing when unchecked.
    let ctx = egui::Context::default();
    let mut counts = Vec::new();
    for (checked, radio) in [(true, false), (true, true), (false, false)] {
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                super::skin_pane::menu_check_button(ui, true, checked, radio, egui::Button::new("item"));
            });
        });
        let quads: usize = output
            .shapes
            .iter()
            .filter_map(|clipped| if let egui::Shape::Mesh(mesh) = &clipped.shape { Some(mesh.vertices.len() / 4) } else { None })
            .sum();
        counts.push(quads);
    }
    assert_eq!(counts, vec![21, 12, 0]);
}

/// Original FUN_0048aa30: the folder info bar of My PC also uses FOLDERINFO (drives are not counted). The internal path "::drives::" is never shown.
/// Original FUN_0048c548: subfolders of the address bar ">" menu come in FindFirstFileW order (NTFS name order).
#[test]
fn my_computer_info_bar_and_crumb_children_follow_the_original() {
    let (root, ctx, mut app) = fixture("drives-info", "en");
    fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let skin = app.current_skin().expect("the skin");
    app.panes[0].path = PathBuf::from(super::remote_window::DRIVES);
    app.panes[0].entries = vec![Entry { name: "C:".into(), path: PathBuf::from("C:\\"), is_dir: true, size: 0, modified: None, hidden: false, attributes: 0x10, drive: true }];
    app.panes[0].summary = None;
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 200.0))), ..Default::default() };
    let output = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| app.skin_folder_info_bar(ui, 0, &skin));
    });
    let mut out = vec![];
    output.shapes.iter().for_each(|c| texts(&c.shape, &mut out));
    let all = out.join(" ");
    assert!(!all.contains("::drives::") && all.contains('0'), "{all}");
    // Subfolder order.
    let folder = root.join("crumbs");
    for name in ["beta", "Alpha", "gamma"] {
        fs::create_dir_all(folder.join(name)).unwrap();
    }
    let names: Vec<String> = app.crumb_children(0, &folder, false).into_iter().map(|(_, n)| n).collect();
    assert_eq!(names, vec!["Alpha", "beta", "gamma"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn skin_list_menu_has_only_the_original_items() {
    let (root, ctx, mut app) = fixture("skin-list-menu", "en");
    fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let menu = |app: &mut App| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 900.0))), ..Default::default() };
        let output = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = app.list_context_menu(ui, 0);
            });
        });
        let mut out = vec![];
        output.shapes.iter().for_each(|c| texts(&c.shape, &mut out));
        out
    };
    let copy_path = app.language_pack.raw("formMain.Menu.miCopyFullPath").map(|t| t.replace('&', "")).unwrap_or_else(|| "전체 경로 복사".into());
    app.settings.set_text("Theme", "Black");
    assert!(app.current_skin().is_some());
    let skin = menu(&mut app);
    assert!(!skin.iter().any(|t| t.contains("other pane") || *t == copy_path), "{skin:?}");
    assert!(skin.iter().any(|t| t.contains("Properties")), "{skin:?}");
    let _ = fs::remove_dir_all(root);
}

/// Original toolbar UserCommand (FUN_0044381c, not internal) and Container (pmToolBarPopup FUN_00444e60 -> FUN_00444e04).
#[test]
fn toolbar_user_command_and_container_follow_the_original() {
    let (root, ctx, mut app) = fixture("toolbar-container", "en");
    let xml = br#"<toolbar><item type="Container"><name>More</name><subitems><item type="InternalCommand"><name>Up</name><command>GoToParent</command></item><item type="Separator"/><item type="UserCommand"><command>[Calc]calc.exe</command></item></subitems></item></toolbar>"#;
    app.toolbar = ndr::commands::parse_toolbar(xml).unwrap();
    app.show_toolbar_container(0, Some(egui::pos2(10.0, 40.0)));
    let popup = app.popup.as_ref().expect("the container menu");
    assert_eq!(popup.position, Some(egui::pos2(10.0, 40.0)));
    let rows: Vec<String> = popup.items.iter().map(|item| match item {
        super::favorites_dialog::PopupItem::Entry { label, .. } => label.clone(),
        super::favorites_dialog::PopupItem::Separator => "-".into(),
    }).collect();
    assert_eq!(rows, ["Up", "-", "Calc"], "names, and the original caption rule for an unnamed user command");
    thread_local!(static RAN: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) });
    fn runner(path: &Path, _: &str, _: &Path) -> Result<(), String> {
        RAN.with(|r| r.borrow_mut().push(path.to_string_lossy().into_owned()));
        Ok(())
    }
    app.shell_runner = runner;
    app.toolbar_command("user:[Calc]calc.exe", &ctx);
    assert_eq!(RAN.with(|r| r.borrow().clone()), ["calc.exe"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn empty_temp_asks_with_counts_and_empties_only_the_contents_like_the_original() {
    let (root, ctx, mut app) = fixture("empty-temp", "en");
    // Fake temp folder for tests (the real GetTempPathW folder is not touched).
    let temp = root.join("fake-temp");
    fs::create_dir_all(temp.join("sub")).unwrap();
    fs::write(temp.join("a.txt"), "12345").unwrap();
    fs::write(temp.join("sub").join("b.txt"), "123").unwrap();
    let locked = temp.join("readonly.txt");
    fs::write(&locked, "1").unwrap();
    let mut permissions = fs::metadata(&locked).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&locked, permissions).unwrap();
    // When GetTempPathW fails (None), nothing happens.
    TEMP_FOLDER.with(|t| *t.borrow_mut() = None);
    app.dispatch("emptyTemp", &ctx);
    assert!(app.msgboxes.is_empty() && app.original_dialogs.is_empty());
    TEMP_FOLDER.with(|t| *t.borrow_mut() = Some(temp.clone()));
    app.dispatch("emptyTemp", &ctx);
    // Counting runs separately: the screen does not freeze meanwhile, and it asks when done.
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    let (title, text) = app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone())).expect("the MSG_EMPTYTEMP question");
    assert_eq!(title, app.language_pack.text("RES.TITLE_EMPTYTEMP", "?"));
    let expected = app.language_pack.text("RES.MSG_EMPTYTEMP", "?").replacen("%d", "3", 1).replacen("%s", &ndr::native::format_byte_size(9), 1);
    assert_eq!(text, expected);
    assert!(app.original_dialogs.is_empty(), "no cleanup scan window (C# flow) any more");
    // No: unchanged.
    app.answer_message_box(ndr::msgbox::IDNO, &ctx);
    assert!(temp.join("a.txt").exists());
    // Yes: deletes only the contents (read-only ones too, without asking) and keeps the folder.
    app.dispatch("emptyTemp", &ctx);
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    wait_job(&mut app, &ctx);
    assert!(temp.is_dir(), "the temporary folder itself stays (flag 0x10)");
    assert_eq!(fs::read_dir(&temp).unwrap().count(), 0, "every item inside is gone");
    assert!(app.original_dialogs.is_empty() && app.msgboxes.is_empty(), "no confirmation or error report (flags 0x20/0x40/0x80)");
    TEMP_FOLDER.with(|t| *t.borrow_mut() = None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn drive_root_goes_up_to_this_pc_and_drive_items_rename_their_volume_label() {
    let (root, ctx, mut app) = fixture("drives-view", "en");
    let (left, right) = (root.join("left"), root.join("right"));
    DRIVES.with(|d| *d.borrow_mut() = vec![left.clone(), right.clone()]);
    LABELS.with(|l| *l.borrow_mut() = vec![(left.clone(), "OLD".into())]);
    let drive_root = PathBuf::from(left.components().next().unwrap().as_os_str()).join("\\");
    // Original FUN_00459ba8: does not go up from a root.
    assert!(app.current_skin().is_some());
    app.navigate(0, drive_root.clone(), false);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.parent();
    assert_eq!(app.panes[0].path, drive_root, "the original stays at a drive root");
    // Original FUN_00486474: with "show disk drives in the list" on, drive items follow the folder list.
    app.settings.set_flag("ShowDriveList", true);
    app.navigate(0, left.clone(), false);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].drive_count(), 2);
    let first_drive = app.panes[0].entries.len() - 2;
    app.settings.set_flag("ShowDriveList", false);
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].drive_count(), 0);
    app.settings.set_flag("ShowDriveList", true);
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    // Drive branch of original FUN_004351cc: TfGetText(window title, MSG_DRIVELABEL, current volume name) -> SetVolumeLabelW.
    app.active = 0;
    app.panes[0].cursor = first_drive;
    app.dispatch("rename", &ctx);
    let (title, message, value, _) = prompt(&app);
    assert_eq!((title.as_str(), message, value.as_str()), ("NDR", app.language_pack.text("RES.MSG_DRIVELABEL", "?"), "OLD"));
    app.answer_dialog(DialogResult::Text(Some("NEW".into())), &ctx);
    assert_eq!(fake_volume_label(&left).as_deref(), Some("NEW"));
    // ERROR_ACCESS_DENIED uses the original TfMessage (a brief notice in the middle of the list, 3 seconds); other failures and empty names are silent.
    app.dispatch("rename", &ctx);
    app.answer_dialog(DialogResult::Text(Some("deny-me".into())), &ctx);
    assert_eq!(app.pane_message.as_ref().map(|(t, i, _)| (t.clone(), *i)), Some((app.language_pack.text("RES.ACCESS_DENIED", "?"), 0)));
    assert!(app.status.is_empty(), "not on the status line");
    // Original Timer1 (3000 ms): closes after 3 seconds.
    app.pane_message.as_mut().unwrap().2 = Instant::now() - Duration::from_millis(3100);
    app.pane_message_ui(&ctx);
    assert!(app.pane_message.is_none());
    app.dispatch("rename", &ctx);
    app.answer_dialog(DialogResult::Text(Some("fail-me".into())), &ctx);
    app.dispatch("rename", &ctx);
    app.answer_dialog(DialogResult::Text(Some(String::new())), &ctx);
    assert!(app.status.is_empty() && app.msgboxes.is_empty() && app.pane_message.is_none());
    assert_eq!(fake_volume_label(&left).as_deref(), Some("NEW"));
    // A drive that is not ready (no volume info) asks nothing.
    app.panes[0].cursor = first_drive + 1;
    app.dispatch("rename", &ctx);
    assert!(app.original_dialogs.is_empty());
    // Opening a drive item goes to its root.
    app.navigate(0, right.clone(), false);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.panes[0].cursor = app.panes[0].entries.len() - 2;
    app.open_current();
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].path, left);
    DRIVES.with(|d| d.borrow_mut().clear());
    LABELS.with(|l| l.borrow_mut().clear());
    fs::remove_dir_all(root).unwrap();
}


#[test]
fn closing_while_working_asks_the_original_question_and_columns_use_field_names() {
    let (root, ctx, mut app) = fixture("close-working", "ko");
    // Original column headers FIELD_* ("type" is a C# column, only in the C# theme). The skin theme has the six original cells.
    let [name, ext, size, date, attr, desc] = ["FIELD_NAME", "FIELD_EXT", "FIELD_SIZE", "FIELD_DATEMODIFIED", "FIELD_ATTR", "FIELD_DESC"].map(|key| app.language_pack.text(&format!("RES.{key}"), "?"));
    assert!(app.current_skin().is_some(), "the default theme is the Black skin");
    let titles: Vec<String> = app.detail_columns(true).into_iter().map(|(_, title, _)| title).collect();
    assert_eq!(titles, [name.clone(), ext.clone(), size.clone(), date.clone(), attr.clone(), desc.clone()]);
    // Original DrawListItem description cell: the file type text when there is no description and showfiletype is on.
    app.compute_view(0);
    let file = app.panes[0].entries.iter().position(|e| !e.is_dir && !e.drive).expect("a file");
    let cells = app.panes[0].view.cells[file].clone();
    assert_eq!(cells[DetailColumn::Description.index()], cells[DetailColumn::Kind.index()]);
    // Original GetList: the type text is the shell name (SHGetFileInfoW SHGFI_TYPENAME).
    let entry = &app.panes[0].entries[file];
    assert_eq!(cells[DetailColumn::Kind.index()], native::shell_type_name(ndr::parity::dotnet_extension(&entry.name), false));
    // Original FormCloseQuery (0x42a798): during a job STILL_HAVE_OPERATION (TITLE_INFO, 0x2124). No = keep, yes = stop and close.
    assert!(app.start_task("long".into(), |_, cancel| {
        while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }));
    app.confirm_close_while_working();
    let question = app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone(), m.flags)).expect("the close question");
    assert_eq!(question, (app.language_pack.text("RES.TITLE_INFO", "?"), app.language_pack.text("RES.STILL_HAVE_OPERATION", "?"), 0x2124));
    app.confirm_close_while_working();
    assert_eq!(app.msgboxes.len(), 1, "asked once");
    app.answer_message_box(ndr::msgbox::IDNO, &ctx);
    assert!(app.job.as_ref().is_some_and(|j| !j.cancel.load(std::sync::atomic::Ordering::Relaxed)), "No keeps working");
    app.confirm_close_while_working();
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    assert!(app.job.as_ref().is_some_and(|j| j.cancel.load(std::sync::atomic::Ordering::Relaxed)), "Yes stops the operation");
    wait_job(&mut app, &ctx);
    // Original BACKHINT/FORWARDHINT: tooltips with the path the button opens. Without history the button text.
    let left = root.join("left");
    app.active = 0;
    app.navigate(0, left.join("folder1"), true);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.toolbar_hint("historyBack", "Back"), app.language_pack.text("RES.BACKHINT", "?").replace("%s", &left.to_string_lossy()));
    assert_eq!(app.toolbar_hint("historyForward", "Forward"), "Forward");
    assert_eq!(app.toolbar_hint("refresh", "Refresh"), "Refresh");
    assert_eq!(app.toolbar_hint("deleteToolbar", "Delete"), app.language_pack.text("RES.RECYCLEBIN_HINT", "?"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn work_folder_buttons_set_open_and_clear_like_the_original() {
    let (root, ctx, mut app) = fixture("work-buttons", "ko");
    let left = root.join("left");
    app.active = 0;
    // Original FUN_0042f27c tooltip: an empty slot = WORKFOLDER_SETHINT.
    assert_eq!(app.work_folder_hint(2), app.language_pack.text("RES.WORKFOLDER_SETHINT", "?"));
    // Original FUN_0042fc84: clicking an empty slot sets it to the current folder without asking.
    app.work_folder_click(2);
    assert!(app.msgboxes.is_empty());
    assert_eq!(app.settings.strings("WorkingFolders")[2], left.to_string_lossy());
    assert_eq!(app.work_folder_hint(2), format!("{}\n{}", left.display(), app.language_pack.text("RES.WORKFOLDER_UNSETHINT", "?")));
    // Clicking a set slot goes to that folder.
    app.navigate(0, left.join("folder1"), true);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.work_folder_click(2);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].path, left);
    assert_eq!(app.settings.strings("WorkingFolders")[2], left.to_string_lossy(), "opening does not overwrite");
    // Original FUN_0042fe60: the right button clears it.
    app.work_folder_clear(2);
    assert!(app.settings.strings("WorkingFolders")[2].is_empty());
    // Original pVertBar buttons: select none, sync paths (bSyncAddressClick).
    app.panes[0].selected = [0usize].into_iter().collect();
    app.vertical_bar_click("bClearSelection", &ctx);
    assert!(app.panes[0].selected.is_empty());
    app.vertical_bar_click("bSyncPathToRight", &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[1].loading);
    assert_eq!(app.panes[1].path, app.panes[0].path, "right path = left path");
    let right = root.join("right");
    app.navigate(1, right.clone(), true);
    wait_until(&mut app, &ctx, |app| !app.panes[1].loading);
    app.vertical_bar_click("bSyncPathToLeft", &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].path, right, "left path = right path");
    // Divider (C# PaneDivider LeftPaneWidth %, 20-80; original pmiFrameWidthSame = 50%).
    assert!((app.left_pane_ratio() - 0.5).abs() < 1e-6);
    app.set_left_pane_width(30.0);
    assert!((app.left_pane_ratio() - 0.3).abs() < 1e-6);
    app.set_left_pane_width(95.0);
    assert!((app.left_pane_ratio() - 0.8).abs() < 1e-6, "clamped to 80%");
    assert_eq!(app.settings.values.get("RightPaneWidth").and_then(|v| v.as_f64()), Some(20.0));
    app.set_left_pane_width(50.0);
    // Also draws the bar (skin strip images). Original iVertThumb: vertthumb.png at its own size centered at the bar bottom (height - 25 cell).
    app.settings.set_text("Theme", "Black");
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let mut bar = egui::Rect::NOTHING;
    let output = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            bar = ui.max_rect();
            app.vertical_bar(ui, ctx)
        });
    });
    let thumb = app.toolbar_icon(&ctx, "vertthumb.png").expect("skin vertthumb.png");
    let drawn = output.shapes.iter().find_map(|clipped| match &clipped.shape {
        egui::Shape::Mesh(mesh) if mesh.texture_id == thumb.id() => Some(mesh.calc_bounds()),
        _ => None,
    });
    let drawn = drawn.expect("the grip image is painted");
    assert_eq!(drawn.size(), thumb.size_vec2());
    // Original iVertThumb (Left 3, 12x22) and FUN_0042cc2c (Top = bar height - 22 - 4): the image centered in that cell.
    assert!((drawn.center().x - (bar.min.x + 9.0)).abs() < 0.01 && (drawn.center().y - (bar.max.y - 26.0 + 11.0)).abs() < 0.01, "{drawn:?} in {bar:?}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn control_panel_and_add_remove_programs_follow_original() {
    let (root, ctx, mut app) = fixture("system-menu", "ko");
    launched();
    VERBS.with(|v| v.borrow_mut().clear());
    // Original miControlPanelClick: control.exe. miAddRemovePorgClick (0x432fc8): rundll32.exe "shell32.dll,Control_RunDLL appwiz.cpl".
    app.dispatch("controlPanel", &ctx);
    assert_eq!(VERBS.with(|v| v.borrow().clone()), vec![(PathBuf::from("control.exe"), "open".to_owned())]);
    app.dispatch("addRemovePrograms", &ctx);
    assert_eq!(launched(), vec![("rundll32.exe".to_owned(), "shell32.dll,Control_RunDLL appwiz.cpl".to_owned(), PathBuf::new())]);
    let _ = fs::remove_dir_all(root);
}
