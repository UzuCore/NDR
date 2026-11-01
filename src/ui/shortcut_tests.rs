//! Checks in the Rust UI the C# HandleKeyboardAsync order (original shortcut table -> NDR shortcuts -> action definitions), user shortcuts first, CycleFileSelection, work folders,
//! favorites keys, the Esc action, column count and the last drive folder. Korean and English only, isolated folders only.
use super::*;
use std::fs;

fn fixture(label: &str, language: &str) -> (PathBuf, egui::Context, App) {
    let root = std::env::temp_dir().join(format!("ndr-keys-{}-{}-{label}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let (left, right) = (root.join("left"), root.join("right"));
    fs::create_dir_all(left.join("folder1")).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("a.txt"), b"a").unwrap();
    fs::write(left.join("b.txt"), b"b").unwrap();
    let ctx = egui::Context::default();
    let options = Options { profile: Some(root.join("profile")), left: Some(left), right: Some(right), language: Some(language.into()), ..Default::default() };
    let mut app = App::from_context(&ctx, options);
    app.clipboard_writer = |_| Ok(());
    app.shell_runner = |_, _, _| Ok(());
    wait(&mut app, &ctx);
    (root, ctx, app)
}
fn wait(app: &mut App, ctx: &egui::Context) {
    let end = Instant::now() + Duration::from_secs(20);
    while app.panes.iter().any(|p| p.loading) {
        assert!(Instant::now() < end, "timeout");
        app.poll(ctx);
        thread::sleep(Duration::from_millis(5));
    }
}
/// Runs only the list key handling on a frame where one key is pressed.
fn press(app: &mut App, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    // Release in the same frame so the next press is not seen as a repeat.
    let down = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let up = egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers };
    let raw = egui::RawInput { events: vec![down, up], modifiers, ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
    wait(app, ctx);
}
const NONE: egui::Modifiers = egui::Modifiers::NONE;

fn archive_clipboard_fixture(label: &str) -> (PathBuf, egui::Context, App, PathBuf) {
    let (root, ctx, mut app) = fixture(label, "ko");
    let left = root.join("left");
    fs::write(left.join("folder1/nested.txt"), b"nested").unwrap();
    let zip = left.join("source.zip");
    ndr::archive::create(&[left.join("a.txt"), left.join("folder1")], &zip, None, &AtomicBool::new(false)).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "source.zip").unwrap();
    app.open_current();
    wait(&mut app, &ctx);
    assert!(app.panes[0].archive.is_some());
    (root, ctx, app, zip)
}

/// Both panes point at different inner folders of the *same* ZIP.
fn same_archive_panes_fixture(label: &str) -> (PathBuf, egui::Context, App, PathBuf) {
    let (root, ctx, mut app, zip) = archive_clipboard_fixture(label);
    let receive = root.join("left/receive");
    fs::create_dir_all(&receive).unwrap();
    fs::write(receive.join("existing.txt"), b"existing").unwrap();
    ndr::archive::add_entries(&zip, "", &[receive], None, &AtomicBool::new(false)).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    app.navigate(1, root.join("left"), false);
    wait(&mut app, &ctx);
    app.active = 1;
    app.panes[1].cursor = app.panes[1].entries.iter().position(|e| e.name == "source.zip").unwrap();
    app.open_current();
    wait(&mut app, &ctx);
    app.panes[1].cursor = app.panes[1].entries.iter().position(|e| e.name == "receive").unwrap();
    app.open_current();
    wait(&mut app, &ctx);
    assert_eq!(app.panes[1].archive.as_ref().unwrap().prefix.replace('\\', "/"), "receive/");
    assert_eq!(ndr::archive::first_volume_path(&app.panes[1].archive.as_ref().unwrap().file), zip);
    (root, ctx, app, zip)
}

fn finish_archive_clipboard_job(app: &mut App, ctx: &egui::Context) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        app.poll(ctx);
        if app.job.as_ref().is_none_or(|j| j.finished) && app.panes.iter().all(|p| !p.loading) {
            return;
        }
        assert!(Instant::now() < until, "archive clipboard operation timed out: {}", app.status);
        thread::sleep(Duration::from_millis(5));
    }
}

