//! Checks C# NDR behavior (r66 copy path, r69 find, r70 original message boxes) in the Rust UI. Only Korean and English are checked.
//! The real clipboard is not touched (clipboard_writer is replaced).
use super::*;
use std::{cell::RefCell, fs, rc::Rc};

thread_local! {
    static CLIPBOARD: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}
fn capture_clipboard(text: &str) -> Result<(), String> {
    CLIPBOARD.with(|c| c.borrow_mut().push(text.to_owned()));
    Ok(())
}
fn clipboard() -> Vec<String> {
    CLIPBOARD.with(|c| c.borrow().clone())
}

fn fixture(label: &str, language: &str) -> (PathBuf, egui::Context, App) {
    let root = std::env::temp_dir().join(format!(
        "ndr-parity-{}-{}-{label}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let left = root.join("left");
    let right = root.join("right");
    fs::create_dir_all(left.join("folder1")).unwrap();
    fs::create_dir(&right).unwrap();
    for name in ["alpha.txt", "Alphabet.doc", "beta.txt", "gamma.log"] {
        fs::write(left.join(name), name).unwrap();
    }
    let ctx = egui::Context::default();
    let options = Options { profile: Some(root.join("profile")), left: Some(left), right: Some(right), language: Some(language.into()), ..Default::default() };
    let mut app = App::from_context(&ctx, options);
    app.clipboard_writer = capture_clipboard;
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
fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        events,
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))),
        ..Default::default()
    };
    let _ = ctx.run(raw, |ctx| app.msgbox_ui(ctx));
}
fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}
fn ko_raw(app: &App, key: &str) -> String {
    app.language_pack.raw(&format!("RES.{key}")).unwrap()
}
fn result_cell() -> Rc<std::cell::Cell<i32>> {
    Rc::new(std::cell::Cell::new(-1))
}
fn show(app: &mut App, title: &str, text: &str, flags: u32) -> Rc<std::cell::Cell<i32>> {
    let cell = result_cell();
    let sink = cell.clone();
    app.message_box(title, "", text, flags, move |_, result, _| sink.set(result));
    cell
}
fn cursor_name(app: &App) -> String {
    let pane = &app.panes[0];
    pane.entries[pane.cursor].name.clone()
}
fn index_of(app: &App, name: &str) -> usize {
    app.panes[0].entries.iter().position(|e| e.name == name).unwrap()
}

#[test]
fn msgbox_layout_focus_and_keyboard() {
    let (root, ctx, mut app) = fixture("msgbox", "ko");
    let ok = show(&mut app, "Title", "A message that is long enough for every button row.", ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONASTERISK);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let layout = app.msgboxes[0].layout.clone().unwrap();
    assert_eq!([layout.slots[0].is_some(), layout.slots[1].is_some(), layout.slots[2].is_some()], [false, false, true]);
    assert_eq!(layout.captions[2], ko_raw(&app, "OK"), "caption from [RES] with its accelerator");
    assert!(layout.title_shown);
    assert_eq!((layout.text_top, layout.image_top), (41.0, 40.0));
    let line_height = ctx.fonts_mut(|f| f.row_height(&egui::FontId::proportional(12.0)));
    assert!((layout.client.y - (119.0 - 20.0 + line_height)).abs() < 0.01, "client height from the DFM");
    let button = layout.slots[2].unwrap();
    assert!((button.min.x - layout.origin.x - (layout.client.x - 93.0)).abs() < 0.01, "btn1 anchored 15 from the right");
    assert!((button.min.y - layout.origin.y - (layout.client.y - 33.0)).abs() < 0.01, "btn1 anchored 8 from the bottom");
    assert!(ctx.memory(|m| m.has_focus(layout.slot_ids[2].unwrap())), "first visible button has the focus");
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert_eq!(ok.get(), ndr::msgbox::IDOK, "Esc presses the Cancel button (OK on an OK-only box)");
    assert!(app.msgboxes.is_empty());

    let yes_no = show(&mut app, "T", "Pressing N picks the No button directly.", ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONQUESTION);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::N)]);
    assert_eq!(yes_no.get(), ndr::msgbox::IDNO, "accelerator without Alt");

    let enter = show(&mut app, "T", "Enter presses the focused Yes button.", ndr::msgbox::MB_YESNO);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(enter.get(), ndr::msgbox::IDYES);

    let ari = show(&mut app, "T", "Abort, retry or ignore without a cancel button.", ndr::msgbox::MB_ABORTRETRYIGNORE);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert_eq!(ari.get(), -1, "Esc without a Cancel button does nothing");
    app.answer_message_box(5, &ctx);
    assert_eq!(ari.get(), ndr::msgbox::IDIGNORE);

    let blank = show(&mut app, "T", "Type 7 keeps the DFM buttons.", 0x07);
    frame(&mut app, &ctx, vec![]);
    let layout = app.msgboxes[0].layout.clone().unwrap();
    assert!(layout.slots.iter().all(Option::is_some) && layout.captions.iter().all(String::is_empty));
    app.answer_message_box(0, &ctx);
    assert_eq!(blank.get(), -1, "ModalResult 0 does not close");
    app.answer_message_box(2, &ctx);
    assert_eq!(blank.get(), ndr::msgbox::IDCANCEL, "the close box returns IDCANCEL");

    let two = show(&mut app, "", "Two\nlines", ndr::msgbox::MB_YESNOCANCEL | ndr::msgbox::MB_ICONQUESTION);
    frame(&mut app, &ctx, vec![]);
    let layout = app.msgboxes[0].layout.clone().unwrap();
    assert_eq!(app.msgboxes[0].title, ndr::msgbox::DEFAULT_TITLE, "empty title becomes the product name");
    assert_eq!(layout.line_count, 2);
    assert!((layout.client.y - (119.0 - 20.0 + 2.0 * line_height + 4.0)).abs() < 0.01, "LineSpace 4");
    assert_eq!(layout.client.x, ndr::msgbox::minimum_client_width(3), "short text: widened for three buttons");
    assert_eq!(layout.captions[0], ko_raw(&app, "YES"));
    app.answer_message_box(6, &ctx);
    assert_eq!(two.get(), ndr::msgbox::IDYES);
    fs::remove_dir_all(root).unwrap();
}

