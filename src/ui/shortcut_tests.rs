//! C# HandleKeyboardAsync 순서(원본 단축키 표 → NDR 단축키 → 동작 정의), 사용자 단축키 우선, CycleFileSelection, 작업 폴더,
//! 즐겨찾기 글쇠, Esc 동작, 열 개수, 드라이브 마지막 폴더를 Rust UI에서 확인한다. 한국어와 영어만, 격리 폴더만 쓴다.
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
/// 글쇠 하나를 누른 장면에서 목록 글쇠 처리만 돌린다.
fn press(app: &mut App, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    // 같은 장면에서 떼어야 다음 누름이 반복(repeat)으로 보이지 않는다.
    let down = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let up = egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers };
    let raw = egui::RawInput { events: vec![down, up], modifiers, ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
    wait(app, ctx);
}
const NONE: egui::Modifiers = egui::Modifiers::NONE;

/// 원본 miSelect(Spacebar, miSelectClick 0x4311a4) = SelectCurrAndMove(0x7dca04): 폴더·파일 모두 커서 항목을 고르거나 풀고
/// 다음 항목으로 간다. ".."(Select 0x7dc834)는 고르지 않는다. Ctrl+Shift+화살표는 원본 ArrowKey처럼 선택을 지우지 않고
/// 옮기기 전 자리부터 새 자리까지 더하고, Shift만 누르면 선택을 지우고 Shift를 누른 자리부터 고른다.
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
    // 메뉴 항목(편집 메뉴 "선택")도 같은 동작.
    app.panes[0].selected.clear();
    app.panes[0].cursor = 1;
    app.dispatch("select", &ctx);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![1], 2));
    // Ctrl+Shift: 지우지 않고 더한다. 그다음 Shift만: Shift를 누른 자리(2)부터 다시.
    let ctrl_shift = egui::Modifiers { ctrl: true, shift: true, ..Default::default() };
    app.panes[0].selected = [3].into_iter().collect();
    app.panes[0].cursor = 2;
    press(&mut app, &ctx, egui::Key::ArrowUp, ctrl_shift);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![1, 2, 3], 1), "Ctrl+Shift adds the moved range");
    press(&mut app, &ctx, egui::Key::ArrowUp, egui::Modifiers::SHIFT);
    assert_eq!((marks(&app), app.panes[0].cursor), (vec![1, 2], 0), "Shift selects from where Shift went down");
    fs::remove_dir_all(root).unwrap();
}

/// 두 패널을 한 번 그린다(그린 목록이 쪽 정보를 남긴다).
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

/// 원본 ArrowKey·UpdateDisplayNames·SetItemIndex: 이름 보기는 열로 흐르는 목록이라 오른쪽·왼쪽이 한 열(한 열의 줄 수)씩,
/// PageDown은 보이는 쪽의 끝 항목. Shift로 옮기면 Shift를 누른 자리부터 다시 고른다(되돌아오면 줄어든다).
/// 자세히 보기에서 End로 옮기면 끝 항목이 보이게 넘긴다.
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
    // Shift: Shift를 누른 자리(지금 커서)부터 커서까지.
    press(&mut app, &ctx, egui::Key::Home, NONE);
    let start = app.panes[0].cursor;
    press(&mut app, &ctx, egui::Key::ArrowDown, SHIFT);
    press(&mut app, &ctx, egui::Key::ArrowDown, SHIFT);
    press(&mut app, &ctx, egui::Key::ArrowUp, SHIFT);
    let expected: Vec<usize> = [start, start + 1].into_iter().filter(|&i| !app.panes[0].entries[i].is_parent()).collect();
    assert_eq!(app.panes[0].selected.iter().copied().collect::<Vec<_>>(), expected, "the range shrinks back (\"..\" is never marked)");
    // 자세히: End 뒤 그리면 끝 항목까지 보인다.
    app.settings.set_text("LeftViewStyle", "Details");
    draw_panes(&mut app, &ctx);
    press(&mut app, &ctx, egui::Key::End, NONE);
    assert_eq!(app.panes[0].cursor, count - 1);
    draw_panes(&mut app, &ctx);
    draw_panes(&mut app, &ctx);
    let page = app.panes[0].page;
    assert_eq!((page.style, page.last), (super::list_keys::ListStyle::Details, count - 1), "End reveals the last item: {page:?}");
    // 원본 SetItemIndex(형식 2, 쪽 단위 = 처음 값): 맨 위 항목은 커서가 든 쪽의 처음 줄.
    assert_eq!(page.top, ((count - 1) / page.rows) * page.rows, "page-unit reveal starts the page on a boundary: {page:?}");
    fs::remove_dir_all(root).unwrap();
}
/// 원본 ArrowKey Ctrl 글쇠: Ctrl+PageUp 첫 항목, Ctrl+PageDown 마지막 폴더, Ctrl+Home 첫 파일, Ctrl+End 마지막 파일.
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
/// 원본 UpdateDisplayAuto: 좁아서 한 열만 들어가도 한 열에 다 들어가지 않으면 한 열 너비로 가로로 넘기는 목록이다.
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
/// 가짜 드라이브 루트(격리 임시 폴더 둘).
fn fake_drive_roots() -> Vec<PathBuf> {
    ["A", "B"].iter().map(|l| std::env::temp_dir().join(format!("ndr-keys-fake-drive-{l}"))).collect()
}
fn fake_drive_info(_: &std::path::Path) -> Option<(String, u32, Option<(u64, u64)>)> {
    Some(("Disk".into(), 3, Some((25, 100))))
}