/// Ctrl+C must stage real files for CF_HDROP, then Ctrl+V must extract from the
/// original ZIP even when its pane has navigated away in the meantime.
#[test]
fn ctrl_c_in_archive_copies_file_to_the_other_pane_after_leaving_the_archive() {
    let (root, ctx, mut app, zip) = archive_clipboard_fixture("archive-copy");
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    let menu = app.list_menu_items(0);
    assert!(menu.iter().flatten().any(|(_, command, enabled)| *command == "copy" && *enabled));
    assert!(menu.iter().flatten().any(|(_, command, enabled)| *command == "cut" && *enabled));
    press(&mut app, &ctx, egui::Key::C, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    let clipboard = app.archive_clipboard.as_ref().expect("Ctrl+C should prepare an archive clipboard");
    let job = app.job.as_ref().expect("clipboard extraction should have a completed progress job");
    assert!(job.archive_clipboard_started.is_some() && job.files > 0);
    assert_eq!(job.progress.as_ref().and_then(|p| p.total_files), Some(job.files), "clipboard progress should receive real extracted entry counts");
    assert!(app.archive_clipboard_notice.is_some(), "completed clipboard preparation shows a brief completion cue");
    assert!(!clipboard.moving);
    assert_eq!(fs::read(clipboard.stage.join("a.txt")).unwrap(), b"a", "Windows must receive a real file, not a virtual archive key");
    app.navigate(0, root.join("left"), false);
    wait(&mut app, &ctx);
    app.active = 1;
    press(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    assert_eq!(fs::read(root.join("right/a.txt")).unwrap(), b"a");
    assert!(ndr::archive::list(&zip, None).unwrap().iter().any(|e| e.key == "a.txt"), "copy must preserve source");
    let stage = app.archive_clipboard.take().unwrap().stage;
    fs::remove_dir_all(stage).unwrap();
    fs::remove_dir_all(root).unwrap();
}

/// The clipboard UI has an animated progress indicator only after its delay;
/// it uses a real per-entry total and dismisses its completion cue by itself.
#[test]
fn archive_clipboard_progress_and_completion_notice_have_bounded_lifetimes() {
    let (root, ctx, mut app) = fixture("clipboard-dialog-timing", "ko");
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut job = Job::new(rx, Arc::new(AtomicBool::new(false)), "압축에서 복사".to_owned());
    job.archive_clipboard_started = Some(Instant::now());
    job.files = 1;
    job.path = "folder/one.txt".into();
    job.progress.as_mut().unwrap().total_files = Some(3);
    app.job = Some(job);
    let _ = ctx.run(egui::RawInput::default(), |ctx| app.file_progress_ui(ctx));
    app.job.as_mut().unwrap().archive_clipboard_started = Some(Instant::now() - Duration::from_millis(400));
    let output = ctx.run(egui::RawInput::default(), |ctx| app.file_progress_ui(ctx));
    assert!(!output.shapes.is_empty(), "visible clipboard progress shows a real UI rather than only a status line");
    app.job = None;
    app.archive_clipboard_notice = Some((Instant::now(), false));
    let output = ctx.run(egui::RawInput::default(), |ctx| app.archive_clipboard_notice_ui(ctx));
    assert!(!output.shapes.is_empty(), "success cue is visible immediately after completion");
    app.archive_clipboard_notice = Some((Instant::now() - Duration::from_millis(1200), false));
    let _ = ctx.run(egui::RawInput::default(), |ctx| app.archive_clipboard_notice_ui(ctx));
    assert!(app.archive_clipboard_notice.is_none(), "completion cue closes without a user click");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsupported_archive_formats_do_not_enter_panes_or_open_extract_dialogs() {
    let (root, ctx, mut app) = fixture("unsupported-formats", "ko");
    for extension in ["ace", "tar.lz", "tlz", "uefi"] {
        let file = root.join(format!("left/unsupported.{extension}"));
        fs::write(&file, b"unsupported format marker").unwrap();
    }
    app.refresh(0);
    wait(&mut app, &ctx);
    for extension in ["ace", "tar.lz", "tlz", "uefi"] {
        let name = format!("unsupported.{extension}");
        app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
        app.open_current();
        assert!(app.panes[0].archive.is_none(), "{extension}: should remain in local folder");
        assert!(!app.msgboxes.is_empty(), "{extension}: unsupported action should explain why");
        app.msgboxes.clear();
        app.dispatch("extractAuto", &ctx);
        assert!(app.job.is_none() && app.original_dialogs.is_empty(), "{extension}: extraction must not start");
        assert!(!app.msgboxes.is_empty(), "{extension}: extraction refusal should be visible");
        app.msgboxes.clear();
    }
    fs::remove_dir_all(root).unwrap();
}

/// A failed archive open must not strand a pane with a red error and no
/// reliable way to navigate back to the containing folder.
#[test]
fn broken_archive_open_returns_to_local_folder_instead_of_trapping_pane() {
    let (root, ctx, mut app) = fixture("archive-open-recovery", "ko");
    let broken = root.join("left/broken.7z");
    fs::write(&broken, b"not a 7zip file").unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "broken.7z").unwrap();
    app.open_current();
    wait(&mut app, &ctx);
    assert!(app.panes[0].archive.is_none(), "failed archive open must exit archive mode");
    assert_eq!(app.panes[0].path, root.join("left"));
    assert!(app.panes[0].error.is_none(), "stale red archive error must be cleared");
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "broken.7z", "failed source stays focused");
    assert!(app.status.contains("압축파일을 열지 못해"), "status must explain why the archive was not opened: {}", app.status);
    fs::remove_dir_all(root).unwrap();
}

/// Optional UI test: the user's real 7z with Unix-created Windows-reserved
/// `nul` filename. Only a test copy is opened; the original is untouched.
#[test]
fn actual_bk3_archive_opens_and_navigates_without_top_panel_error() {
    let Ok(source) = std::env::var("NDR_LOCAL_BK3") else { return };
    let (root, ctx, mut app) = fixture("archive-bk3-real-gui", "ko");
    let name = "bk3.7z";
    let archive = root.join("left").join(name);
    fs::copy(&source, &archive).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    app.open_current();
    wait(&mut app, &ctx);
    assert!(app.panes[0].archive.is_some(), "real archive must remain open with safe members visible");
    assert!(app.panes[0].error.is_none(), "no red panel error while browsing");
    assert!(app.panes[0].entries.iter().any(|e| e.name == "assets" && e.is_dir));
    assert!(!app.panes[0].entries.iter().any(|e| e.name.eq_ignore_ascii_case("nul")), "reserved device name must not become a virtual file");
    assert!(app.status.contains("예약 장치명"), "skipped reserved entries must be disclosed");
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "assets").unwrap();
    app.open_current();
    wait(&mut app, &ctx);
    assert!(app.panes[0].archive.as_ref().unwrap().prefix.ends_with("assets/"));
    assert!(app.panes[0].error.is_none());
    app.parent();
    wait(&mut app, &ctx);
    assert!(app.panes[0].archive.is_some() && app.panes[0].archive.as_ref().unwrap().prefix.is_empty());
    app.parent();
    wait(&mut app, &ctx);
    assert!(app.panes[0].archive.is_none(), "back from archive root must return to regular folder");
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, name);
    fs::remove_dir_all(root).unwrap();
}

