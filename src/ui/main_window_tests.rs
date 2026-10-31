//! C# MainWindow 틀(제목 줄, 즐겨찾기 줄, 패널 머리 줄·탭·주소 표시줄·정보 표시줄)을 Rust UI에서 확인한다. 한국어만.
//! 격리된 임시 폴더만 쓴다.
use super::*;
use std::fs;

fn fixture(label: &str) -> (PathBuf, egui::Context, App) {
    let root = std::env::temp_dir().join(format!("ndr-main-{}-{}-{label}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let (left, right) = (root.join("left"), root.join("right"));
    fs::create_dir_all(left.join("folder")).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("a.txt"), b"aaaa").unwrap();
    fs::write(left.join("b.txt"), b"bb").unwrap();
    fs::write(left.join("c.log"), b"c").unwrap();
    let ctx = egui::Context::default();
    let options = Options { profile: Some(root.join("profile")), left: Some(left), right: Some(right), language: Some("ko".into()), ..Default::default() };
    let mut app = App::from_context(&ctx, options);
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
/// 메뉴·즐겨찾기 줄·두 패널을 한 번 그린다(그리기 중 오류가 없는지).
fn draw(app: &mut App, ctx: &egui::Context) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| app.menu(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |columns| {
                app.pane_ui(&mut columns[0], 0, ctx);
                app.pane_ui(&mut columns[1], 1, ctx);
            });
        });
    });
}

#[test]
fn main_menu_follows_the_original_tfmain_dropdowns() {
    let (root, ctx, mut app) = fixture("menus");
    // 원본 TfMain 메뉴 단추 tbFile … tbHelp(글 " 파일(&F) "에서 공백·& 뺌)와 그 순서.
    let titles: Vec<String> = menu_data::MENUS.iter().map(|(key, ko, _, _)| app.language_pack.text(key, ko).trim().replace('&', "")).collect();
    assert_eq!(titles, ["파일(F)", "폴더(O)", "편집(E)", "보기(W)", "시스템(S)", "압축(A)", "네트워크(N)", "도구(T)", "도움말(H)"]);
    let ids = |index: usize| menu_data::MENUS[index].3.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
    // pmFile 첫 항목·마지막 항목, pmFolder의 즐겨찾기·작업 폴더, pmView의 하위 메뉴, pmTools의 분할·합치기.
    assert_eq!((ids(0)[0], *ids(0).last().unwrap()), ("open", "exit"));
    assert!(ids(1).starts_with(&["folderTree", "favoritesMenu"]) && ids(1).contains(&"@workFolders") && ids(1).last() == Some(&"refresh"));
    assert!(["@listStyle", "@columns", "@sort", "@filter"].iter().all(|s| ids(3).contains(s)));
    assert!(ids(7).contains(&"split") && ids(7).contains(&"join") && ids(7).contains(&"cleanup"));
    // 원본 항목 글: Menu.miFind(MENU_CAPTIONS), Menu.miOpen(ORIGINAL_ACTIONS), 없으면 C# 글.
    assert_eq!(app.menu_label("find", "?"), app.language_pack.text("Menu.miFind", "?").replace('&', ""));
    assert_eq!(app.menu_label("open", "?"), app.language_pack.text("Menu.miOpen", "?").replace('&', ""));
    assert_eq!(app.menu_label("createZip", "ZIP 만들기…"), "ZIP 만들기…");
    // 원본 메뉴 항목 오른쪽 단축키 글(TMenuItem.ShortCut → shortcut.ini → ShortCutToText). C# 판에만 있는 항목은 없음.
    let shortcuts = compat::ShortcutMap::parse(None);
    let code = |name: &str| shortcuts.menu_shortcuts.get(name).copied().unwrap_or(0);
    let open_with_parameters = menu_data::ORIGINAL_ACTIONS.iter().find(|(name, _)| *name == "miParameterOpen").map(|(_, id)| *id).unwrap();
    assert_eq!(App::menu_shortcut_text(open_with_parameters, &shortcuts), compat::shortcut_to_text(code("miParameterOpen")));
    assert!(!App::menu_shortcut_text(open_with_parameters, &shortcuts).is_empty());
    assert_eq!(App::menu_shortcut_text("createZip", &shortcuts), "");
    // 원본 miSelect = Space(지금 항목 표시 전환), 보기 메뉴의 정보 표시줄 항목은 체크 항목.
    app.active = 0;
    app.panes[0].cursor = 1;
    app.dispatch("select", &ctx);
    assert!(app.panes[0].selected.contains(&1));
    assert_eq!(app.menu_checked("toggleFileInfoBar"), Some(true));
    draw(&mut app, &ctx);
    // 원본 TToolBar CM_DIALOGCHAR: 단축키가 아닌 Alt+글자(폴더(&O) → Alt+O)는 그 메뉴 단추의 펼침을 연다.
    assert!(!egui::Popup::is_any_open(&ctx));
    let alt_o = egui::Event::Key { key: egui::Key::O, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
    let raw = egui::RawInput { events: vec![alt_o], screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        app.keyboard(ctx);
        egui::TopBottomPanel::top("menu").show(ctx, |ui| app.menu(ui));
    });
    assert!(app.menu_access.is_none(), "consumed by the menu bar");
    assert!(egui::Popup::is_any_open(&ctx), "Alt+O opens a menu");
    // 메뉴가 열린 동안 글자는 패널로 가지 않고 그 메뉴의 접근 글자 항목을 고른다: 폴더 메뉴 "&V. 즐겨찾기..." → 즐겨찾기 드롭다운.
    assert_eq!(app.menu_access_letter("favoritesMenu"), Some('v'));
    let cursor = app.panes[0].cursor;
    let typed = vec![egui::Event::Key { key: egui::Key::V, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }, egui::Event::Text("v".into())];
    let raw = egui::RawInput { events: typed, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        app.keyboard(ctx);
        egui::TopBottomPanel::top("menu").show(ctx, |ui| app.menu(ui));
    });
    assert!(app.popup.is_some(), "the favorites dropdown opened from the menu letter");
    assert_eq!(app.panes[0].cursor, cursor);
    app.popup = None;
    // Windows 메뉴 줄: 한 메뉴(파일)가 열린 동안 다른 제목(폴더) 위로 마우스를 옮기면 그 메뉴로 바뀐다(누르지 않아도).
    let favorites = app.menu_label("favoritesMenu", "?");
    let screen = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0)));
    let texts = |output: &egui::FullOutput| output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Text(t) => Some(t.galley.text().to_owned()), _ => None }).collect::<Vec<_>>();
    let alt_f = egui::Event::Key { key: egui::Key::F, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
    let frame = |app: &mut App, events: Vec<egui::Event>| {
        ctx.run(egui::RawInput { events, screen_rect: screen, ..Default::default() }, |ctx| {
            app.keyboard(ctx);
            egui::TopBottomPanel::top("menu").show(ctx, |ui| app.menu(ui));
        })
    };
    frame(&mut app, vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }]);
    frame(&mut app, vec![alt_f]);
    let opened = frame(&mut app, vec![]);
    assert!(egui::Popup::is_any_open(&ctx) && !texts(&opened).contains(&favorites), "the File menu is open");
    let mut switched = false;
    for x in (10..400).step_by(10) {
        let output = frame(&mut app, vec![egui::Event::PointerMoved(egui::pos2(x as f32, 10.0))]);
        if texts(&output).contains(&favorites) {
            switched = true;
            break;
        }
    }
    assert!(switched, "hovering the Folder title while a menu is open opens the Folder menu");
    fs::remove_dir_all(root).unwrap();
}

fn menu_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        app.keyboard(ctx);
        egui::TopBottomPanel::top("menu").show(ctx, |ui| app.menu(ui));
    });
}
fn typed(key: egui::Key, text: &str) -> Vec<egui::Event> {
    vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }, egui::Event::Text(text.into())]
}

/// Windows 메뉴: 같은 접근 글자의 항목이 둘 이상이면(원본 보기 메뉴의 C = "&C. 기능키 표시줄"·"&C. 열 보기") 글자를 칠 때마다
/// 그 사이로 선택(초점)만 옮기고 실행하지 않는다. Enter가 선택한 항목을 실행한다.
#[test]
fn shared_access_letters_move_between_items_like_windows_menus() {
    let (root, ctx, mut app) = fixture("shared-letters");
    draw(&mut app, &ctx);
    app.active = 0;
    let alt_w = egui::Event::Key { key: egui::Key::W, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
    menu_frame(&mut app, &ctx, vec![alt_w]);
    menu_frame(&mut app, &ctx, Vec::new());
    menu_frame(&mut app, &ctx, Vec::new());
    let bar = app.settings.flag("ShowFunctionBar", true);
    let c_targets = |app: &App| app.menu_targets.borrow().iter().filter(|(c, _)| *c == 'c').map(|(_, id)| *id).collect::<Vec<_>>();
    assert_eq!(c_targets(&app).len(), 2, "function key bar and the column submenu");
    let (first, second) = (c_targets(&app)[0], c_targets(&app)[1]);
    menu_frame(&mut app, &ctx, typed(egui::Key::C, "c"));
    assert_eq!(ctx.memory(|m| m.focused()), Some(first));
    assert_eq!(app.settings.flag("ShowFunctionBar", true), bar, "not run");
    menu_frame(&mut app, &ctx, typed(egui::Key::C, "c"));
    assert_eq!(ctx.memory(|m| m.focused()), Some(second));
    menu_frame(&mut app, &ctx, typed(egui::Key::C, "c"));
    assert_eq!(ctx.memory(|m| m.focused()), Some(first), "wraps around");
    assert!(egui::Popup::is_any_open(&ctx), "the menu stays open");
    // Enter가 선택한 항목(기능키 표시줄)을 실행한다.
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    menu_frame(&mut app, &ctx, vec![enter]);
    assert_ne!(app.settings.flag("ShowFunctionBar", true), bar);
    // 글자가 하나뿐인 항목은 그대로 바로 실행한다(위 submenu_access_letters… 시험).
    fs::remove_dir_all(root).unwrap();
}

/// 원본 하위 메뉴의 접근 글자: 메뉴가 열린 동안 하위 메뉴 이름의 글자(보기 "&C. 열 보기", 폴더 "&G. 작업 폴더로 가기")를 치면
/// Windows 메뉴처럼 그 하위 메뉴가 열리고, 이어서 친 글자("&3. 열로 보기", "작업폴더 &1")가 그 안의 항목을 고른다.
#[test]
fn submenu_access_letters_open_the_submenu_and_pick_its_item() {
    let (root, ctx, mut app) = fixture("submenu-letters");
    draw(&mut app, &ctx);
    app.active = 0;
    let alt = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
    menu_frame(&mut app, &ctx, vec![alt(egui::Key::W)]);
    assert!(egui::Popup::is_any_open(&ctx), "Alt+W opens the View menu");
    // 보기 메뉴 "&A. 정렬" → "&S. 크기순"(원본 보기 메뉴의 C는 기능키 표시줄과 열 보기가 같이 쓴다).
    menu_frame(&mut app, &ctx, typed(egui::Key::A, "a"));
    menu_frame(&mut app, &ctx, Vec::new());
    assert_ne!(app.settings.text("LeftSortMode", ""), "Size");
    menu_frame(&mut app, &ctx, typed(egui::Key::S, "s"));
    assert_eq!(app.settings.text("LeftSortMode", ""), "Size", "A opened Sort by, S picked size");
    // 폴더 메뉴: G → 작업 폴더로 가기, 1 → 작업 폴더 1.
    let target = root.join("right");
    app.settings.set_strings("WorkingFolders", vec![target.to_string_lossy().into_owned()]);
    for _ in 0..3 {
        menu_frame(&mut app, &ctx, Vec::new());
    }
    menu_frame(&mut app, &ctx, vec![alt(egui::Key::O)]);
    assert!(egui::Popup::is_any_open(&ctx), "Alt+O opens the Folder menu");
    menu_frame(&mut app, &ctx, typed(egui::Key::G, "g"));
    menu_frame(&mut app, &ctx, Vec::new());
    menu_frame(&mut app, &ctx, typed(egui::Key::Num1, "1"));
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].path, target, "G opened Go to Work Folder, 1 opened work folder 1");
    fs::remove_dir_all(root).unwrap();
}

