//! C# NDIR(r66 경로 복사, r69 찾기, r70 원본 메시지 상자) 동작을 Rust UI에서 확인한다. 한국어와 영어만 검수한다.
//! 실제 클립보드는 건드리지 않는다(clipboard_writer 교체).
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
        "ndir-parity-{}-{}-{label}",
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
    let ok = show(&mut app, "Title", "A message that is long enough for every button row.", ndir::msgbox::MB_OK | ndir::msgbox::MB_ICONASTERISK);
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
    assert_eq!(ok.get(), ndir::msgbox::IDOK, "Esc presses the Cancel button (OK on an OK-only box)");
    assert!(app.msgboxes.is_empty());

    let yes_no = show(&mut app, "T", "Pressing N picks the No button directly.", ndir::msgbox::MB_YESNO | ndir::msgbox::MB_ICONQUESTION);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::N)]);
    assert_eq!(yes_no.get(), ndir::msgbox::IDNO, "accelerator without Alt");

    let enter = show(&mut app, "T", "Enter presses the focused Yes button.", ndir::msgbox::MB_YESNO);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
    assert_eq!(enter.get(), ndir::msgbox::IDYES);

    let ari = show(&mut app, "T", "Abort, retry or ignore without a cancel button.", ndir::msgbox::MB_ABORTRETRYIGNORE);
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert_eq!(ari.get(), -1, "Esc without a Cancel button does nothing");
    app.answer_message_box(5, &ctx);
    assert_eq!(ari.get(), ndir::msgbox::IDIGNORE);

    let blank = show(&mut app, "T", "Type 7 keeps the DFM buttons.", 0x07);
    frame(&mut app, &ctx, vec![]);
    let layout = app.msgboxes[0].layout.clone().unwrap();
    assert!(layout.slots.iter().all(Option::is_some) && layout.captions.iter().all(String::is_empty));
    app.answer_message_box(0, &ctx);
    assert_eq!(blank.get(), -1, "ModalResult 0 does not close");
    app.answer_message_box(2, &ctx);
    assert_eq!(blank.get(), ndir::msgbox::IDCANCEL, "the close box returns IDCANCEL");

    let two = show(&mut app, "", "Two\nlines", ndir::msgbox::MB_YESNOCANCEL | ndir::msgbox::MB_ICONQUESTION);
    frame(&mut app, &ctx, vec![]);
    let layout = app.msgboxes[0].layout.clone().unwrap();
    assert_eq!(app.msgboxes[0].title, ndir::msgbox::DEFAULT_TITLE, "empty title becomes the product name");
    assert_eq!(layout.line_count, 2);
    assert!((layout.client.y - (119.0 - 20.0 + 2.0 * line_height + 4.0)).abs() < 0.01, "LineSpace 4");
    assert_eq!(layout.client.x, ndir::msgbox::minimum_client_width(3), "short text: widened for three buttons");
    assert_eq!(layout.captions[0], ko_raw(&app, "YES"));
    app.answer_message_box(6, &ctx);
    assert_eq!(two.get(), ndir::msgbox::IDYES);
    fs::remove_dir_all(root).unwrap();
}

/// 원본 miCopyToClick → FUN_0042bdc4 → TfSelectDest(FUN_0043cb24): 경로 칸 처음 값은 다른 패널 폴더이고, 고쳐서 같은 패널 안
/// 폴더(상대 경로도)로 복사·이동할 수 있다(스킨 테마).
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
    // 같은 패널 안 하위 폴더(상대 경로)로.
    if let original_dialogs::DialogKind::Destination(form) = &mut app.original_dialogs.last_mut().unwrap().kind {
        form.path = "folder1".into();
    }
    app.destination_action(super::destination_dialog::DestinationAction::Accept, &ctx);
    assert!(app.original_dialogs.is_empty());
    wait_job(&mut app, &ctx);
    assert_eq!(fs::read(root.join("left/folder1/alpha.txt")).unwrap(), b"alpha.txt", "copied inside the same pane");
    // 없는 폴더는 창을 닫지 않는다.
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
    assert_eq!(app.msgboxes[0].flags, ndir::msgbox::MB_YESNO | ndir::msgbox::MB_ICONQUESTION);
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
    // 새로 읽어도 커서는 alpha.txt에 남으므로(원본 새로 고침), 오른쪽에 없는 gamma.log를 고른다.
    app.panes[0].cursor = index_of(&app, "gamma.log");
    app.copy_to_other(0, false, &ctx);
    assert!(app.msgboxes.is_empty(), "no box when the confirmation is off");
    wait_job(&mut app, &ctx);
    assert!(root.join("right/gamma.log").exists());
    // 원본 FUN_0042d518/FUN_0042d910(수직 막대·단축키, 스킨 테마): confirmdrop과 상관없이 묻고, Shift 단추 = 이동.
    assert!(app.current_skin().is_some());
    app.panes[0].cursor = index_of(&app, "beta.txt");
    app.original_copy_to_other(0, true, &ctx);
    let expected = app.language_pack.text("RES.MSG_TORIGHT", "?").replace("%s", &app.language_pack.text("RES.MOVE", "?"));
    assert_eq!(app.msgboxes.first().map(|b| b.text.clone()), Some(expected), "always asked");
    app.answer_message_box(7, &ctx);
    // 패널이 하나면 아무것도 하지 않는다.
    let dual = app.dual;
    app.dual = false;
    app.original_copy_to_other(0, false, &ctx);
    assert!(app.msgboxes.is_empty() && app.job.as_ref().is_none_or(|j| j.finished));
    app.dual = dual;
    app.settings.set_flag("ConfirmDragDrop", true);
    // 원본 내부 명령 0x25 FocusAddressBar(FUN_0048d3c8) = C# BeginAddressEdit: 활성 패널 주소 편집.
    app.active = 1;
    assert!(app.original_internal("FocusAddressBar", &ctx));
    assert!(app.panes[1].address_editing && !app.panes[0].address_editing);
    assert!(app.original_dialogs.is_empty(), "not the command line prompt");
    app.end_address_edit(1);
    // 원본 내부 명령 0x2a Select: 커서 항목 선택만 뒤집는다(커서는 그대로, 고급 선택 창이 아님).
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

    app.dispatch("email", &ctx);
    assert_eq!(clipboard(), vec!["xiles@xiles.net".to_string()]);
    assert_eq!(app.msgboxes[0].flags, ndir::msgbox::MB_OK | ndir::msgbox::MB_ICONASTERISK);
    assert_eq!(app.msgboxes[0].text, app.language_pack.text("RES.EMAILCOPIED", "?"));
    assert_eq!(ndir::msgbox::lines(&app.msgboxes[0].text).len(), 2);
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
    app.copy_item_paths(ndir::parity::PathPart::Full, true);
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