/// Ctrl+X must keep the original until a successful Ctrl+V, then remove that
/// entry from its ZIP and consume the internal clipboard.
#[test]
fn ctrl_x_in_archive_moves_file_to_the_other_pane_only_on_paste() {
    let (root, ctx, mut app, zip) = archive_clipboard_fixture("archive-cut");
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    press(&mut app, &ctx, egui::Key::X, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    assert!(app.archive_clipboard.as_ref().unwrap().moving);
    assert!(ndr::archive::list(&zip, None).unwrap().iter().any(|e| e.key == "a.txt"), "Ctrl+X alone must never delete the archive source");
    let stage = app.archive_clipboard.as_ref().unwrap().stage.clone();
    app.active = 1;
    press(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    assert_eq!(fs::read(root.join("right/a.txt")).unwrap(), b"a");
    assert!(!ndr::archive::list(&zip, None).unwrap().iter().any(|e| e.key == "a.txt"), "successful move must delete only the transferred entry");
    assert!(app.archive_clipboard.is_none(), "successful Ctrl+X/Ctrl+V should consume its clipboard");
    assert!(!stage.exists(), "staged files must be cleaned up after a successful internal move");
    fs::remove_dir_all(root).unwrap();
}

/// Clipboard-pasted folders must recursively copy their contents rather than
/// copying only the virtual directory entry.
#[test]
fn ctrl_c_in_archive_recursively_copies_a_folder() {
    let (root, ctx, mut app, zip) = archive_clipboard_fixture("archive-folder-copy");
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "folder1").unwrap();
    press(&mut app, &ctx, egui::Key::C, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    assert_eq!(fs::read(app.archive_clipboard.as_ref().unwrap().stage.join("folder1/nested.txt")).unwrap(), b"nested");
    app.active = 1;
    press(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    assert_eq!(fs::read(root.join("right/folder1/nested.txt")).unwrap(), b"nested");
    assert!(ndr::archive::list(&zip, None).unwrap().iter().any(|e| e.key.replace('\\', "/") == "folder1/nested.txt"));
    fs::remove_dir_all(app.archive_clipboard.take().unwrap().stage).unwrap();
    fs::remove_dir_all(root).unwrap();
}

/// Ctrl+C -> Ctrl+V between separate archive panes must add the extracted
/// item to the destination ZIP instead of treating it as a local path.
#[test]
fn ctrl_c_from_archive_pastes_into_another_archive() {
    let (root, ctx, mut app, source) = archive_clipboard_fixture("archive-to-archive");
    let right = root.join("right");
    fs::write(right.join("old.txt"), b"old").unwrap();
    let target = right.join("target.zip");
    ndr::archive::create(&[right.join("old.txt")], &target, None, &AtomicBool::new(false)).unwrap();
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    press(&mut app, &ctx, egui::Key::C, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    app.refresh(1);
    wait(&mut app, &ctx);
    app.active = 1;
    app.panes[1].cursor = app.panes[1].entries.iter().position(|e| e.name == "target.zip").unwrap();
    app.open_current();
    wait(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    let entries = ndr::archive::list(&target, None).unwrap();
    assert!(entries.iter().any(|e| e.key == "a.txt") && entries.iter().any(|e| e.key == "old.txt"));
    assert!(ndr::archive::list(&source, None).unwrap().iter().any(|e| e.key == "a.txt"));
    fs::remove_dir_all(app.archive_clipboard.take().unwrap().stage).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_archive_two_panes_ctrl_c_copies_between_inner_folders_without_removing_source() {
    let (root, ctx, mut app, zip) = same_archive_panes_fixture("same-zip-copy");
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    press(&mut app, &ctx, egui::Key::C, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    app.active = 1;
    press(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    let entries = ndr::archive::list(&zip, None).unwrap();
    assert!(entries.iter().any(|e| e.key.replace('\\', "/") == "receive/a.txt"));
    assert!(entries.iter().any(|e| e.key == "a.txt"), "copy must retain the source");
    assert!(entries.iter().any(|e| e.key.replace('\\', "/") == "receive/existing.txt"), "unrelated destination content must remain");
    fs::remove_dir_all(app.archive_clipboard.take().unwrap().stage).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_archive_two_panes_ctrl_x_moves_only_after_successful_paste() {
    let (root, ctx, mut app, zip) = same_archive_panes_fixture("same-zip-move");
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    press(&mut app, &ctx, egui::Key::X, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    assert!(ndr::archive::list(&zip, None).unwrap().iter().any(|e| e.key == "a.txt"), "cut alone must not delete the source");
    let staged = app.archive_clipboard.as_ref().unwrap().stage.clone();
    app.active = 1;
    press(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    let entries = ndr::archive::list(&zip, None).unwrap();
    assert!(!entries.iter().any(|e| e.key == "a.txt"), "move deletes the source after the destination is committed");
    assert!(entries.iter().any(|e| e.key.replace('\\', "/") == "receive/a.txt"));
    assert!(entries.iter().any(|e| e.key.replace('\\', "/") == "receive/existing.txt"));
    assert!(app.archive_clipboard.is_none(), "successful cut must be consumed");
    assert!(!staged.exists(), "temporary clipboard files are cleaned");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn f4_move_between_folders_of_same_archive_is_no_longer_blocked() {
    let (root, ctx, mut app, zip) = same_archive_panes_fixture("same-zip-f4");
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    assert!(app.archive_route_to_other(0, true));
    finish_archive_clipboard_job(&mut app, &ctx);
    let entries = ndr::archive::list(&zip, None).unwrap();
    assert!(!entries.iter().any(|e| e.key == "a.txt"));
    assert!(entries.iter().any(|e| e.key.replace('\\', "/") == "receive/a.txt"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_archive_self_folder_and_descendant_moves_are_refused_before_writing() {
    let (root, ctx, mut app, zip) = same_archive_panes_fixture("same-zip-reject");
    let bytes = fs::read(&zip).unwrap();
    // Moving a file to the folder it already occupies must not delete it.
    app.archive_transfer_out(0, vec!["a.txt".into()], app.panes[1].path.clone(), Some((zip.clone(), "".into(), None)), true);
    assert!(!app.msgboxes.is_empty(), "same-path move must give an explicit explanation");
    app.msgboxes.clear();
    assert!(app.job.as_ref().is_none_or(|job| job.finished));
    // Neither copying nor moving a folder into a descendant is safe.
    app.archive_transfer_out(0, vec!["folder1".into()], app.panes[1].path.clone(), Some((zip.clone(), "folder1/deeper/".into(), None)), false);
    assert!(!app.msgboxes.is_empty(), "descendant copy must be rejected");
    assert_eq!(fs::read(&zip).unwrap(), bytes, "rejected transfers must not change archive bytes");
    fs::remove_dir_all(root).unwrap();
    let _ = ctx;
}

/// A Ctrl+X paste skipped at a name conflict must retain the original entry.
#[test]
fn ctrl_x_archive_paste_skipped_conflict_keeps_source_and_destination() {
    let (root, ctx, mut app, source) = archive_clipboard_fixture("archive-cut-conflict");
    let right = root.join("right");
    fs::write(right.join("a.txt"), b"existing").unwrap();
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    press(&mut app, &ctx, egui::Key::X, egui::Modifiers::CTRL);
    finish_archive_clipboard_job(&mut app, &ctx);
    app.active = 1;
    press(&mut app, &ctx, egui::Key::V, egui::Modifiers::CTRL);
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        app.poll(&ctx);
        if matches!(app.original_dialogs.last().map(|d| &d.kind), Some(super::original_dialogs::DialogKind::FileConflict(_))) {
            break;
        }
        assert!(Instant::now() < until, "no conflict prompt for Ctrl+X paste: {}", app.status);
        thread::sleep(Duration::from_millis(5));
    }
    app.answer_dialog(super::original_dialogs::DialogResult::FileConflict(Some((ndr::fsops::ConflictChoice::Skip, false))), &ctx);
    finish_archive_clipboard_job(&mut app, &ctx);
    assert_eq!(fs::read(right.join("a.txt")).unwrap(), b"existing");
    assert!(ndr::archive::list(&source, None).unwrap().iter().any(|e| e.key == "a.txt"));
    assert_eq!(app.archive_clipboard.as_ref().unwrap().keys, ["a.txt"]);
    fs::remove_dir_all(app.archive_clipboard.take().unwrap().stage).unwrap();
    fs::remove_dir_all(root).unwrap();
}

/// Original miSelect (Spacebar, miSelectClick 0x4311a4) = SelectCurrAndMove (0x7dca04): marks or unmarks the cursor item, folders and files alike, and
/// moves to the next item. ".." (Select 0x7dc834) is never marked. Ctrl+Shift+arrows, like the original ArrowKey, keep the selection and add from the position
/// before moving to the new one; Shift alone clears the selection and selects from where Shift was pressed.
#[test]
fn space_marks_and_moves_like_the_original() {
    let (root, ctx, mut app) = fixture("space", "ko");
    app.active = 0;
    draw_panes(&mut app, &ctx);
    app.active = 0;
    let names: Vec<String> = app.panes[0].entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, ["..", "folder1", "a.txt", "b.txt"]);
    let marks = |app: &App| app.panes[0].selected.iter().copied().collect::<Vec<_>>();
    app.panes[0].cursor = 0;
    press(&mut app, &ctx, egui::Key::Space, NONE);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![], 1), "\"..\" is not marked, the cursor moves on");
    press(&mut app, &ctx, egui::Key::Space, NONE);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![1], 2), "a folder is marked");
    press(&mut app, &ctx, egui::Key::Space, NONE);
    press(&mut app, &ctx, egui::Key::Space, NONE);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![1, 2, 3], 3), "files are marked, the last item keeps the cursor");
    press(&mut app, &ctx, egui::Key::Space, NONE);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![1, 2], 3), "Space again unmarks");
    // The menu item (Edit menu "select") does the same.
    app.panes[0].selected.clear();
    app.panes[0].cursor = 1;
    app.dispatch("select", &ctx);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![1], 2));
    // Ctrl+Shift adds without clearing. Then Shift alone: again from where Shift was pressed (2).
    let ctrl_shift = egui::Modifiers { ctrl: true, shift: true, ..Default::default() };
    app.panes[0].selected = [3].into_iter().collect();
    app.panes[0].cursor = 2;
    press(&mut app, &ctx, egui::Key::ArrowUp, ctrl_shift);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![1, 2, 3], 1), "Ctrl+Shift adds the moved range");
    press(&mut app, &ctx, egui::Key::ArrowUp, egui::Modifiers::SHIFT);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![1, 2], 0), "Shift selects from where Shift went down");
    fs::remove_dir_all(root).unwrap();
}