/// 원본 FUN_0048aa30(압축 모드): 주소 줄은 PathCombine(압축 파일, 안 경로) — 압축 파일이 있는 폴더 조각부터 그린다.
/// 폴더 조각으로 가면 압축에서 나온다.
#[test]
fn archive_address_shows_the_archive_folders_and_leaves_the_archive() {
    let (root, ctx, mut app) = fixture("archive-crumbs");
    let zip = root.join("left").join("x.zip");
    fs::write(&zip, b"PK\x05\x06\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
    app.panes[0].archive = Some(ArchiveLocation { file: zip.clone(), prefix: "inner/".into(), password: String::new() });
    app.refresh(0);
    wait(&mut app, &ctx);
    draw(&mut app, &ctx);
    app.go_crumb(0, super::pane_chrome::CrumbTarget::Local(root.join("left")));
    wait(&mut app, &ctx);
    assert!(app.panes[0].archive.is_none());
    assert_eq!(app.panes[0].path, root.join("left"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn main_window_frame_info_bars_tabs_and_address_follow_csharp() {
    let (root, ctx, mut app) = fixture("frame");
    let left = root.join("left");
    draw(&mut app, &ctx);
    // C# FileVisualConverter: 확장자마다(폴더는 "<DIR>") 작은 셸 아이콘 하나(시험은 가짜 아이콘).
    assert!(app.file_icons.get("<DIR>").is_some_and(|t| t.is_some()) && app.file_icons.get(".txt").is_some_and(|t| t.is_some()), "{:?}", app.file_icons.keys().collect::<Vec<_>>());
    app.settings.set_flag("ShowIcons", false);
    assert!(app.file_icon(&ctx, 0, 0).is_none(), "ShowIcons off");
    app.settings.set_flag("ShowIcons", true);
    // C# UseOverlayIcons: 먼저 확장자 아이콘, 따로 된 STA 스레드가 오버레이 아이콘(시험 가짜 3x3)을 읽어 오면 그 경로는 그것.
    app.settings.set_flag("UseOverlayIcons", true);
    let row = app.panes[0].entries.iter().position(|e| !e.is_dir).unwrap();
    let path = app.panes[0].entries[row].path.clone();
    assert_eq!(app.file_icon(&ctx, 0, row).map(|t| t.size()), Some([2, 2]), "extension icon until the overlay arrives");
    let end = Instant::now() + Duration::from_secs(10);
    while !app.overlay_icons.contains_key(&path) {
        assert!(Instant::now() < end, "overlay timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.file_icon(&ctx, 0, row).map(|t| t.size()), Some([3, 3]));
    app.settings.set_flag("UseOverlayIcons", false);
    // 정보 표시줄: 원본 FOLDERINFO(폴더, 파일, StrFormatByteSizeW 크기), 확장자는 많은 순.
    let [folder, file, extension] = app.info_texts(0);
    let bytes: u64 = app.panes[0].entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
    let counts = format!("<c000000><b>1</b></c> 폴더, <c000000><b>3</b></c> 파일 (<b>{}</b>)", native::format_byte_size(bytes));
    assert_eq!(folder, counts, "no disk text in tests (disk_info hook)");
    let cursor = app.panes[0].entries[app.panes[0].cursor].clone();
    // 원본 FUN_0048a5cc: 초점 항목 "크기  |  날짜  |  속성  |  이름"(크기를 모르는 폴더는 FOLDER).
    let size = if cursor.is_dir { "폴더".to_owned() } else { super::file_operation::n0(cursor.size as usize) };
    assert!(file.starts_with(&format!("{size}  |  ")) && file.ends_with(&format!("  |  {}", cursor.name)), "{file}");
    assert_eq!(file.matches("  |  ").count(), 3, "{file}");
    assert_eq!(extension, ".txt 2개 · .log 1개");
    // 원본 디스크 글: 드라이브 글 + DISKINFO_SHORT(여유), 도움말 DISKINFO_LONG(여유, 전체).
    app.panes[0].disk = Some(("로컬 디스크 (C:) ".into(), 2048, 4096));
    let (_, drive, right, hint) = app.folder_info(0);
    let (free, total) = (native::format_byte_size(2048), native::format_byte_size(4096));
    assert_eq!((drive.as_str(), right), ("로컬 디스크 (C:) ", format!("<c000000><b>{free}</b></c> 남음")));
    assert_eq!(hint, format!("{free} 남음 (총 {total})"));
    assert_eq!(app.info_texts(0)[0], format!("{counts}  로컬 디스크 (C:) <c000000><b>{free}</b></c> 남음"));
    app.panes[0].selected = (0..app.panes[0].entries.len()).collect();
    let [folder, file, extension] = app.info_texts(0);
    // 원본 FUN_00484b24: 표시한 항목이 있으면 디스크 글 대신 SELECTED_INFO(개수, 바이트 수 + 폴더가 끼면 "+").
    let marked = super::file_operation::n0(bytes as usize);
    assert_eq!(folder, format!("{counts}  선택정보: <b>4</b> 항목 (<b>{marked}+</b> bytes)"));
    assert_eq!(file, app.file_info(0).unwrap(), "the file bar keeps showing the focused item");
    // 설명이 있으면 이름 대신 6B9FFF 색 설명.
    let focused = app.panes[0].entries[app.panes[0].cursor].name.to_lowercase();
    app.panes[0].descriptions.insert(focused, "메모".into());
    assert!(app.file_info(0).unwrap().ends_with("  |  <c6B9FFF>메모</c>"));
    app.panes[0].descriptions.clear();
    assert_eq!(extension, app.info_texts(0)[2]);
    assert!(extension.contains(".txt 2개"));
    app.panes[0].selected.clear();
    app.panes[0].disk = None;
    // 정보 표시줄 메뉴 명령은 설정을 뒤집고 저장한다.
    app.dispatch("toggleExtensionInfoBar", &ctx);
    assert!(app.settings.flag("ShowExtensionInfoBar", false));
    app.dispatch("toggleFileInfoBar", &ctx);
    assert!(!app.settings.flag("ShowFileInfoBar", true));
    draw(&mut app, &ctx);
    // 탭: 새 탭은 같은 위치, 다른 탭 모두 닫기.
    app.dispatch("newTab", &ctx);
    assert_eq!(app.panes[0].tabs.len(), 2);
    draw(&mut app, &ctx);
    // 주소 입력: 상대 경로는 지금 폴더 기준, 없는 폴더는 "폴더 이동" 경고.
    app.navigate_address(0, "folder");
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].path, left.join("folder"));
    app.navigate_address(0, "..\\missing");
    assert_eq!(app.msgboxes.first().map(|m| m.title.as_str()), Some("폴더 이동"));
    app.answer_message_box(1, &ctx);
    // 원본 주소 칸 Enter(FUN_0048cb30): http(s)://는 기본 프로그램으로 열고, ftp://는 그 패널의 FTP 빠른 연결 창.
    app.shell_runner = launched;
    app.original_address_enter(0, "  HTTP://example.invalid/a  ");
    assert_eq!(FEED.with(|f| f.borrow().clone()), b"HTTP://example.invalid/a");
    app.original_address_enter(0, "https://example.invalid/b");
    assert_eq!(FEED.with(|f| f.borrow().clone()), b"https://example.invalid/b");
    assert!(app.msgboxes.is_empty() && app.original_dialogs.is_empty(), "no folder warning for a web address");
    app.active = 1;
    app.original_address_enter(0, "ftp://host.invalid/pub");
    assert_eq!(app.active, 0);
    assert!(matches!(&app.original_dialogs.last().map(|d| &d.kind), Some(super::original_dialogs::DialogKind::FtpQuickConnect(f)) if f.host == "ftp://host.invalid/pub" && f.pane == 0));
    app.original_dialogs.clear();
    // 주소 편집: … 단추와 같은 시작(지금 경로, 편집 상태), Esc 끝.
    app.begin_address_edit(0, &ctx);
    assert!(app.panes[0].address_editing);
    draw(&mut app, &ctx);
    app.end_address_edit(0);
    assert!(!app.panes[0].address_editing);
    draw(&mut app, &ctx);
    fs::remove_dir_all(root).unwrap();
}

thread_local! {
    static FEED: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}
fn newer_feed(_: &str, _: usize) -> Result<ndr::update::HttpResponse, ndr::update::HttpError> {
    Ok(ndr::update::HttpResponse { status: 200, secure: true, body: br#"{"version":"99.0","url":"https://example.invalid/ndr"}"#.to_vec() })
}
fn latest_feed(_: &str, _: usize) -> Result<ndr::update::HttpResponse, ndr::update::HttpError> {
    Ok(ndr::update::HttpResponse { status: 200, secure: true, body: br#"{"version":"1.0"}"#.to_vec() })
}
fn missing_feed(_: &str, _: usize) -> Result<ndr::update::HttpResponse, ndr::update::HttpError> {
    Ok(ndr::update::HttpResponse { status: 404, secure: true, body: vec![] })
}
fn timeout_feed(_: &str, _: usize) -> Result<ndr::update::HttpResponse, ndr::update::HttpError> {
    Err(ndr::update::HttpError::Timeout)
}
fn launched(path: &Path, _: &str, _: &Path) -> Result<(), String> {
    FEED.with(|f| *f.borrow_mut() = path.to_string_lossy().as_bytes().to_vec());
    Ok(())
}

#[test]
fn update_check_context_menu_and_folder_sizes_follow_csharp() {
    let (root, ctx, mut app) = fixture("update");
    let wait_box = |app: &mut App, ctx: &egui::Context| {
        let end = Instant::now() + Duration::from_secs(10);
        while app.msgboxes.is_empty() {
            assert!(Instant::now() < end, "no message box");
            app.poll(ctx);
            thread::sleep(Duration::from_millis(5));
        }
        let first = app.msgboxes.first().unwrap();
        (first.title.clone(), first.text.clone())
    };
    // 피드가 없으면 안내, HTTPS가 아니면 오류.
    app.dispatch("update", &ctx);
    assert_eq!(wait_box(&mut app, &ctx), (app.language_pack.text("NDRUpdate.Title", "?"), app.language_pack.text("NDRUpdate.FeedMissing", "?")));
    app.answer_message_box(1, &ctx);
    app.settings.set_text("UpdateManifestUrl", "http://example.invalid/feed.json");
    app.dispatch("update", &ctx);
    assert_eq!(wait_box(&mut app, &ctx).1, app.language_pack.text("NDRUpdate.FeedHttpsRequired", "?"));
    app.answer_message_box(1, &ctx);
    // 새 버전: 원본 UPDATE_NEWUPDATE(제목 UPDATE), 예면 다운로드 주소를 연다.
    app.settings.set_text("UpdateManifestUrl", "https://example.invalid/feed.json");
    app.http_get = newer_feed;
    app.shell_runner = launched;
    app.dispatch("update", &ctx);
    assert_eq!(wait_box(&mut app, &ctx), (app.language_pack.text("RES.UPDATE", "?"), app.language_pack.text("RES.UPDATE_NEWUPDATE", "?")));
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    assert_eq!(FEED.with(|f| f.borrow().clone()), b"https://example.invalid/ndr");
    app.http_get = latest_feed;
    app.dispatch("update", &ctx);
    assert_eq!(wait_box(&mut app, &ctx), ("NDR".to_owned(), app.language_pack.text("RES.UPDATE_NOUPDATE", "?")));
    app.answer_message_box(1, &ctx);
    app.http_get = missing_feed;
    app.dispatch("update", &ctx);
    assert_eq!(wait_box(&mut app, &ctx), ("NDR".to_owned(), app.language_pack.text("NDRUpdate.RequestFailedStatus", "?").replace("{0}", "404")));
    app.answer_message_box(1, &ctx);
    app.http_get = timeout_feed;
    app.dispatch("update", &ctx);
    assert_eq!(wait_box(&mut app, &ctx).1, app.language_pack.text("NDRUpdate.RequestTimedOut", "?"));
    app.answer_message_box(1, &ctx);
    // 원본 조용한 확인(시작할 때, FUN_00441dd8 [+0x54]): 상자 없이 새 버전이면 메뉴 줄 "새 업데이트" 단추만. 누르면 그 주소.
    app.update_done(newer_feed("", 0), true);
    assert!(app.msgboxes.is_empty());
    assert_eq!(app.update_available.as_deref(), Some("https://example.invalid/ndr"));
    app.update_done(timeout_feed("", 0), true);
    assert!(app.msgboxes.is_empty(), "silent errors");
    draw(&mut app, &ctx);
    // 후원 단추(원본 tbDonation)는 사용자 요청으로 뺐다: 메뉴 줄을 그려도 후원 주소를 열 길이 없다.
    assert!(FEED.with(|f| !f.borrow().windows(8).any(|w| w == b"donation")));
    // 오른쪽 단추 메뉴는 그려진다(누른 것이 없으면 명령 없음).
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut chosen = Some("x");
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| chosen = app.list_context_menu(ui, 0));
    });
    assert_eq!(chosen, None);
    // 폴더 크기 자동 표시: 보이는 폴더 줄을 그리면 따로 세어 크기 글이 된다.
    fs::write(root.join("left").join("folder").join("x.bin"), vec![0u8; 2048]).unwrap();
    app.settings.set_flag("LeftAutoFolderSize", true);
    draw(&mut app, &ctx);
    let folder = root.join("left").join("folder");
    assert!(app.panes[0].folder_sizes.contains_key(&folder), "queued while drawing");
    let end = Instant::now() + Duration::from_secs(10);
    while !matches!(app.panes[0].folder_sizes.get(&folder), Some(Some(_))) {
        assert!(Instant::now() < end, "folder size timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.panes[0].folder_sizes[&folder].as_deref(), Some("2.00 KB"), "original SizeToStr");
    // 원본 FUN_0048a5cc: 파일 정보 표시줄은 센 폴더의 크기를 바이트(천 단위)로.
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.path == folder).unwrap();
    assert!(app.file_info(0).unwrap().starts_with("2,048  |  "), "{:?}", app.file_info(0));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn drag_and_drop_follows_the_original() {
    let (root, ctx, mut app) = fixture("drop");
    let (left, right) = (root.join("left"), root.join("right"));
    let wait_job = |app: &mut App, ctx: &egui::Context| {
        let end = Instant::now() + Duration::from_secs(20);
        while app.job.as_ref().is_some_and(|j| !j.finished) || app.panes.iter().any(|p| p.loading) {
            assert!(Instant::now() < end, "job timeout");
            app.poll(ctx);
            thread::sleep(Duration::from_millis(5));
        }
    };
    // ConfirmDragDrop(기본) = 원본 FUN_00488b64: 같은 드라이브면 이동을 DRAGDROP / DROPMOVE_CONFIRM(개수)로 묻는다. 예면 옮긴다.
    app.handle_drop(1, vec![left.join("a.txt")], right.clone(), egui::Modifiers::NONE);
    let question = app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone(), m.flags)).expect("the drop question");
    assert_eq!(question, (app.language_pack.text("RES.DRAGDROP", "?"), app.language_pack.text("RES.DROPMOVE_CONFIRM", "?").replace("%d", "1"), ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONQUESTION));
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    wait_job(&mut app, &ctx);
    assert!(right.join("a.txt").exists() && !left.join("a.txt").exists());
    // 폴더 안의 폴더로는 옮기지 않는다. ConfirmDragDrop을 끄면 같은 드라이브는 묻지 않고 이동.
    app.settings.set_flag("ConfirmDragDrop", false);
    app.handle_drop(0, vec![left.join("folder")], left.join("folder"), egui::Modifiers::NONE);
    assert!(app.original_dialogs.is_empty() && app.msgboxes.is_empty());
    wait_job(&mut app, &ctx);
    app.handle_drop(1, vec![left.join("c.log")], right.clone(), egui::Modifiers::NONE);
    wait_job(&mut app, &ctx);
    // 원본 끌기 그림(FUN_0048514c): 놓기 전에는 DRAGFILES_INFO 머리글과 이름을 그리고 끌기를 이어 간다.
    app.file_drag = Some(super::drag_drop::FileDrag { paths: vec![right.join("a.txt"), right.join("c.log")], secondary: false, source: 1, allowed: 7 });
    let raw = egui::RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(50.0, 50.0))], ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.drag_drop_frame(ctx));
    assert!(app.file_drag.is_some(), "still dragging");
    app.file_drag = None;
    assert!(right.join("c.log").exists() && !left.join("c.log").exists(), "same drive → move");
    // 원본 오른쪽 단추 끌기: 놓으면 pmDrop(여기에 복사, 여기에 이동, 바로 가기, -, 취소). 고르면 ConfirmDragDrop을 묻지 않는다.
    app.settings.set_flag("ConfirmDragDrop", true);
    draw(&mut app, &ctx);
    app.folder_rows.clear();
    let target = app.pane_rects[0].center();
    app.file_drag = Some(super::drag_drop::FileDrag { paths: vec![right.join("c.log")], secondary: true, source: 1, allowed: 7 });
    let raw = egui::RawInput {
        events: vec![egui::Event::PointerMoved(target), egui::Event::PointerButton { pos: target, button: egui::PointerButton::Secondary, pressed: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    };
    let _ = ctx.run(raw, |ctx| app.drag_drop_frame(ctx));
    assert!(app.file_drag.is_none());
    let rows: Vec<String> = app.popup.as_ref().expect("the pmDrop menu").items.iter().map(|item| match item {
        super::favorites_dialog::PopupItem::Entry { label, .. } => label.clone(),
        _ => "-".into(),
    }).collect();
    let caption = |key: &str| app.language_pack.text(key, "?");
    assert_eq!(rows, [caption("Menu.miDropCopy"), caption("Menu.miDropMove"), caption("Menu.miDropShortcut"), "-".into(), caption("Menu.miDropCancel")]);
    let Some(super::favorites_dialog::PopupItem::Entry { action, .. }) = app.popup.take().unwrap().items.into_iter().next() else { panic!() };
    app.run_popup_action(action);
    assert!(app.msgboxes.is_empty(), "no DROPCOPY_CONFIRM from the menu");
    wait_job(&mut app, &ctx);
    assert!(right.join("c.log").exists() && left.join("c.log").exists(), "Copy Here copies");
    // 끌기 시작: 표시가 없으면 누른 줄.
    app.panes[0].selected.clear();
    let row = app.panes[0].entries.iter().position(|e| e.name == "folder").unwrap();
    app.begin_file_drag(0, row, false, egui::Modifiers::NONE);
    assert_eq!(app.file_drag.as_ref().map(|d| d.paths.clone()), Some(vec![left.join("folder")]));
    // 놓을 곳: 그린 폴더 줄 위면 그 폴더, 끄는 항목 자신 위면 패널 폴더.
    app.folder_rows.clear();
    draw(&mut app, &ctx);
    let (_, rect, _) = app.folder_rows.iter().find(|(p, _, path)| *p == 0 && *path == left.join("folder")).cloned().expect("the folder row was drawn");
    assert_eq!(app.drop_target(Some(rect.center()), &[]), Some((0, left.join("folder"))));
    assert_eq!(app.drop_target(Some(rect.center()), &[left.join("folder")]), Some((0, left.clone())));
    assert_eq!(app.drop_target(Some(egui::pos2(-50.0, -50.0)), &[]), None);
    app.file_drag = None;
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn network_share_prompt_only_for_credential_failures() {
    // C# connectNetworkShare: 자격 증명 실패(5, 65, 86, 1326)만 연결 창, 그 밖의 오류는 그대로.
    use super::tools_dialog::network_share_code;
    assert_eq!(network_share_code(r"\\server\share\x", |_| 1326, |_| 1223), 1223);
    assert_eq!(network_share_code(r"\\server\share\x", |_| 86, |_| 0), 0);
    assert_eq!(network_share_code(r"\\server\share\x", |_| 53, |_| 0), 53);
    assert_eq!(network_share_code(r"C:\x", |_| 0, |_| 5), 0);
    assert_eq!(native::win32_message(5), native::win32_message(5).trim());
    assert!(!native::win32_message(5).contains("os error"));
}

#[test]
fn auto_view_columns_follow_the_original() {
    let (root, ctx, mut app) = fixture("auto");
    app.settings.set_text("LeftViewStyle", "Auto");
    // 설정한 열 수(1~19)면 그 수, 아니면 너비로 나눈 수(1400 너비의 한 패널이면 둘 이상).
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let mut counts = (0, 0);
    let _ = ctx.run(raw.clone(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            counts.0 = app.auto_column_count(ui, 0, 1400.0);
            app.settings.values.insert("LeftColumnCount".into(), serde_json::json!(3));
            counts.1 = app.auto_column_count(ui, 0, 1400.0);
        });
    });
    assert!(counts.0 >= 2, "{}", counts.0);
    assert_eq!(counts.1, 3);
    draw(&mut app, &ctx);
    app.settings.values.insert("LeftColumnCount".into(), serde_json::json!(1));
    draw(&mut app, &ctx);
    // 원본 FitVisibleFields: 칸은 내용 너비라 넓은 열에서는 다 들어가고 이름은 늘지 않으며, 좁은 열에서는 뒤 칸부터 빠진다.
    assert!(app.current_skin().is_some());
    let mut fitted = (0.0, [None; 6], 0.0, [None; 6]);
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (n, w) = app.original_auto_fields(ui, 0, 900.0);
            let (n2, w2) = app.original_auto_fields(ui, 0, 140.0);
            fitted = (n, w, n2, w2);
        });
    });
    let (name, wide, _, narrow) = fitted;
    let used: f32 = name + wide.iter().flatten().sum::<f32>();
    assert!(used < 900.0, "the original leaves the rest of the column empty: {name} {wide:?}");
    assert!(wide[0].is_some() && wide[3].is_some() && wide[4].is_some(), "extension, date and attributes fit: {wide:?}");
    assert!(narrow[3].is_none(), "a narrow column drops the later cells: {narrow:?}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn names_view_and_adjusted_widths_follow_the_original() {
    let (root, ctx, mut app) = fixture("names-original");
    assert!(app.current_skin().is_some());
    let dir = root.join("many");
    fs::create_dir_all(&dir).unwrap();
    for i in 0..30 {
        fs::write(dir.join(format!("f{i:02}.txt")), b"x").unwrap();
    }
    fs::write(dir.join(format!("{}.verylongextension", "n".repeat(80))), b"x").unwrap();
    app.navigate(0, dir, true);
    wait(&mut app, &ctx);
    app.compute_view(0);
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let mut got = None;
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let fm = app.original_field_max(ui, 0);
            let name = app.original_adjusted_name_width(ui, &fm);
            let ext = app.original_adjusted_ext_width(ui, &fm);
            let multi = app.original_names_fields(ui, 0, 1000.0, 250.0, 10);
            let single = app.original_names_fields(ui, 0, 1000.0, 1000.0, 100);
            let narrow = app.original_names_fields(ui, 0, 120.0, 120.0, 10);
            let fits = app.original_names_fits(ui, 0, 1000.0);
            got = Some((fm, name, ext, multi, single, narrow, fits));
        });
    });
    let (fm, name, ext, multi, single, narrow, fits) = got.unwrap();
    // 원본 GetAdjustedNameWidth/GetAdjustedExtWidth: 긴 이름·확장자 하나가 있으면 Trunc(평균 × 1.5 + 10).
    assert_eq!(fm.items, 32, "\"..\" and 31 files");
    let average = (fm.name_sum / fm.items as f32).floor();
    assert_eq!(name, (average * 1.5 + 10.0).trunc(), "{fm:?}");
    assert!(name < fm.max[0] + 10.0);
    assert!(fm.ext_average > 0.0);
    assert_eq!(ext, (fm.ext_average * 1.5 + 10.0).trunc(), "{fm:?}");
    assert!(ext < fm.max[1] + 10.0);
    // 원본 UpdateDisplayNames: 여러 열이면 확장자 = Adjusted, 이름이 열의 나머지를 채운다.
    assert!(fits >= 2, "{fits}");
    assert_eq!(multi.1[0], Some(ext));
    assert!((multi.0 + ext - 250.0).abs() < 0.01, "{multi:?}");
    assert!(multi.1.iter().skip(1).all(Option::is_none), "only name and extension: {multi:?}");
    // 한 열에 다 들어가면 제 너비(가장 긴 글 + 10)이고 남는 자리는 비운다.
    assert_eq!(single.1[0], Some(fm.max[1] + 10.0));
    assert!((single.0 - (30.0 + fm.max[0] + 10.0)).abs() < 0.01, "{single:?}");
    // 들어가는 열이 2 미만이고 한 항목보다 좁으면 이름 = 너비 − 머리 − Adjusted 확장자(확장자 칸은 제 너비).
    assert!((narrow.0 - (120.0 - ext)).abs() < 0.01, "{narrow:?}");
    assert_eq!(narrow.1[0], Some(fm.max[1] + 10.0));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn refresh_keeps_marks_and_folder_sizes_like_the_original() {
    let (root, ctx, mut app) = fixture("refresh-keep");
    assert!(app.current_skin().is_some());
    let left = root.join("left");
    let row = |app: &App, name: &str| app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    let marked = |app: &App| app.panes[0].selected.iter().map(|&r| app.panes[0].entries[r].name.clone()).collect::<Vec<_>>();
    // 원본 FUN_00487ce8(1, 1): 표시는 이름으로, 센 폴더 크기도 되돌린다.
    let (a, c) = (row(&app, "a.txt"), row(&app, "c.log"));
    app.panes[0].selected = [a, c].into_iter().collect();
    app.panes[0].cursor = row(&app, "b.txt");
    app.panes[0].folder_sizes.insert(left.join("folder"), Some("12 B".into()));
    fs::write(left.join("0new.txt"), b"n").unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    assert_eq!(marked(&app), vec!["a.txt", "c.log"]);
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "b.txt");
    assert_eq!(app.panes[0].folder_sizes.get(&left.join("folder")), Some(&Some("12 B".into())));
    // 커서 항목이 없어지면 옛 번호에 둔다.
    let old = app.panes[0].cursor;
    fs::remove_file(left.join("b.txt")).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].cursor, old);
    // 원본 miShowHiddenClick은 (1, 0): 표시는 지키지 않는다.
    app.dispatch("toggleHidden", &ctx);
    wait(&mut app, &ctx);
    assert!(app.panes[0].selected.is_empty());
    // C# RefreshPaneAsync는 모두 비운다.
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shortened_thumbnail_names_show_the_original_tooltip() {
    let (root, ctx, mut app) = fixture("thumb-tip");
    assert!(app.current_skin().is_some());
    let left = root.join("left");
    let long = format!("{}.txt", "long-name ".repeat(15).trim());
    fs::write(left.join(&long), b"x").unwrap();
    app.settings.set_text("LeftViewStyle", "Thumbnails");
    app.active = 0;
    app.refresh(0);
    wait(&mut app, &ctx);
    let show = |app: &mut App, name: &str| {
        let row = app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
        app.panes[0].cursor = row;
        draw(app, &ctx);
        // 원본 ToolTipTimer(600ms)가 지난 것으로 둔다.
        app.tip_since[0] = Some((row, Instant::now() - Duration::from_secs(1)));
        draw(app, &ctx);
        app.name_tip_shown[0]
    };
    let rect = show(&mut app, &long).expect("a shortened thumbnail name shows the full name");
    let thumb = app.settings.values.get("ThumbnailSize").and_then(|v| v.as_i64()).unwrap_or(120) as f32;
    assert!(rect.width() >= thumb + 8.0 - 0.5, "{rect:?}");
    assert!(rect.height() > 30.0, "the full name breaks into several lines: {rect:?}");
    assert_eq!(show(&mut app, "a.txt"), None, "a name that fits has no tooltip");
    // 원본 목록 생성자(0x7d4ef5): 목록 도움말은 3초 뒤 숨는다(TNXToolTipWnd 처음 값 5초가 아니다).
    let row = app.panes[0].entries.iter().position(|e| e.name == long).unwrap();
    app.panes[0].cursor = row;
    draw(&mut app, &ctx);
    app.tip_since[0] = Some((row, Instant::now() - Duration::from_millis(600 + super::panes::TIP_HIDE_MS + 100)));
    draw(&mut app, &ctx);
    assert_eq!((super::panes::TIP_HIDE_MS, app.name_tip_shown[0]), (3000, None), "hidden after three seconds");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn skin_status_text_is_a_brief_overlay_not_a_status_bar() {
    // 원본 창에는 상태 줄이 없다: 스킨 테마는 목록 자리를 줄이지 않고 기능키 줄 위에 4초 동안 겹쳐 보인다.
    let (root, ctx, mut app) = fixture("status-overlay");
    assert!(app.current_skin().is_some());
    let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let mut shown = None;
    app.status = "요청한 2개 항목을 복사했습니다.".into();
    let _ = ctx.run(raw(), |ctx| shown = app.skin_status_overlay(ctx, 20.0));
    let rect = shown.expect("the status text is shown");
    assert!(rect.bottom() <= 850.0 - 20.0, "above the function key bar: {rect:?}");
    // 4초가 지나면 숨는다(글은 그대로).
    app.status_seen = Some((app.status.clone(), Instant::now() - Duration::from_secs(5)));
    let _ = ctx.run(raw(), |ctx| shown = app.skin_status_overlay(ctx, 20.0));
    assert!(shown.is_none());
    assert!(!app.status.is_empty());
    // 새 글은 다시 보인다.
    app.status = "다른 글".into();
    let _ = ctx.run(raw(), |ctx| shown = app.skin_status_overlay(ctx, 20.0));
    assert!(shown.is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn drive_list_cache_does_not_ask_the_drives_on_every_listing() {
    // 원본 TNXDriveList처럼 들고 있는 목록: 5초 안이면 다시 묻지 않고, 무효로 하면 바로 다시 묻는다.
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ASKED: AtomicUsize = AtomicUsize::new(0);
    fn ready() -> Vec<PathBuf> {
        ASKED.fetch_add(1, Ordering::SeqCst);
        vec![PathBuf::from("C:\\")]
    }
    fn item(_: &Path) -> Option<super::panes::DriveInfo> {
        Some(("Local Disk".into(), 3, Some((1, 2))))
    }
    let cache: Arc<Mutex<super::panes::DriveListCache>> = Default::default();
    let first = super::panes::cached_drive_items(&cache, ready, item);
    assert_eq!(first.len(), 1);
    let _ = super::panes::cached_drive_items(&cache, ready, item);
    assert_eq!(ASKED.load(Ordering::SeqCst), 1, "a fresh cache is reused");
    super::panes::DriveListCache::invalidate(&cache);
    let again = super::panes::cached_drive_items(&cache, ready, item);
    assert_eq!((again, ASKED.load(Ordering::SeqCst)), (first, 2), "after invalidate the drives are read again");
}

/// 원본 FUN_004856d8: 표시가 있으면 목록 빈 곳에서 눌러 8화소 넘게 끌어도 표시한 항목을 끈다. 끌어서 목록을 굴리지는 않는다.
#[test]
fn skin_drag_from_empty_list_space_drags_the_marks() {
    let (root, ctx, mut app) = fixture("emptydrag");
    assert!(app.current_skin().is_some());
    let left = app.panes[0].path.clone();
    let b = app.panes[0].entries.iter().position(|e| e.name == "b.txt").unwrap();
    app.panes[0].selected.insert(b);
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    run(&mut app, vec![]);
    let pane = app.pane_rects[0];
    let at = egui::pos2(pane.center().x, pane.bottom() - 90.0);
    run(&mut app, vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }]);
    assert!(app.file_drag.is_none(), "a press alone does not drag");
    run(&mut app, vec![egui::Event::PointerMoved(at + egui::vec2(20.0, 0.0))]);
    assert_eq!(app.file_drag.as_ref().map(|d| d.paths.clone()), Some(vec![left.join("b.txt")]), "the marked items are dragged from empty space");
    app.file_drag = None;
    run(&mut app, vec![egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
    fs::remove_dir_all(root).unwrap();
}

/// 원본 TNXFramePanel: 패널 안 빈 곳을 누르면(왼쪽·오른쪽 단추, 보기 형식 모두) 그 패널이 지금 패널이 된다.
/// update는 패널을 그리기 전에 pane_rects를 비우므로 여기서도 그렇게 한다.
#[test]
fn clicking_empty_pane_space_activates_the_pane() {
    for view in ["Details", "Auto", "Names"] {
        for button in [egui::PointerButton::Primary, egui::PointerButton::Secondary] {
            let (root, ctx, mut app) = fixture("emptyclick");
            app.settings.set_text("LeftViewStyle", view);
            app.settings.set_text("RightViewStyle", view);
            let run = |app: &mut App, events: Vec<egui::Event>| {
                let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
                let shown = app.pane_rects;
                app.pane_rects = [egui::Rect::NOTHING; 2];
                let _ = ctx.run(raw, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.columns(2, |columns| {
                            app.pane_ui(&mut columns[0], 0, ctx);
                            app.pane_ui(&mut columns[1], 1, ctx);
                        });
                    });
                });
                if app.pane_rects[0] == egui::Rect::NOTHING {
                    app.pane_rects = shown;
                }
            };
            for side in [0usize, 1] {
                app.active = 1 - side;
                run(&mut app, vec![]);
                run(&mut app, vec![]);
                let pane = app.pane_rects[side];
                for at in [egui::pos2(pane.center().x, pane.bottom() - 60.0), egui::pos2(pane.right() - 20.0, pane.top() + 70.0)] {
                    app.active = 1 - side;
                    let b = |pressed: bool| egui::Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() };
                    run(&mut app, vec![egui::Event::PointerMoved(at), b(true)]);
                    run(&mut app, vec![b(false)]);
                    run(&mut app, vec![]);
                    assert_eq!(app.active, side, "{view} {button:?}: a press at {at:?} activates pane {side}");
                }
            }
            fs::remove_dir_all(root).unwrap();
        }
    }
}