/// Original miCopyToClick -> FUN_0042bdc4 -> TfSelectDest (FUN_0043cb24): the path field starts with the other pane folder and can be edited to copy or move
/// into a folder in the same pane (relative paths too) (skin theme).
#[test]
fn original_copy_menu_lets_the_destination_be_typed() {
    let (root, ctx, mut app) = fixture("seldest", "ko");
    assert!(app.current_skin().is_some());
    app.panes[0].cursor = index_of(&app, "alpha.txt");
    app.dispatch("copyToOther", &ctx);
    let path = match &app.original_dialogs.last().unwrap().kind {
        original_dialogs::DialogKind::Destination(form) => form.path.clone(),
        _ => panic!("the destination dialog"),
    };
    assert_eq!(PathBuf::from(&path), std::path::absolute(root.join("right")).unwrap(), "the other pane by default");
    // Into a subfolder of the same pane (relative path).
    if let original_dialogs::DialogKind::Destination(form) = &mut app.original_dialogs.last_mut().unwrap().kind {
        form.path = "folder1".into();
    }
    app.destination_action(super::destination_dialog::DestinationAction::Accept, &ctx);
    assert!(app.original_dialogs.is_empty());
    wait_job(&mut app, &ctx);
    assert_eq!(fs::read(root.join("left/folder1/alpha.txt")).unwrap(), b"alpha.txt", "copied inside the same pane");
    // A missing folder does not close the window.
    app.panes[0].cursor = index_of(&app, "beta.txt");
    app.dispatch("moveToOther", &ctx);
    if let original_dialogs::DialogKind::Destination(form) = &mut app.original_dialogs.last_mut().unwrap().kind {
        form.path = "no such folder".into();
    }
    app.destination_action(super::destination_dialog::DestinationAction::Accept, &ctx);
    assert_eq!(app.original_dialogs.len(), 1, "a missing folder keeps the dialog");
    app.destination_action(super::destination_dialog::DestinationAction::Cancel, &ctx);
    assert!(root.join("left/beta.txt").exists());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn copy_to_other_asks_the_original_direction_box() {
    let (root, ctx, mut app) = fixture("direction", "ko");
    app.panes[0].cursor = index_of(&app, "alpha.txt");
    app.copy_to_other(0, false, &ctx);
    assert_eq!(app.msgboxes.len(), 1);
    let expected = app.language_pack.text("RES.MSG_TORIGHT", "?").replace("%s", &app.language_pack.text("RES.COPY", "?"));
    assert_eq!(app.msgboxes[0].text, expected);
    assert_eq!(app.msgboxes[0].title, app.language_pack.text("RES.TITLE_FILEOPERATION", "?"));
    assert_eq!(app.msgboxes[0].flags, ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONQUESTION);
    app.answer_message_box(7, &ctx);
    assert!(app.job.is_none(), "No declines");
    app.copy_to_other(0, false, &ctx);
    app.answer_message_box(6, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(fs::read(root.join("right/alpha.txt")).unwrap(), b"alpha.txt");
    assert_eq!(cursor_name(&app), "alpha.txt", "the source pane keeps its cursor after the copy");
    // Internal #MoveToLeft uses the right pane as the source whatever pane is active.
    app.panes[1].cursor = app.panes[1].entries.iter().position(|e| e.name == "alpha.txt").unwrap();
    assert!(app.original_internal("MoveToLeft", &ctx));
    let expected = app.language_pack.text("RES.MSG_TOLEFT", "?").replace("%s", &app.language_pack.text("RES.MOVE", "?"));
    assert_eq!(app.msgboxes[0].text, expected);
    app.answer_message_box(2, &ctx);
    assert!(app.job.as_ref().is_none_or(|j| j.finished), "close box declines");
    app.settings.set_flag("ConfirmDragDrop", false);
    // Rereading keeps the cursor on alpha.txt (original refresh), so choose gamma.log, which is not on the right.
    app.panes[0].cursor = index_of(&app, "gamma.log");
    app.copy_to_other(0, false, &ctx);
    assert!(app.msgboxes.is_empty(), "no box when the confirmation is off");
    wait_job(&mut app, &ctx);
    assert!(root.join("right/gamma.log").exists());
    // Original FUN_0042d518/FUN_0042d910 (vertical bar and shortcut, skin theme): asks regardless of confirmdrop; Shift on the button = move.
    assert!(app.current_skin().is_some());
    app.panes[0].cursor = index_of(&app, "beta.txt");
    app.original_copy_to_other(0, true, &ctx);
    let expected = app.language_pack.text("RES.MSG_TORIGHT", "?").replace("%s", &app.language_pack.text("RES.MOVE", "?"));
    assert_eq!(app.msgboxes.first().map(|b| b.text.clone()), Some(expected), "always asked");
    app.answer_message_box(7, &ctx);
    // With a single pane nothing happens.
    let dual = app.dual;
    app.dual = false;
    app.original_copy_to_other(0, false, &ctx);
    assert!(app.msgboxes.is_empty() && app.job.as_ref().is_none_or(|j| j.finished));
    app.dual = dual;
    app.settings.set_flag("ConfirmDragDrop", true);
    // Original internal command 0x25 FocusAddressBar (FUN_0048d3c8) = C# BeginAddressEdit: edit the active pane's address.
    app.active = 1;
    assert!(app.original_internal("FocusAddressBar", &ctx));
    assert!(app.panes[1].address_editing && !app.panes[0].address_editing);
    assert!(app.original_dialogs.is_empty(), "not the command line prompt");
    app.end_address_edit(1);
    // Original internal command 0x2a Select: flips only the cursor item's selection (cursor stays; not the advanced select window).
    assert!(!app.panes[1].entries.is_empty());
    app.panes[1].selected.clear();
    let cursor = app.panes[1].cursor;
    assert!(app.original_internal("Select", &ctx));
    assert!(app.panes[1].selected.contains(&cursor) && app.panes[1].cursor == cursor && app.original_dialogs.is_empty());
    assert!(app.original_internal("Select", &ctx));
    assert!(app.panes[1].selected.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn clear_descriptions_and_email_use_the_original_box() {
    let (root, ctx, mut app) = fixture("descriptions", "en");
    app.dispatch("clearDescriptions", &ctx);
    assert!(app.msgboxes.is_empty(), "no box without descript.dat");
    let data = root.join("left/descript.dat");
    fs::write(&data, b"alpha.txt owned comment\r\n").unwrap();
    app.dispatch("clearDescriptions", &ctx);
    assert_eq!(app.msgboxes[0].title, app.language_pack.text("RES.TITLE_DELDESCFILE", "?"));
    assert_eq!(app.msgboxes[0].text, app.language_pack.text("RES.MSG_DELDESCFILE", "?"));
    app.answer_message_box(7, &ctx);
    assert!(data.exists(), "No keeps descript.dat");
    app.dispatch("clearDescriptions", &ctx);
    app.answer_message_box(6, &ctx);
    assert!(!data.exists(), "Yes deletes descript.dat");

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn copy_paths_follow_copypathtoclipboard() {
    let (root, ctx, mut app) = fixture("copypath", "ko");
    let left = root.join("left");
    app.panes[0].selected = [index_of(&app, "gamma.log"), index_of(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("copyFullPath", &ctx);
    app.dispatch("copyName", &ctx);
    app.dispatch("copyPathOnly", &ctx);
    app.copy_item_paths(ndr::parity::PathPart::Full, true);
    app.panes[0].selected.clear();
    app.panes[0].cursor = index_of(&app, "beta.txt");
    app.dispatch("copyName", &ctx);
    let (a, g) = (left.join("alpha.txt").to_string_lossy().into_owned(), left.join("gamma.log").to_string_lossy().into_owned());
    let (first, second) = if index_of(&app, "alpha.txt") < index_of(&app, "gamma.log") { (a.clone(), g.clone()) } else { (g.clone(), a.clone()) };
    let folder = left.to_string_lossy().into_owned();
    let copied = clipboard();
    assert_eq!(copied[0], format!("{first}\r\n{second}"), "marked items in list order, CRLF, no trailing break");
    assert!(copied[1] == "alpha.txt\r\ngamma.log" || copied[1] == "gamma.log\r\nalpha.txt");
    assert_eq!(copied[2], format!("{folder}\r\n{folder}"));
    assert_eq!(copied[3], format!("{first}\r\n{second}").replace('\\', "\\\\"), "Shift doubles backslashes");
    assert_eq!(copied[4], "beta.txt", "without marks the focused item");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn find_and_find_next_follow_the_original() {
    let (root, ctx, mut app) = fixture("find", "ko");
    app.dispatch("find", &ctx);
    let dialog = app.original_dialogs.last().expect("TfGetText");
    assert_eq!(dialog.title, app.language_pack.text("RES.TITLE_FIND", "?"));
    assert!(matches!(&dialog.kind, original_dialogs::DialogKind::Prompt { message, value, .. } if value.is_empty() && *message == app.language_pack.text("RES.MSG_FIND", "?")), "TfGetText(TITLE_FIND, MSG_FIND, empty)");
    app.answer_dialog(original_dialogs::DialogResult::Text(None), &ctx);
    app.panes[0].cursor = 0;
    app.panes[0].selected = [index_of(&app, "beta.txt")].into_iter().collect();
    app.typed_search[0].query = "x".into();
    app.original_find("PHA");
    assert!(app.typed_search[0].query.is_empty(), "Find clears the typed text");
    let first = cursor_name(&app);
    assert!(first == "alpha.txt" || first == "Alphabet.doc");
    assert_eq!(app.last_find, "PHA");
    assert_eq!(app.panes[0].selected.len(), 1, "marks unchanged");
    app.find_next();
    let second = cursor_name(&app);
    assert_ne!(first, second);
    app.find_next();
    assert_eq!(cursor_name(&app), first, "wraps");
    app.original_find("zzz");
    assert_eq!((cursor_name(&app), app.last_find.as_str()), (first.clone(), "PHA"), "no match keeps the cursor and last text");
    app.original_find("");
    assert_eq!(app.last_find, "PHA", "empty text finds nothing (StrStrIW)");
    app.typed_search[0].query = "G".into();
    app.find_next();
    assert_eq!(cursor_name(&app), "gamma.log", "typed text is a prefix search");
    assert_eq!(app.last_find, "G", "FUN_00437828 stores the text of every hit, prefix hits included");
    app.typed_search[0].query = "eta".into();
    app.panes[0].cursor = 0;
    app.find_next();
    assert_eq!(app.panes[0].cursor, 0, "prefix only");
    app.typed_search[0].query.clear();
    app.original_find("bet");
    app.panes[0].cursor = index_of(&app, "folder1");
    app.find_next();
    assert_eq!(cursor_name(&app), "Alphabet.doc", "without typed text the last find text (substring)");
    fs::remove_dir_all(root).unwrap();
}

fn keyboard_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
}

/// Original MouseDown (0x7d63c8) (skin theme): a plain click or the right button keeps the marks and moves only the cursor, Shift adds cursor..clicked row,
/// Ctrl flips (also the previous cursor row when nothing is selected). In the C# theme a plain click clears the marks.
#[test]
fn original_row_press_keeps_marks() {
    let (root, _ctx, mut app) = fixture("rowpress", "ko");
    let marks = |pane: &Pane| {
        let mut list: Vec<usize> = pane.selected.iter().copied().collect();
        list.sort();
        list
    };
    let pane = &mut app.panes[0];
    assert!(pane.entries.len() >= 4);
    pane.selected = [1].into_iter().collect();
    pane.cursor = 0;
    super::panes::row_pressed(pane, 2, egui::Modifiers::NONE, false, true);
    assert_eq!((pane.cursor, marks(pane)), (2, vec![1]), "a plain click keeps the marks");
    super::panes::row_pressed(pane, 3, egui::Modifiers::SHIFT, false, true);
    assert_eq!(marks(pane), vec![1, 2, 3], "Shift adds cursor..clicked");
    pane.selected.clear();
    super::panes::row_pressed(pane, 1, egui::Modifiers::CTRL, false, true);
    assert_eq!((pane.cursor, marks(pane)), (1, vec![1, 3]), "Ctrl with no marks also marks the old cursor item");
    super::panes::row_pressed(pane, 1, egui::Modifiers::CTRL, false, true);
    assert_eq!(marks(pane), vec![3]);
    super::panes::row_pressed(pane, 0, egui::Modifiers::NONE, true, true);
    assert_eq!((pane.cursor, marks(pane)), (0, vec![3]), "right button: cursor only");
    super::panes::row_pressed(pane, 2, egui::Modifiers::NONE, false, false);
    assert!(pane.selected.is_empty(), "C# theme: a plain click clears the marks");
    fs::remove_dir_all(root).unwrap();
}

/// Fixed: list clicks read raw input, so with another layer on top (a dialog etc.) the click is ignored (when the archive window OK button lay
/// over a row of the other pane, that pane used to become current).
#[test]
fn list_presses_under_a_dialog_are_ignored() {
    let (root, ctx, mut app) = fixture("underdialog", "ko");
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 20.0), egui::vec2(200.0, 20.0));
    let at = egui::pos2(100.0, 25.0);
    let run = |pane: &mut Pane, events: Vec<egui::Event>, covered: bool| {
        let mut hit = false;
        let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| hit = super::panes::original_press_input(pane, 1, rect, ui));
            if covered {
                egui::Area::new(egui::Id::new("cover")).order(egui::Order::Foreground).fixed_pos(egui::pos2(50.0, 10.0)).show(ctx, |ui| {
                    ui.allocate_exact_size(egui::vec2(120.0, 40.0), egui::Sense::click());
                });
            }
        });
        hit
    };
    let press = || vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }];
    let release = || vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }];
    let pane = &mut app.panes[0];
    pane.cursor = 0;
    run(pane, vec![], true);
    assert!(!run(pane, press(), true), "the press belongs to the layer on top");
    assert_eq!(pane.cursor, 0);
    run(pane, release(), true);
    run(pane, vec![], false);
    // Outside the Windows double-click box of the previous press (another spot on the same row).
    let other = egui::pos2(190.0, 25.0);
    let press_other = vec![egui::Event::PointerMoved(other), egui::Event::PointerButton { pos: other, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }];
    assert!(run(pane, press_other, false), "without a layer on top the row takes the press");
    assert_eq!(pane.cursor, 1);
    fs::remove_dir_all(root).unwrap();
}