/// Draws both panes once (the drawn list leaves its page info).
fn draw_panes(app: &mut App, ctx: &egui::Context) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |columns| {
                app.pane_ui(&mut columns[0], 0, ctx);
                app.pane_ui(&mut columns[1], 1, ctx);
            });
        });
    });
}

/// Original ArrowKey, UpdateDisplayNames and SetItemIndex: the Names view is a column-flowing list, so Right/Left move by one column (rows per column),
/// PageDown goes to the last item of the visible page. Moving with Shift selects again from where Shift was pressed (moving back shrinks it).
/// In Details, End scrolls so the last item is visible.
#[test]
fn list_keys_follow_the_original_arrowkey() {
    let (root, ctx, mut app) = fixture("arrowkeys", "ko");
    for i in 0..80 {
        fs::write(root.join("left").join(format!("file{i:02}.txt")), b"x").unwrap();
    }
    app.settings.set_text("LeftViewStyle", "Names");
    app.refresh(0);
    wait(&mut app, &ctx);
    app.active = 0;
    draw_panes(&mut app, &ctx);
    let count = app.panes[0].entries.len();
    let page = app.panes[0].page;
    assert_eq!(page.style, super::list_keys::ListStyle::Flow);
    assert!(page.rows > 1 && page.rows < count, "names flow into columns: {page:?}");
    assert_eq!(app.panes[0].cursor, 0);
    press(&mut app, &ctx, egui::Key::ArrowRight, NONE);
    assert_eq!(app.panes[0].cursor, page.rows, "Right moves one column");
    press(&mut app, &ctx, egui::Key::ArrowLeft, NONE);
    assert_eq!(app.panes[0].cursor, 0, "Left moves one column back");
    press(&mut app, &ctx, egui::Key::PageDown, NONE);
    assert_eq!(app.panes[0].cursor, super::list_keys::arrow_target(&page, 0, count, super::list_keys::ListKey::PageDown));
    // Shift: from where Shift was pressed (the current cursor) to the cursor.
    press(&mut app, &ctx, egui::Key::Home, NONE);
    let start = app.panes[0].cursor;
    press(&mut app, &ctx, egui::Key::ArrowDown, SHIFT);
    press(&mut app, &ctx, egui::Key::ArrowDown, SHIFT);
    press(&mut app, &ctx, egui::Key::ArrowUp, SHIFT);
    let expected: Vec<usize> = [start, start + 1].into_iter().filter(|&i| !app.panes[0].entries[i].is_parent()).collect();
    assert_eq!(app.panes[0].selected.iter().copied().collect::<Vec<_>>(), expected, "the range shrinks back (\"..\" is never marked)");
    // Details: after End and drawing, the list shows down to the last item.
    app.settings.set_text("LeftViewStyle", "Details");
    draw_panes(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::End, NONE);
    assert_eq!(app.panes[0].cursor, count - 1);
    draw_panes(&mut app, &ctx);
    draw_panes(&mut app, &ctx);
    let page = app.panes[0].page;
    assert_eq!((page.style, page.last), (super::list_keys::ListStyle::Details, count - 1), "End reveals the last item: {page:?}");
    // Original SetItemIndex (style 2, by page = default): the top item is the first row of the page that holds the cursor.
    assert_eq!(page.top, ((count - 1) / page.rows) * page.rows, "page-unit reveal starts the page on a boundary: {page:?}");
    fs::remove_dir_all(root).unwrap();
}
/// Original ArrowKey Ctrl keys: Ctrl+PageUp first item, Ctrl+PageDown last folder, Ctrl+Home first file, Ctrl+End last file.
#[test]
fn ctrl_list_keys_jump_between_folders_and_files() {
    let (root, ctx, mut app) = fixture("ctrljump", "ko");
    fs::create_dir_all(root.join("left").join("folder2")).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    app.active = 0;
    let entries = &app.panes[0].entries;
    let folders = entries.iter().filter(|e| e.is_dir).count();
    assert_eq!((folders, entries.len()), (3, 5), "\"..\", two folders, two files");
    assert!(entries[..folders].iter().all(|e| e.is_dir), "folders come first");
    press(&mut app, &ctx, egui::Key::End, CTRL);
    assert_eq!(app.panes[0].cursor, 4, "Ctrl+End = last file");
    press(&mut app, &ctx, egui::Key::PageDown, CTRL);
    assert_eq!(app.panes[0].cursor, 2, "Ctrl+PageDown = last folder");
    press(&mut app, &ctx, egui::Key::Home, CTRL);
    assert_eq!(app.panes[0].cursor, 3, "Ctrl+Home = first file");
    press(&mut app, &ctx, egui::Key::PageUp, CTRL);
    assert_eq!(app.panes[0].cursor, 0, "Ctrl+PageUp = first item");
    assert!(app.panes[0].selected.is_empty(), "Ctrl jumps do not select");
    fs::remove_dir_all(root).unwrap();
}
/// Original UpdateDisplayAuto: even when only one column fits because it is narrow, a list that does not fit in one column scrolls horizontally by one column width.
#[test]
fn narrow_auto_view_flows_one_column_at_a_time() {
    let (root, ctx, mut app) = fixture("narrowauto", "ko");
    for i in 0..80 {
        fs::write(root.join("left").join(format!("file{i:02}.txt")), b"x").unwrap();
    }
    app.settings.set_text("LeftViewStyle", "Auto");
    app.refresh(0);
    wait(&mut app, &ctx);
    app.active = 0;
    let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 850.0))), ..Default::default() };
    let draw = |app: &mut App| {
        let _ = ctx.run(raw(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    draw(&mut app);
    let count = app.panes[0].entries.len();
    let page = app.panes[0].page;
    assert_eq!((page.style, page.columns), (super::list_keys::ListStyle::Flow, 1), "one-column flow: {page:?}");
    assert!(page.rows > 1 && page.rows < count, "the column does not hold every item: {page:?}");
    press(&mut app, &ctx, egui::Key::ArrowRight, NONE);
    assert_eq!(app.panes[0].cursor, page.rows, "Right moves one column");
    fs::remove_dir_all(root).unwrap();
}
/// Original list OnContextPopup (FUN_00484e8c): local items get pmMain; without marks and with the cursor on a drive item, that drive's Explorer menu;
/// with "right click: use the Explorer menu", the Explorer menu of the marked items (otherwise the cursor item). pmMain always has the Explorer menu item.
#[test]
fn list_right_click_menu_follows_the_original_context_popup() {
    use super::panes::ListMenu;
    let (root, ctx, mut app) = fixture("ctxmenu", "ko");
    for drive in fake_drive_roots() {
        fs::create_dir_all(&drive).unwrap();
    }
    app.system.ready_drives = fake_drive_roots;
    app.system.drive_item = fake_drive_info;
    app.refresh(0);
    wait(&mut app, &ctx);
    let entries = &app.panes[0].entries;
    let file = entries.iter().position(|e| e.name == "a.txt").unwrap();
    let drive = entries.iter().position(|e| e.drive).unwrap();
    let drive_path = entries[drive].path.clone();
    app.panes[0].cursor = file;
    assert!(matches!(app.list_menu_kind(0), ListMenu::Main), "a file: pmMain");
    app.panes[0].cursor = drive;
    assert!(matches!(app.list_menu_kind(0), ListMenu::Shell(ref p) if *p == [drive_path.clone()]), "an unmarked drive item: its shell menu");
    app.panes[0].selected.insert(file);
    assert!(matches!(app.list_menu_kind(0), ListMenu::Main), "with marks the drive cursor gets pmMain");
    app.settings.set_flag("UseExplorerMenu", true);
    let a = app.panes[0].entries[file].path.clone();
    assert!(matches!(app.list_menu_kind(0), ListMenu::Shell(ref p) if *p == [a.clone()]), "explorer menu setting: the marked items");
    // Original FUN_00485078 (left button while holding the right button): the opposite of the Explorer menu setting.
    assert!(matches!(app.rocker_menu_kind(0), ListMenu::Main), "explorer menu setting on: the rocker gesture gives pmMain");
    app.settings.set_flag("UseExplorerMenu", false);
    assert!(matches!(app.rocker_menu_kind(0), ListMenu::Shell(ref p) if *p == [a.clone()]), "setting off: the shell menu of the marked items");
    app.panes[0].selected.clear();
    assert!(matches!(app.rocker_menu_kind(0), ListMenu::Shell(ref p) if *p == [drive_path.clone()]), "an unmarked drive cursor: its shell menu");
    app.panes[0].selected.insert(file);
    app.settings.set_flag("UseExplorerMenu", false);
    // pmMain: the Explorer menu is always at the top and items other than paste are enabled (clipboard files are not assumed absent here).
    app.panes[0].selected.clear();
    app.panes[0].cursor = file;
    let items = app.list_menu_items(0);
    let commands: Vec<(&str, bool)> = items.iter().flatten().map(|(_, command, enabled)| (*command, *enabled)).collect();
    assert_eq!(commands.first().map(|c| c.0), Some("explorerMenu"), "{commands:?}");
    for (command, enabled) in &commands {
        if *command != "paste" {
            assert!(enabled, "{command} is enabled like the original pmMain");
        }
    }
    assert!(commands.iter().any(|c| c.0 == "openWith"), "a focused file shows open with / view / edit");
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "folder1").unwrap();
    let commands: Vec<&str> = app.list_menu_items(0).iter().flatten().map(|(_, c, _)| *c).collect();
    assert!(!commands.contains(&"openWith") && commands.contains(&"openInNewTab"), "a folder: open in new tab, no open with: {commands:?}");
    fs::remove_dir_all(root).unwrap();
    for drive in fake_drive_roots() {
        let _ = fs::remove_dir_all(drive);
    }
}

/// Original pane OnEnter (FUN_004849c8): when the current pane is a local folder, that folder is the process current directory (the test checks only the record).
#[test]
fn active_local_pane_becomes_the_current_directory() {
    let (root, _ctx, mut app) = fixture("cwd", "ko");
    app.active = 0;
    app.sync_current_directory();
    assert_eq!(app.current_directory.as_ref(), Some(&app.panes[0].path));
    app.active = 1;
    app.sync_current_directory();
    assert_eq!(app.current_directory.as_ref(), Some(&app.panes[1].path));
    fs::remove_dir_all(root).unwrap();
}

/// Fake drive roots (two isolated temp folders).
fn fake_drive_roots() -> Vec<PathBuf> {
    ["A", "B"].iter().map(|l| std::env::temp_dir().join(format!("ndr-keys-fake-drive-{l}"))).collect()
}
fn fake_drive_info(_: &std::path::Path) -> Option<(String, u32, Option<(u64, u64)>)> {
    Some(("Disk".into(), 3, Some((25, 100))))
}

/// Original FUN_00486474, ArrowKey, Select and SelectAll: with "show disk drives in the list" on, drive items follow the ordinary folder list.
/// They stay at the end regardless of sorting, are not counted as folders or files, and are never selected or job targets. Ctrl+End is the last file before the drives,
/// [ ] move between drives, and opening one goes to that drive. Turning the setting off does not append them.
#[test]
fn drive_items_follow_the_original_list_end() {
    let (root, ctx, mut app) = fixture("drives", "ko");
    for drive in fake_drive_roots() {
        fs::create_dir_all(&drive).unwrap();
    }
    app.system.ready_drives = fake_drive_roots;
    app.system.drive_item = fake_drive_info;
    app.refresh(0);
    wait(&mut app, &ctx);
    app.active = 0;
    let drives: Vec<bool> = app.panes[0].entries.iter().map(|e| e.drive).collect();
    assert_eq!(drives, [false, false, false, false, true, true], "\"..\", folder1, a.txt, b.txt, then two drives");
    let summary = super::main_window::summarize(&app.panes[0].entries, true);
    assert_eq!((summary.folders, summary.files), (1, 2), "drives and \"..\" are not counted");
    assert_eq!(app.panes[0].view.cells[4][0], format!("{} Disk", app.panes[0].entries[4].name));
    press(&mut app, &ctx, egui::Key::End, CTRL);
    assert_eq!(app.panes[0].cursor, 3, "Ctrl+End = last file before the drives");
    press(&mut app, &ctx, egui::Key::OpenBracket, NONE);
    assert_eq!(app.panes[0].cursor, 4, "[ = first drive");
    press(&mut app, &ctx, egui::Key::OpenBracket, NONE);
    assert_eq!(app.panes[0].cursor, 5, "[ = next drive");
    press(&mut app, &ctx, egui::Key::OpenBracket, NONE);
    assert_eq!(app.panes[0].cursor, 4, "[ wraps to the first drive");
    press(&mut app, &ctx, egui::Key::CloseBracket, NONE);
    assert_eq!(app.panes[0].cursor, 5, "] on the first drive wraps to the last item");
    assert!(app.panes[0].targets().is_empty(), "a drive item is never an operation target");
    press(&mut app, &ctx, egui::Key::Space, NONE);
    press(&mut app, &ctx, egui::Key::ArrowUp, SHIFT);
    assert!(app.panes[0].selected.iter().all(|&i| !app.panes[0].entries[i].drive), "drive items are never marked");
    app.panes[0].selected.clear();
    app.cycle_selection(true);
    assert_eq!(app.panes[0].selected.iter().copied().collect::<Vec<_>>(), [1, 2, 3], "select all skips the drives and \"..\"");
    app.settings.set_flag("LeftSortDescending", true);
    app.sort_pane(0);
    let drives: Vec<bool> = app.panes[0].entries.iter().map(|e| e.drive).collect();
    assert_eq!(drives, [false, false, false, false, true, true], "drives stay at the end after sorting");
    app.panes[0].cursor = 4;
    let target = app.panes[0].entries[4].path.clone();
    app.open_current();
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].path, target, "opening a drive item goes to that drive");
    app.settings.set_flag("ShowDriveList", false);
    app.navigate(0, root.join("left"), false);
    wait(&mut app, &ctx);
    assert!(app.panes[0].entries.iter().all(|e| !e.drive), "no drive items when the option is off");
    fs::remove_dir_all(root).unwrap();
}
const ALT: egui::Modifiers = egui::Modifiers::ALT;
const CTRL: egui::Modifiers = egui::Modifiers::CTRL;
const SHIFT: egui::Modifiers = egui::Modifiers::SHIFT;
fn names(app: &App, pane: usize) -> Vec<String> {
    let mut v: Vec<String> = app.panes[pane].selected.iter().map(|&i| app.panes[pane].entries[i].name.clone()).collect();
    v.sort();
    v
}