/// 원본 FUN_00486474·ArrowKey·Select·SelectAll: "목록에 디스크 드라이브 목록 보기"가 켜져 있으면 일반 폴더 목록 끝에 드라이브 항목.
/// 정렬과 상관없이 끝에 있고, 폴더·파일 수에 들지 않으며 고르거나 작업 대상이 되지 않는다. Ctrl+End는 드라이브 앞 마지막 파일,
/// [ ]는 드라이브 사이를 옮기고, 열면 그 드라이브로 간다. 설정을 끄면 덧붙이지 않는다.
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
    // Alt+U(원본 miSelectAll = CycleFileSelection): 없으면 파일만(SelectFolders 끔), 파일이 다 표시되고 폴더가 하나도 없으면 폴더도, 그다음은 모두 해제.
    press(&mut app, &ctx, egui::Key::U, ALT);
    assert_eq!(names(&app, 0), ["a.txt", "b.txt"]);
    press(&mut app, &ctx, egui::Key::U, ALT);
    assert_eq!(names(&app, 0), ["a.txt", "b.txt", "folder1"]);
    press(&mut app, &ctx, egui::Key::U, ALT);
    assert!(app.panes[0].selected.is_empty());
    // Esc: EscapeKeyAction이 없으면 아무것도 하지 않고, 3이면 모두 해제.
    app.panes[0].selected.insert(0);
    press(&mut app, &ctx, egui::Key::Escape, NONE);
    assert_eq!(app.panes[0].selected.len(), 1, "no EscapeKeyAction: Esc does nothing");
    app.settings.values.insert("EscapeKeyAction".into(), serde_json::json!(3));
    press(&mut app, &ctx, egui::Key::Escape, NONE);
    assert!(app.panes[0].selected.is_empty());
    // 작업 폴더: Ctrl+1 저장(원본 miWorkFolder1), Alt+1 이동, 저장 안 한 Alt+2는 알림.
    press(&mut app, &ctx, egui::Key::Num1, CTRL);
    assert_eq!(app.status, "작업 폴더 1 저장됨");
    assert_eq!(app.settings.strings("WorkingFolders")[0], left.to_string_lossy());
    // 원본 FUN_0042fff58: 이미 지정된 작업 폴더면 MSG_UNSETWORKFOLDER(지금 값) 예/아니요. 아니요면 그대로, 예면 바꾼다.
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
    assert_eq!(app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone())), Some(("NDR".to_owned(), "작업 폴더 2이 설정되지 않았습니다.".to_owned())));
    app.answer_message_box(1, &ctx);
    // Alt+Left = 뒤로(원본 miHistoryBack): 작업 폴더로 옮기기 전 폴더.
    press(&mut app, &ctx, egui::Key::ArrowLeft, ALT);
    assert_eq!(app.panes[0].path, left.join("folder1"));
    // 마지막 드라이브 폴더(SaveDriveFolders): 이동하면 기억하고, 드라이브 루트로 가면 되살린다.
    let drive_root = PathBuf::from(left.to_string_lossy()[..3].to_owned());
    assert_eq!(app.restore_drive_folder(drive_root.clone()), left.join("folder1"));
    app.settings.set_flag("SaveDriveFolders", false);
    assert_eq!(app.restore_drive_folder(drive_root.clone()), drive_root);
    // 열 개수: column:N(0~19로 자름), 원본 이름 column3도 같다.
    app.dispatch("column:25", &ctx);
    assert_eq!(app.settings.values["LeftColumnCount"], 19);
    app.dispatch("column3", &ctx);
    assert_eq!(app.settings.values["LeftColumnCount"], 3);
    fs::remove_dir_all(root).unwrap();
}