/// Original GetSelAreaAt (0x7dae7c), MouseDown and MouseMove (0x7d65f0): clicking the left 12 px of a row flips that row,
/// dragging while pressed selects from the first row to the row under the pointer, and from the second row on other marks are cleared unless Ctrl is held.
#[test]
fn original_selection_area_toggles_and_drags() {
    let (root, ctx, mut app) = fixture("selarea", "ko");
    let rect = |row: usize| egui::Rect::from_min_size(egui::pos2(0.0, row as f32 * 20.0), egui::vec2(200.0, 20.0));
    let run = |pane: &mut Pane, events: Vec<egui::Event>, row: usize| {
        let mut hit = false;
        let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| hit = super::panes::original_press_input(pane, row, rect(row), ui));
        });
        hit
    };
    let marks = |pane: &Pane| {
        let mut list: Vec<usize> = pane.selected.iter().copied().collect();
        list.sort();
        list
    };
    let pane = &mut app.panes[0];
    assert!(pane.entries.len() >= 4);
    pane.selected = [0].into_iter().collect();
    let at = egui::pos2(5.0, 25.0);
    let press = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }];
    assert!(run(pane, press, 1), "a press in the left 12 pixels is the selection area");
    assert_eq!((pane.cursor, marks(pane)), (1, vec![0, 1]));
    assert!(!run(pane, vec![egui::Event::PointerMoved(egui::pos2(60.0, 45.0))], 2));
    assert_eq!(marks(pane), vec![0, 1, 2], "first move: anchor..row added");
    run(pane, vec![egui::Event::PointerMoved(egui::pos2(60.0, 65.0))], 3);
    assert_eq!(marks(pane), vec![1, 2, 3], "later moves without Ctrl clear the other marks");
    assert_eq!(pane.cursor, 1, "dragging does not move the cursor");
    // Releasing the button (MouseUp) ends the drag. A plain click outside the mark cell moves only the cursor.
    let _ = ctx.run(egui::RawInput { events: vec![egui::Event::PointerButton { pos: egui::pos2(60.0, 65.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }], ..Default::default() }, |_| {});
    pane.range_drag = None;
    let at = egui::pos2(100.0, 5.0);
    let plain = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }];
    assert!(run(pane, plain, 0));
    assert_eq!((pane.cursor, marks(pane), pane.range_drag), (0, vec![1, 2, 3], None));
    let _ = ctx.run(egui::RawInput { events: vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }], ..Default::default() }, |_| {});
    // Shift+click adds cursor..clicked row, and dragging while pressed takes the range again.
    let at = egui::pos2(100.0, 45.0);
    let shift = egui::Modifiers::SHIFT;
    let press = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: shift }];
    pane.selected = [3].into_iter().collect();
    let _ = ctx.run(egui::RawInput { events: press, modifiers: shift, ..Default::default() }, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| super::panes::original_press_input(pane, 2, rect(2), ui));
    });
    assert_eq!((pane.cursor, marks(pane), pane.range_drag), (2, vec![0, 1, 2, 3], Some((0, 2))));
    let _ = ctx.run(egui::RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(100.0, 25.0))], modifiers: shift, ..Default::default() }, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| super::panes::original_press_input(pane, 1, rect(1), ui));
    });
    assert_eq!(marks(pane), vec![0, 1], "the drag re-takes the range from the old cursor and clears the rest");
    fs::remove_dir_all(root).unwrap();
}