/// 목록 오른쪽 단추 메뉴(원본 pmMain)는 연 뒤 누른 줄이 목록에서 안 보이게 넘어가도 닫히지 않는다(전체 update 경로).
/// 고친 것: 메뉴를 줄 응답에 붙였을 때는 그 줄을 그리지 않은 그림에서 egui가 메뉴를 닫아 바로 사라졌다.
#[test]
fn list_menu_stays_open_when_its_row_is_not_drawn() {
    for view in ["Details", "Auto"] {
        let (root, ctx, mut app) = fixture("listmenu");
        app.settings.set_text("LeftViewStyle", view);
        app.settings.set_text("RightViewStyle", view);
        for i in 0..150 {
            fs::write(app.panes[1].path.join(format!("file{i:03}.txt")), b"x").unwrap();
        }
        app.refresh(1);
        wait(&mut app, &ctx);
        let mut frame = eframe::Frame::_new_kittest();
        let mut run = |app: &mut App, events: Vec<egui::Event>| {
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
            let _ = ctx.run(raw, |ctx| eframe::App::update(app, ctx, &mut frame));
        };
        app.active = 0;
        run(&mut app, vec![]);
        run(&mut app, vec![]);
        let pane = app.pane_rects[1];
        let at = egui::pos2(pane.left() + 80.0, pane.top() + 120.0);
        let b = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
        run(&mut app, vec![egui::Event::PointerMoved(at), b(true)]);
        run(&mut app, vec![]);
        run(&mut app, vec![b(false)]);
        assert!(egui::Popup::is_any_open(&ctx), "{view}: the menu opens");
        assert_eq!(app.active, 1, "{view}: the right-clicked pane is active");
        for k in 0..4 {
            run(&mut app, vec![egui::Event::PointerMoved(at + egui::vec2(4.0 + k as f32 * 8.0, 4.0 + k as f32 * 6.0))]);
            assert!(egui::Popup::is_any_open(&ctx), "{view}: the menu stays open while the pointer moves into it");
        }
        // 누른 줄이 그려지지 않게 목록 끝으로 넘긴다.
        let last = app.panes[1].entries.len() - 1;
        app.panes[1].cursor = last;
        app.panes[1].reveal = true;
        for _ in 0..4 {
            run(&mut app, vec![]);
            assert!(egui::Popup::is_any_open(&ctx), "{view}: the menu stays open when its row scrolls away");
        }
        run(&mut app, vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
        run(&mut app, vec![]);
        assert!(!egui::Popup::is_any_open(&ctx), "{view}: Esc closes the menu");
        fs::remove_dir_all(root).unwrap();
    }
}
/// 원본 FUN_00484e8c 글쇠 경로(Apps 글쇠, MousePos -1, -1): 지금 패널의 커서 항목 칸 아래에 pmMain을 띄우고,
/// 표시가 없고 커서 항목이 ".."이면 띄우지 않는다.
#[test]
fn apps_key_opens_the_list_menu_at_the_cursor_item() {
    let (root, ctx, mut app) = fixture("appskey");
    let mut frame = eframe::Frame::_new_kittest();
    let mut run = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| eframe::App::update(app, ctx, &mut frame));
    };
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    run(&mut app, vec![]);
    run(&mut app, vec![]);
    app.pending_context_key = true;
    run(&mut app, vec![]);
    run(&mut app, vec![]);
    assert!(egui::Popup::is_any_open(&ctx), "Apps opens pmMain for the cursor item");
    assert!(!app.pending_context_key);
    run(&mut app, vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }]);
    run(&mut app, vec![]);
    assert!(!egui::Popup::is_any_open(&ctx));
    app.panes[0].cursor = 0;
    assert!(app.panes[0].entries[0].is_parent());
    app.pending_context_key = true;
    run(&mut app, vec![]);
    run(&mut app, vec![]);
    assert!(!egui::Popup::is_any_open(&ctx), "\"..\" without marks: no menu");
    fs::remove_dir_all(root).unwrap();
}