#[test]
fn keyboard_follows_the_csharp_shortcut_engine() {
    let (root, ctx, mut app) = fixture("ko", "ko");
    let left = root.join("left");
    // Alt+U (original miSelectAll = CycleFileSelection): with nothing, files only (SelectFolders off); with all files marked and no folder, folders too; then clear all.
    press(&mut app, &ctx, egui::Key::U, ALT);
    assert_eq!(names(&app, 0), ["a.txt", "b.txt"]);
    press(&mut app, &ctx, egui::Key::U, ALT);
    assert_eq!(names(&app, 0), ["a.txt", "b.txt", "folder1"]);
    press(&mut app, &ctx, egui::Key::U, ALT);
    assert!(app.panes[0].selected.is_empty());
    // Esc: nothing without EscapeKeyAction, clear all with 3.
    app.panes[0].selected.insert(0);
    press(&mut app, &ctx, egui::Key::Escape, NONE);
    assert_eq!(app.panes[0].selected.len(), 1, "no EscapeKeyAction: Esc does nothing");
    app.settings.values.insert("EscapeKeyAction".into(), serde_json::json!(3));
    press(&mut app, &ctx, egui::Key::Escape, NONE);
    assert!(app.panes[0].selected.is_empty());
    // Work folders: Ctrl+1 saves (original miWorkFolder1) without a notice, Alt+1 goes there, Alt+2 without a saved folder does nothing.
    press(&mut app, &ctx, egui::Key::Num1, CTRL);
    assert!(app.msgboxes.is_empty());
    assert_eq!(app.settings.strings("WorkingFolders")[0], left.to_string_lossy());
    // Original FUN_0042fff58: an already set work folder asks MSG_UNSETWORKFOLDER (current value) yes/no. No keeps it, yes changes it.
    app.navigate(0, left.join("folder1"), true);
    wait(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::Num1, CTRL);
    let question = app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone())).expect("the replace question");
    assert_eq!(question.0, app.language_pack.text("RES.WORKFOLDER", "?"));
    assert_eq!(question.1, app.language_pack.text("RES.MSG_UNSETWORKFOLDER", "?").replace("%s", &left.to_string_lossy()));
    app.answer_message_box(ndr::msgbox::IDNO, &ctx);
    assert_eq!(app.settings.strings("WorkingFolders")[0], left.to_string_lossy());
    press(&mut app, &ctx, egui::Key::Num1, CTRL);
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    assert_eq!(app.settings.strings("WorkingFolders")[0], left.join("folder1").to_string_lossy());
    let mut folders = app.settings.strings("WorkingFolders");
    folders[0] = left.to_string_lossy().into_owned();
    app.settings.set_strings("WorkingFolders", folders);
    app.navigate(0, left.join("folder1"), true);
    wait(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::Num1, ALT);
    assert_eq!(app.panes[0].path, left);
    press(&mut app, &ctx, egui::Key::Num2, ALT);
    // Original miGoToWorkFolder2 with WF2 empty: nothing happens (no box, pane and slot unchanged).
    assert!(app.msgboxes.is_empty());
    assert_eq!(app.panes[0].path, left);
    assert_eq!(app.settings.strings("WorkingFolders")[1], "");
    // Alt+Left = back (original miHistoryBack): the folder before moving to the work folder.
    press(&mut app, &ctx, egui::Key::ArrowLeft, ALT);
    assert_eq!(app.panes[0].path, left.join("folder1"));
    // Last drive folder (SaveDriveFolders): remembered on navigation, restored when going to the drive root.
    let drive_root = PathBuf::from(left.to_string_lossy()[..3].to_owned());
    assert_eq!(app.restore_drive_folder(drive_root.clone()), left.join("folder1"));
    app.settings.set_flag("SaveDriveFolders", false);
    assert_eq!(app.restore_drive_folder(drive_root.clone()), drive_root);
    // Column count: column:N (clamped to 0-19); the original name column3 works the same.
    app.dispatch("column:25", &ctx);
    assert_eq!(app.settings.values["LeftColumnCount"], 19);
    app.dispatch("column3", &ctx);
    assert_eq!(app.settings.values["LeftColumnCount"], 3);
    fs::remove_dir_all(root).unwrap();
}