/// Original TfSelectDest lbListKeyDown (0x454130): 1-9 / Alt+1-9 = work folders, Ctrl+1-9 = most used folders, 0 = initial folder, Shift+Delete = clear history.
#[test]
fn destination_list_keys_follow_the_original() {
    use super::destination_dialog::{destination_ui_original, DestinationAction, DestinationForm};
    let (root, ctx, _app) = fixture("destkeys", "ko");
    let initial = std::env::temp_dir();
    let mut form = DestinationForm {
        most_used: vec![("C:\\a".into(), 3), ("C:\\b".into(), 1)],
        work_folders: ["W1".into(), "W2".into(), String::new(), String::new(), String::new()],
        browse_label: "Browse".into(),
        cancel_label: "Cancel".into(),
        remove_label: "Remove".into(),
        clear_label: "Clear".into(),
        initial: initial.clone(),
        path: initial.to_string_lossy().into_owned(),
        ok_label: "OK".into(),
        original: true,
        selected: 0,
        drives: Vec::new(),
        free_label: String::new(),
        special_folders: Default::default(),
    };
    let mut run = |key: egui::Key, modifiers: egui::Modifiers| {
        // Key focus in the list (not the path field).
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("ndr-test-elsewhere")));
        let raw = egui::RawInput { modifiers, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() };
        let mut got = None;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| got = destination_ui_original(ui, &mut form, false, None, &[], 850.0));
        });
        got
    };
    assert!(matches!(run(egui::Key::Num2, egui::Modifiers::CTRL), Some(DestinationAction::MostUsed(p)) if p == "C:\\b"));
    assert!(matches!(run(egui::Key::Num2, egui::Modifiers::ALT), Some(DestinationAction::Work(1))));
    assert!(matches!(run(egui::Key::Num1, egui::Modifiers::NONE), Some(DestinationAction::Work(0))));
    assert!(run(egui::Key::Num3, egui::Modifiers::NONE).is_none(), "an empty work folder slot does nothing");
    assert!(run(egui::Key::Num3, egui::Modifiers::CTRL).is_none(), "no third most used folder");
    assert!(matches!(run(egui::Key::Num0, egui::Modifiers::NONE), Some(DestinationAction::MostUsed(p)) if p == initial.to_string_lossy()));
    assert!(matches!(run(egui::Key::Delete, egui::Modifiers::SHIFT), Some(DestinationAction::Clear)));
    fs::remove_dir_all(root).unwrap();
}

/// Original TfSelectDest (skin theme): list = initial folder, recent folders other than it, existing work folders. A click selects and puts the path in the path field, Up/Down select,
/// Delete removes the selected recent row, a double click confirms.
#[test]
fn original_destination_list_follows_tfselectdest() {
    use super::destination_dialog::{destination_ui_original, original_items, DestinationAction, DestinationForm, DestinationItem};
    let (root, ctx, _app) = fixture("destlist", "ko");
    let (initial, recent, work) = (root.join("right"), root.join("recent"), root.join("work"));
    fs::create_dir_all(&recent).unwrap();
    fs::create_dir_all(&work).unwrap();
    let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let mut form = DestinationForm {
        most_used: vec![(s(&initial), 3), (s(&recent), 1)],
        work_folders: [s(&work), String::new(), s(&root.join("missing")), String::new(), String::new()],
        browse_label: "Browse".into(),
        cancel_label: "Cancel".into(),
        remove_label: "Remove".into(),
        clear_label: "Clear".into(),
        initial: initial.clone(),
        path: s(&initial),
        ok_label: "OK".into(),
        original: true,
        selected: 0,
        drives: Vec::new(),
        free_label: String::new(),
        special_folders: [None, None, Some(s(&root.join("pictures"))), None, None],
    };
    let items: Vec<_> = original_items(&form).into_iter().map(|(item, _)| item).collect();
    assert_eq!(items, vec![DestinationItem::Initial(s(&initial)), DestinationItem::Recent(s(&recent)), DestinationItem::Work(0, s(&work))], "the default folder is not repeated among the recents; only existing work folders");
    // Original FormKeyDown (0x454b8c): Ctrl+Q = Pictures folder (FUN_0040db88(2)) into the path field. An empty special folder (Ctrl+D) leaves it as is.
    let ctrl = egui::Modifiers::CTRL;
    for key in [egui::Key::D, egui::Key::Q] {
        let raw = egui::RawInput { time: Some(1.0), modifiers: ctrl, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: ctrl }], ..Default::default() };
        let before = form.path.clone();
        let _ = ctx.run(raw, |ctx| {
            egui::Area::new(egui::Id::new("dest-test")).fixed_pos(egui::Pos2::ZERO).show(ctx, |ui| destination_ui_original(ui, &mut form, false, None, &[], 850.0));
        });
        assert_eq!(form.path, if key == egui::Key::Q { s(&root.join("pictures")) } else { before });
    }
    form.path = s(&initial);
    let mut time = 100.0;
    let mut run = |form: &mut DestinationForm, events: Vec<egui::Event>| {
        time += 0.05;
        let raw = egui::RawInput { time: Some(time), events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let mut got = None;
        let _ = ctx.run(raw, |ctx| {
            egui::Area::new(egui::Id::new("dest-test")).fixed_pos(egui::Pos2::ZERO).show(ctx, |ui| got = destination_ui_original(ui, form, false, None, &[], 850.0));
        });
        got
    };
    let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(&mut form, vec![]);
    run(&mut form, vec![]);
    // Second row (recent, the first row of its group so + 3): 18 to 39.
    let at = egui::pos2(100.0, 28.0);
    run(&mut form, vec![egui::Event::PointerMoved(at), press(at, true)]);
    run(&mut form, vec![press(at, false)]);
    assert_eq!((form.selected, form.path.clone()), (1, s(&recent)));
    assert!(matches!(run(&mut form, vec![egui::Event::Key { key: egui::Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]), Some(DestinationAction::Remove(p)) if p == s(&recent)));
    run(&mut form, vec![egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
    assert_eq!((form.selected, form.path.clone()), (2, s(&work)));
    // Original cbPath Hint "Alt+D": key focus to the path field.
    let alt = egui::Modifiers::ALT;
    let raw = egui::RawInput { modifiers: alt, events: vec![egui::Event::Key { key: egui::Key::D, physical_key: None, pressed: true, repeat: false, modifiers: alt }], ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        egui::Area::new(egui::Id::new("dest-test")).fixed_pos(egui::Pos2::ZERO).show(ctx, |ui| destination_ui_original(ui, &mut form, false, None, &[], 850.0));
    });
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("ndr-destination-path"))), "Alt+D focuses the path box");
    // Double-clicking the first row confirms (pause first so it does not pair with the previous click as a double click).
    for _ in 0..10 {
        run(&mut form, vec![]);
    }
    let first = egui::pos2(100.0, 8.0);
    run(&mut form, vec![egui::Event::PointerMoved(first), press(first, true), press(first, false)]);
    let accepted = run(&mut form, vec![press(first, true), press(first, false)]);
    assert!(matches!(accepted, Some(DestinationAction::Accept)), "double click = OK");
    assert_eq!(form.path, s(&initial));
    // Original FUN_004532d4 kind 8: drive rows are the last group, CD/DVD (5) left out. With key focus in the list, a letter (A-Z, with Shift too) confirms that drive.
    use super::destination_dialog::DestinationDrive;
    let drive = |root: &str, kind: u32| DestinationDrive { root: root.into(), name: "Disk".into(), kind, sizes: Some((10, 100)) };
    form.drives = vec![drive("C:\\", 3), drive("E:\\", 5), drive("Z:\\", 4)];
    let items = original_items(&form);
    let drives: Vec<(String, bool)> = items.iter().filter_map(|(item, first)| if let DestinationItem::Drive(d) = item { Some((d.root.clone(), *first)) } else { None }).collect();
    assert_eq!(drives, [("C:\\".to_owned(), true), ("Z:\\".to_owned(), false)], "no CD/DVD; the drives start a new group");
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("ndr-destination-list")));
    let key = |key: egui::Key, modifiers: egui::Modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    assert!(matches!(run(&mut form, vec![key(egui::Key::Z, egui::Modifiers::NONE)]), Some(DestinationAction::MostUsed(p)) if p == "Z:\\"));
    assert!(matches!(run(&mut form, vec![key(egui::Key::C, egui::Modifiers::SHIFT)]), Some(DestinationAction::MostUsed(p)) if p == "C:\\"));
    assert!(run(&mut form, vec![key(egui::Key::Q, egui::Modifiers::NONE)]).is_none(), "no such drive");
    fs::remove_dir_all(root).unwrap();
}