/// 원본 MouseDown(0x7d63c8)(스킨 테마): 그냥 누르기·오른쪽 단추는 표시를 두고 커서만, Shift는 커서~누른 줄을 더함,
/// Ctrl은 뒤집기(고른 줄이 없으면 전 커서 줄도). C# 테마는 그냥 누르기가 표시를 지운다.
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

/// 원본 GetSelAreaAt(0x7dae7c)·MouseDown·MouseMove(0x7d65f0): 줄 왼쪽 12화소를 누르면 그 줄을 뒤집고,
/// 누른 채 끌면 처음 줄부터 포인터 아래 줄까지 고르며, 두 줄째부터는 Ctrl이 없으면 다른 표시를 지운다.
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
    // 단추를 놓으면(MouseUp) 끌기 끝. 표시 칸 밖 그냥 누르기는 커서만.
    let _ = ctx.run(egui::RawInput { events: vec![egui::Event::PointerButton { pos: egui::pos2(60.0, 65.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }], ..Default::default() }, |_| {});
    pane.range_drag = None;
    let at = egui::pos2(100.0, 5.0);
    let plain = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }];
    assert!(run(pane, plain, 0));
    assert_eq!((pane.cursor, marks(pane), pane.range_drag), (0, vec![1, 2, 3], None));
    let _ = ctx.run(egui::RawInput { events: vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }], ..Default::default() }, |_| {});
    // Shift+누르기: 커서~누른 줄을 더하고, 누른 채 끌면 범위를 다시 잡는다.
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

/// 원본 TfSelectDest lbListKeyDown(0x454130): 1~9·Alt+1~9 = 작업 폴더, Ctrl+1~9 = 자주 쓴 폴더, 0 = 처음 폴더, Shift+Delete = 기록 지우기.
#[test]
fn destination_list_keys_follow_the_original() {
    use super::destination_dialog::{destination_ui, DestinationAction, DestinationForm};
    let (root, ctx, _app) = fixture("destkeys", "ko");
    let initial = std::env::temp_dir();
    let mut form = DestinationForm {
        hint: String::new(),
        most_used_title: String::new(),
        most_used: vec![("C:\\a".into(), 3), ("C:\\b".into(), 1)],
        work_folders: ["W1".into(), "W2".into(), String::new(), String::new(), String::new()],
        empty_text: String::new(),
        browse_label: "Browse".into(),
        cancel_label: "Cancel".into(),
        remove_label: "Remove".into(),
        clear_label: "Clear".into(),
        initial: initial.clone(),
        path: initial.to_string_lossy().into_owned(),
        ok_label: "OK".into(),
        original: false,
        selected: 0,
    };
    let mut run = |key: egui::Key, modifiers: egui::Modifiers| {
        // 목록에 글쇠 초점(경로 칸이 아님).
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("ndir-test-elsewhere")));
        let raw = egui::RawInput { modifiers, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() };
        let mut got = None;
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| got = destination_ui(ui, &mut form, false));
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