/// 원본 pathclick(설정 +0xf8): 0이면 주소 표시줄 빈 곳을 누르면 경로 편집, 1이면 그 자리에 주소 표시줄 메뉴(FUN_0048d10c),
/// 2면 조각을 눌러도 메뉴(그 폴더로 가지 않는다).
#[test]
fn path_click_mode_follows_the_original_path_label() {
    let (root, ctx, mut app) = fixture("pathclick");
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    let click = |app: &mut App, at: egui::Pos2| {
        let b = |pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(app, vec![egui::Event::PointerMoved(at), b(true)]);
        run(app, vec![b(false)]);
        run(app, vec![]);
    };
    run(&mut app, vec![]);
    run(&mut app, vec![]);
    let pane = app.pane_rects[0];
    // 주소 표시줄 = 틀 안 첫 줄(높이 19): 왼쪽 단추 20, ▼ 16 뒤에 첫 조각, 오른쪽 단추 20 앞까지 이름표.
    let bar_y = pane.top() + 2.0 + super::skin_pane::ADDRESS_BAR_HEIGHT / 2.0;
    let empty = egui::pos2(pane.right() - 26.0, bar_y);
    click(&mut app, empty);
    assert!(app.panes[0].address_editing, "mode 0: an empty click starts editing");
    app.end_address_edit(0);
    run(&mut app, vec![]);
    app.settings.values.insert("PathClickMode".into(), serde_json::json!(1));
    click(&mut app, empty);
    assert!(!app.panes[0].address_editing, "mode 1: no editing");
    assert!(app.popup.is_some(), "mode 1: the address bar menu");
    app.popup = None;
    // 2: 조각(첫 조각)을 눌러도 메뉴, 그 폴더로 가지 않는다.
    app.settings.values.insert("PathClickMode".into(), serde_json::json!(2));
    let path = app.panes[0].path.clone();
    click(&mut app, egui::pos2(pane.left() + 2.0 + 20.0 + 16.0 + 6.0, bar_y));
    assert!(app.popup.is_some(), "mode 2: a segment click opens the menu");
    assert!(!app.panes[0].loading);
    assert_eq!(app.panes[0].path, path);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn skin_drag_follows_the_original_list_mouse_move() {
    // 원본 FUN_004856d8(목록 OnMouseMove): 가로·세로 8화소를 넘게 움직여야 끌고, 표시 칸에서 누르면 끌지 않는다.
    let (root, ctx, mut app) = fixture("skindrag");
    assert!(app.current_skin().is_some());
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 20.0), egui::vec2(300.0, 20.0));
    let probe = |from: egui::Pos2, to: egui::Pos2| {
        let got = std::cell::Cell::new(false);
        let frame = |events: Vec<egui::Event>| {
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = ui.interact(rect, egui::Id::new("skin-drag-row"), egui::Sense::click_and_drag());
                    got.set(super::panes::drag_begins(&response, egui::PointerButton::Primary, true));
                });
            });
        };
        let button = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(vec![egui::Event::PointerMoved(from), button(from, true)]);
        frame(vec![egui::Event::PointerMoved(to)]);
        let begun = got.get();
        frame(vec![button(to, false)]);
        begun
    };
    assert!(!probe(egui::pos2(100.0, 30.0), egui::pos2(107.0, 34.0)), "7 pixels is not yet a drag (egui would start at 6)");
    assert!(probe(egui::pos2(100.0, 30.0), egui::pos2(109.0, 30.0)), "more than 8 pixels across");
    assert!(probe(egui::pos2(100.0, 30.0), egui::pos2(100.0, 39.0)), "more than 8 pixels down");
    assert!(!probe(egui::pos2(5.0, 30.0), egui::pos2(40.0, 30.0)), "a press in the selection area never drags");
    // 끄는 항목: 표시가 있으면 누른 항목과 상관없이 표시한 항목, 없으면 누른 항목. ".."는 끌지 않는다.
    let left = app.panes[0].path.clone();
    let row = |app: &App, name: &str| app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    let (parent, a, b) = (row(&app, ".."), row(&app, "a.txt"), row(&app, "b.txt"));
    app.begin_file_drag(0, parent, false, egui::Modifiers::NONE);
    assert!(app.file_drag.is_none(), "\"..\" without marks is not dragged");
    app.panes[0].selected.insert(b);
    app.begin_file_drag(0, a, false, egui::Modifiers::NONE);
    assert_eq!(app.file_drag.as_ref().map(|d| d.paths.clone()), Some(vec![left.join("b.txt")]), "the marks, not the pressed item");
    // 놓아서 옮기면 끈 패널의 표시를 마지막 선택에 남기고 지운다(원본 Execute < 3 → CopySelection + ClearSelection).
    app.pane_rects[1] = egui::Rect::from_min_size(egui::pos2(700.0, 0.0), egui::vec2(700.0, 800.0));
    let at = egui::pos2(900.0, 400.0);
    let release = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::CTRL }];
    let _ = ctx.run(egui::RawInput { events: release, modifiers: egui::Modifiers::CTRL, ..Default::default() }, |ctx| app.drag_drop_frame(ctx));
    assert!(app.file_drag.is_none());
    // 원본 FUN_00488b64: Ctrl(복사)이어도 "드래그 앤 드롭 확인"이면 DROPCOPY_CONFIRM으로 묻는다.
    let question = app.msgboxes.first().map(|m| m.text.clone()).expect("the drop question");
    assert_eq!(question, app.language_pack.text("RES.DROPCOPY_CONFIRM", "?").replace("%d", "1"));
    assert_eq!(app.panes[0].selected.len(), 1, "the marks stay until the drop is applied");
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    assert!(app.panes[0].selected.is_empty(), "the source marks are cleared");
    assert_eq!(app.last_selection[0], vec![left.join("b.txt")], "and kept as the last selection");
    let end = Instant::now() + Duration::from_secs(20);
    while app.job.as_ref().is_some_and(|j| !j.finished) {
        assert!(Instant::now() < end, "job timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(app.panes[1].path.join("b.txt").exists(), "Ctrl = copy");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn skin_drop_effects_follow_the_original_drop_target() {
    // 원본 FUN_00488700(효과)·FUN_00488b64(놓기)·FUN_004856d8(시작할 때 허용 효과).
    let (root, ctx, mut app) = fixture("skindrop");
    assert!(app.current_skin().is_some());
    let (left, right) = (root.join("left"), root.join("right"));
    let wait_job = |app: &mut App| {
        let end = Instant::now() + Duration::from_secs(20);
        while app.job.as_ref().is_some_and(|j| !j.finished) || app.panes.iter().any(|p| p.loading) {
            assert!(Instant::now() < end, "job timeout");
            app.poll(&ctx);
            thread::sleep(Duration::from_millis(5));
        }
    };
    let question = |app: &App| app.msgboxes.first().map(|m| m.text.clone());
    let text = |app: &App, key: &str| app.language_pack.text(key, "?").replace("%d", "1");
    // 같은 폴더에 놓으면 효과 0: 묻지도 않고 바로 가기도 만들지 않는다.
    app.handle_drop(0, vec![left.join("a.txt")], left.clone(), egui::Modifiers::NONE);
    assert!(app.msgboxes.is_empty() && app.original_dialogs.is_empty() && !left.join("a.txt.lnk").exists());
    // 글쇠 없음: 같은 드라이브면 이동을 묻는다.
    app.handle_drop(1, vec![left.join("a.txt")], right.clone(), egui::Modifiers::NONE);
    assert_eq!(question(&app), Some(text(&app, "RES.DROPMOVE_CONFIRM")));
    app.answer_message_box(ndr::msgbox::IDNO, &ctx);
    assert!(left.join("a.txt").exists(), "No keeps the file");
    // 끌기를 Ctrl로 시작하면 복사만 허용: 글쇠 없이 놓아도 복사.
    app.handle_drop_with(1, vec![left.join("a.txt")], right.clone(), egui::Modifiers::NONE, 1);
    assert_eq!(question(&app), Some(text(&app, "RES.DROPCOPY_CONFIRM")));
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    wait_job(&mut app);
    assert!(left.join("a.txt").exists() && right.join("a.txt").exists());
    // Shift로 시작하면 이동만: Ctrl을 누르고 놓아도 이동.
    app.handle_drop_with(1, vec![left.join("b.txt")], right.clone(), egui::Modifiers::CTRL, 2);
    assert_eq!(question(&app), Some(text(&app, "RES.DROPMOVE_CONFIRM")));
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    wait_job(&mut app);
    assert!(!left.join("b.txt").exists() && right.join("b.txt").exists());
    // Ctrl+Shift = 바로 가기, 묻지 않는다.
    app.handle_drop(1, vec![left.join("c.log")], right.clone(), egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    assert!(app.msgboxes.is_empty());
    assert!(right.join("c.log.lnk").exists(), "\"name.ext.lnk\" shortcut");
    // 끄는 동안의 효과(DoDragDrop 포인터): 다른 패널 = 이동(같은 드라이브)·Ctrl = 복사·같은 폴더 = 놓을 수 없음.
    use super::drag_drop::DropEffect as E;
    let dragged = vec![left.join("a.txt")];
    assert_eq!(app.original_drop_effect(1, &dragged, &right, egui::Modifiers::NONE, 7), Some(E::Move));
    assert_eq!(app.original_drop_effect(1, &dragged, &right, egui::Modifiers::CTRL, 7), Some(E::Copy));
    assert_eq!(app.original_drop_effect(1, &dragged, &right, egui::Modifiers::CTRL | egui::Modifiers::SHIFT, 7), Some(E::Link));
    assert_eq!(app.original_drop_effect(0, &dragged, &left, egui::Modifiers::NONE, 7), None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn thumbnail_tooltip_breaks_lines_between_words() {
    // 원본 ShowToolTip 형식 3의 DT_WORDBREAK: 빈칸에서만 나누고, 폭보다 긴 낱말은 그대로 한 줄.
    let mut measure = |s: &str| s.chars().count() as f32;
    assert_eq!(super::panes::word_break_lines("a bb ccc", 4.0, &mut measure), vec!["a bb", "ccc"]);
    assert_eq!(super::panes::word_break_lines("abcdefgh ij", 4.0, &mut measure), vec!["abcdefgh", "ij"]);
    assert_eq!(super::panes::word_break_lines("photo.jpg", 4.0, &mut measure), vec!["photo.jpg"]);
}

#[test]
fn only_the_vertical_wheel_scrolls_like_the_original() {
    // 원본 WMMouseWheel은 Shift를 가리지 않고, WMMouseHWheel(기울이는 바퀴)은 아무것도 옮기지 않는다.
    let ctx = egui::Context::default();
    let wheel = |delta: egui::Vec2, modifiers: egui::Modifiers| {
        let raw = egui::RawInput {
            modifiers,
            events: vec![egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta, modifiers }],
            ..Default::default()
        };
        let mut got = 0.0;
        let _ = ctx.run(raw, |ctx| got = ctx.input(App::vertical_wheel));
        got
    };
    assert!(wheel(egui::vec2(0.0, -1.0), egui::Modifiers::NONE) < 0.0);
    assert!(wheel(egui::vec2(0.0, -1.0), egui::Modifiers::SHIFT) < 0.0, "Shift + wheel still scrolls");
    assert_eq!(wheel(egui::vec2(1.0, 0.0), egui::Modifiers::NONE), 0.0, "the tilt wheel is ignored");
}

#[test]
fn tabs_keep_their_own_history_like_csharp() {
    let (root, ctx, mut app) = fixture("tabs");
    let left = root.join("left");
    app.active = 0;
    app.navigate(0, left.join("folder"), true);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].back, vec![left.clone()]);
    // 새 탭: 같은 위치, 기록은 비어 있다.
    app.dispatch("newTab", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].tab, app.panes[0].path.clone()), (2, 1, left.join("folder")));
    assert!(app.panes[0].back.is_empty());
    app.navigate(0, left.clone(), true);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].back, vec![left.join("folder")]);
    // 첫 탭으로 돌아가면 그 탭의 위치와 기록.
    app.tab_command(0, "tab:0", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].path.clone(), app.panes[0].back.clone()), (left.join("folder"), vec![left.clone()]));
    // 지금 탭을 닫으면 남은 탭의 위치와 기록.
    app.dispatch("closeTab", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].path.clone(), app.panes[0].back.clone()), (1, left.clone(), vec![left.join("folder")]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn saved_tabs_restore_like_csharp_restore_pane_tabs() {
    let saved: Vec<String> = ["C:\\a", "C:\\gone", "C:\\b", "C:\\a"].iter().map(|s| s.to_string()).collect();
    let exists = |p: &Path| p != Path::new("C:\\gone");
    // C#: 없는 폴더의 탭은 빼고, 저장한 고른 탭(원래 번호 3 = 두 번째 C:\a)을 고른다.
    assert_eq!(super::restore_tabs(&saved, Some(3), Path::new("C:\\a"), false, exists), (vec![PathBuf::from("C:\\a"), PathBuf::from("C:\\b"), PathBuf::from("C:\\a")], 2));
    // 고른 탭이 없어진 폴더면 지금 폴더의 탭, 없으면 맨 앞에 더한다.
    assert_eq!(super::restore_tabs(&saved, Some(1), Path::new("C:\\b"), false, exists).1, 1);
    assert_eq!(super::restore_tabs(&saved, None, Path::new("C:\\c"), false, exists), (vec![PathBuf::from("C:\\c"), PathBuf::from("C:\\a"), PathBuf::from("C:\\b"), PathBuf::from("C:\\a")], 0));
    // 명령줄·시작 폴더로 정한 폴더는 저장한 고른 탭보다 먼저.
    assert_eq!(super::restore_tabs(&saved, Some(3), Path::new("C:\\b"), true, exists).1, 1);
    // 남은 탭이 없으면 시작 폴더 하나.
    assert_eq!(super::restore_tabs(&saved, Some(0), Path::new("D:\\x"), false, |_| false), (vec![PathBuf::from("D:\\x")], 0));
}

#[test]
fn window_bounds_are_saved_like_csharp_closing() {
    let (root, _ctx, mut app) = fixture("window-bounds");
    let outer = egui::Rect::from_min_size(egui::pos2(120.0, 80.0), egui::vec2(1216.0, 839.0));
    let inner = egui::Rect::from_min_size(egui::pos2(128.0, 111.0), egui::vec2(1200.0, 800.0));
    app.track_window(Some(outer), Some(inner), Some(false), Some(false));
    // 최대화하면 보통 창 위치·크기는 그대로 두고 최대화만 기록한다. 최소화 중에는 아무것도 바꾸지 않는다.
    app.track_window(Some(egui::Rect::from_min_size(egui::pos2(-8.0, -8.0), egui::vec2(1936.0, 1048.0))), Some(inner), Some(true), Some(false));
    app.track_window(Some(egui::Rect::from_min_size(egui::pos2(-32000.0, -32000.0), egui::vec2(160.0, 28.0))), None, Some(false), Some(true));
    app.save();
    let number = |key: &str| app.settings.values.get(key).and_then(|v| v.as_f64());
    assert_eq!((number("WindowLeft"), number("WindowTop"), number("WindowWidth"), number("WindowHeight")), (Some(120.0), Some(80.0), Some(1216.0), Some(839.0)), "outer size like the original TForm/C# Window");
    // 다시 열 때 빼는 테두리·제목 줄 크기(보통 창).
    let (frame_width, frame_height) = ndr::native::window_frame_points().expect("AdjustWindowRectEx");
    assert!(frame_width > 0.0 && frame_height > frame_width, "{frame_width} x {frame_height}");
    assert!(app.settings.flag("WindowMaximized", false));
    assert!(app.settings.flag("HasWindowBounds", false));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn menus_are_drawn_at_the_original_menu_alpha() {
    let ctx = egui::Context::default();
    let (menu, window, dialog) = (egui::Id::new("fade-menu"), egui::Id::new("fade-window"), egui::Id::new("fade-dialog"));
    let square = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 10.0));
    let mut fills = (None, None, None);
    let mut combo = (false, false);
    // 새 Area는 첫 프레임에 보이지 않는 크기 재기만 하므로 뒤 프레임에서 본다(펼친 목록은 한 프레임 더 늦다).
    for _ in 0..3 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::Area::new(menu).order(egui::Order::Foreground).fade_in(false).show(ctx, |ui| ui.painter().rect_filled(square, 0.0, Color32::WHITE));
        egui::Area::new(window).order(egui::Order::Middle).fade_in(false).show(ctx, |ui| ui.painter().rect_filled(square, 0.0, Color32::WHITE));
        // egui::Modal 원본 창처럼 앞쪽 층이지만 메뉴가 아닌 것.
        super::keep_opaque(ctx, dialog);
        egui::Area::new(dialog).order(egui::Order::Foreground).fade_in(false).show(ctx, |ui| ui.painter().rect_filled(square, 0.0, Color32::WHITE));
        // 펼친 목록(ComboBox)도 앞쪽 층이지만 메뉴가 아니다(원본·C# 판은 메뉴만 투명): show_opaque가 목록 층을 알린다.
        egui::CentralPanel::default().show(ctx, |ui| {
            use super::ComboOpaque;
            let id = ui.make_persistent_id(egui::Id::new("fade-combo")).with("popup");
            egui::Popup::open_id(ui.ctx(), id);
            egui::ComboBox::from_id_salt("fade-combo").selected_text("a").show_opaque(ui, |ui| ui.label("b"));
            let registered = ui.ctx().data(|d| d.get_temp::<Vec<egui::Id>>(egui::Id::new(super::OPAQUE_LAYERS))).unwrap_or_default().contains(&id);
            combo = (ui.ctx().memory(|m| m.areas().visible_layer_ids()).contains(&egui::LayerId::new(egui::Order::Foreground, id)), registered);
        });
        super::fade_foreground_layers(ctx, super::MENU_ALPHA);
        let fill = |ctx: &egui::Context, layer: egui::LayerId| {
            ctx.graphics_mut(|g| {
                g.get(layer).and_then(|list| {
                    list.all_entries().find_map(|clipped| match &clipped.shape {
                        egui::Shape::Rect(rect) => Some(rect.fill),
                        _ => None,
                    })
                })
            })
        };
        fills = (
            fill(ctx, egui::LayerId::new(egui::Order::Foreground, menu)),
            fill(ctx, egui::LayerId::new(egui::Order::Middle, window)),
            fill(ctx, egui::LayerId::new(egui::Order::Foreground, dialog)),
        );
        });
    }
    // 원본 SetLayeredWindowAttributes(메뉴 창, 0xF0) = C# Opacity 0.94: 메뉴 층만 흐리고 창 층·원본 창(Modal)·펼친 목록은 그대로.
    assert_eq!(fills.0, Some(Color32::WHITE.gamma_multiply(240.0 / 255.0)));
    assert_eq!(fills.1, Some(Color32::WHITE));
    assert_eq!(fills.2, Some(Color32::WHITE));
    assert_eq!(combo, (true, true), "the open ComboBox list layer is shown and kept opaque");
}