/// Original MouseDown (0x7d63c8): the second press of a double click (ssDouble) does not touch the marks. Thumbnails (style 3) have no mark cell.
#[test]
fn a_double_press_toggles_once_and_thumbnails_have_no_selection_area() {
    let (root, ctx, mut app) = fixture("dblpress", "ko");
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 20.0), egui::vec2(200.0, 20.0));
    let at = egui::pos2(5.0, 25.0);
    let press = |pane: &mut Pane, time: f64, modifiers: egui::Modifiers, thumbnails: bool| {
        let down = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers }];
        let _ = ctx.run(egui::RawInput { time: Some(time), events: down, modifiers, ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if thumbnails { super::panes::original_thumbnail_press(pane, 1, rect, ui) } else { super::panes::original_press_input(pane, 1, rect, ui) }
            });
        });
        let up = vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers }];
        let _ = ctx.run(egui::RawInput { time: Some(time + 0.02), events: up, modifiers, ..Default::default() }, |_| {});
        pane.range_drag = None;
    };
    let pane = &mut app.panes[0];
    pane.selected.clear();
    press(pane, 10.0, egui::Modifiers::NONE, false);
    assert!(pane.selected.contains(&1), "the first press toggles the selection area");
    assert!(std::mem::take(&mut pane.item_pressed), "a normal press clears the quick search");
    press(pane, 10.1, egui::Modifiers::NONE, false);
    assert!(pane.selected.contains(&1), "the double press does not toggle it back");
    assert!(!pane.item_pressed, "the double press skips the whole MouseDown block");
    press(pane, 12.0, egui::Modifiers::NONE, false);
    assert!(!pane.selected.contains(&1), "a later press toggles again");
    // Thumbnails: the left 12 px is a plain click too (cursor only), Ctrl flips on press.
    pane.cursor = 0;
    press(pane, 20.0, egui::Modifiers::NONE, true);
    assert_eq!((pane.cursor, pane.selected.contains(&1)), (1, false));
    press(pane, 22.0, egui::Modifiers::CTRL, true);
    assert!(pane.selected.contains(&1));
    // Windows rule (WM_LBUTTONDBLCLK): only a second press within GetDoubleClickTime of the previous one and inside the SM_CXDOUBLECLK x SM_CYDOUBLECLK box.
    let probe = |time: f64, pos: egui::Pos2| {
        let mut got = false;
        let down = vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }];
        let _ = ctx.run(egui::RawInput { time: Some(time), events: down, ..Default::default() }, |ctx| got = super::panes::windows_double_press(ctx));
        let up = vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }];
        let _ = ctx.run(egui::RawInput { time: Some(time + 0.02), events: up, ..Default::default() }, |_| {});
        got
    };
    let (ms, _, _) = native::double_click_metrics();
    assert!(!probe(30.0, egui::pos2(5.0, 25.0)));
    assert!(!probe(30.1, egui::pos2(60.0, 25.0)), "a quick press on another spot is not a double click");
    assert!(probe(30.2, egui::pos2(61.0, 25.0)), "the second press inside the double-click rectangle");
    assert!(!probe(30.3, egui::pos2(61.0, 25.0)), "the third press is a normal press again");
    assert!(!probe(30.3 + ms as f64 / 1000.0 + 0.05, egui::pos2(61.0, 25.0)), "later than GetDoubleClickTime");
    assert_eq!(ctx.options(|o| o.input_options.max_double_click_delay), ms as f64 / 1000.0, "egui double clicks use the Windows double-click time");
    // Click one spot, then soon double-click another: egui reports a triple click so double_clicked is false, but Windows sees a double click.
    let click = |time: f64, pos: egui::Pos2| {
        let mut got = false;
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
        let widget = |ctx: &egui::Context, got: &mut bool| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.interact(area, egui::Id::new("dbl-widget"), egui::Sense::click());
                *got = super::panes::windows_double_clicked(&response);
            });
        };
        let down = vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }];
        let _ = ctx.run(egui::RawInput { time: Some(time), events: down, ..Default::default() }, |ctx| widget(ctx, &mut got));
        let up = vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }];
        let _ = ctx.run(egui::RawInput { time: Some(time + 0.02), events: up, ..Default::default() }, |ctx| widget(ctx, &mut got));
        got
    };
    assert!(!click(50.0, egui::pos2(20.0, 20.0)));
    assert!(!click(50.15, egui::pos2(20.0, 120.0)), "another spot");
    assert!(click(50.3, egui::pos2(20.0, 120.0)), "the double click right after a click elsewhere");
    fs::remove_dir_all(root).unwrap();
}