/// 원본 TfSelectDest(스킨 테마): 목록 = 처음 폴더, 그것과 다른 최근 폴더, 있는 작업 폴더. 누르면 고르고 경로 칸에 그 경로, 위/아래로 고르고,
/// Delete는 고른 최근 줄을 지우고, 두 번 누르면 확인.
#[test]
fn original_destination_list_follows_tfselectdest() {
    use super::destination_dialog::{destination_ui_original, original_items, DestinationAction, DestinationForm, DestinationItem};
    let (root, ctx, _app) = fixture("destlist", "ko");
    let (initial, recent, work) = (root.join("right"), root.join("recent"), root.join("work"));
    fs::create_dir_all(&recent).unwrap();
    fs::create_dir_all(&work).unwrap();
    let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let mut form = DestinationForm {
        hint: String::new(),
        most_used_title: String::new(),
        most_used: vec![(s(&initial), 3), (s(&recent), 1)],
        work_folders: [s(&work), String::new(), s(&root.join("missing")), String::new(), String::new()],
        empty_text: String::new(),
        browse_label: "Browse".into(),
        cancel_label: "Cancel".into(),
        remove_label: "Remove".into(),
        clear_label: "Clear".into(),
        initial: initial.clone(),
        path: s(&initial),
        ok_label: "OK".into(),
        original: true,
        selected: 0,
    };
    let items: Vec<_> = original_items(&form).into_iter().map(|(item, _)| item).collect();
    assert_eq!(items, vec![DestinationItem::Initial(s(&initial)), DestinationItem::Recent(s(&recent)), DestinationItem::Work(0, s(&work))], "the default folder is not repeated among the recents; only existing work folders");
    let mut time = 100.0;
    let mut run = |form: &mut DestinationForm, events: Vec<egui::Event>| {
        time += 0.05;
        let raw = egui::RawInput { time: Some(time), events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let mut got = None;
        let _ = ctx.run(raw, |ctx| {
            egui::Area::new(egui::Id::new("dest-test")).fixed_pos(egui::Pos2::ZERO).show(ctx, |ui| got = destination_ui_original(ui, form, false, None, 850.0));
        });
        got
    };
    let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(&mut form, vec![]);
    run(&mut form, vec![]);
    // 둘째 줄(최근, 묶음 첫 줄이라 + 3): 18 ~ 39.
    let at = egui::pos2(100.0, 28.0);
    run(&mut form, vec![egui::Event::PointerMoved(at), press(at, true)]);
    run(&mut form, vec![press(at, false)]);
    assert_eq!((form.selected, form.path.clone()), (1, s(&recent)));
    assert!(matches!(run(&mut form, vec![egui::Event::Key { key: egui::Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]), Some(DestinationAction::Remove(p)) if p == s(&recent)));
    run(&mut form, vec![egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
    assert_eq!((form.selected, form.path.clone()), (2, s(&work)));
    // 원본 cbPath Hint "Alt+D": 경로 칸으로 글쇠 초점.
    let alt = egui::Modifiers::ALT;
    let raw = egui::RawInput { modifiers: alt, events: vec![egui::Event::Key { key: egui::Key::D, physical_key: None, pressed: true, repeat: false, modifiers: alt }], ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        egui::Area::new(egui::Id::new("dest-test")).fixed_pos(egui::Pos2::ZERO).show(ctx, |ui| destination_ui_original(ui, &mut form, false, None, 850.0));
    });
    assert!(ctx.memory(|m| m.has_focus(egui::Id::new("ndir-destination-path"))), "Alt+D focuses the path box");
    // 첫 줄을 두 번 누르면 확인(앞 누르기와 두 번 누르기로 묶이지 않게 쉬었다가).
    for _ in 0..10 {
        run(&mut form, vec![]);
    }
    let first = egui::pos2(100.0, 8.0);
    run(&mut form, vec![egui::Event::PointerMoved(first), press(first, true), press(first, false)]);
    let accepted = run(&mut form, vec![press(first, true), press(first, false)]);
    assert!(matches!(accepted, Some(DestinationAction::Accept)), "double click = OK");
    assert_eq!(form.path, s(&initial));
    fs::remove_dir_all(root).unwrap();
}

/// 원본 MouseDown(0x7d63c8): 두 번 누르기의 두 번째 누름(ssDouble)은 표시를 건드리지 않는다. 미리 보기(형식 3)에는 표시 칸이 없다.
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
    // 미리 보기: 왼쪽 12화소도 그냥 누르기(커서만), Ctrl은 누르는 순간 뒤집는다.
    pane.cursor = 0;
    press(pane, 20.0, egui::Modifiers::NONE, true);
    assert_eq!((pane.cursor, pane.selected.contains(&1)), (1, false));
    press(pane, 22.0, egui::Modifiers::CTRL, true);
    assert!(pane.selected.contains(&1));
    // Windows 기준(WM_LBUTTONDBLCLK): 앞 누름에서 GetDoubleClickTime 안이고 SM_CXDOUBLECLK × SM_CYDOUBLECLK 사각 안인 두 번째 누름만.
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
    // 한 곳을 누르고 곧 다른 곳을 두 번 누르기: egui는 세 번 누르기로 돌려 double_clicked가 아니지만 Windows에는 두 번 누르기다.
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

/// 원본 WMChar(0x7d5a18)·QuickSearch(0x7d5ca8, 0x7d5f9c): 글자 거르기, 찾는 차례, 시간 제한 없음, 다른 길로 커서를 옮기면 지움.
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
    app.typed_search[0].last_input = Some(Instant::now() - Duration::from_secs(10));
    type_text(&mut app, &ctx, "e");
    assert_eq!(app.typed_search[0].query, "alphabe", "no timeout: the text keeps growing");
    type_text(&mut app, &ctx, "z");
    assert_eq!(app.typed_search[0].query, "", "no match even for the new character alone: the text is emptied");
    type_text(&mut app, &ctx, "b");
    assert_eq!((app.typed_search[0].query.as_str(), cursor_name(&app).as_str()), ("b", "beta.txt"));
    // 원본 FormShortCut: 찾는 글이 있으면 Backspace는 빠른 찾기. 비면 원본처럼 0번 항목으로 간다.
    keyboard_frame(&mut app, &ctx, vec![key(egui::Key::Backspace)]);
    assert!(app.typed_search[0].query.is_empty());
    assert_eq!(app.panes[0].cursor, 0);
    assert_eq!(app.panes[0].path, root.join("left"), "Backspace went to the quick search, not to history back");
    // 다른 길로 커서를 옮기면 찾는 글을 지운다(원본 ArrowKey).
    type_text(&mut app, &ctx, "g");
    assert_eq!(app.typed_search[0].query, "g");
    app.panes[0].cursor = index_of(&app, "beta.txt");
    keyboard_frame(&mut app, &ctx, vec![]);
    assert!(app.typed_search[0].query.is_empty());
    // 원본 MouseDown은 같은 항목을 눌러도 찾는 글을 지운다.
    type_text(&mut app, &ctx, "g");
    assert_eq!(app.typed_search[0].query, "g");
    app.panes[0].item_pressed = true;
    keyboard_frame(&mut app, &ctx, vec![]);
    assert!(app.typed_search[0].query.is_empty(), "a press on the found item clears the text");
    // 원본 WMIMEComposition(0x7d584c): 조합 글자를 끝에 붙이고(바뀌면 끝 글자를 바꿈), 결과가 오면 조합을 끝낸다.
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
    // 조합 글이 비면(결과 없이 끝나면) 끝 글자를 지운다.
    keyboard_frame(&mut app, &ctx, vec![ime(egui::ImeEvent::Disabled)]);
    assert_eq!(app.typed_search[0].query, "가");
    assert!(app.typed_search[0].ime_base.is_none());
    fs::remove_dir_all(root).unwrap();
}
/// 원본 주 창은 글쇠를 늘 목록이 받는다. egui 단추가 글쇠 초점을 가져도(Tab·화살표 초점 이동) 목록 글쇠가 끊기지 않고,
/// 초점 단추에서 Space·Enter가 누르기로 바뀌지 않는다.
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
    // Tab은 원본처럼 패널을 바꾸고, egui가 단추에 초점을 주지 않는다. 다시 누르면 다시 바뀐다.
    let active = app.active;
    run(&mut app, vec![key(egui::Key::Tab)]);
    run(&mut app, vec![]);
    assert!(ctx.memory(|m| m.focused()).is_none());
    assert_ne!(app.active, active);
    run(&mut app, vec![key(egui::Key::Tab)]);
    assert_eq!(app.active, active, "Tab keeps switching panes");
    fs::remove_dir_all(root).unwrap();
}
/// 원본 GetList(0x45a34c): 목록은 그 자리에서 읽고(스킨 테마, "읽는 중" 없음), 루트가 아니면 맨 앞에 ".."(고를 수 없고 열면 상위).
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
    // C# 테마는 C# 판처럼 따로 읽는다.
    app.settings.set_text("Theme", "Light");
    app.refresh(0);
    assert!(app.panes[0].loading);
    wait_listing(&mut app, &ctx);
    fs::remove_dir_all(root).unwrap();
}
/// egui-winit의 Copy·Cut·Paste 사건은 원본 단축키 글쇠(Ctrl+C·X·V)로 되돌린다. 클립보드에 글이 없어 egui가 버린 붙여넣기는
/// 창 하위 클래스가 센 만큼 덧붙인다.
#[test]
fn clipboard_shortcuts_reach_the_file_list() {
    let ctrl = egui::Modifiers::CTRL;
    let mut raw = egui::RawInput { modifiers: ctrl, events: vec![egui::Event::Copy, egui::Event::Cut, egui::Event::Paste("x".into())], ..Default::default() };
    super::restore_clipboard_keys(&mut raw, 2, false);
    let keys: Vec<egui::Key> = raw.events.iter().filter_map(|e| if let egui::Event::Key { key, modifiers, pressed: true, .. } = e { assert_eq!(*modifiers, ctrl); Some(*key) } else { None }).collect();
    assert_eq!(keys, [egui::Key::C, egui::Key::X, egui::Key::V, egui::Key::V], "one Paste event + one dropped paste key");
    let mut raw = egui::RawInput { modifiers: egui::Modifiers::SHIFT, events: vec![egui::Event::Cut], ..Default::default() };
    super::restore_clipboard_keys(&mut raw, 1, false);
    let keys: Vec<egui::Key> = raw.events.iter().filter_map(|e| if let egui::Event::Key { key, .. } = e { Some(*key) } else { None }).collect();
    assert_eq!(keys, [egui::Key::Delete, egui::Key::Insert], "Shift+Delete cut, Shift+Insert paste");
    // 파일 목록에서 Ctrl+C는 원본 miEditCopy(= copy 명령)로 간다.
    let (root, ctx, mut app) = fixture("clipkeys", "ko");
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    app.panes[0].cursor = index_of(&app, "alpha.txt");
    let mut raw = egui::RawInput { modifiers: ctrl, events: vec![egui::Event::Copy], ..Default::default() };
    super::restore_clipboard_keys(&mut raw, 0, false);
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
    assert_eq!(app.clipboard.as_ref().map(|(paths, cut)| (paths.len(), *cut)), Some((1, false)), "Ctrl+C copied the cursor item");
    // 원본 단축키 표의 숫자판 글쇠: Num * = miInvertSelection, Num + = miAdvSelect, Num / = miSelectSameExt, Ctrl+Num / = miSelectSameName.
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
/// 원본 GetItemAt·TNXFramePanel: 줄 어디를 눌러도(이름 칸 밖, 날짜 칸 등) 그 항목, 패널 어디를 눌러도 그 패널이 지금 패널.
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
/// 원본 GetItemAt·TNXFramePanel: 줄 어디를 눌러도(이름 칸 밖, 날짜 칸 등) 그 항목, 패널 어디를 눌러도 그 패널이 지금 패널.
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
    // 오른쪽 패널을 지금 패널로 두고 왼쪽 패널 아래쪽 빈 곳을 누르면 왼쪽이 지금 패널.
    app.active = 1;
    let left = app.pane_rects[0];
    click(&mut app, egui::pos2(left.center().x, left.bottom() - 60.0));
    assert_eq!(app.active, 0, "a press on the empty pane area activates it");
    // 날짜 칸 쪽(이름 칸 밖)을 위에서부터 눌러 보면 커서가 그 줄로 간다.
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
fn typed_quick_search_keeps_one_buffer_per_pane() {
    let (root, ctx, mut app) = fixture("quicksearch", "ko");
    // C# 판 빠른 찾기는 C# 테마의 동작이다. 스킨 테마는 아래 original_quick_search_follows_the_file_list_view.
    app.settings.set_text("Theme", "Light");
    app.panes[0].cursor = index_of(&app, "folder1");
    app.panes[0].selected = [index_of(&app, "gamma.log")].into_iter().collect();
    type_text(&mut app, &ctx, "a");
    let first = cursor_name(&app);
    assert!(first == "alpha.txt" || first == "Alphabet.doc");
    type_text(&mut app, &ctx, "lphab");
    assert_eq!(cursor_name(&app), "Alphabet.doc", "typed within the timeout: one query");
    assert_eq!(app.status, "빠른 찾기: alphab");
    assert_eq!(app.panes[0].selected.len(), 1, "marks unchanged (original select-bar move)");
    type_text(&mut app, &ctx, "z");
    assert_eq!(app.typed_search[0].query, "z", "no match: retry with the new character alone");
    assert_eq!(app.status, "빠른 찾기: z · 찾을 수 없음");
    keyboard_frame(&mut app, &ctx, vec![key(egui::Key::Backspace)]);
    assert!(app.typed_search[0].query.is_empty());
    assert_eq!(app.status, "빠른 찾기를 지웠습니다.");
    assert_eq!(app.panes[0].path, root.join("left"), "Backspace went to the quick search, not to the parent folder");

    type_text(&mut app, &ctx, "g");
    app.typed_search[0].last_input = Some(Instant::now() - Duration::from_millis(1500));
    type_text(&mut app, &ctx, "b");
    assert_eq!((app.typed_search[0].query.as_str(), cursor_name(&app).as_str()), ("b", "beta.txt"), "after 1100 ms a new query starts");
    type_text(&mut app, &ctx, "e");
    app.find_next();
    assert_eq!(cursor_name(&app), "beta.txt", "Find Next uses the typed prefix");
    app.active = 1;
    type_text(&mut app, &ctx, "q");
    assert_eq!(app.typed_search[0].query, "be", "each pane keeps its own buffer");
    assert_eq!(app.typed_search[1].query, "q");
    app.active = 0;
    keyboard_frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.typed_search[0].query.is_empty());
    assert_eq!(app.panes[0].selected.len(), 1, "Esc cleared the quick search, not the marks");
    type_text(&mut app, &ctx, "a");
    app.refresh(0);
    assert!(app.typed_search[0].query.is_empty(), "a refresh drops the buffer");
    wait_listing(&mut app, &ctx);
    fs::remove_dir_all(root).unwrap();
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
fn function_key_bar_buttons_follow_buildfunctionkeybar() {
    let (root, ctx, mut app) = fixture("fkeys", "en");
    app.settings.values.insert(
        "FunctionKeyBindings".into(),
        serde_json::json!({"F9":{"Name":"Owned tool","Command":"#Refresh","IsInternalCommand":true}}),
    );
    // The bar sits in the bottom panel like App::update; its height must not creep up frame after frame.
    let mut heights = vec![];
    for _ in 0..6 {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            let panel = egui::TopBottomPanel::bottom("functionbar").show(ctx, |ui| app.function_key_bar(ui, &ctx.clone()));
            heights.push(panel.response.rect.height());
            egui::CentralPanel::default().show(ctx, |_| {});
        });
    }
    assert!(heights.iter().all(|h| *h < 60.0), "the function-key bar keeps one row: {heights:?}");
    // 원본 FUN_00430940: Alt만 = ALTFUNCTIONS, Ctrl만 = CTRLFUNCTIONS("글쇠=이름" 줄), 그 밖은 F1~F12.
    let alt = app.modifier_functions(egui::Modifiers::ALT).unwrap();
    assert_eq!((alt[0].0.as_str(), alt[0].1.as_str(), alt.len()), ("C", "Copy", 8));
    let ctrl = app.modifier_functions(egui::Modifiers::CTRL).unwrap();
    assert_eq!(ctrl.last().map(|(k, n)| (k.as_str(), n.as_str())), Some(("->", "Copy To Right")));
    assert_eq!(ctrl[6].0, "<-");
    assert!(app.modifier_functions(egui::Modifiers::NONE).is_none());
    assert!(app.modifier_functions(egui::Modifiers::ALT | egui::Modifiers::CTRL).is_none());
    // 원본 OD_PROGRAMFILTER(TOpenDialog.Filter "이름|무늬|…")로 프로그램 고르기 상자의 거르개를 만든다.
    let filters = original_dialogs::program_filter(&app.language_pack);
    assert_eq!(filters, [("Program Files(*.exe)".to_owned(), "*.exe".to_owned()), ("All Files(*.*)".to_owned(), "*.*".to_owned())]);
    assert_eq!(original_dialogs::delphi_filter("a|b|c"), [("a".to_owned(), "b".to_owned())], "an unpaired tail is dropped");
    // 원본 FUN_00404890: 7-Zip 오류는 원본 압축 오류 글로, 다른 글은 그대로.
    assert_eq!(app.archive_error_text("7-Zip failed (exit code: 2): ERROR: Wrong password : a"), app.language_pack.text("RES.INVALID_PASSWORD", "?"));
    assert_eq!(app.archive_error_text("7-Zip failed (exit code: 2): ERROR: Missing volume : x.002"), app.language_pack.text("RES.CANNOT_FIND_NEXTVOLUME", "?"));
    assert_eq!(app.archive_error_text("Access is denied. (os error 5)"), "Access is denied. (os error 5)");
    let raw = egui::RawInput { modifiers: egui::Modifiers::ALT, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        egui::TopBottomPanel::bottom("functionbar").show(ctx, |ui| app.function_key_bar(ui, &ctx.clone()));
    });
    app.run_function_key_button("F1", &ctx);
    assert_eq!(app.status, "F1: No action is configured.");
    app.run_function_key_button("F7", &ctx);
    assert!(matches!(&app.original_dialogs[0].kind, original_dialogs::DialogKind::Prompt { .. }), "F7 without a binding: New Folder");
    assert_eq!(app.original_dialogs[0].title, app.language_pack.text("RES.TITLE_NEWFOLDER", "?"));
    app.answer_dialog(original_dialogs::DialogResult::Text(None), &ctx);
    let generation = app.panes[app.active].generation;
    app.run_function_key_button("F9", &ctx);
    assert!(app.panes[app.active].generation > generation, "F9 runs its #Refresh binding");
    wait_listing(&mut app, &ctx);
    fs::remove_dir_all(root).unwrap();
}