#[test]
fn start_folder_setting_is_used_at_startup_like_csharp() {
    let (root, _ctx, mut app) = fixture("start-folder");
    let start = root.join("right");
    app.settings.set_flag("LeftStartPathEnabled", true);
    app.settings.set_text("LeftStartPath", start.to_string_lossy().into_owned());
    app.settings.set_text("LeftPath", root.join("left").to_string_lossy().into_owned());
    app.settings.save(&root.join("profile").join("settings.json")).unwrap();
    let options = Options { profile: Some(root.join("profile")), language: Some("ko".into()), ..Options::default() };
    let ctx = egui::Context::default();
    let restarted = App::from_context(&ctx, options);
    // C# leftCandidate = StartPathEnabled && StartPath ? StartPath : LeftPath.
    assert_eq!(restarted.panes[0].path, start);
    assert_eq!(restarted.panes[0].tabs[restarted.panes[0].tab], start);
    let _ = fs::remove_dir_all(root);
}


thread_local! {
    static PER_FILE_ASKED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[test]
fn executables_get_their_own_icon_like_the_original_shell_image_list() {
    let (root, ctx, mut app) = fixture("own-icons");
    fs::write(root.join("left").join("tool.exe"), b"MZ").unwrap();
    app.refresh(0);
    let end = Instant::now() + Duration::from_secs(10);
    while app.panes[0].loading || !app.panes[0].entries.iter().any(|e| e.name == "tool.exe") {
        assert!(Instant::now() < end, "listing timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    // 겹침 아이콘(UseOverlayIcons, 원본 처음 값 켜짐)을 끄고 파일마다 아이콘만 본다.
    app.settings.set_flag("UseOverlayIcons", false);
    draw(&mut app, &ctx);
    assert!(!app.settings.flag("UseOverlayIcons", true));
    // 원본 GetIconIndex(실제 경로): 실행 파일은 그 파일의 아이콘(시험 가짜 5x5). 오기 전에는 확장자 아이콘.
    let exe = app.panes[0].entries.iter().position(|e| e.name == "tool.exe").unwrap();
    let path = app.panes[0].entries[exe].path.clone();
    assert_eq!(app.file_icon(&ctx, 0, exe).map(|t| t.size()), Some([2, 2]));
    while !app.overlay_icons.contains_key(&path) {
        assert!(Instant::now() < end, "icon timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.file_icon(&ctx, 0, exe).map(|t| t.size()), Some([5, 5]));
    // 확장자마다 같은 아이콘인 형식은 확장자 캐시 그대로.
    let txt = app.panes[0].entries.iter().position(|e| e.name.ends_with(".txt")).unwrap();
    assert_eq!(app.file_icon(&ctx, 0, txt).map(|t| t.size()), Some([2, 2]));
    assert!(!app.overlay_icons.contains_key(&app.panes[0].entries[txt].path));
    // 고정 목록 밖이어도 HKCR이 파일마다 아이콘이라고 하면(DefaultIcon "%1"·IconHandler, 가짜 훅) 실제 경로로 읽는다. 확장자마다 한 번 묻는다.
    app.system.per_file_icon = |extension| {
        PER_FILE_ASKED.with(|a| a.borrow_mut().push(extension.to_owned()));
        extension == "msi"
    };
    assert!(app.per_file_icon_type(std::path::Path::new("C:\\x\\Setup.MSI")) && app.per_file_icon_type(std::path::Path::new("C:\\y\\other.msi")));
    assert!(!app.per_file_icon_type(std::path::Path::new("C:\\x\\a.doc")) && app.per_file_icon_type(std::path::Path::new("C:\\x\\b.LNK")));
    assert_eq!(PER_FILE_ASKED.with(|a| a.take()), ["msi", "doc"], "the fixed list is not asked; each extension once");
    assert!(ndr::native::per_file_icon_rule(Some("\"%1\""), false) && ndr::native::per_file_icon_rule(None, true));
    assert!(!ndr::native::per_file_icon_rule(Some("%SystemRoot%\\System32\\imageres.dll,-102"), false));
    // 이 PC의 HKCR(읽기만): 실행 파일은 "%1", 글 파일은 확장자마다 하나.
    assert!(ndr::native::extension_has_per_file_icon("exe") && !ndr::native::extension_has_per_file_icon("txt"));
    // 폴더는 셸이 desktop.ini를 읽는 읽기 전용·시스템 속성일 때만 실제 경로로 읽는다(사용자 지정 폴더 아이콘). 보통 폴더는 "<DIR>" 캐시.
    let (custom, plain) = (root.join("left").join("custom"), root.join("left").join("plain"));
    fs::create_dir(&custom).unwrap();
    fs::create_dir(&plain).unwrap();
    let mut permissions = fs::metadata(&custom).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&custom, permissions).unwrap();
    app.refresh(0);
    while app.panes[0].loading || !app.panes[0].entries.iter().any(|e| e.name == "custom") {
        assert!(Instant::now() < end, "listing timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    let folder = app.panes[0].entries.iter().position(|e| e.name == "custom").unwrap();
    let folder_path = app.panes[0].entries[folder].path.clone();
    let _ = app.file_icon(&ctx, 0, folder);
    while !app.overlay_icons.contains_key(&folder_path) {
        assert!(Instant::now() < end, "folder icon timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.file_icon(&ctx, 0, folder).map(|t| t.size()), Some([5, 5]));
    let plain_row = app.panes[0].entries.iter().position(|e| e.name == "plain").unwrap();
    let _ = app.file_icon(&ctx, 0, plain_row);
    assert!(!app.overlay_icons.contains_key(&app.panes[0].entries[plain_row].path));
    let mut permissions = fs::metadata(&custom).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    let _ = fs::set_permissions(&custom, permissions);
    let _ = fs::remove_dir_all(root);
}

/// 사용자 지적(2026-10-31) 패널 동작: 뒤로 가면 떠나온 하위 폴더에 커서가 가고 그 쪽이 보인다, 바퀴는 커서를 위·아래로 옮긴다,
/// ".gitconfig" 같은 점 파일은 확장자 없이 이름 전체를 보인다.
#[test]
fn back_wheel_and_dot_files_follow_the_user_requests() {
    let (root, ctx, mut app) = fixture("panel-requests");
    let left = app.panes[0].path.clone();
    for i in 0..900 {
        fs::create_dir_all(left.join(format!("dir{i:03}"))).unwrap();
    }
    fs::write(left.join(".gitconfig"), b"x").unwrap();
    fs::write(left.join(".config.json"), b"x").unwrap();
    app.settings.set_text("LeftViewStyle", "Names");
    app.refresh(0);
    wait(&mut app, &ctx);
    let row = |app: &App, name: &str| app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    // 점 파일: 이름 칸 = 이름 전체, 확장자 칸은 비움. ".config.json"은 ".config" + "json".
    app.compute_view(0);
    let cells = |app: &App, name: &str| app.panes[0].view.cells[row(app, name)].clone();
    assert_eq!((cells(&app, ".gitconfig")[0].as_str(), cells(&app, ".gitconfig")[1].as_str()), (".gitconfig", ""));
    assert_eq!((cells(&app, ".config.json")[0].as_str(), cells(&app, ".config.json")[1].as_str()), (".config", "json"));
    // 원본 GetAttrString: "____"에 A·H·R·S(폴더 dir000은 속성 없음).
    assert_eq!(cells(&app, "dir000")[5], "____");
    assert_eq!(super::list_view::permission_text(true, 0o755), "drwxr-xr-x");
    assert_eq!(super::list_view::permission_text(false, 0o640), "-rw-r-----");
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    run(&mut app, vec![]);
    run(&mut app, vec![]);
    // 바퀴: 아래로 두 칸 = 커서 두 항목 아래, 위로 한 칸 = 한 항목 위. 오른쪽 패널 위에서 굴리면 그 패널이 활성.
    app.active = 1;
    app.panes[0].cursor = 0;
    let at = app.pane_rects[0].center();
    let wheel = |y: f32| egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, y), modifiers: Default::default() };
    run(&mut app, vec![egui::Event::PointerMoved(at), wheel(-2.0)]);
    assert_eq!((app.panes[0].cursor, app.active), (2, 0), "two notches down move the cursor two items and activate the pane");
    run(&mut app, vec![wheel(1.0)]);
    assert_eq!(app.panes[0].cursor, 1);
    // 뒤 쪽의 폴더로 들어갔다가 뒤로: 그 폴더에 커서, 그 쪽이 보인다(첫 열이 0이 아님).
    let target = row(&app, "dir800");
    app.panes[0].cursor = target;
    app.panes[0].reveal = true;
    run(&mut app, vec![]);
    let first = app.panes[0].first_column;
    assert!(first > 0, "the folder is on a later page");
    app.navigate(0, left.join("dir800"), true);
    wait(&mut app, &ctx);
    run(&mut app, vec![]);
    assert_eq!(app.panes[0].first_column, 0);
    app.dispatch("back", &ctx);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "dir800", "back puts the cursor on the folder we came from");
    run(&mut app, vec![]);
    assert_eq!(app.panes[0].first_column, first, "and shows its page again");
    // 원본 기록 용량 20: 뒤로 갈 곳은 19개(지금 폴더까지 20)를 넘지 않는다.
    for i in 0..25 {
        app.navigate(0, left.join(format!("dir{i:03}")), true);
    }
    wait(&mut app, &ctx);
    assert_eq!(app.history_list(0).len(), 20);
    assert_eq!(app.panes[0].back.first(), Some(&left.join("dir005")), "the oldest entries are dropped");
    let _ = fs::remove_dir_all(root);
}

/// 원본 shortcut.ini miDescription=Alt+Enter: Alt+Enter는 "설명 붙여두기"이고 전체 화면 전환은 없다(사용자 요청으로 되돌림).
#[test]
fn alt_enter_edits_the_description_like_the_original() {
    let (root, ctx, mut app) = fixture("fullscreen");
    let on_file = |app: &mut App| app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    on_file(&mut app);
    let alt_enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
    let raw = egui::RawInput { modifiers: egui::Modifiers::ALT, events: vec![alt_enter], screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let output = ctx.run(raw, |ctx| app.keyboard(ctx));
    let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(!commands.iter().any(|c| matches!(c, egui::ViewportCommand::Fullscreen(_))), "{commands:?}");
    let (expected_root, _, mut expected) = fixture("fullscreen-expected");
    on_file(&mut expected);
    expected.dispatch("description", &ctx);
    let kinds = |app: &App| (app.original_dialogs.len(), app.original_dialogs.last().map(|d| d.title.clone()), app.msgboxes.len());
    assert_eq!(kinds(&app), kinds(&expected), "Alt+Enter = miDescription");
    assert!(kinds(&app).0 + kinds(&app).2 > 0, "the description command opened something: {:?}", kinds(&app));
    let _ = fs::remove_dir_all(expected_root);
    let _ = fs::remove_dir_all(root);
}

/// 원본 AdjustPage(0x7daf24): 목록 크기가 바뀌거나(WMSize) 같은 폴더를 다시 읽으면(FileListChanged) 커서 열이 마지막 보이는 열이 되게
/// 가로 위치를 다시 잡는다(커서 열이 왼쪽 밖이면 그 열부터).
#[test]
fn resize_and_refresh_adjust_the_page_like_the_original() {
    let (root, ctx, mut app) = fixture("adjust-page");
    let left = app.panes[0].path.clone();
    for i in 0..600 {
        fs::create_dir_all(left.join(format!("dir{i:03}"))).unwrap();
    }
    app.settings.set_text("LeftViewStyle", "Names");
    app.refresh(0);
    wait(&mut app, &ctx);
    let run = |app: &mut App, height: f32| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, height))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    run(&mut app, 850.0);
    run(&mut app, 850.0);
    let rows = app.panes[0].page.rows;
    // 커서를 뒤쪽에 두고 보이게 한 뒤, 커서를 보이는 첫 열로 옮긴다(위치는 그대로).
    app.panes[0].cursor = 500;
    app.panes[0].reveal = true;
    run(&mut app, 850.0);
    let first = app.panes[0].first_column;
    assert!(first > 0);
    app.panes[0].cursor = first * rows;
    run(&mut app, 850.0);
    assert_eq!(app.panes[0].first_column, first, "moving inside the page keeps the position");
    // 목록 크기가 바뀌면(창 높이) 커서 열이 마지막 보이는 열이 된다.
    run(&mut app, 650.0);
    run(&mut app, 650.0);
    let columns = app.panes[0].page.columns;
    let rows = app.panes[0].page.rows;
    let column = app.panes[0].cursor / rows;
    assert_eq!(app.panes[0].first_column, (column / columns) * columns, "page scrolling (default): WMSize → AdjustPage shows the cursor's page");
    // 줄 단위(ScrollStep 1): 커서 열이 마지막 보이는 열이 된다.
    app.settings.values.insert("LeftScrollStep".into(), serde_json::json!(1));
    app.panes[0].first_column = column;
    run(&mut app, 650.0);
    run(&mut app, 600.0);
    run(&mut app, 600.0);
    let rows = app.panes[0].page.rows;
    let column = app.panes[0].cursor / rows;
    assert!(column >= first);
    assert_eq!(app.panes[0].first_column, (column + 1).saturating_sub(columns), "WMSize → AdjustPage");
    // 같은 폴더를 다시 읽어도 같다(커서 열을 처음 보이는 열로 둔 뒤 새로 고침).
    app.panes[0].first_column = column;
    run(&mut app, 600.0);
    assert_eq!(app.panes[0].first_column, column);
    app.refresh(0);
    wait(&mut app, &ctx);
    run(&mut app, 600.0);
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, format!("dir{:03}", app.panes[0].cursor - 1), "cursor stays");
    assert_eq!(app.panes[0].first_column, (column + 1).saturating_sub(columns), "FileListChanged → AdjustPage");
    let _ = fs::remove_dir_all(root);
}

/// 원본 FUN_00487ce8·SetSortType(0x7dbb28): 자세히 보기에서 같은 폴더를 다시 읽으면 옛 세로 위치(SetScrollTop), 정렬을 바꾸면 커서
/// 항목이 보이게(SetItemIndex), 다른 폴더로 가면 첫 항목부터 보인다.
#[test]
fn details_scroll_follows_refresh_sort_and_navigation_like_the_original() {
    let (root, ctx, mut app) = fixture("details-scroll");
    let left = app.panes[0].path.clone();
    for i in 0..300 {
        fs::write(left.join(format!("f{i:03}.txt")), b"x").unwrap();
    }
    fs::create_dir_all(left.join("sub")).unwrap();
    for i in 0..300 {
        fs::write(left.join("sub").join(format!("g{i:03}.txt")), b"x").unwrap();
    }
    app.settings.set_text("LeftViewStyle", "Details");
    app.refresh(0);
    wait(&mut app, &ctx);
    let run = |app: &mut App| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    run(&mut app);
    run(&mut app);
    let row = |app: &App, name: &str| app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    app.panes[0].cursor = row(&app, "f250.txt");
    app.panes[0].reveal = true;
    run(&mut app);
    let top = app.panes[0].page.top;
    assert!(top > 0, "scrolled down to the cursor");
    // 같은 폴더 다시 읽기: 옛 위치를 지킨다.
    app.refresh(0);
    wait(&mut app, &ctx);
    run(&mut app);
    run(&mut app);
    assert_eq!(app.panes[0].page.top, top, "SetScrollTop(old top)");
    // 내림차순으로 정렬: 커서 항목(f250)이 앞쪽으로 옮겨 가고 보인다.
    assert!(app.sort_command("sortDescending"));
    run(&mut app);
    run(&mut app);
    let page = app.panes[0].page.clone();
    let cursor = app.panes[0].cursor;
    assert_eq!(app.panes[0].entries[cursor].name, "f250.txt");
    assert!(page.top <= cursor && cursor <= page.last, "SetSortDescend → SetItemIndex: {page:?} {cursor}");
    // 다른 폴더: 첫 항목부터.
    app.navigate(0, left.join("sub"), true);
    wait(&mut app, &ctx);
    run(&mut app);
    run(&mut app);
    assert_eq!((app.panes[0].cursor, app.panes[0].page.top), (0, 0), "a new folder starts at the top");
    let _ = fs::remove_dir_all(root);
}


/// 원본 MouseDown(0x7d63c8): 가운데 단추로 눌러도 SetItemFocused(커서만 옮기고 표시는 그대로).
#[test]
fn middle_click_moves_the_cursor_like_the_original() {
    let (root, ctx, mut app) = fixture("middle-click");
    let left = app.panes[0].path.clone();
    for i in 0..20 {
        fs::create_dir_all(left.join(format!("d{i:03}"))).unwrap();
    }
    app.settings.set_text("LeftViewStyle", "Details");
    app.refresh(0);
    wait(&mut app, &ctx);
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    run(&mut app, vec![]);
    run(&mut app, vec![]);
    let target = left.join("d005");
    let rect = app.folder_rows.iter().find(|(pane, _, path)| *pane == 0 && *path == target).map(|(_, rect, _)| *rect).expect("the folder row is drawn");
    let at = rect.center();
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Middle, pressed, modifiers: Default::default() };
    run(&mut app, vec![egui::Event::PointerMoved(at), button(true)]);
    run(&mut app, vec![button(false)]);
    let pane = &app.panes[0];
    assert_eq!((pane.entries[pane.cursor].name.as_str(), pane.selected.len()), ("d005", 0));
    let _ = fs::remove_dir_all(root);
}

/// 원본 TNXShellChangeNotify(패널 +0x374): 다른 프로그램이 지금 폴더에 파일을 만들면 Delay(1500ms) 뒤 그 폴더를 다시 읽는다.
#[test]
fn panes_refresh_after_outside_changes_like_tnxshellchangenotify() {
    let (root, ctx, mut app) = fixture("change-notify");
    let left = app.panes[0].path.clone();
    app.watch_folders = true;
    app.poll(&ctx);
    assert!(app.panes[0].watch.as_ref().is_some_and(|(p, _)| *p == left), "the local pane folder is watched");
    std::thread::sleep(std::time::Duration::from_millis(400));
    let started = std::time::Instant::now();
    fs::write(left.join("made-outside.txt"), b"x").unwrap();
    let end = started + std::time::Duration::from_secs(8);
    while !app.panes[0].entries.iter().any(|e| e.name == "made-outside.txt") {
        assert!(std::time::Instant::now() < end, "no refresh after the outside change");
        app.poll(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(started.elapsed() >= std::time::Duration::from_millis(1400), "refreshed after the 1500 ms delay, not at once");
    app.watch_folders = false;
    app.poll(&ctx);
    assert!(app.panes[0].watch.is_none(), "watching stops when disabled");
    let _ = fs::remove_dir_all(root);
}

/// 원본 TfMain DFM(pmFile … pmHelp, Visible=False 항목 뺌, 겹친·끝 구분선은 하나로/없이): 주 메뉴에 보이는 항목과 차례.
#[test]
fn main_menu_items_follow_the_tfmain_dfm() {
    let submenu = |id: &str| match id {
        "@checksum" => "miChecksum",
        "@workFolders" => "miSetWorkFolder+miGoToWorkFolder",
        "@listStyle" => "miListStyle",
        "@columns" => "miColumn",
        "@sort" => "miSortBy",
        "@filter" => "miFilter",
        "@ftpSites" => "miFTPConnect",
        _ => "?",
    };
    let shown = |items: &[(&str, &str, &str)]| {
        let mut out: Vec<String> = Vec::new();
        let (mut any, mut pending) = (false, false);
        for (id, _, _) in items {
            if *id == "-" {
                pending = any;
                continue;
            }
            let name = match *id {
                "language" => Some("miLanguage".to_owned()),
                "theme" => Some("miSkin".to_owned()),
                _ if id.starts_with('@') => Some(submenu(id).to_owned()),
                _ if App::csharp_only_menu_item(id) || super::menu_data::ORIGINAL_HIDDEN.contains(id) => None,
                _ => App::original_menu_name(id).map(str::to_owned),
            };
            let Some(name) = name else { continue };
            if pending {
                out.push("-".into());
                pending = false;
            }
            any = true;
            out.push(name);
        }
        out.join(" ")
    };
    let expected = [
        "miOpen miParameterOpen miOpenAllSelectedFiles - miCopyTo miMoveTo - miDelete miShred - miRename miAdvRename miChangeAttributes - miCalculateSelectedFolderSize - miProperties miDescription miFileInfo - miCreateEmptyFile miShortcutOnDesktop miShortcutOnQuickLaunch - miChecksum - miExit",
        "miFolderTree miFavorites - miNewFolder - miHistoryBack miHistoryForward - miToParent miToRoot miToLastDest - miNetwork - miSetWorkFolder+miGoToWorkFolder - miRefresh",
        "miEditCopy miEditCut miEditPaste miPasteAsShortcut - miCopyFullPath miCopyFileName miCopyPath - miSelect miSelectAll miInvertSelection miAdvSelect miAdvDeselect - miRestoreLastSel miSelectSameExt miSelectSameName - miFind miFindNext",
        "miLanguage miSkin - miDualView miShowFolders - miViewToolbar miViewFindResult miViewVertBar miViewButtonBar - miShowDriveButtons miViewFolderInfoBar miViewFileInfoBar - miNewTab - miListStyle miColumn - miSortBy miFilter - miShowHidden",
        "miToDesktop miToDocuments miToPictures miToMusic miToVideos miToDownloads - miControlPanel miAddRemoveProg - miRecycleBin miEmptyRecycleBin - miToTemp miEmptyTemp - miCommandLine",
        "miArcAdd - miArcExtract miArcExtractToCurr miArcExtractAuto",
        "miFTPQuickConnect - miFTPConnect miFTPDisconnect - miFTPGet miFTPPut - miMapNetworkDrive miUnmapNetworkDrive",
        "miTextViewer miTextEditor - miClearDescriptions - miSaveList - miSplitFile miJoinFile - miCleanupDisk - miCompareFolder - miOptions",
        "miHelp miShortcut - miHomepage - miCheckUpdate - miAbout",
    ];
    let menus: Vec<String> = super::menu_data::MENUS.iter().map(|(_, _, _, items)| shown(items)).collect();
    assert_eq!(menus.len(), expected.len());
    let wrong: Vec<String> = menus.iter().zip(expected).filter(|(got, want)| got.as_str() != *want).map(|(got, want)| format!("\n got: {got}\nwant: {want}")).collect();
    assert!(wrong.is_empty(), "{}", wrong.concat());
}

/// 원본 GetItemAt(0x7dac1c): 자세히(형식 2) 줄 간격 = 항목 높이 + LineSpace이고 줄 위의 틈은 그 항목, 열로 흐르는 목록(형식 0·1)은
/// 항목 아래 틈이 그 항목이다.
#[test]
fn clicks_in_the_line_space_pick_the_original_item() {
    let (root, ctx, mut app) = fixture("line-space");
    let left = app.panes[0].path.clone();
    for i in 0..8 {
        fs::create_dir_all(left.join(format!("d{i:03}"))).unwrap();
    }
    app.settings.values.insert("ListLineSpace".into(), serde_json::json!(6));
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    for (view, gap_above) in [("Details", true), ("Auto", false)] {
        app.settings.set_text("LeftViewStyle", view);
        app.refresh(0);
        wait(&mut app, &ctx);
        run(&mut app, vec![]);
        app.folder_rows.clear();
        run(&mut app, vec![]);
        let rect = |app: &App, name: &str| app.folder_rows.iter().find(|(pane, _, path)| *pane == 0 && *path == left.join(name)).map(|(_, rect, _)| *rect).expect("the folder row is drawn");
        let (a, b) = (rect(&app, "d002"), rect(&app, "d003"));
        assert_eq!(b.top() - a.bottom(), 6.0, "{view}: the gap between items is exactly LineSpace");
        let at = egui::pos2(a.center().x, a.bottom() + 3.0);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Middle, pressed, modifiers: Default::default() };
        run(&mut app, vec![egui::Event::PointerMoved(at), button(true)]);
        run(&mut app, vec![button(false)]);
        let pane = &app.panes[0];
        assert_eq!(pane.entries[pane.cursor].name, if gap_above { "d003" } else { "d002" }, "{view}");
    }
    let _ = fs::remove_dir_all(root);
}

/// 원본 miShowFoldersClick(0x445258) → FUN_0048da38: 지금 패널에만 목록 왼쪽 폴더 트리(너비 treewidth, 처음 120)와 3화소 나눔 막대,
/// 목록은 그 오른쪽. 그 패널 설정(ShowLeft/RightFolderTree)에 남는다.
#[test]
fn show_folders_adds_a_tree_inside_the_active_pane() {
    let (root, ctx, mut app) = fixture("pane-tree");
    let left = app.panes[0].path.clone();
    for i in 0..3 {
        fs::create_dir_all(left.join(format!("d{i:03}"))).unwrap();
    }
    app.settings.set_text("LeftViewStyle", "Details");
    app.refresh(0);
    wait(&mut app, &ctx);
    let run = |app: &mut App| {
        app.folder_rows.clear();
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    let row_left = |app: &App| app.folder_rows.iter().find(|(pane, _, path)| *pane == 0 && *path == left.join("d001")).map(|(_, rect, _)| rect.left()).expect("drawn");
    run(&mut app);
    run(&mut app);
    let before = row_left(&app);
    app.active = 0;
    app.dispatch("toggleFolders", &ctx);
    assert!(app.settings.flag("ShowLeftFolderTree", false) && !app.settings.flag("ShowRightFolderTree", false), "only the active pane");
    assert_eq!(app.menu_checked("toggleFolders"), Some(true));
    run(&mut app);
    run(&mut app);
    assert_eq!(row_left(&app) - before, 123.0, "the list starts after the 120 px tree and the 3 px splitter");
    assert!(app.panes[0].tree_open.iter().any(|p| left.starts_with(p)), "SetPath opens the ancestors of the pane folder");
    app.dispatch("toggleFolders", &ctx);
    run(&mut app);
    run(&mut app);
    assert_eq!(row_left(&app), before);
    let _ = fs::remove_dir_all(root);
}

/// 원본 패널 트리(TNXShellTreeView = VCL TTreeView, OnChange FUN_0048e320): 트리에 초점이 있으면 ↓↑는 고른 항목을 옮기고
/// 그때마다 패널이 그 폴더로 간다. ←는 펼친 항목을 접고, 접힌 항목에서는 부모로. 글자는 그 글자로 시작하는 다음 항목.
/// 목록을 누르면 초점이 목록으로 돌아간다.
#[test]
fn pane_tree_keys_follow_the_ttreeview() {
    use ndr::shell_tree::{ShellNode, Special};
    let (root, ctx, mut app) = fixture("tree-keys");
    let left = app.panes[0].path.clone();
    for name in ["alpha", "beta"] {
        fs::create_dir_all(left.join(name).join("inner")).unwrap();
    }
    let node = |path: &Path, has_children: bool| ShellNode { key: path.to_path_buf(), name: path.file_name().unwrap().to_string_lossy().into_owned(), path: Some(path.to_path_buf()), has_children, special: Special::None };
    app.settings.set_flag("ShowLeftFolderTree", true);
    app.active = 0;
    app.tree_focus = Some(0);
    app.panes[0].tree_open.insert(left.clone());
    app.panes[0].tree_rows = vec![
        (node(&left, true), 0, true, true),
        (node(&left.join("alpha"), true), 1, true, false),
        (node(&left.join("beta"), true), 1, true, false),
    ];
    let press = |app: &mut App, key: egui::Key, text: Option<&str>| {
        let mut events = vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }];
        events.extend(text.map(|t| egui::Event::Text(t.into())));
        let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
        wait(app, &ctx);
    };
    press(&mut app, egui::Key::ArrowDown, None);
    assert_eq!(app.panes[0].path, left.join("alpha"), "↓ = next item, OnChange moves the pane");
    // 글자 b: 다음 "b…" 항목.
    app.panes[0].tree_synced = Some(app.panes[0].path.clone());
    press(&mut app, egui::Key::B, Some("b"));
    assert_eq!(app.panes[0].path, left.join("beta"));
    // ←(접힌 항목): 부모로.
    app.panes[0].tree_synced = Some(app.panes[0].path.clone());
    press(&mut app, egui::Key::ArrowLeft, None);
    assert_eq!(app.panes[0].path, left, "← on a collapsed item selects its parent");
    // ←(펼친 항목): 접는다.
    press(&mut app, egui::Key::ArrowLeft, None);
    assert!(!app.panes[0].tree_open.iter().any(|k| ndr::shell_tree::same_key(k, &left)), "← on an open item collapses it");
    // 목록을 누르면 초점이 목록으로.
    app.pane_rects[0] = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(600.0, 800.0));
    app.panes[0].tree_rect = Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(120.0, 800.0)));
    let click = vec![
        egui::Event::PointerMoved(egui::pos2(300.0, 300.0)),
        egui::Event::PointerButton { pos: egui::pos2(300.0, 300.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
    ];
    let raw = egui::RawInput { events: click, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
    assert_eq!(app.tree_focus, None, "a click in the list gives the focus back to the list");
    let _ = fs::remove_dir_all(root);
}


/// 원본 UpdateScrollBar·ArrowKey·AdjustPage(형식 2): 칸 너비 합이 목록보다 넓으면 가로로 넘길 수 있고, 오른쪽·왼쪽 글쇠는 20씩,
/// 목록 크기가 바뀌면 가로 위치가 0이 된다.
#[test]
fn narrow_details_scroll_horizontally_like_the_original() {
    let (root, ctx, mut app) = fixture("details-hscroll");
    let left = app.panes[0].path.clone();
    for i in 0..5 {
        fs::write(left.join(format!("a-rather-long-file-name-{i}.txt")), b"x").unwrap();
    }
    app.settings.set_text("LeftViewStyle", "Details");
    app.refresh(0);
    wait(&mut app, &ctx);
    let run = |app: &mut App, events: Vec<egui::Event>, height: f32| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, height))), events, ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            app.keyboard(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.columns(2, |columns| {
                    app.pane_ui(&mut columns[0], 0, ctx);
                    app.pane_ui(&mut columns[1], 1, ctx);
                });
            });
        });
    };
    app.active = 0;
    for _ in 0..3 {
        run(&mut app, vec![], 700.0);
    }
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    run(&mut app, vec![key(egui::Key::ArrowRight), key(egui::Key::ArrowRight)], 700.0);
    run(&mut app, vec![], 700.0);
    assert_eq!(app.panes[0].detail_x, 40.0, "two Right presses scroll 2 × 20 px");
    let cursor = app.panes[0].cursor;
    run(&mut app, vec![key(egui::Key::ArrowLeft)], 700.0);
    run(&mut app, vec![], 700.0);
    assert_eq!((app.panes[0].detail_x, app.panes[0].cursor), (20.0, cursor), "Left scrolls back and the cursor stays");
    // 목록 크기가 바뀌면(AdjustPage) 0.
    run(&mut app, vec![], 650.0);
    run(&mut app, vec![], 650.0);
    assert_eq!(app.panes[0].detail_x, 0.0);
    let _ = fs::remove_dir_all(root);
}