/// Original WMChar (0x7d5a18) and QuickSearch (0x7d5ca8, 0x7d5f9c): character filtering, search order, no time limit, cleared when the cursor moves another way.
#[test]
fn original_quick_search_follows_the_file_list_view() {
    use super::quick_search::{OriginalSearchKey as K, original_quick_search, original_search_key};
    assert_eq!(original_search_key('A', false, false), None, "uppercase without Caps Lock is ignored");
    assert_eq!(original_search_key('A', true, false), Some(K::Char('A')));
    assert_eq!(original_search_key('.', false, true), None, "'.' cannot start a search");
    assert_eq!(original_search_key('.', false, false), Some(K::Char('.')));
    assert_eq!(original_search_key('\u{8}', false, true), None);
    assert_eq!(original_search_key('!', false, false), Some(K::Clear));
    for c in [' ', '\\', '/', ':', '*', '?', '가'] {
        assert_eq!(original_search_key(c, false, false), None, "{c:?}");
    }
    let names: Vec<String> = ["..", "alpha", "beta", "gamma", "xalpha"].iter().map(|s| s.to_string()).collect();
    assert_eq!(original_quick_search(&names, 2, "a", true), Some(1), "prefix wraps to the top");
    assert_eq!(original_quick_search(&names, 2, "ALP", true), Some(1), "case-insensitive");
    assert_eq!(original_quick_search(&names, 2, "mm", true), Some(3), "then inside names");
    assert_eq!(original_quick_search(&names, 0, "mm", true), None, "starting at the first item the original skips the inside search");
    assert_eq!(original_quick_search(&names, 1, "al", false), Some(4), "next item onwards: inside names after the prefix pass");
    assert_eq!(original_quick_search(&names, 3, "", true), Some(0), "empty text = item 0");

    let (root, ctx, mut app) = fixture("original-quicksearch", "ko");
    assert!(app.current_skin().is_some(), "the default theme is the Black skin");
    app.panes[0].cursor = index_of(&app, "folder1");
    type_text(&mut app, &ctx, "a");
    let first = cursor_name(&app);
    assert!(first == "alpha.txt" || first == "Alphabet.doc");
    type_text(&mut app, &ctx, "lphab");
    assert_eq!(cursor_name(&app), "Alphabet.doc");
    thread::sleep(Duration::from_millis(1200));
    type_text(&mut app, &ctx, "e");
    assert_eq!(app.typed_search[0].query, "alphabe", "no timeout: the text keeps growing");
    type_text(&mut app, &ctx, "z");
    assert_eq!(app.typed_search[0].query, "", "no match even for the new character alone: the text is emptied");
    type_text(&mut app, &ctx, "b");
    assert_eq!((app.typed_search[0].query.as_str(), cursor_name(&app).as_str()), ("b", "beta.txt"));
    // Original FormShortCut: Backspace goes to quick search while there is search text. When empty it goes to item 0 like the original.
    keyboard_frame(&mut app, &ctx, vec![key(egui::Key::Backspace)]);
    assert!(app.typed_search[0].query.is_empty());
    assert_eq!(app.panes[0].cursor, 0);
    assert_eq!(app.panes[0].path, root.join("left"), "Backspace went to the quick search, not to history back");
    // Moving the cursor another way clears the search text (original ArrowKey).
    type_text(&mut app, &ctx, "g");
    assert_eq!(app.typed_search[0].query, "g");
    app.panes[0].cursor = index_of(&app, "beta.txt");
    keyboard_frame(&mut app, &ctx, vec![]);
    assert!(app.typed_search[0].query.is_empty());
    // Original MouseDown clears the search text even when the same item is clicked.
    type_text(&mut app, &ctx, "g");
    assert_eq!(app.typed_search[0].query, "g");
    app.panes[0].item_pressed = true;
    keyboard_frame(&mut app, &ctx, vec![]);
    assert!(app.typed_search[0].query.is_empty(), "a press on the found item clears the text");
    // Original WMIMEComposition (0x7d584c): appends the composing character (replacing the last one when it changes) and ends composition when the result arrives.
    fs::write(root.join("left").join("가나다.txt"), b"x").unwrap();
    fs::write(root.join("left").join("가방.txt"), b"x").unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.panes[0].cursor = 0;
    let ime = |e: egui::ImeEvent| egui::Event::Ime(e);
    keyboard_frame(&mut app, &ctx, vec![ime(egui::ImeEvent::Enabled), ime(egui::ImeEvent::Preedit("ㄱ".into()))]);
    assert_eq!((app.typed_search[0].query.as_str(), app.panes[0].cursor), ("ㄱ", 0), "a miss while composing keeps the text and the cursor (fixed)");
    keyboard_frame(&mut app, &ctx, vec![ime(egui::ImeEvent::Preedit("가".into()))]);
    assert_eq!((app.typed_search[0].query.as_str(), cursor_name(&app).starts_with('가')), ("가", true));
    app.typed_search[0] = Default::default();
    app.panes[0].cursor = 0;
    keyboard_frame(&mut app, &ctx, vec![ime(egui::ImeEvent::Preedit("가".into()))]);
    keyboard_frame(&mut app, &ctx, vec![ime(egui::ImeEvent::Disabled), ime(egui::ImeEvent::Commit("가".into())), ime(egui::ImeEvent::Disabled)]);
    keyboard_frame(&mut app, &ctx, vec![ime(egui::ImeEvent::Preedit("바".into()))]);
    assert_eq!((app.typed_search[0].query.as_str(), cursor_name(&app).as_str()), ("가바", "가나다.txt"), "no match mid-syllable: kept");
    keyboard_frame(&mut app, &ctx, vec![ime(egui::ImeEvent::Preedit("방".into()))]);
    assert_eq!((app.typed_search[0].query.as_str(), cursor_name(&app).as_str()), ("가방", "가방.txt"), "the composing character is replaced");
    // When the composition text becomes empty (ending without a result), the last character is removed.
    keyboard_frame(&mut app, &ctx, vec![ime(egui::ImeEvent::Disabled)]);
    assert_eq!(app.typed_search[0].query, "가");
    assert!(app.typed_search[0].ime_base.is_none());
    fs::remove_dir_all(root).unwrap();
}
/// In the original main window the list always takes keys. Even when an egui button gets key focus (Tab or arrow focus movement), list keys keep working,
/// and Space/Enter on a focused button do not become clicks.
#[test]
fn widget_focus_never_takes_the_list_keyboard() {
    let (root, ctx, mut app) = fixture("focus", "ko");
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    let button_id = std::cell::Cell::new(None);
    let clicks = std::cell::Cell::new(0);
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            app.keyboard(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                let response = ui.button("toolbar");
                button_id.set(Some(response.id));
                if response.clicked() {
                    clicks.set(clicks.get() + 1);
                }
                let _ = ui.button("next");
            });
        });
    };
    run(&mut app, vec![]);
    ctx.memory_mut(|m| m.request_focus(button_id.get().unwrap()));
    run(&mut app, vec![]);
    let before = app.panes[0].cursor;
    run(&mut app, vec![key(egui::Key::ArrowDown)]);
    assert_eq!(app.panes[0].cursor, before + 1, "the list moved");
    assert!(ctx.memory(|m| m.focused()).is_none(), "the button gave the focus back");
    ctx.memory_mut(|m| m.request_focus(button_id.get().unwrap()));
    run(&mut app, vec![key(egui::Key::Space)]);
    assert_eq!(clicks.get(), 0, "Space did not press the focused button");
    // Tab switches panes like the original and egui does not focus a button. Pressing again switches again.
    let active = app.active;
    run(&mut app, vec![key(egui::Key::Tab)]);
    run(&mut app, vec![]);
    assert!(ctx.memory(|m| m.focused()).is_none());
    assert_ne!(app.active, active);
    run(&mut app, vec![key(egui::Key::Tab)]);
    assert_eq!(app.active, active, "Tab keeps switching panes");
    fs::remove_dir_all(root).unwrap();
}
/// Original GetList (0x45a34c): the list is read in place (skin theme, no "reading"), with ".." first unless at a root (not selectable; opening it goes up).
#[test]
fn original_listing_is_read_in_place_with_a_parent_item() {
    let (root, ctx, mut app) = fixture("inplace", "ko");
    assert!(app.current_skin().is_some());
    app.refresh(0);
    assert!(!app.panes[0].loading, "read in place");
    assert_eq!(app.panes[0].entries[0].name, "..");
    assert!(app.panes[0].entries[0].is_parent() && app.panes[0].cursor == 0);
    assert!(app.panes[0].targets().is_empty(), "\"..\" is never an operation target");
    app.panes[0].selected.insert(0);
    app.panes[0].drop_drive_marks();
    assert!(app.panes[0].selected.is_empty(), "\"..\" is never marked");
    app.open_current();
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, root, "opening \"..\" goes up");
    assert_eq!(cursor_name(&app), "left", "the cursor is on the folder we came from (original FUN_00486694)");
    fs::remove_dir_all(root).unwrap();
}
/// egui-winit Copy, Cut and Paste events are turned back into the original shortcut keys (Ctrl+C, X, V). Pastes egui dropped because the clipboard has no text
/// are added back as counted by the window subclass.
#[test]
fn clipboard_shortcuts_reach_the_file_list() {
    let ctrl = egui::Modifiers::CTRL;
    let mut raw = egui::RawInput { modifiers: ctrl, events: vec![egui::Event::Copy, egui::Event::Cut, egui::Event::Paste("x".into())], ..Default::default() };
    super::restore_clipboard_keys(&mut raw, 2, false, false, true);
    let keys: Vec<egui::Key> = raw.events.iter().filter_map(|e| if let egui::Event::Key { key, modifiers, pressed: true, .. } = e { assert_eq!(*modifiers, ctrl); Some(*key) } else { None }).collect();
    assert_eq!(keys, [egui::Key::C, egui::Key::X, egui::Key::V, egui::Key::V], "one Paste event + one dropped paste key");
    let mut raw = egui::RawInput { modifiers: egui::Modifiers::SHIFT, events: vec![egui::Event::Cut], ..Default::default() };
    super::restore_clipboard_keys(&mut raw, 1, false, false, false);
    let keys: Vec<egui::Key> = raw.events.iter().filter_map(|e| if let egui::Event::Key { key, .. } = e { Some(*key) } else { None }).collect();
    assert_eq!(keys, [egui::Key::Delete, egui::Key::Insert], "Shift+Delete cut, Shift+Insert paste");
    // egui-winit also reports Ctrl+Shift+Delete (original Shred) as Cut; Ctrl+Shift+X stays X.
    for (delete_down, x_down, expected) in [(true, false, egui::Key::Delete), (false, true, egui::Key::X)] {
        let mut raw = egui::RawInput { modifiers: ctrl | egui::Modifiers::SHIFT, events: vec![egui::Event::Cut], ..Default::default() };
        super::restore_clipboard_keys(&mut raw, 0, false, delete_down, x_down);
        let keys: Vec<egui::Key> = raw.events.iter().filter_map(|e| if let egui::Event::Key { key, .. } = e { Some(*key) } else { None }).collect();
        assert_eq!(keys, [expected], "Ctrl+Shift Cut with Delete down = {delete_down}");
    }
    // In the file list, Ctrl+C goes to the original miEditCopy (= copy command).
    let (root, ctx, mut app) = fixture("clipkeys", "ko");
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.panes[0].cursor = index_of(&app, "alpha.txt");
    let mut raw = egui::RawInput { modifiers: ctrl, events: vec![egui::Event::Copy], ..Default::default() };
    super::restore_clipboard_keys(&mut raw, 0, false, false, false);
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
    assert_eq!(app.clipboard.as_ref().map(|(paths, cut)| (paths.len(), *cut)), Some((1, false)), "Ctrl+C copied the cursor item");
    // Numpad keys of the original shortcut table: Num * = miInvertSelection, Num + = miAdvSelect, Num / = miSelectSameExt, Ctrl+Num / = miSelectSameName.
    assert_eq!(app.numpad_shortcut(0x6a, false, false, false).as_deref(), Some("miInvertSelection"));
    assert_eq!(app.numpad_shortcut(0x6b, false, false, false).as_deref(), Some("miAdvSelect"));
    assert_eq!(app.numpad_shortcut(0x6f, true, false, false).as_deref(), Some("miSelectSameName"));
    assert_eq!(app.numpad_shortcut(0x61, false, false, false), None, "Num 1 is not bound: it stays a typed digit");
    let mut raw = egui::RawInput { events: vec![key(egui::Key::Plus), egui::Event::Text("+".into()), egui::Event::Text("*".into())], ..Default::default() };
    super::strip_numpad_events(&mut raw, 0x6b);
    super::strip_numpad_events(&mut raw, 0x6a);
    assert!(raw.events.is_empty(), "handled numpad keys never reach the quick search");
    app.panes[0].selected.clear();
    app.pending_numpad.push(("miInvertSelection".into(), false));
    let _ = ctx.run(egui::RawInput::default(), |ctx| app.keyboard(ctx));
    assert!(!app.panes[0].selected.is_empty(), "Num * inverted the selection");
    fs::remove_dir_all(root).unwrap();
}
/// Original GetItemAt and TNXFramePanel: clicking anywhere on a row (outside the name cell, the date cell etc.) picks that item, and clicking anywhere in a pane makes it current.
#[test]
fn empty_pane_area_press_activates_in_the_main_layout() {
    let (root, ctx, mut app) = fixture("emptyclick", "ko");
    let draw = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let full = ui.max_rect();
                let left = egui::Rect::from_min_max(full.min, egui::pos2(full.center().x - 10.0, full.bottom()));
                let right = egui::Rect::from_min_max(egui::pos2(full.center().x + 10.0, full.top()), full.max);
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| app.pane_ui(ui, 0, ctx));
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| app.pane_ui(ui, 1, ctx));
            });
        });
    };
    let click = |app: &mut App, pos: egui::Pos2| {
        draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
        draw(app, vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
    };
    draw(&mut app, vec![]);
    draw(&mut app, vec![]);
    let right = app.pane_rects[1];
    assert!(right.width() > 100.0, "{right:?}");
    for pos in [egui::pos2(right.center().x, right.bottom() - 80.0), egui::pos2(right.right() - 20.0, right.top() + 120.0), egui::pos2(right.right() - 30.0, right.top() + 10.0)] {
        app.active = 0;
        click(&mut app, pos);
        assert_eq!(app.active, 1, "press at {pos:?} in {right:?}");
    }
    fs::remove_dir_all(root).unwrap();
}
/// Original GetItemAt and TNXFramePanel: clicking anywhere on a row (outside the name cell, the date cell etc.) picks that item, and clicking anywhere in a pane makes it current.
#[test]
fn clicks_anywhere_on_a_row_or_pane_follow_the_original() {
    let (root, ctx, mut app) = fixture("rowclick", "ko");
    app.settings.set_text("LeftViewStyle", "Details");
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    let draw = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    let click = |app: &mut App, pos: egui::Pos2| {
        draw(app, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
        draw(app, vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
    };
    draw(&mut app, vec![]);
    draw(&mut app, vec![]);
    // With the right pane current, clicking empty space low in the left pane makes the left one current.
    app.active = 1;
    let left = app.pane_rects[0];
    click(&mut app, egui::pos2(left.center().x, left.bottom() - 60.0));
    assert_eq!(app.active, 0, "a press on the empty pane area activates it");
    // Clicking down the date cell side (outside the name cell) from the top moves the cursor to that row.
    app.panes[0].cursor = 0;
    let x = left.right() - 150.0;
    let mut moved = false;
    let mut y = left.top() + 40.0;
    while y < left.top() + 260.0 && !moved {
        click(&mut app, egui::pos2(x, y));
        moved = app.panes[0].cursor > 1;
        y += 9.0;
    }
    assert!(moved, "pressing outside the name cell picks that row");
    fs::remove_dir_all(root).unwrap();
}
fn type_text(app: &mut App, ctx: &egui::Context, text: &str) {
    keyboard_frame(app, ctx, vec![egui::Event::Text(text.into())]);
}
fn names_of(app: &App, pane: usize) -> Vec<String> {
    let mut names: Vec<String> = app.panes[pane].entries.iter().map(|e| e.name.clone()).collect();
    names.sort();
    names
}

#[test]
fn filters_apply_to_the_listing() {
    let (root, ctx, mut app) = fixture("filter", "ko");
    app.quick_filter(0, "  ALP ");
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].filter, "*ALP*");
    assert_eq!(names_of(&app, 0), ["..", "Alphabet.doc", "alpha.txt"], "the parent item stays (original GetList adds it after the filter)");
    app.dispatch("selectAll", &ctx);
    assert_eq!(app.panes[0].targets().len(), 2, "hidden-by-filter items are not operation targets");
    app.quick_filter(0, "*.log");
    wait_listing(&mut app, &ctx);
    assert_eq!(names_of(&app, 0), ["..", "gamma.log"]);
    app.dispatch("filter", &ctx);
    let dialog = &app.original_dialogs[0];
    assert_eq!(dialog.title, app.language_pack.text("RES.TITLE_FILTER", "?"), "original miFilterCustomClick");
    assert!(matches!(&dialog.kind, original_dialogs::DialogKind::Prompt { value, message, .. } if value == "*.log" && *message == app.language_pack.text("RES.MSG_FILTER", "?")), "the current filter is the default");
    app.answer_dialog(original_dialogs::DialogResult::Text(Some(" *.txt ; folder? ".into())), &ctx);
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].filter, "*.txt ; folder?");
    assert_eq!(names_of(&app, 0), ["..", "alpha.txt", "beta.txt", "folder1"]);
    assert_eq!(app.status, "목록 필터: *.txt ; folder?");
    app.dispatch("filter", &ctx);
    app.answer_dialog(original_dialogs::DialogResult::Text(Some("".into())), &ctx);
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].entries.len(), 6, "five items and \"..\"");
    assert_eq!(app.status, "목록 필터를 해제했습니다.");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn function_key_bar_texts_and_error_texts_follow_the_original() {
    let (root, _ctx, app) = fixture("fkeys", "en");
    // Original FUN_00430940: Alt only = ALTFUNCTIONS, Ctrl only = CTRLFUNCTIONS ("key=name" rows), otherwise F1-F12.
    let alt = app.modifier_functions(egui::Modifiers::ALT).unwrap();
    assert_eq!((alt[0].0.as_str(), alt[0].1.as_str(), alt.len()), ("C", "Copy", 8));
    let ctrl = app.modifier_functions(egui::Modifiers::CTRL).unwrap();
    assert_eq!(ctrl.last().map(|(k, n)| (k.as_str(), n.as_str())), Some(("->", "Copy To Right")));
    assert_eq!(ctrl[6].0, "<-");
    assert!(app.modifier_functions(egui::Modifiers::NONE).is_none());
    assert!(app.modifier_functions(egui::Modifiers::ALT | egui::Modifiers::CTRL).is_none());
    // The program picker filter is built from the original OD_PROGRAMFILTER (TOpenDialog.Filter "name|pattern|...").
    let filters = original_dialogs::program_filter(&app.language_pack);
    assert_eq!(filters, [("Program Files(*.exe)".to_owned(), "*.exe".to_owned()), ("All Files(*.*)".to_owned(), "*.*".to_owned())]);
    assert_eq!(original_dialogs::delphi_filter("a|b|c"), [("a".to_owned(), "b".to_owned())], "an unpaired tail is dropped");
    // Original FUN_00404890: 7-Zip errors become original archive error texts, other texts stay.
    assert_eq!(app.archive_error_text("7-Zip failed (exit code: 2): ERROR: Wrong password : a"), app.language_pack.text("RES.INVALID_PASSWORD", "?"));
    assert_eq!(app.archive_error_text("7-Zip failed (exit code: 2): ERROR: Missing volume : x.002"), app.language_pack.text("RES.CANNOT_FIND_NEXTVOLUME", "?"));
    assert_eq!(app.archive_error_text("Access is denied. (os error 5)"), "Access is denied. (os error 5)");
    fs::remove_dir_all(root).unwrap();
}