/// 원본 miSelectAllClick(0x4311c4)·bSelectAllClick(0x431450)·miInvertSelectionClick(0x43163c)·miSelectSameExt/NameClick·
/// miRestoreLastSelClick(0x43742c)·CopySelection(스킨 테마).
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
    // 메뉴 모두 선택: 파일만 → 파일을 다 골랐으면 폴더까지 → 그 밖에는 지움.
    app.dispatch("selectAll", &ctx);
    assert_eq!((app.panes[0].selected.len(), app.panes[0].selected.contains(&folder)), (files, false));
    app.dispatch("selectAll", &ctx);
    assert!(app.panes[0].selected.contains(&folder), "all files marked: folders are added");
    app.dispatch("selectAll", &ctx);
    assert!(app.panes[0].selected.is_empty(), "everything marked: cleared");
    // 수직 막대 단추: 다 골랐으면 지우고 파일만.
    app.original_button_select_all(false);
    app.original_button_select_all(false);
    assert!(app.panes[0].selected.contains(&folder));
    app.original_button_select_all(false);
    assert_eq!((app.panes[0].selected.len(), app.panes[0].selected.contains(&folder)), (files, false));
    // 반전: 파일만(폴더는 selectfolder일 때만).
    app.panes[0].selected = [index_of(&app, "beta.txt")].into_iter().collect();
    app.dispatch("invertSelection", &ctx);
    assert!(!app.panes[0].selected.contains(&folder) && !app.panes[0].selected.contains(&index_of(&app, "beta.txt")));
    assert_eq!(app.panes[0].selected.len(), files - 1);
    // 같은 확장자: 하나라도 안 골랐으면 모두 고르고(다른 표시는 그대로), 다 골랐으면 푼다.
    app.panes[0].selected = [index_of(&app, "gamma.log")].into_iter().collect();
    app.panes[0].cursor = index_of(&app, "alpha.txt");
    app.dispatch("selectSameExt", &ctx);
    assert_eq!(marked(&app), ["alpha.txt", "beta.txt", "gamma.log"]);
    app.dispatch("selectSameExt", &ctx);
    assert_eq!(marked(&app), ["gamma.log"]);
    app.dispatch("selectSameName", &ctx);
    assert_eq!(marked(&app), ["ALPHA.md", "alpha.log", "alpha.txt", "gamma.log"]);
    // 선택 복원은 파일 작업을 시작할 때 남긴 표시(CopySelection). 선택 명령은 남기지 않는다.
    assert!(app.last_selection[0].is_empty());
    app.dispatch("restoreSelection", &ctx);
    assert_eq!(marked(&app), ["ALPHA.md", "alpha.log", "alpha.txt", "gamma.log"], "nothing remembered: unchanged");
    app.dispatch("description", &ctx);
    assert_eq!(app.last_selection[0].len(), 4);
    app.original_dialogs.clear();
    app.panes[0].selected.clear();
    app.dispatch("restoreSelection", &ctx);
    assert_eq!(marked(&app), ["ALPHA.md", "alpha.log", "alpha.txt", "gamma.log"]);
    // 원본 miCalculateSelectedFolderSizeClick(0x4457d4): 고른 폴더의 크기를 목록 크기 칸으로 세고 표시를 지운다(요약 상자 없음).
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