/// Original favorites menu (FUN_0042eedc): items 0-9 Shift+F1-F10, items 10-19 Shift+Ctrl+F1-F10. Keys of empty slots do nothing.
#[test]
fn favorites_menu_shortcuts_follow_the_original() {
    let (root, ctx, mut app) = fixture("favkeys", "ko");
    let (left, right) = (root.join("left"), root.join("right"));
    let mut paths = vec![String::new(); 20];
    let mut names = vec![String::new(); 20];
    (paths[0], names[0]) = (right.to_string_lossy().into_owned(), "Right".into());
    (paths[10], names[10]) = (left.join("folder1").to_string_lossy().into_owned(), "Folder".into());
    app.settings.set_strings("Favorites", paths);
    app.settings.set_strings("FavoriteNames", names);
    let shift = egui::Modifiers::SHIFT;
    press(&mut app, &ctx, egui::Key::F1, shift);
    assert_eq!(app.panes[0].path, right);
    press(&mut app, &ctx, egui::Key::F1, shift | egui::Modifiers::CTRL);
    assert_eq!(app.panes[0].path, left.join("folder1"));
    press(&mut app, &ctx, egui::Key::F2, shift);
    assert_eq!(app.panes[0].path, left.join("folder1"), "an empty slot has no menu item");
    assert_eq!((super::favorites_dialog::favorite_gesture(&app.settings, 0), super::favorites_dialog::favorite_gesture(&app.settings, 10)), ("Shift+F1".to_owned(), "Shift+Ctrl+F1".to_owned()));
    fs::remove_dir_all(root).unwrap();
}