/// Original miSelectAllClick (0x4311c4), bSelectAllClick (0x431450), miInvertSelectionClick (0x43163c), miSelectSameExt/NameClick,
/// miRestoreLastSelClick (0x43742c) and CopySelection (skin theme).
#[test]
fn original_selection_commands_follow_the_handlers() {
    let (root, ctx, mut app) = fixture("origsel", "ko");
    let left = root.join("left");
    fs::write(left.join("alpha.log"), b"x").unwrap();
    fs::write(left.join("ALPHA.md"), b"x").unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    assert!(app.current_skin().is_some());
    let marked = |app: &App| { let mut v: Vec<String> = app.panes[0].selected.iter().map(|i| app.panes[0].entries[*i].name.clone()).collect(); v.sort(); v };
    let files = app.panes[0].entries.iter().filter(|e| !e.is_dir && !e.drive).count();
    let folder = index_of(&app, "folder1");
    // Menu select all: files only -> with all files selected, folders too -> otherwise clear.
    app.dispatch("selectAll", &ctx);
    assert_eq!((app.panes[0].selected.len(), app.panes[0].selected.contains(&folder)), (files, false));
    app.dispatch("selectAll", &ctx);
    assert!(app.panes[0].selected.contains(&folder), "all files marked: folders are added");
    app.dispatch("selectAll", &ctx);
    assert!(app.panes[0].selected.is_empty(), "everything marked: cleared");
    // Vertical bar button: if all are selected, clear and select files only.
    app.original_button_select_all(false);
    app.original_button_select_all(false);
    assert!(app.panes[0].selected.contains(&folder));
    app.original_button_select_all(false);
    assert_eq!((app.panes[0].selected.len(), app.panes[0].selected.contains(&folder)), (files, false));
    // Invert: files only (folders only with selectfolder).
    app.panes[0].selected = [index_of(&app, "beta.txt")].into_iter().collect();
    app.dispatch("invertSelection", &ctx);
    assert!(!app.panes[0].selected.contains(&folder) && !app.panes[0].selected.contains(&index_of(&app, "beta.txt")));
    assert_eq!(app.panes[0].selected.len(), files - 1);
    // Same extension: mark all if any is unmarked (other marks stay), unmark when all are marked.
    app.panes[0].selected = [index_of(&app, "gamma.log")].into_iter().collect();
    app.panes[0].cursor = index_of(&app, "alpha.txt");
    app.dispatch("selectSameExt", &ctx);
    assert_eq!(marked(&app), ["alpha.txt", "beta.txt", "gamma.log"]);
    app.dispatch("selectSameExt", &ctx);
    assert_eq!(marked(&app), ["gamma.log"]);
    app.dispatch("selectSameName", &ctx);
    assert_eq!(marked(&app), ["ALPHA.md", "alpha.log", "alpha.txt", "gamma.log"]);
    // Restore Selection uses the marks saved when a file job started (CopySelection). Selection commands do not save.
    assert!(app.last_selection[0].is_empty());
    app.dispatch("restoreSelection", &ctx);
    assert_eq!(marked(&app), ["ALPHA.md", "alpha.log", "alpha.txt", "gamma.log"], "nothing remembered: unchanged");
    app.dispatch("description", &ctx);
    assert_eq!(app.last_selection[0].len(), 4);
    app.original_dialogs.clear();
    app.panes[0].selected.clear();
    app.dispatch("restoreSelection", &ctx);
    assert_eq!(marked(&app), ["ALPHA.md", "alpha.log", "alpha.txt", "gamma.log"]);
    // Original miCalculateSelectedFolderSizeClick (0x4457d4): counts the chosen folders' sizes into the list size cell and clears the marks (no summary box).
    let folder_path = app.panes[0].entries[folder].path.clone();
    app.panes[0].selected = [folder, index_of(&app, "beta.txt")].into_iter().collect();
    app.dispatch("calculateFolderSize", &ctx);
    assert!(app.panes[0].selected.is_empty() && app.msgboxes.is_empty() && app.job.is_none());
    let end = Instant::now() + Duration::from_secs(10);
    while !app.panes[0].folder_sizes.get(&folder_path).is_some_and(|s| s.is_some()) {
        assert!(Instant::now() < end, "folder size timeout");
        app.poll(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    fs::remove_dir_all(root).unwrap();
}

fn system_temp() -> Option<PathBuf> {
    Some(std::env::temp_dir())
}

/// Original miSortByClick (0x431c88): choosing the same sort kind again toggles descending. miToTempClick (0x4330b0): GetTempPath + GetLongPathName,
/// unchanged on failure.
#[test]
fn original_sort_and_temp_commands_follow_the_handlers() {
    let (root, ctx, mut app) = fixture("origsort", "ko");
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    assert!(app.current_skin().is_some());
    app.dispatch("sortSize", &ctx);
    assert_eq!((app.settings.text("LeftSortMode", ""), app.settings.flag("LeftSortDescending", false)), ("Size".to_owned(), false));
    app.dispatch("sortSize", &ctx);
    assert!(app.settings.flag("LeftSortDescending", false), "the same sort type again: descending");
    app.dispatch("sortName", &ctx);
    assert_eq!((app.settings.text("LeftSortMode", ""), app.settings.flag("LeftSortDescending", false)), ("Name".to_owned(), true));
    app.dispatch("sortNone", &ctx);
    app.dispatch("sortNone", &ctx);
    assert!(app.settings.flag("LeftSortDescending", false), "None never toggles");
    // To Temp: does not move on failure (the default test fake returns None).
    let before = app.panes[0].path.clone();
    app.dispatch("temp", &ctx);
    assert_eq!(app.panes[0].path, before);
    app.system.temp_folder = system_temp;
    app.dispatch("temp", &ctx);
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, std::env::temp_dir());
    fs::remove_dir_all(root).unwrap();
}

/// Original DrawListItem (0x7d8f58) and TNXShellImageList.Draw (0x7b8ebc): cut items and hidden items get icons faded with ILD_BLEND50 (white).
/// (The cut command uses the real clipboard, so only the mark is set here.)
#[test]
fn original_ghosted_icons_follow_draw_list_item() {
    let (root, ctx, mut app) = fixture("origghost", "ko");
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    let (alpha, beta, gamma) = (index_of(&app, "alpha.txt"), index_of(&app, "beta.txt"), index_of(&app, "gamma.log"));
    let icon = app.icon_texture(&ctx, "ghost-test".into(), 1, 1, &[0, 100, 200, 128]);
    assert!(app.ghost_icons.contains_key(&icon.id()), "the ghosted variant is made with the icon");
    assert!(!app.row_ghosted(0, alpha));
    app.panes[0].cut_marks.insert(app.panes[0].entries[alpha].path.clone());
    app.panes[0].entries[beta].attributes |= 2;
    assert!(app.row_ghosted(0, alpha) && app.row_ghosted(0, beta) && !app.row_ghosted(0, gamma));
    let ghost = app.row_icon_for_skin(0, alpha, Some(icon.clone())).unwrap();
    assert_eq!(ghost.id(), app.ghost_icons[&icon.id()].id());
    assert_eq!(app.row_icon_for_skin(0, gamma, Some(icon.clone())).unwrap().id(), icon.id());
    // New list = new items: the cut marks are gone.
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    assert!(app.panes[0].cut_marks.is_empty());
    fs::remove_dir_all(root).unwrap();
}