/// 원본 miSortByClick(0x431c88): 같은 정렬 종류를 다시 고르면 내림차순 전환. miToTempClick(0x4330b0): GetTempPath + GetLongPathName,
/// 실패하면 그대로. (스킨 테마. C# 테마는 같은 종류를 다시 골라도 그대로.)
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
    app.settings.set_text("Theme", "Light");
    app.dispatch("sortName", &ctx);
    app.dispatch("sortName", &ctx);
    assert!(app.settings.flag("LeftSortDescending", false), "C# themes keep C# SetSortMode");
    app.settings.set_text("Theme", "Black");
    assert!(app.current_skin().is_some());
    // To Temp: 실패하면 움직이지 않는다(시험 기본 가짜는 None).
    let before = app.panes[0].path.clone();
    app.dispatch("temp", &ctx);
    assert_eq!(app.panes[0].path, before);
    app.system.temp_folder = system_temp;
    app.dispatch("temp", &ctx);
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path, std::env::temp_dir());
    fs::remove_dir_all(root).unwrap();
}

/// 원본 DrawListItem(0x7d8f58)·TNXShellImageList.Draw(0x7b8ebc): 잘라낸 항목·숨김 속성은 아이콘을 ILD_BLEND50(흰색)으로 흐리게.
/// (잘라내기 명령은 실제 클립보드를 쓰므로 여기서는 표시만 넣는다.)
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
    // C# 테마는 흐리게 그리지 않는다.
    app.settings.set_text("Theme", "Light");
    assert!(!app.row_ghosted(0, alpha));
    app.settings.set_text("Theme", "Black");
    // 새 목록 = 새 항목: 잘라낸 표시가 없어진다.
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    assert!(app.panes[0].cut_marks.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pane_commands_follow_the_csharp_handlers() {
    let (root, ctx, mut app) = fixture("panecmd", "ko");
    // C# 판 선택 명령은 C# 테마의 동작이다. 스킨 테마는 original_selection_commands_follow_the_handlers.
    app.settings.set_text("Theme", "Light");
    let left = root.join("left");
    fs::write(left.join("alpha.log"), b"x").unwrap();
    fs::write(left.join("ALPHA.md"), b"x").unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    // Select All leaves folders out unless SelectFolders is on.
    app.dispatch("selectAll", &ctx);
    assert!(!app.panes[0].selected.contains(&index_of(&app, "folder1")));
    assert_eq!(app.panes[0].selected.len(), app.panes[0].entries.len() - 2, "no folder1, no \"..\"");
    app.settings.set_flag("SelectFolders", true);
    app.dispatch("selectAll", &ctx);
    assert_eq!(app.panes[0].selected.len(), app.panes[0].entries.len() - 1, "everything but \"..\"");
    // Same extension / same base name, case-insensitive, anchored on the cursor; the old marks can be restored.
    app.panes[0].selected = [index_of(&app, "beta.txt")].into_iter().collect();
    app.panes[0].cursor = index_of(&app, "alpha.txt");
    app.dispatch("selectSameExt", &ctx);
    let marked = |app: &App| { let mut v: Vec<String> = app.panes[0].selected.iter().map(|i| app.panes[0].entries[*i].name.clone()).collect(); v.sort(); v };
    assert_eq!(marked(&app), ["alpha.txt", "beta.txt"]);
    app.dispatch("selectSameName", &ctx);
    assert_eq!(marked(&app), ["ALPHA.md", "alpha.log", "alpha.txt"]);
    app.dispatch("restoreSelection", &ctx);
    assert_eq!(marked(&app), ["alpha.txt", "beta.txt"]);
    // Hidden toggle, view style text and status.
    app.dispatch("toggleHidden", &ctx);
    assert!(app.settings.flag("LeftShowHiddenFiles", false) && app.settings.flag("ShowHiddenFiles", false));
    assert_eq!(app.status, "왼쪽 패널 숨김 항목 표시");
    wait_listing(&mut app, &ctx);
    app.dispatch("viewNames", &ctx);
    assert_eq!((app.settings.text("LeftViewStyle", ""), app.status.as_str()), ("Names".to_string(), "왼쪽 패널 보기: Names"));
    // Last destination missing: an information box, no navigation.
    app.dispatch("goLastDestination", &ctx);
    assert_eq!(app.msgboxes[0].text, "아직 사용할 수 있는 마지막 대상 폴더가 없습니다.");
    assert_eq!((app.msgboxes[0].title.as_str(), app.msgboxes[0].flags), ("NDIR", ndir::msgbox::MB_OK | ndir::msgbox::MB_ICONASTERISK));
    app.answer_message_box(1, &ctx);
    // Folder size: one summary box, nested roots counted once.
    fs::write(left.join("folder1/inner.bin"), [0u8; 10]).unwrap();
    app.panes[0].selected = [index_of(&app, "folder1"), index_of(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("calculateFolderSize", &ctx);
    let end = Instant::now() + Duration::from_secs(10);
    while app.msgboxes.is_empty() {
        assert!(Instant::now() < end, "folder size timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.msgboxes[0].title, "선택 항목 크기");
    assert_eq!(app.msgboxes[0].text, "폴더: 1개\n파일: 2개\n전체 크기: 19 bytes (19 B)");
    assert_eq!(app.msgboxes[0].flags, ndir::msgbox::MB_OK | ndir::msgbox::MB_ICONASTERISK);
    wait_job(&mut app, &ctx);
    // Go to root: the drive root of the listed folder.
    app.dispatch("goRoot", &ctx);
    wait_listing(&mut app, &ctx);
    assert_eq!(app.panes[0].path.parent(), None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn details_list_follows_the_csharp_grid() {
    let (root, ctx, mut app) = fixture("details", "ko");
    // C# 격자(종류 열)는 C# 테마의 모양이다. 스킨 테마는 tools_tests가 본다.
    app.settings.set_text("Theme", "Light");
    let left = root.join("left");
    compat::set_description(&left, "beta.txt", "베타 설명", true).unwrap();
    fs::write(left.join("hidden.txt"), b"h").unwrap();
    native::set_attributes(&left.join("hidden.txt"), false, true).unwrap();
    fs::write(left.join("big.bin"), vec![0u8; 3000]).unwrap();
    app.refresh(0);
    wait_listing(&mut app, &ctx);
    assert!(app.panes[0].entries.iter().all(|e| e.name != "hidden.txt"), "hidden items are skipped while hidden display is off");
    let row = index_of(&app, "beta.txt");
    let cells = app.panes[0].view.cells[row].clone();
    assert_eq!(cells[DetailColumn::Name.index()], "beta", "the extension has its own column");
    assert_eq!(cells[DetailColumn::Extension.index()], ".txt");
    assert_eq!(cells[DetailColumn::Kind.index()], "TXT 파일");
    assert_eq!(cells[DetailColumn::Size.index()], "8 B");
    assert_eq!(cells[DetailColumn::Attributes.index()], ndir::listing::attribute_text(ndir::native::attributes(&left.join("beta.txt")).unwrap()));
    assert_eq!(cells[DetailColumn::Description.index()], "베타 설명");
    let folder = app.panes[0].view.cells[index_of(&app, "folder1")].clone();
    assert_eq!((folder[DetailColumn::Size.index()].as_str(), folder[DetailColumn::Kind.index()].as_str()), ("", "폴더"));
    assert_eq!(app.panes[0].view.cells[index_of(&app, "big.bin")][DetailColumn::Size.index()], "2.9 KB");
    app.settings.set_flag("AttachExtensionToName", true);
    app.settings.values.insert("CharacterCase".into(), serde_json::json!(2));
    app.compute_view(0);
    assert_eq!(app.panes[0].view.cells[row][DetailColumn::Name.index()], "BETA.TXT");
    assert!(app.detail_columns(true).iter().all(|(c, _, _)| *c != DetailColumn::Extension));
    app.settings.set_flag("AttachExtensionToName", false);
    app.settings.values.insert("CharacterCase".into(), serde_json::json!(0));

    // Sort by size keeps the marks and the cursor on the same items; folders stay first.
    app.panes[0].selected = [index_of(&app, "gamma.log")].into_iter().collect();
    app.panes[0].cursor = index_of(&app, "alpha.txt");
    app.dispatch("sort:Size", &ctx);
    assert_eq!(app.settings.text("LeftSortMode", ""), "Size");
    let order: Vec<String> = app.panes[0].entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!((order[0].as_str(), order[1].as_str()), ("..", "folder1"), "the parent item stays first, then folders");
    assert_eq!(order.last().unwrap(), "big.bin");
    assert_eq!(cursor_name(&app), "alpha.txt");
    assert_eq!(app.panes[0].selected.iter().map(|i| app.panes[0].entries[*i].name.clone()).collect::<Vec<_>>(), ["gamma.log"]);
    app.dispatch("sortDescending", &ctx);
    assert_eq!(app.panes[0].entries[2].name, "big.bin", "descending: the largest file first after \"..\" and the folders");

    // Groups by extension with headers; the folder group comes first.
    app.dispatch("groupMode:Extension", &ctx);
    let headers: Vec<String> = app.panes[0].view.rows.iter().filter_map(|r| if let DetailRow::Header(h) = r { Some(h.clone()) } else { None }).collect();
    assert_eq!(headers[0], app.language_pack.text("RES.FOLDERS", "?"));
    assert!(headers.contains(&".txt".to_string()) && headers.contains(&".log".to_string()));
    app.dispatch("groupShowName", &ctx);
    assert!(app.panes[0].view.rows.iter().all(|r| matches!(r, DetailRow::Item(_))), "group names hidden");
    app.dispatch("groupMode:Bogus", &ctx);
    assert_eq!(app.settings.text("LeftGroupMode", ""), "Extension", "unknown modes are ignored");
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| app.pane_ui(ui, 0, &ctx.clone()));
    });
    let _ = native::set_attributes(&left.join("hidden.txt"), false, false);
    fs::remove_dir_all(root).unwrap();
}