/// Original [FuncKey] F3=#CopyTo, F4=#MoveTo -> TfSelectDest. Copy and move must finish with keys alone (F3, Enter).
#[test]
fn f3_and_f4_copy_and_move_with_keys_only() {
    let (root, ctx, mut app) = fixture("f3copy", "ko");
    let (left, right) = (root.join("left"), root.join("right"));
    let draw = |app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            app.keyboard(ctx);
            app.popup_menu_ui(ctx);
            app.original_dialog_ui(ctx);
            app.msgbox_ui(ctx);
        });
    };
    let cursor_on = |app: &mut App, name: &str| app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    let key = |key: egui::Key| vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: NONE }, egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: NONE }];
    let settle = |app: &mut App, ctx: &egui::Context| {
        let end = Instant::now() + Duration::from_secs(20);
        while app.job.as_ref().is_some_and(|j| !j.finished) || app.panes.iter().any(|p| p.loading) {
            assert!(Instant::now() < end, "timeout");
            app.poll(ctx);
            thread::sleep(Duration::from_millis(5));
        }
        for _ in 0..3 {
            draw(app, ctx, vec![]);
        }
    };
    draw(&mut app, &ctx, vec![]);
    cursor_on(&mut app, "a.txt");
    draw(&mut app, &ctx, key(egui::Key::F3));
    assert_eq!(app.original_dialogs.len(), 1, "F3 opens the destination window");
    draw(&mut app, &ctx, vec![]);
    draw(&mut app, &ctx, key(egui::Key::Enter));
    settle(&mut app, &ctx);
    assert!(app.original_dialogs.is_empty() && app.msgboxes.is_empty(), "no window left");
    assert_eq!(fs::read(right.join("a.txt")).unwrap(), b"a", "F3 + Enter copies to the other pane");
    cursor_on(&mut app, "b.txt");
    draw(&mut app, &ctx, key(egui::Key::F4));
    draw(&mut app, &ctx, vec![]);
    draw(&mut app, &ctx, key(egui::Key::Enter));
    settle(&mut app, &ctx);
    assert!(right.join("b.txt").is_file() && !left.join("b.txt").exists(), "F4 + Enter moves to the other pane");
    // Current pane: "." in the path field -> the same folder.
    cursor_on(&mut app, "a.txt");
    draw(&mut app, &ctx, key(egui::Key::F3));
    app.edit_dialog_kind(|kind| if let super::original_dialogs::DialogKind::Destination(form) = kind { form.path = ".".into() });
    draw(&mut app, &ctx, vec![]);
    draw(&mut app, &ctx, key(egui::Key::Enter));
    use super::original_dialogs::{DialogKind, DialogResult};
    let wait_for = |app: &mut App, ctx: &egui::Context, what: &dyn Fn(&App) -> bool| {
        let end = Instant::now() + Duration::from_secs(20);
        while !what(app) {
            assert!(Instant::now() < end, "timeout");
            app.poll(ctx);
            thread::sleep(Duration::from_millis(5));
        }
    };
    // Original FUN_004134e0: the destination (itself) exists, so TfFileExists. Rename -> copy under the new name.
    wait_for(&mut app, &ctx, &|app| app.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::FileConflict(_))));
    app.answer_dialog(DialogResult::FileConflict(Some((ndr::fsops::ConflictChoice::Rename, false))), &ctx);
    wait_for(&mut app, &ctx, &|app| app.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::Prompt { .. })));
    app.answer_dialog(DialogResult::Text(Some("a2.txt".into())), &ctx);
    settle(&mut app, &ctx);
    assert_eq!(fs::read(left.join("a2.txt")).unwrap(), b"a", "copied in the current pane under the new name");
    // Overwrite skips without destroying the item itself (fixed).
    cursor_on(&mut app, "a.txt");
    draw(&mut app, &ctx, key(egui::Key::F3));
    app.edit_dialog_kind(|kind| if let DialogKind::Destination(form) = kind { form.path = left.to_string_lossy().into_owned() });
    draw(&mut app, &ctx, vec![]);
    draw(&mut app, &ctx, key(egui::Key::Enter));
    wait_for(&mut app, &ctx, &|app| app.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::FileConflict(_))));
    app.answer_dialog(DialogResult::FileConflict(Some((ndr::fsops::ConflictChoice::Overwrite, false))), &ctx);
    settle(&mut app, &ctx);
    assert_eq!(fs::read(left.join("a.txt")).unwrap(), b"a");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn work_folder_messages_in_english() {
    let (root, ctx, mut app) = fixture("en", "en");
    // The original menu names saveWorkFolder2/openWorkFolder2 work like workSet/workOpen; saving shows no notice.
    app.dispatch("saveWorkFolder2", &ctx);
    assert_eq!(app.settings.strings("WorkingFolders")[1], root.join("left").to_string_lossy());
    app.navigate(0, root.join("right"), true);
    wait(&mut app, &ctx);
    app.dispatch("openWorkFolder2", &ctx);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].path, root.join("left"));
    // A saved folder that no longer exists is reported in English.
    let mut folders = app.settings.strings("WorkingFolders");
    folders[4] = root.join("missing").to_string_lossy().into_owned();
    app.settings.set_strings("WorkingFolders", folders);
    app.dispatch("workOpen:4", &ctx);
    assert_eq!(app.msgboxes.first().map(|m| m.text.clone()), Some("Work folder 5 is not set.".to_owned()));
    fs::remove_dir_all(root).unwrap();
}