/// 원본 즐겨찾기 메뉴(FUN_0042eedc): 0~9번 Shift+F1~F10, 10~19번 Shift+Ctrl+F1~F10. 빈 칸의 글쇠는 아무것도 하지 않는다.
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

/// 원본 [FuncKey] F3=#CopyTo, F4=#MoveTo → TfSelectDest. 글쇠만으로(F3, Enter) 복사·이동이 끝나야 한다.
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
    // 현재 패널: 경로 칸에 "." → 같은 폴더.
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
    // 원본 FUN_004134e0: 대상(자기 자신)이 있으므로 TfFileExists. 이름 바꾸기 → 새 이름으로 복사.
    wait_for(&mut app, &ctx, &|app| app.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::FileConflict(_))));
    app.answer_dialog(DialogResult::FileConflict(Some((ndr::fsops::ConflictChoice::Rename, false))), &ctx);
    wait_for(&mut app, &ctx, &|app| app.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::Prompt { .. })));
    app.answer_dialog(DialogResult::Text(Some("a2.txt".into())), &ctx);
    settle(&mut app, &ctx);
    assert_eq!(fs::read(left.join("a2.txt")).unwrap(), b"a", "copied in the current pane under the new name");
    // 덮어쓰기는 자기 자신을 망가뜨리지 않고 건너뛴다(고친 것).
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
    // workSet은 영어 상태 글, 작업 폴더 메뉴의 원본 이름 saveWorkFolder2/openWorkFolder2도 같다.
    app.dispatch("saveWorkFolder2", &ctx);
    assert_eq!(app.status, "Work folder 2 saved");
    app.navigate(0, root.join("right"), true);
    wait(&mut app, &ctx);
    app.dispatch("openWorkFolder2", &ctx);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].path, root.join("left"));
    app.dispatch("workOpen:4", &ctx);
    assert_eq!(app.msgboxes.first().map(|m| m.text.clone()), Some("Work folder 5 is not set.".to_owned()));
    fs::remove_dir_all(root).unwrap();
}


/// C# HandleKeyboardAsync는 IsRepeat을 거르지 않는다: 누르고 있는 글쇠의 반복 입력도 명령을 다시 실행한다.
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

/// 원본 단축키 표(shortcut.ini 기본 114줄)의 모든 대상이 NDR 동작으로 이어지는가: 메뉴 항목은 ORIGINAL_ACTIONS(또는 C# 특수 항목),
/// 내부 명령은 parity::internal_command 표.
#[test]
fn every_original_shortcut_target_has_an_ndr_action() {
    let shortcuts = compat::ShortcutMap::parse(None);
    // C# 특수 항목, miFind(original_shortcut이 찾기 상자 또는 C# 빠른 찾기 칸), miSelect(Space: NDR 표시 전환 = 원본 miSelectClick).
    let special = ["miSelectAll", "miSearchFile", "miArcMove", "miArcInstall", "miFind", "miSelect"];
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

/// 원본 miFind(Ctrl+F): 원본 도구 모음(빠른 찾기 칸 없음)이면 찾기 상자(TfGetText TITLE_FIND). miColAuto(Ctrl+Alt+0): 열 수 자동(0).
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


/// 메뉴는 원본 항목만: 원본 이름(MENU_CAPTIONS·ORIGINAL_ACTIONS)이 없는 명령(C# 판에만 있던 것)은 메뉴 표에 없다.
#[test]
fn menus_have_only_original_commands() {
    let foreign: Vec<&str> = menu_data::MENUS.iter().flat_map(|(_, _, _, items)| items.iter().map(|(id, _, _)| *id)).filter(|id| *id != "-" && App::csharp_only_menu_item(id)).collect();
    assert!(foreign.is_empty(), "{foreign:?}");
    // ORIGINAL_HIDDEN = 원본 DFM에서 Visible=False인 항목 가운데 NDR 메뉴에 있는 것, 빠짐없이.
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

/// 원본 miSkin(FUN_0043e580: skin\ 폴더마다 항목)·miLanguage(FUN_00441830, 끝에 miMoreLanguages).
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

/// 원본 ApplicationEvents1Message(0x442984): 마우스 XBUTTON1(뒤로)·XBUTTON2(앞으로) = HistoryBack·HistoryForward.
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