/// 원본 단축키 표가 pmMain 팝업 항목·하위 메뉴 이름에 글쇠를 걸었을 때의 명령 id(menu_data::ORIGINAL_ACTIONS)도 받는다.
#[test]
fn shortcut_names_of_popup_and_submenu_items_are_known_commands() {
    let (root, ctx, mut app) = fixture("popup-ids");
    let ids = [
        "contextView", "contextEdit", "contextCopy", "contextCut", "contextPaste", "contextCopyTo", "contextMoveTo", "contextDelete", "contextRename", "contextAttributes",
        "contextDescription", "contextArchiveAdd", "contextAutoExtract", "contextProperties", "dropCopy", "dropMove", "dropShortcut", "dropCancel", "ftpConnect",
        "lastDestination", "setWorkFolder", "treeRefreshRecursive", "theme", "shortcutsHelp",
    ];
    for id in ids {
        app.status.clear();
        app.original_dialogs.clear();
        app.msgboxes.clear();
        app.dispatch(id, &ctx);
        assert!(!app.status.starts_with("Unknown command"), "{id}: {}", app.status);
    }
    let _ = fs::remove_dir_all(root);
}


/// 원본 KeyDown(FUN_0048423c)의 단축키가 아닌 글쇠: '.' = 상위 폴더(빠른 찾기 글이 비었을 때), Ctrl+Tab·Ctrl+Shift+Tab = 다음·이전 탭,
/// Shift+글자 = 그 드라이브(영문 입력이거나 hanshdrive를 켰을 때).
#[test]
fn keydown_period_ctrl_tab_and_shift_drive_follow_the_original() {
    let (root, ctx, mut app) = fixture("keydown");
    let left = app.panes[0].path.clone();
    let press = |app: &mut App, key: egui::Key, modifiers: egui::Modifiers, text: Option<&str>| {
        let mut events = vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
        events.extend(text.map(|t| egui::Event::Text(t.into())));
        let raw = egui::RawInput { modifiers, events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
        wait(app, &ctx);
    };
    app.active = 0;
    app.navigate(0, left.join("folder"), true);
    wait(&mut app, &ctx);
    press(&mut app, egui::Key::Period, egui::Modifiers::NONE, Some("."));
    assert_eq!(app.panes[0].path, left, "'.' = parent folder");
    app.dispatch("newTab", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].tab), (2, 1));
    press(&mut app, egui::Key::Tab, egui::Modifiers::CTRL, None);
    assert_eq!(app.panes[0].tab, 0, "Ctrl+Tab wraps to the first tab");
    press(&mut app, egui::Key::Tab, egui::Modifiers::CTRL | egui::Modifiers::SHIFT, None);
    assert_eq!(app.panes[0].tab, 1, "Ctrl+Shift+Tab = previous tab");
    app.settings.set_flag("UseAlternateDriveHandling", true);
    app.settings.set_flag("SaveDriveFolders", false);
    app.settings.values.remove("LastFoldersByDrive");
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let letter = system.chars().next().unwrap().to_ascii_uppercase();
    let key = egui::Key::from_name(&letter.to_string()).unwrap();
    press(&mut app, key, egui::Modifiers::SHIFT, Some(&letter.to_string()));
    assert_eq!(app.panes[0].path, PathBuf::from(format!("{letter}:\\")), "Shift+{letter} = that drive's root (no remembered folder)");
    let _ = fs::remove_dir_all(root);
}