/// C# HandleKeyboardAsync does not filter IsRepeat: repeat input of a held key reruns the command.
#[test]
fn held_keys_repeat_commands_like_csharp() {
    let (root, ctx, mut app) = fixture("repeat", "en");
    press(&mut app, &ctx, egui::Key::U, ALT);
    assert_eq!(names(&app, 0), ["a.txt", "b.txt"]);
    // the key is still down: a repeat event cycles the selection once more
    let repeat = egui::Event::Key { key: egui::Key::U, physical_key: None, pressed: true, repeat: true, modifiers: ALT };
    let raw = egui::RawInput { events: vec![repeat], modifiers: ALT, ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
    assert_eq!(names(&app, 0), ["a.txt", "b.txt", "folder1"]);
    let _ = fs::remove_dir_all(&root);
}

/// Whether every target of the original shortcut table (the default 114 lines of shortcut.ini) leads to an NDR action: menu items via ORIGINAL_ACTIONS (or C# special items),
/// internal commands via the parity::internal_command table.
#[test]
fn every_original_shortcut_target_has_an_ndr_action() {
    let shortcuts = compat::ShortcutMap::parse(None);
    // Only genuinely special dispatch behavior belongs here. The hidden
    // Search / Move Archive / Install Archive commands now dispatch normally.
    let special = ["miSelectAll", "miFind", "miSelect"];
    let missing_menus: Vec<(&String, String)> = shortcuts
        .menu_shortcuts
        .iter()
        .filter(|(n, code)| **code != 0 && !special.contains(&n.as_str()) && !menu_data::ORIGINAL_ACTIONS.iter().any(|(name, _)| name == *n))
        .map(|(n, code)| (n, compat::shortcut_to_text(*code)))
        .collect();
    assert!(missing_menus.is_empty(), "menu items without an action: {missing_menus:?}");
    let missing_internal: Vec<String> = shortcuts
        .lines
        .iter()
        .filter_map(|l| l.split_once('=').map(|(n, _)| n.trim().to_owned()))
        .filter(|n| !shortcuts.menu_shortcuts.keys().any(|k| k.eq_ignore_ascii_case(n)))
        .filter(|n| matches!(ndr::parity::internal_command(n), ndr::parity::InternalCommand::Unknown))
        .collect();
    assert!(missing_internal.is_empty(), "internal commands without an action: {missing_internal:?}");
}

/// Original miFind (Ctrl+F): with the original toolbar (no quick search field) the find box (TfGetText TITLE_FIND). miColAuto (Ctrl+Alt+0): automatic column count (0).
#[test]
fn ctrl_f_and_ctrl_alt_0_follow_the_original_menu() {
    let (root, ctx, mut app) = fixture("find-keys", "ko");
    assert!(!app.toolbar.items.iter().any(|i| i.kind == ndr::commands::ToolbarKind::QuickSearch), "the original toolbar has no quick search box");
    press(&mut app, &ctx, egui::Key::F, CTRL);
    assert_eq!(app.original_dialogs.last().map(|d| d.title.clone()), Some(app.language_pack.text("RES.TITLE_FIND", "?")));
    app.answer_dialog(original_dialogs::DialogResult::Text(None), &ctx);
    app.settings.values.insert("LeftColumnCount".into(), serde_json::json!(3));
    app.active = 0;
    press(&mut app, &ctx, egui::Key::Num0, CTRL | ALT);
    assert_eq!(app.settings.values["LeftColumnCount"], 0);
    fs::remove_dir_all(root).unwrap();
}


/// A newer Windows clipboard (including text from another program) must not
/// fall back to paths that NDR copied earlier.
#[test]
fn paste_ignores_stale_local_clipboard_when_windows_sequence_has_changed() {
    let (root, ctx, mut app) = fixture("stale-file-clipboard", "ko");
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    app.dispatch("copy", &ctx);
    assert!(app.clipboard.is_some());
    app.clipboard_sequence = Some(native::clipboard_sequence_number().wrapping_add(1));
    app.active = 1;
    app.dispatch("paste", &ctx);
    assert!(!root.join("right/a.txt").exists(), "stale local clipboard paths must not be pasted");
    assert!(app.clipboard.is_none() && app.clipboard_sequence.is_none());
    fs::remove_dir_all(root).unwrap();
}

/// Hidden menu entries previously swallowed their shortcuts without reaching
/// their real handlers. Exercise their actual dialogs / OS verb routing.
#[test]
fn previously_swallowed_search_and_archive_move_shortcuts_open_their_dialogs() {
    let (root, ctx, mut app) = fixture("hidden-menu-shortcuts", "ko");
    app.active = 0;
    press(&mut app, &ctx, egui::Key::H, CTRL);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(original_dialogs::DialogKind::Search(_))),
        "Ctrl+H must open file search, not silently swallow the key");
    app.answer_dialog(original_dialogs::DialogResult::Search(None), &ctx);
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    press(&mut app, &ctx, egui::Key::A, CTRL | egui::Modifiers::SHIFT);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(original_dialogs::DialogKind::ArchiveCreate(_))),
        "Ctrl+Shift+A must open Move to Archive settings");
    app.answer_dialog(original_dialogs::DialogResult::ArchiveCreate(None), &ctx);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn custom_archive_install_shortcut_calls_install_shell_verb() {
    static CAPTURED: std::sync::Mutex<Vec<(PathBuf, String)>> = std::sync::Mutex::new(Vec::new());
    fn capture(path: &Path, verb: &str) -> Result<(), String> {
        CAPTURED.lock().unwrap().push((path.to_path_buf(), verb.to_owned()));
        Ok(())
    }
    let (root, ctx, mut app) = fixture("install-shortcut", "ko");
    let zip = root.join("left/selected.zip");
    ndr::archive::create(&[root.join("left/a.txt")], &zip, None, &AtomicBool::new(false)).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "selected.zip").unwrap();
    app.system.run_verb = capture;
    CAPTURED.lock().unwrap().clear();
    let mut keys = compat::ShortcutMap::parse(None);
    keys.set_binding("miArcInstall", "Ctrl+Shift+I").unwrap();
    app.settings.set_text("OriginalShortcutMapText", &keys.to_text());
    press(&mut app, &ctx, egui::Key::I, CTRL | egui::Modifiers::SHIFT);
    assert_eq!(CAPTURED.lock().unwrap().as_slice(), &[(zip.clone(), "install".to_owned())]);
    fs::remove_dir_all(root).unwrap();
}

/// Menus have original items only: commands without an original name (MENU_CAPTIONS, ORIGINAL_ACTIONS), which existed only in the C# version, are not in the menu table.
#[test]
fn menus_have_only_original_commands() {
    let foreign: Vec<&str> = menu_data::MENUS.iter().flat_map(|(_, _, _, items)| items.iter().map(|(id, _, _)| *id)).filter(|id| *id != "-" && App::csharp_only_menu_item(id)).collect();
    assert!(foreign.is_empty(), "{foreign:?}");
    // ORIGINAL_HIDDEN = every item with Visible=False in the original DFM that is in the NDR menu, none missing.
    let dfm = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/original-menus.txt")).unwrap();
    let invisible: Vec<&str> = dfm.lines().filter(|l| l.contains("Visible=False")).filter_map(|l| l.trim().strip_prefix("TMenuItem ")?.split(' ').next()).collect();
    let menu_ids: Vec<&str> = menu_data::MENUS.iter().flat_map(|(_, _, _, items)| items.iter().map(|(id, _, _)| *id)).collect();
    let mut from_dfm: Vec<&str> = menu_ids.iter().copied().filter(|id| App::original_menu_name(id).is_some_and(|name| invisible.contains(&name))).collect();
    from_dfm.sort_unstable();
    from_dfm.dedup();
    let mut listed = menu_data::ORIGINAL_HIDDEN.to_vec();
    listed.sort_unstable();
    assert_eq!(from_dfm, listed);
}

/// Original miSkin (FUN_0043e580: one item per skin\ folder) and miLanguage (FUN_00441830, miMoreLanguages at the end).
#[test]
fn original_skin_and_language_submenus() {
    let (root, ctx, mut app) = fixture("skin-language-menu", "ko");
    assert_eq!(app.skin_names(), ["Black", "Classic", "Gray"]);
    app.apply_theme("Gray");
    assert_eq!(app.settings.text("Theme", "?"), "Gray");
    assert!(app.current_skin().is_some());
    app.apply_language("en");
    assert_eq!((app.language.as_str(), app.settings.text("Language", "?").as_str()), ("en", "en"));
    thread_local!(static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) });
    fn runner(path: &Path, _: &str, _: &Path) -> Result<(), String> {
        OPENED.with(|o| o.borrow_mut().push(path.to_string_lossy().into_owned()));
        Ok(())
    }
    app.shell_runner = runner;
    app.dispatch("moreLanguages", &ctx);
    assert_eq!(OPENED.with(|o| o.borrow().clone()), ["https://www.xiles.app/languages/#NexusFile"]);
    fs::remove_dir_all(root).unwrap();
}

/// Original ApplicationEvents1Message (0x442984): mouse XBUTTON1 (back) and XBUTTON2 (forward) = HistoryBack and HistoryForward.
#[test]
fn mouse_side_buttons_go_back_and_forward() {
    let (root, ctx, mut app) = fixture("xbuttons", "en");
    app.active = 0;
    let start = app.panes[0].path.clone();
    let folder = start.join("folder1");
    app.navigate(0, folder.clone(), true);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].path, folder);
    let click = |app: &mut App, button: egui::PointerButton| {
        let pos = egui::pos2(5.0, 5.0);
        let events = vec![
            egui::Event::PointerButton { pos, button, pressed: true, modifiers: NONE },
            egui::Event::PointerButton { pos, button, pressed: false, modifiers: NONE },
        ];
        let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| app.keyboard(ctx));
        wait(app, &ctx);
    };
    click(&mut app, egui::PointerButton::Extra1);
    assert_eq!(app.panes[0].path, start, "XBUTTON1 = back");
    click(&mut app, egui::PointerButton::Extra2);
    assert_eq!(app.panes[0].path, folder, "XBUTTON2 = forward");
    fs::remove_dir_all(root).unwrap();
}