/// 큰 폴더(파일 2만 개)에서 한 장면 그리기 시간(손으로만 돌린다: cargo test --release -- --ignored big_folder_frame_time).
#[test]
#[ignore]
fn big_folder_frame_time() {
    let (root, ctx, mut app) = fixture("big-folder");
    let big = root.join("big");
    fs::create_dir_all(&big).unwrap();
    for i in 0..20000 {
        fs::write(big.join(format!("file{i:05}.txt")), b"").unwrap();
    }
    app.navigate(0, big.clone(), false);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].entries.len(), 20001);
    for view in ["Auto", "Details", "Names", "Thumbnails"] {
        app.settings.set_text("LeftViewStyle", view);
        let mut times = Vec::new();
        for frame in 0..30 {
            app.folder_rows.clear();
            if frame % 3 == 0 {
                app.panes[0].cursor = (app.panes[0].cursor + 997) % 20001;
            }
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
            let start = Instant::now();
            let output = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.columns(2, |columns| {
                        app.pane_ui(&mut columns[0], 0, ctx);
                        app.pane_ui(&mut columns[1], 1, ctx);
                    });
                });
            });
            let shapes = output.shapes.len();
            let _ = ctx.tessellate(output.shapes, output.pixels_per_point);
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            if frame == 29 {
                eprintln!("{view}: {shapes} shapes");
            }
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!("{view}: median {:.2} ms, max {:.2} ms", times[times.len() / 2], times[times.len() - 1]);
    }
    let _ = fs::remove_dir_all(root);
}
