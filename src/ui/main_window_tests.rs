//! Verifies the C# MainWindow frame (title row, favorites row, pane header row, tabs, address bar, info bar) in the Rust UI. Korean only.
//! Uses only isolated temporary folders.
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
#[test]
fn thumbnail_cache_revision_is_invalidated_by_sort_and_refresh() {
    let (root, ctx, mut app) = fixture("thumbnail-revision");
    let initial = app.panes[0].thumb_content_revision;
    app.sort_pane(0);
    let sorted = app.panes[0].thumb_content_revision;
    assert_ne!(sorted, initial, "reordering changes the thumbnail cache key");
    app.refresh(0);
    wait(&mut app, &ctx);
    assert_ne!(app.panes[0].thumb_content_revision, sorted,
        "a new listing must invalidate thumbnail row measurements");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sort_keeps_cursor_on_drive_when_parent_has_the_same_path() {
    let (root, _ctx, mut app) = fixture("sort-drive-parent-identity");
    app.active = 0;
    let pane = &mut app.panes[0];
    let drive_path = PathBuf::from(r"Q:\");
    let parent = pane.entries.iter_mut().find(|entry| entry.is_parent()).unwrap();
    parent.path = drive_path.clone();
    pane.entries.push(Entry {
        name: "Q:".into(),
        drive: true,
        ..Entry::parent_item(drive_path.clone(), None)
    });
    pane.cursor = pane.entries.len() - 1;
    let marked = pane.entries.iter().position(|entry| entry.name == "a.txt").unwrap();
    pane.selected.insert(marked);
    pane.selected.insert(pane.entries.iter().position(|entry| entry.is_parent()).unwrap());

    assert!(app.sort_command("sortDescending"));
    let pane = &app.panes[0];
    assert_eq!(pane.entries[0].path, drive_path);
    assert!(pane.entries[0].is_parent());
    assert!(pane.entries[pane.cursor].drive, "the cursor must not jump to '..' (same path)");
    assert_eq!(pane.entries[pane.cursor].path, drive_path);
    assert_eq!(pane.selected.iter().map(|&row| pane.entries[row].name.as_str()).collect::<Vec<_>>(), ["..", "a.txt"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn filename_width_cache_reuses_measurements_until_listing_or_font_changes() {
    let (root, ctx, mut app) = fixture("filename-width-cache");
    let measure = |app: &App, size: f32| {
        let mut result = 0.0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = app.widest_pane_filename(ui, 0, &egui::FontId::proportional(size));
            });
        });
        result
    };
    let original = measure(&app, 14.0);
    assert!(original > 0.0);
    assert_eq!(measure(&app, 14.0), original, "a second frame reuses the same width");

    app.panes[0].entries[1].name = "very_long_filename_for_cache_invalidation_check_0000000000000000000.txt".into();
    assert_eq!(measure(&app, 14.0), original, "the cache key is unchanged between list revisions");
    app.sort_pane(0);
    let updated = measure(&app, 14.0);
    assert!(updated > original, "re-sorting must remeasure a changed name");
    let bigger = measure(&app, 28.0);
    assert!(bigger > updated, "changing font size must remeasure");
    app.settings.set_text("ListFont", "Another Test Font");
    let changed_face = measure(&app, 28.0);
    assert_eq!(changed_face, bigger, "the configured face participates in cache identity even when the test font is unchanged");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn large_marked_list_survives_refresh_and_ignores_removed_names() {
    let (root, ctx, mut app) = fixture("bulk-marks-refresh");
    let folder = app.panes[0].path.clone();
    for i in 0..320 {
        fs::write(folder.join(format!("bulk_{i:03}_Sample.txt")), [i as u8]).unwrap();
    }
    app.refresh(0);
    wait(&mut app, &ctx);
    let mut chosen: Vec<String> = Vec::new();
    for (index, entry) in app.panes[0].entries.iter().enumerate() {
        if let Some(number) = entry.name.strip_prefix("bulk_")
            .and_then(|s| s.get(..3))
            .and_then(|s| s.parse::<usize>().ok())
            && number % 2 == 0 {
            app.panes[0].selected.insert(index);
            chosen.push(entry.name.clone());
        }
    }
    assert_eq!(chosen.len(), 160);
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "bulk_178_Sample.txt").unwrap();
    fs::remove_file(folder.join("bulk_178_Sample.txt")).unwrap();
    fs::write(folder.join("bulk_320_Sample.txt"), b"new").unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    let kept: Vec<String> = app.panes[0].selected.iter()
        .map(|&index| app.panes[0].entries[index].name.clone()).collect();
    assert_eq!(kept.len(), 159);
    assert!(!kept.contains(&"bulk_178_Sample.txt".into()));
    assert!(!kept.contains(&"bulk_320_Sample.txt".into()));
    assert!(kept.contains(&"bulk_184_Sample.txt".into()));
    assert!(app.panes[0].selected.iter().all(|&index| {
        let entry = &app.panes[0].entries[index];
        !entry.is_parent() && !entry.drive
    }));
    fs::remove_dir_all(root).unwrap();
}

/// Draws the menu, the favorites row, and both panes once (to check there are no errors while drawing).
fn draw(app: &mut App, ctx: &egui::Context) {
    for pane in &mut app.panes{pane.folder_auto_visible.clear();pane.folder_auto_candidates.clear();}
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| app.menu(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(2, |columns| {
                app.pane_ui(&mut columns[0], 0, ctx);
                app.pane_ui(&mut columns[1], 1, ctx);
            });
        });
        app.finish_visible_folder_size_frame(ctx);
    });
}

#[test]
fn preview_geometry_is_shared_between_frames() {
    let (root, ctx, mut app) = fixture("shared-geometry");
    app.settings.set_text("LeftViewStyle", "Thumbnails");
    draw(&mut app, &ctx);
    let first = app.panes[0].thumb_heights.as_ref().unwrap().1.clone();
    let (tops, heights, ..) = app.panes[0].thumb_rows.as_ref().unwrap();
    assert!(Arc::ptr_eq(tops, &first.tops));
    assert!(Arc::ptr_eq(heights, &first.heights));
    draw(&mut app, &ctx);
    assert!(Arc::ptr_eq(&first, &app.panes[0].thumb_heights.as_ref().unwrap().1));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn grouped_auto_columns_reuse_rows_and_invalidate_on_changes() {
    let (root, ctx, mut app) = fixture("auto-columns-cache");
    app.settings.set_text("LeftViewStyle", "Auto");
    app.settings.values.insert("LeftColumnCount".into(), serde_json::json!(3));
    app.settings.set_text("LeftGroupMode", "Extension");
    draw(&mut app, &ctx);
    let initial = app.panes[0].auto_rows_cache.as_ref().expect("auto rows cache").rows.clone();
    draw(&mut app, &ctx);
    assert!(Arc::ptr_eq(&initial, &app.panes[0].auto_rows_cache.as_ref().unwrap().rows));
    app.settings.set_text("LeftGroupMode", "Size");
    draw(&mut app, &ctx);
    let changed = app.panes[0].auto_rows_cache.as_ref().unwrap();
    assert!(!Arc::ptr_eq(&initial, &changed.rows), "group mode must invalidate cached rows");
    let revision = changed.revision;
    app.sort_pane(0);
    draw(&mut app, &ctx);
    assert_ne!(revision, app.panes[0].auto_rows_cache.as_ref().unwrap().revision);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn main_menu_follows_the_original_tfmain_dropdowns() {
    let (root, ctx, mut app) = fixture("menus");
    // The original TfMain menu buttons tbFile … tbHelp (from the text " 파일(&F) " with spaces and & removed), and their order.
    let titles: Vec<String> = menu_data::MENUS.iter().map(|(key, ko, _, _)| app.language_pack.text(key, ko).trim().replace('&', "")).collect();
    assert_eq!(titles, ["파일(F)", "폴더(O)", "편집(E)", "보기(W)", "시스템(S)", "압축(A)", "네트워크(N)", "도구(T)", "도움말(H)"]);
    let ids = |index: usize| menu_data::MENUS[index].3.iter().map(|(id, _, _)| *id).collect::<Vec<_>>();
    // pmFile first/last item, pmFolder's favorites and work folders, pmView's submenus, pmTools's split/join.
    assert_eq!((ids(0)[0], *ids(0).last().unwrap()), ("open", "exit"));
    assert!(ids(1).starts_with(&["folderTree", "favoritesMenu"]) && ids(1).contains(&"@workFolders") && ids(1).last() == Some(&"refresh"));
    assert!(["@listStyle", "@columns", "@sort", "@filter"].iter().all(|s| ids(3).contains(s)));
    assert!(ids(7).contains(&"split") && ids(7).contains(&"join") && !ids(7).contains(&"cleanup"), "the disk cleanup item was removed");
    // Original item text: Menu.miFind (MENU_CAPTIONS), Menu.miOpen (ORIGINAL_ACTIONS); the C# text if absent.
    assert_eq!(app.menu_label("find", "?"), app.language_pack.text("Menu.miFind", "?").replace('&', ""));
    assert_eq!(app.menu_label("open", "?"), app.language_pack.text("Menu.miOpen", "?").replace('&', ""));
    assert_eq!(app.menu_label("createZip", "ZIP 만들기…"), "ZIP 만들기…");
    // Original right-hand menu-item shortcut text (TMenuItem.ShortCut → shortcut.ini → ShortCutToText). Items that exist only in the C# version have none.
    let shortcuts = compat::ShortcutMap::parse(None);
    let code = |name: &str| shortcuts.menu_shortcuts.get(name).copied().unwrap_or(0);
    let open_with_parameters = menu_data::ORIGINAL_ACTIONS.iter().find(|(name, _)| *name == "miParameterOpen").map(|(_, id)| *id).unwrap();
    assert_eq!(App::menu_shortcut_text(open_with_parameters, &shortcuts), compat::shortcut_to_text(code("miParameterOpen")));
    assert!(!App::menu_shortcut_text(open_with_parameters, &shortcuts).is_empty());
    assert_eq!(App::menu_shortcut_text("createZip", &shortcuts), "");
    // Original miSelect = Space (toggles the current item's mark); the info bar item in the View menu is a checked item.
    app.active = 0;
    app.panes[0].cursor = 1;
    app.dispatch("select", &ctx);
    assert!(app.panes[0].selected.contains(&1));
    assert_eq!(app.menu_checked("toggleFileInfoBar"), Some(true));
    draw(&mut app, &ctx);
    // Original TToolBar CM_DIALOGCHAR: Alt+letter that is not a shortcut ("폴더(&O)" -> Alt+O) opens the dropdown of that menu button.
    assert!(!egui::Popup::is_any_open(&ctx));
    let alt_o = egui::Event::Key { key: egui::Key::O, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
    let raw = egui::RawInput { events: vec![alt_o], screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        app.keyboard(ctx);
        egui::TopBottomPanel::top("menu").show(ctx, |ui| app.menu(ui));
    });
    assert!(app.menu_access.is_none(), "consumed by the menu bar");
    assert!(egui::Popup::is_any_open(&ctx), "Alt+O opens a menu");
    // While the menu is open the letter does not go to the pane but selects that menu's access-letter item: Folder menu "&V. 즐겨찾기..." → favorites dropdown.
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
    // Windows menu bar: while one menu (File) is open, moving the mouse over another title (Folder) switches to that menu (without pressing).
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

#[test]
fn hovered_file_menu_shortcut_keeps_the_row_highlight() {
    let (root, ctx, mut app) = fixture("menu-shortcut-hover");
    app.settings.set_text("Theme", "Black");
    let command = menu_data::ORIGINAL_ACTIONS.iter()
        .find(|(name, _)| *name == "miParameterOpen").unwrap().1;
    let shortcut = App::menu_shortcut_text(command, &compat::ShortcutMap::parse(None));
    assert!(!shortcut.is_empty());
    let frame = |app: &mut App, events| {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0)));
        ctx.run(egui::RawInput { screen_rect, events, ..Default::default() }, |ctx| {
            app.keyboard(ctx);
            egui::TopBottomPanel::top("menu").show(ctx, |ui| app.menu(ui));
        })
    };
    frame(&mut app, vec![]);
    frame(&mut app, vec![egui::Event::Key {
        key: egui::Key::F, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT,
    }]);
    frame(&mut app, vec![]);
    let opened = frame(&mut app, vec![]);
    let at = opened.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.text() == shortcut =>
            Some(text.pos + egui::vec2(text.galley.size().x + 1.0, text.galley.size().y * 0.5)),
        _ => None,
    }).expect("File menu draws the shortcut");
    let hovered = frame(&mut app, vec![egui::Event::PointerMoved(at)]);
    let background = hovered.shapes.iter().rev().find_map(|shape| match &shape.shape {
        egui::Shape::Rect(rect) if rect.rect.contains(at) && shape.clip_rect.contains(at)
            && rect.fill != Color32::TRANSPARENT => Some(rect.fill),
        _ => None,
    });
    // The menu can still be fading in, so compare the RGB before opacity.
    let [r, g, b, _] = background.expect("a background behind the shortcut").to_srgba_unmultiplied();
    assert!([r, g, b].into_iter().zip([0x35, 0x36, 0x38]).all(|(channel, expected)| channel.abs_diff(expected) <= 3),
        "the highlight extends behind the shortcut, got ({r}, {g}, {b})");
    drop(app);
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

/// Windows menu: when two or more items share the same access letter (the original View menu's C = "&C. 기능키 표시줄" and "&C. 열 보기"), each press of the letter
/// only moves the selection (focus) between them without running it. Enter runs the selected item.
/// Windows menu keyboard: a menu opened with Alt+letter highlights its first row, ↓/↑ move the highlight, Enter runs the highlighted command.
/// Folder menu (original pmFolder): folder tree, favorites, new folder... so ↓ ↓ Enter opens the new folder box.
#[test]
fn menu_arrows_and_enter_follow_windows_menus() {
    let (root, ctx, mut app) = fixture("menu-arrows");
    draw(&mut app, &ctx);
    app.active = 0;
    let key = |key: egui::Key, modifiers: egui::Modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    menu_frame(&mut app, &ctx, vec![key(egui::Key::O, egui::Modifiers::ALT)]);
    menu_frame(&mut app, &ctx, Vec::new());
    menu_frame(&mut app, &ctx, Vec::new());
    assert!(egui::Popup::is_any_open(&ctx), "the folder menu is open");
    menu_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)]);
    menu_frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)]);
    assert!(app.original_dialogs.is_empty(), "arrows only move the highlight");
    menu_frame(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    assert_eq!(app.original_dialogs.last().map(|d| d.title.clone()), Some(app.language_pack.text("RES.TITLE_NEWFOLDER", "?")), "the third row (new folder) ran");
    fs::remove_dir_all(root).unwrap();
}

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
    // Enter runs the selected item (the function key bar).
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    menu_frame(&mut app, &ctx, vec![enter]);
    assert_ne!(app.settings.flag("ShowFunctionBar", true), bar);
    // An item with only one letter runs right away (see the submenu_access_letters… test above).
    fs::remove_dir_all(root).unwrap();
}

/// Original submenu access letters: while the menu is open, typing a submenu name's letter (View "&C. 열 보기", Folder "&G. 작업 폴더로 가기")
/// opens that submenu like a Windows menu, and the next letter typed ("&3. 열로 보기", "작업폴더 &1") picks an item inside it.
#[test]
fn submenu_access_letters_open_the_submenu_and_pick_its_item() {
    let (root, ctx, mut app) = fixture("submenu-letters");
    draw(&mut app, &ctx);
    app.active = 0;
    let alt = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
    menu_frame(&mut app, &ctx, vec![alt(egui::Key::W)]);
    assert!(egui::Popup::is_any_open(&ctx), "Alt+W opens the View menu");
    // View menu "&A. 정렬" → "&S. 크기순" (in the original View menu C is shared by the function key bar and the column view).
    menu_frame(&mut app, &ctx, typed(egui::Key::A, "a"));
    menu_frame(&mut app, &ctx, Vec::new());
    assert_ne!(app.settings.text("LeftSortMode", ""), "Size");
    menu_frame(&mut app, &ctx, typed(egui::Key::S, "s"));
    assert_eq!(app.settings.text("LeftSortMode", ""), "Size", "A opened Sort by, S picked size");
    // Folder menu: G → Go to Work Folder, 1 → work folder 1.
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

/// Original FUN_0048aa30 (archive mode): the address row is PathCombine(archive file, inner path) — it draws starting from the folder crumbs where the archive file lives.
/// Going to a folder crumb leaves the archive.
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
    // C# FileVisualConverter: one small shell icon per extension (folders use "<DIR>") (the test uses fake icons).
    assert!(app.file_icons.get("<DIR>").is_some_and(|t| t.is_some()) && app.file_icons.get(".txt").is_some_and(|t| t.is_some()), "{:?}", app.file_icons.keys().collect::<Vec<_>>());
    app.settings.set_flag("ShowIcons", false);
    assert!(app.file_icon(&ctx, 0, 0).is_none(), "ShowIcons off");
    app.settings.set_flag("ShowIcons", true);
    // C# UseOverlayIcons: the extension icon first; once a separate STA thread reads the overlay icon (a fake 3x3 in the test), that path uses it.
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
    // Info bar: original FOLDERINFO (folders, files, size); the byte unit below 1 KiB follows the selected UI language, not the Windows locale.
    let [folder, file, extension] = app.info_texts(0);
    let bytes: u64 = app.panes[0].entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
    assert_eq!(bytes, 7);
    let counts = format!("폴더 <c000000><b>1</b></c>개 · 파일 <c000000><b>3</b></c>개 · <b>{bytes}바이트</b>");
    assert_eq!(folder, counts, "no disk text in tests (disk_info hook)");
    let cursor = app.panes[0].entries[app.panes[0].cursor].clone();
    // Original FUN_0048a5cc: the focused item "size  |  date  |  attributes  |  name" (a folder with unknown size shows FOLDER).
    let size = if cursor.is_dir { "폴더".to_owned() } else { super::file_operation::n0(cursor.size as usize) };
    assert!(file.starts_with(&format!("{size}  |  ")) && file.ends_with(&format!("  |  {}", cursor.name)), "{file}");
    assert_eq!(file.matches("  |  ").count(), 3, "{file}");
    assert_eq!(extension, ".txt 2개 · .log 1개");
    // Original disk text: the drive label + DISKINFO_SHORT (free); the tooltip uses DISKINFO_LONG (free, total).
    app.panes[0].disk = Some(("로컬 디스크 (C:) ".into(), 2048, 4096));
    let (_, drive, right, hint) = app.folder_info(0);
    let (free, total) = (native::format_byte_size(2048), native::format_byte_size(4096));
    assert_eq!((drive.as_str(), right), ("로컬 디스크 (C:) ", format!("<c000000><b>{free}</b></c>")));
    assert_eq!(hint, format!("남은 공간 {free} / 전체 {total}"));
    assert_eq!(app.info_texts(0)[0], format!("{counts}  로컬 디스크 (C:) <c000000><b>{free}</b></c>"));
    app.panes[0].selected = (0..app.panes[0].entries.len()).collect();
    let [folder, file, extension] = app.info_texts(0);
    // Original FUN_00484b24: when there are marked items, SELECTED_INFO (count, byte count + "+" if a folder is included) replaces the disk text.
    let marked = super::file_operation::n0(bytes as usize);
    assert_eq!(folder, format!("{counts}  <b>4</b>개 선택 · <b>{marked}+</b>바이트"));
    assert_eq!(file, app.file_info(0).unwrap(), "the file bar keeps showing the focused item");
    // If there is a description, show the 6B9FFF-colored description instead of the name.
    let focused = app.panes[0].entries[app.panes[0].cursor].name.to_lowercase();
    app.panes[0].descriptions.insert(focused, "메모".into());
    assert!(app.file_info(0).unwrap().ends_with("  |  <c6B9FFF>메모</c>"));
    app.panes[0].descriptions.clear();
    assert_eq!(extension, app.info_texts(0)[2]);
    assert!(extension.contains(".txt 2개"));
    app.panes[0].selected.clear();
    app.panes[0].disk = None;
    // The info-bar menu commands flip the setting and save it.
    app.dispatch("toggleExtensionInfoBar", &ctx);
    assert!(app.settings.flag("ShowExtensionInfoBar", false));
    app.dispatch("toggleFileInfoBar", &ctx);
    assert!(!app.settings.flag("ShowFileInfoBar", true));
    draw(&mut app, &ctx);
    // Tabs: a new tab is at the same location; close all other tabs.
    app.dispatch("newTab", &ctx);
    assert_eq!(app.panes[0].tabs.len(), 2);
    draw(&mut app, &ctx);
    // Address input: a relative path is based on the current folder; a missing folder gives a "folder move" warning.
    app.navigate_address(0, "folder");
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].path, left.join("folder"));
    app.navigate_address(0, "..\\missing");
    assert_eq!(app.msgboxes.first().map(|m| m.title.as_str()), Some("폴더 이동"));
    app.answer_message_box(1, &ctx);
    // Original address field Enter (FUN_0048cb30): http(s):// opens in the default program; ftp:// opens that pane's FTP quick-connect window.
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
    // Address editing: same start as the … button (current path, editing state); Esc ends it.
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
fn github_update_installs_after_one_approval_without_second_dialog() {
    let (root, ctx, mut app) = fixture("single-approval-update");
    let before = app.msgboxes.len();
    app.update_launcher = |path, language| {
        assert_eq!(path, Path::new("mock-update.zip"));
        assert_eq!(language.code, "ko");
        Ok(())
    };
    app.update_progress = Some(super::main_window::UpdateProgress {
        tag: "build-99".into(), phase: ndr::update::UpdatePhase::Preparing,
        downloaded: 1024, total: 1024, error: None, failed_phase: None,
    });
    app.update_package_done(Ok(PathBuf::from("mock-update.zip")), &ctx);
    assert_eq!(app.msgboxes.len(), before, "no second install confirmation");
    assert!(matches!(app.update_progress.as_ref().map(|v| v.phase), Some(ndr::update::UpdatePhase::Installing)));
    app.update_progress = None;
    app.update_launcher = |_, _| Err("installer start error".into());
    app.update_progress = Some(super::main_window::UpdateProgress {
        tag: "build-99".into(), phase: ndr::update::UpdatePhase::Preparing,
        downloaded: 1024, total: 1024, error: None, failed_phase: None,
    });
    app.update_package_done(Ok(PathBuf::from("mock-update.zip")), &ctx);
    assert_eq!(app.msgboxes.len(), before, "failure stays in progress window");
    assert_eq!(app.update_progress.as_ref().unwrap().phase, ndr::update::UpdatePhase::Failed);
    assert_eq!(app.update_progress.as_ref().unwrap().failed_phase, Some(ndr::update::UpdatePhase::Installing));
    assert_eq!(app.update_progress.as_ref().unwrap().error.as_deref(), Some("installer start error"));
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn update_progress_remains_inside_small_viewports_and_throttles_animation() {
    use ndr::update::UpdatePhase;
    let (root, _ctx, mut app) = fixture("update-progress-bounds");
    for phase in [UpdatePhase::Downloading, UpdatePhase::Verifying, UpdatePhase::Installing, UpdatePhase::Failed] {
        let ctx = egui::Context::default();
        // Actual NDR installs the named bold font used by its common popup title.
        super::set_app_fonts(&ctx, None, false, 1.0, &app.language);
        app.update_progress = Some(super::main_window::UpdateProgress {
            tag: "build-99".into(), phase, downloaded: 64, total: 128,
            error: (phase == UpdatePhase::Failed).then(|| "A deliberately long diagnostic. ".repeat(80)),
            failed_phase: Some(UpdatePhase::Verifying),
        });
        for (w, h, scale) in [(1400.0, 850.0, 1.0), (548.0, 411.0, 1.75), (320.0, 250.0, 2.0)] {
            let mut delay = Duration::ZERO;
            for _ in 0..8 {
                let mut raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))),
                    ..Default::default()
                };
                raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(scale);
                let output = ctx.run(raw, |ctx| app.update_progress_ui(ctx));
                delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
            }
            let area = ctx.memory(|m| m.area_rect(egui::Id::new("ndr-update-progress"))).expect("update window");
            assert!(area.left() >= -1.0 && area.top() >= -1.0 && area.right() <= w + 1.0 && area.bottom() <= h + 1.0,
                "{phase:?}: {w}x{h}: {area:?}");
            assert!(delay >= Duration::from_millis(50),
                "{phase:?}: update dialog should not request continuous repaint: {delay:?}");
        }
    }
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn github_update_dialog_identifies_installed_and_target_builds() {
    let (root, ctx, mut app) = fixture("github-build-message");
    let installed = ndr::update::current_build();
    let response = |build| {
        let tag = format!("build-{build}");
        let body = serde_json::json!({
            "tag_name": tag, "draft": false, "prerelease": false,
            "assets": [{
                "name": "NDR-windows-x64.zip", "size": 123,
                "digest": format!("sha256:{}", "a".repeat(64)),
                "browser_download_url": format!("https://github.com/UzuCore/NDR/releases/download/{tag}/NDR-windows-x64.zip"),
            }]
        });
        Ok(ndr::update::HttpResponse { status: 200, secure: true, body: serde_json::to_vec(&body).unwrap() })
    };
    for (language, title, current, local, new) in [
        ("ko", "NDR 업데이트", "현재 빌드", "로컬 빌드", "새 빌드"),
        ("en", "NDR Update", "Installed build", "Local build", "New build"),
    ] {
        app.apply_language(language);
        app.update_done(response(installed + 1), false);
        let msg = app.msgboxes.first().unwrap();
        let label = ndr::update::build_label(installed).unwrap_or_else(|| local.into());
        assert_eq!(msg.title, title);
        assert!(msg.text.contains(&format!("{current}: {label}")));
        assert!(msg.text.contains(&format!("{new}: build-{}", installed + 1)));
        app.answer_message_box(2, &ctx);
        assert!(app.update_progress.is_none(), "declining cannot start a download");
        if installed > 0 {
            app.update_available = Some("stale update button".into());
            app.update_done(response(installed), false);
            let msg = app.msgboxes.first().unwrap();
            assert_eq!(msg.title, title);
            assert!(msg.text.contains(&format!("{current}: {label}")));
            assert!(app.update_available.is_none());
            app.answer_message_box(1, &ctx);
        }
        app.about_command();
        assert!(app.msgboxes.first().unwrap().text.starts_with(&format!("NDR {label}\n")));
        app.answer_message_box(1, &ctx);
    }
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn about_dialog_uses_large_ndr_artwork_and_preserves_localized_layout() {
    let (root, ctx, mut app) = fixture("about-icon");
    for &(locale, _) in ndr::compat::BUILTIN_LANGUAGES {
        app.apply_language(locale);
        for (width, height, icon_size) in [(1400.0, 850.0, 96.0), (548.0, 411.0, 96.0), (320.0, 250.0, 64.0)] {
            app.msgboxes.clear();
            app.about_command();
            assert!(app.msgboxes[0].program_icon, "{locale}");
            assert_eq!(ndr::msgbox::icon_id(app.msgboxes[0].flags), 0);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
            for _ in 0..3 {
                let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
                    app.msgbox_ui(ctx);
                });
            }
            let box_layout = app.msgboxes[0].layout.as_ref().unwrap();
            let icon = box_layout.icon_rect.expect("embedded NDR artwork");
            let button = box_layout.slots[2].expect("close button");
            assert_eq!(icon.size(), egui::vec2(icon_size, icon_size), "{locale}");
            assert!(screen.contains_rect(icon) && screen.contains_rect(button), "{locale} {width}x{height}: {icon:?}, {button:?}");
            assert!(icon.right() + 8.0 <= box_layout.origin.x + box_layout.text_left, "{locale}");
            assert!(icon.bottom() <= button.top(), "{locale}");
            assert_eq!(app.msgbox_icons[&101].size(), [256, 256]);
            app.answer_message_box(1, &ctx);
            assert!(app.msgboxes.is_empty());
        }
    }
    // Other information boxes keep their original icon and geometry.
    app.system_box("NDR", "Information", ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONASTERISK, |_, _, _| {});
    assert!(!app.msgboxes[0].program_icon);
    let _ = ctx.run(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() }, |ctx| app.msgbox_ui(ctx));
    assert_eq!(app.msgboxes[0].layout.as_ref().unwrap().text_left, 64.0);
    assert_eq!(app.msgboxes[0].layout.as_ref().unwrap().icon_rect.unwrap().size(), egui::vec2(32.0, 32.0));
    app.answer_message_box(1, &ctx);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn all_forty_locale_choices_open_their_own_update_dialog_and_save_canonical_id() {
    let (root, ctx, mut app) = fixture("all-forty-update-dialogs");
    let build = ndr::update::current_build() + 10;
    let tag = format!("build-{build}");
    let response = serde_json::json!({
        "tag_name": tag, "draft": false, "prerelease": false,
        "assets": [{
            "name": "NDR-windows-x64.zip", "size": 123,
            "digest": format!("sha256:{}", "a".repeat(64)),
            "browser_download_url": format!("https://github.com/UzuCore/NDR/releases/download/{tag}/NDR-windows-x64.zip"),
        }]
    });
    let body = serde_json::to_vec(&response).unwrap();
    assert_eq!(super::settings_window::available_languages(&app.resources).len(), 40);
    for &(locale, _) in ndr::compat::BUILTIN_LANGUAGES {
        app.apply_language(locale);
        assert_eq!(app.language, locale);
        assert_eq!(app.settings.text("Language", ""), locale);
        assert_eq!(app.language_pack.code, locale);
        let primary = app.language_pack.primary_raw("NDR.UPD_TITLE").expect("localized update title").to_owned();
        app.update_done(Ok(ndr::update::HttpResponse {
            status: 200, secure: true, body: body.clone(),
        }), false);
        let msg = app.msgboxes.first().expect("update dialog should open");
        assert_eq!(msg.title, primary, "{locale}: update dialog used fallback");
        assert!(msg.text.contains(&tag), "{locale}: build placeholder did not resolve");
        app.answer_message_box(2, &ctx);
        assert!(app.update_progress.is_none());
    }
    app.apply_language("pt_PT");
    assert_eq!(app.settings.text("Language", ""), "pt-PT");
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ndr_encrypted_folder_opens_in_internal_browser() {
    let (root, ctx, mut app) = fixture("vault-open");
    let entry = app.panes[0].path.join("private.ndr");
    std::fs::write(&entry, b"test fake file").unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    let index = app.panes[0].entries.iter().position(|e| e.name == "private").unwrap();
    assert!(app.panes[0].entries[index].is_dir, "NDR shows the container as a folder");
    assert!(app.panes[0].entries[index].path.is_file(), "encrypted bytes remain in a file");
    app.active = 0;
    app.panes[0].cursor = index;
    app.open_current();
    assert!(app.vault_dialog.is_some(), "NDR folder should request a password before entering the pane");
    app.vault_dialog = None;
    app.dispatch("newVault", &ctx);
    assert!(app.vault_dialog.is_some(), "File > New encrypted folder should open creator");
    app.vault_dialog = None;
    std::fs::remove_dir_all(&root).unwrap();
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
    // With no custom feed NDR now checks GitHub Releases by default.
    app.http_get = latest_feed;
    app.dispatch("update", &ctx);
    assert_eq!(wait_box(&mut app, &ctx), ("NDR".to_owned(), app.language_pack.text("RES.UPDATE_NOUPDATE", "?")));
    app.answer_message_box(1, &ctx);
    app.settings.set_text("UpdateManifestUrl", "http://example.invalid/feed.json");
    app.dispatch("update", &ctx);
    assert_eq!(wait_box(&mut app, &ctx).1, app.language_pack.text("NDRUpdate.FeedHttpsRequired", "?"));
    app.answer_message_box(1, &ctx);
    // New version: original UPDATE_NEWUPDATE (title UPDATE); Yes opens the download URL.
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
    // Original silent check (at startup, FUN_00441dd8 [+0x54]): with no box, if a new version exists only a "new update" button in the menu row. Pressing it opens that URL.
    app.update_done(newer_feed("", 0), true);
    assert!(app.msgboxes.is_empty());
    assert_eq!(app.update_available.as_deref(), Some("https://example.invalid/ndr"));
    app.update_done(timeout_feed("", 0), true);
    assert!(app.msgboxes.is_empty(), "silent errors");
    draw(&mut app, &ctx);
    // The donation button (original tbDonation) was removed at the user's request: drawing the menu row has no way to open the donation URL.
    assert!(FEED.with(|f| !f.borrow().windows(8).any(|w| w == b"donation")));
    // The right-click menu is drawn (no command if nothing was pressed).
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut chosen = Some("x");
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| chosen = app.list_context_menu(ui, 0));
    });
    assert_eq!(chosen, None);
    // Automatic folder-size display: drawing a visible folder row counts it separately and turns it into a size text.
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
    // Original FUN_0048a5cc: the file info bar shows a counted folder's size in bytes (with thousands separators).
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
    // ConfirmDragDrop (default) = original FUN_00488b64: on the same drive it asks about a move with DRAGDROP / DROPMOVE_CONFIRM (count). Yes moves it.
    app.handle_drop(1, vec![left.join("a.txt")], right.clone(), egui::Modifiers::NONE);
    let question = app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone(), m.flags)).expect("the drop question");
    assert_eq!(question, (app.language_pack.text("RES.DRAGDROP", "?"), app.language_pack.text("RES.DROPMOVE_CONFIRM", "?").replace("%d", "1"), ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONQUESTION));
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    wait_job(&mut app, &ctx);
    assert!(right.join("a.txt").exists() && !left.join("a.txt").exists());
    // Does not move a folder into itself. With ConfirmDragDrop off, a same-drive move does not ask.
    app.settings.set_flag("ConfirmDragDrop", false);
    app.handle_drop(0, vec![left.join("folder")], left.join("folder"), egui::Modifiers::NONE);
    assert!(app.original_dialogs.is_empty() && app.msgboxes.is_empty());
    wait_job(&mut app, &ctx);
    app.handle_drop(1, vec![left.join("c.log")], right.clone(), egui::Modifiers::NONE);
    wait_job(&mut app, &ctx);
    // Original drag image (FUN_0048514c): before dropping, it draws the DRAGFILES_INFO header and the names and keeps the drag going.
    app.file_drag = Some(super::drag_drop::FileDrag { paths: vec![right.join("a.txt"), right.join("c.log")], secondary: false, source: 1, allowed: 7 });
    let raw = egui::RawInput { events: vec![egui::Event::PointerMoved(egui::pos2(50.0, 50.0))], ..Default::default() };
    let _ = ctx.run(raw, |ctx| app.drag_drop_frame(ctx));
    assert!(app.file_drag.is_some(), "still dragging");
    app.file_drag = None;
    assert!(right.join("c.log").exists() && !left.join("c.log").exists(), "same drive → move");
    // Original right-button drag: on drop, pmDrop (Copy Here, Move Here, Shortcut, -, Cancel). Choosing one does not ask ConfirmDragDrop.
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
    // Drag start: if there are no marks, the pressed row.
    app.panes[0].selected.clear();
    let row = app.panes[0].entries.iter().position(|e| e.name == "folder").unwrap();
    app.begin_file_drag(0, row, false, egui::Modifiers::NONE);
    assert_eq!(app.file_drag.as_ref().map(|d| d.paths.clone()), Some(vec![left.join("folder")]));
    // Drop target: over a drawn folder row it is that folder; over the dragged item itself it is the pane folder.
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
    // C# connectNetworkShare: only credential failures (5, 65, 86, 1326) open the connect window; other errors pass through.
    use super::tools_dialog::network_share_code;
    assert_eq!(network_share_code(r"\\server\share\x", |_| 1326, |_| 1223), 1223);
    assert_eq!(network_share_code(r"\\server\share\x", |_| 86, |_| 0), 0);
    assert_eq!(network_share_code(r"\\server\share\x", |_| 53, |_| 0), 53);
    assert_eq!(network_share_code(r"C:\x", |_| 0, |_| 5), 0);
    assert_eq!(native::win32_message(5), native::win32_message(5).trim());
    assert!(!native::win32_message(5).contains("os error"));
}

#[test]
fn foreground_listing_wait_prioritizes_network_ui_responsiveness() {
    use super::panes::foreground_listing_wait;
    assert_eq!(foreground_listing_wait(Path::new(r"\\server\share\slow")), Duration::from_millis(8));
    assert_eq!(foreground_listing_wait(Path::new(r"\\?\UNC\server\share")), Duration::from_millis(8));
    assert_eq!(foreground_listing_wait(Path::new(r"relative\local\folder")), Duration::from_millis(24));
    assert_eq!(foreground_listing_wait(Path::new(r"C:\local\folder")), Duration::from_millis(24));
}

#[test]
fn auto_view_columns_follow_the_original() {
    let (root, ctx, mut app) = fixture("auto");
    app.settings.set_text("LeftViewStyle", "Auto");
    // A configured column count (1–19) is used as-is; otherwise the count from width-based division (more than one for a single pane at width 1400).
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
    // Original FitVisibleFields: cells are content width, so in a wide column they all fit and the name does not stretch; in a narrow column the later cells drop off first.
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
fn grouped_auto_distribution_matches_the_original_across_modes_and_column_counts() {
    let (root, ctx, mut app) = fixture("auto-groups-parity");
    let dir = root.join("groups");
    fs::create_dir_all(&dir).unwrap();
    for i in 0..96 {
        let extension = ["zip", "txt", "png", "mp3", "exe", "json"][i % 6];
        fs::write(dir.join(format!("file-{i:03}.{extension}")), vec![0_u8; i * 137]).unwrap();
    }
    for i in 0..12 { fs::create_dir(dir.join(format!("folder-{i:02}"))).unwrap(); }
    app.navigate(0, dir, false);
    wait(&mut app, &ctx);
    for mode in ["None", "Extension", "Size", "Time", "Category"] {
        app.settings.set_text("LeftGroupMode", mode);
        for columns in [1, 2, 3, 7, 12] {
            let pane = &app.panes[0];
            let count = pane.entries.len();
            let per_column = count.div_ceil(columns).max(1);
            let mut expected = vec![Vec::new(); columns];
            if mode == "None" {
                for row in 0..count { expected[(row / per_column).min(columns - 1)].push(row); }
            } else {
                let category = app.settings.text("LeftFolderCategoryMode", "Auto");
                let mut column = 0;
                let mut row = 0;
                while row < count {
                    let key = ndr::listing::group_key(&pane.entries[row], mode, &category);
                    let end = (row..count).find(|&r| ndr::listing::group_key(&pane.entries[r], mode, &category) != key).unwrap_or(count);
                    if column < columns - 1 && !expected[column].is_empty() && expected[column].len() + end - row > per_column { column += 1; }
                    expected[column].extend(row..end);
                    row = end;
                }
            }
            assert_eq!(app.auto_distribution(0, columns), expected, "mode={mode}, columns={columns}");
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_column_width_cache_reuses_measured_value_until_listing_changes() {
    let (root, ctx, mut app) = fixture("auto-width-cache");
    // Grouped Auto and Names now share the same measured-width cache,
    // including the configured font face/style in the invalidation key.
    let key = egui::Id::new(("ndr-widest-pane-filename", 0));
    let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let run = |app: &mut App| {
        let _ = ctx.run(raw(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(app.auto_column_count(ui, 0, 1400.0) >= 1);
            });
        });
    };
    app.settings.values.insert("LeftColumnCount".into(), serde_json::json!(0));
    run(&mut app);
    let first = ctx.data(|d| d.get_temp::<((u64, egui::FontId, String, u8), f32)>(key)).unwrap();
    run(&mut app);
    let second = ctx.data(|d| d.get_temp::<((u64, egui::FontId, String, u8), f32)>(key)).unwrap();
    assert_eq!(first, second, "same listing and font must reuse the width");
    app.sort_pane(0);
    run(&mut app);
    let refreshed = ctx.data(|d| d.get_temp::<((u64, egui::FontId, String, u8), f32)>(key)).unwrap();
    assert_ne!(first.0.0, refreshed.0.0, "sorting invalidates measured widths");
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
    // Original GetAdjustedNameWidth/GetAdjustedExtWidth: with one long name/extension present, Trunc(average × 1.5 + 10).
    assert_eq!(fm.items, 32, "\"..\" and 31 files");
    let average = (fm.name_sum / fm.items as f32).floor();
    assert_eq!(name, (average * 1.5 + 10.0).trunc(), "{fm:?}");
    assert!(name < fm.max[0] + 10.0);
    assert!(fm.ext_average > 0.0);
    assert_eq!(ext, (fm.ext_average * 1.5 + 10.0).trunc(), "{fm:?}");
    assert!(ext < fm.max[1] + 10.0);
    // Original UpdateDisplayNames: with multiple columns, extension = Adjusted and the name fills the rest of the column.
    assert!(fits >= 2, "{fits}");
    assert_eq!(multi.1[0], Some(ext));
    assert!((multi.0 + ext - 250.0).abs() < 0.01, "{multi:?}");
    assert!(multi.1.iter().skip(1).all(Option::is_none), "only name and extension: {multi:?}");
    // If everything fits in one column, each has its own width (longest text + 10) and the leftover space is empty.
    assert_eq!(single.1[0], Some(fm.max[1] + 10.0));
    assert!((single.0 - (30.0 + fm.max[0] + 10.0)).abs() < 0.01, "{single:?}");
    // When fewer than 2 columns fit and the column is narrower than one item, name = width − header − Adjusted extension (the extension cell keeps its own width).
    assert!((narrow.0 - (120.0 - ext)).abs() < 0.01, "{narrow:?}");
    assert_eq!(narrow.1[0], Some(fm.max[1] + 10.0));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_size_options_are_on_file_list_page_in_all_40_languages() {
    use super::settings_window::{Row, SettingsForm};
    let resources = resource_dir();
    let codes = compat::available_codes(&resources.join("Languages"));
    assert_eq!(codes.len(), 40);
    for code in codes {
        let language = compat::Language::load(&resources.join("Languages"), &code);
        let settings = ndr::settings::Settings::default();
        let mut form = SettingsForm::new(&settings, &language, &resources);
        for key in ["LeftAutoFolderSize", "RightAutoFolderSize"] {
            let rows: Vec<_> = form.tabs.iter().enumerate().flat_map(|(tab, rows)| rows.iter().filter_map(move |row| match row {
                Row::Check { key: k, label, value } if k == key => Some((tab, label, value)),
                _ => None,
            })).collect();
            assert_eq!(rows.len(), 1, "{code} {key}");
            assert_eq!(rows[0].0, 0, "{code} {key} must be on File List");
            assert!(!*rows[0].2 && !rows[0].1.is_empty(), "{code} {key}");
            let expected = language.text(if key.starts_with("Left") { "NDRVault.AutoFolderSizeLeft" } else { "NDRVault.AutoFolderSizeRight" }, "missing");
            assert_eq!(rows[0].1, &expected, "{code} {key}");
            assert_ne!(expected, "missing", "{code} missing catalog key");
        }
        let Some(Row::Check { value, .. }) = form.row_mut("LeftAutoFolderSize") else { panic!("{code}"); };
        *value = true;
        let committed = form.commit().unwrap();
        assert!(committed.flag("LeftAutoFolderSize", false));
        assert!(!committed.flag("RightAutoFolderSize", false));
        let mut reopened = SettingsForm::new(&committed, &language, &resources);
        assert!(matches!(reopened.row_mut("LeftAutoFolderSize"), Some(Row::Check { value: true, .. })));
    }
}

#[test]
fn automatic_folder_sizes_cache_navigation_and_foreground_priority() {
    let (root, ctx, mut app) = fixture("auto-folder-size");
    let left = root.join("left");
    let folder = left.join("folder");
    fs::create_dir_all(folder.join("nested")).unwrap();
    fs::write(folder.join("a.bin"), vec![1u8; 2048]).unwrap();
    fs::write(folder.join("nested/b.bin"), vec![2u8; 1024]).unwrap();
    let row = |app: &App| app.panes[0].entries.iter().position(|e| e.path == folder).unwrap();
    let wait_size = |app: &mut App, bytes: u64| {
        let end = Instant::now() + Duration::from_secs(15);
        while app.panes[0].folder_bytes.get(&folder) != Some(&bytes) {
            assert!(Instant::now() < end, "folder size timed out: {:?}", app.panes[0].folder_sizes);
            app.poll(&ctx);
            thread::sleep(Duration::from_millis(5));
        }
    };
    let r = row(&app);
    app.queue_folder_size(0, r);
    assert!(!app.folder_size_workers, "default-off must not spawn a worker");
    app.settings.set_flag("LeftAutoFolderSize", true);
    app.queue_folder_size(0, r);
    assert_eq!(app.panes[0].folder_size_display(&folder), None, "keep the localized folder marker until ready");
    wait_size(&mut app, 3072);
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(), Some("3.00 KB"));
    assert!(app.panes[0].folder_auto_paths.contains(&folder));
    assert!(app.folder_size_cache.contains_key(&folder));
    assert!(app.folder_size_cache[&folder].watch.is_some(), "local fixture requires recursive change notification");
    let original_scans = app.panes[0].folder_auto_scans;
    // Expiry validates the root identity without walking a clean subtree.
    let expired = Instant::now() - super::pane_chrome::FOLDER_SIZE_TTL - Duration::from_millis(5);
    app.folder_size_cache.get_mut(&folder).unwrap().observed_at = expired;
    app.panes[0].folder_auto_times.insert(folder.clone(), expired);
    app.queue_folder_size(0, r);
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(), Some("3.00 KB"));
    wait_for_folder_idle(&mut app, &ctx, 0);
    assert_eq!(app.panes[0].folder_auto_scans, original_scans);
    // An unrelated change in the parent listing must not invalidate a
    // recursive observation of this folder.
    fs::write(left.join("unrelated.txt"), b"outside the watched subtree").unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    let r = row(&app);
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(), Some("3.00 KB"));
    app.queue_folder_size(0, r);
    wait_for_folder_idle(&mut app, &ctx, 0);
    assert_eq!(app.panes[0].folder_auto_scans, original_scans);
    // Navigate elsewhere and return: a recent complete total is immediately
    // reused; no recursive scan is launched for this row.
    app.navigate(0, root.join("right"), true);
    wait(&mut app, &ctx);
    app.navigate(0, left.clone(), true);
    wait(&mut app, &ctx);
    let r = row(&app);
    app.queue_folder_size(0, r);
    assert_eq!(app.panes[0].folder_bytes.get(&folder), Some(&3072));
    // A foreground file job suppresses automatic scans, but never
    // removes an already displayed total or cancels the foreground job.
    let (_tx, rx) = mpsc::channel();
    let job_cancel = Arc::new(AtomicBool::new(false));
    app.job = Some(Job::new(rx, job_cancel.clone(), "fixture copy".into()));
    app.sync_folder_size_activity();
    assert!(!job_cancel.load(Ordering::Relaxed));
    assert_eq!(app.panes[0].folder_bytes.get(&folder), Some(&3072));
    assert!(app.folder_size_cache.contains_key(&folder));
    app.queue_folder_size(0, r);
    assert_eq!(app.panes[0].folder_bytes.get(&folder), Some(&3072));
    fs::write(folder.join("nested/b.bin"), vec![3u8; 4096]).unwrap();
    wait_for_folder_watch_change(&app, &folder);
    app.job = None;
    app.queue_folder_size(0, r);
    assert_eq!(app.panes[0].folder_bytes.get(&folder), Some(&3072), "never blank while refreshing");
    wait_size(&mut app, 6144);
    // A refresh retains old totals until the changed descendant is counted.
    fs::write(folder.join("nested/b.bin"), vec![4u8; 512]).unwrap();
    wait_for_folder_watch_change(&app, &folder);
    app.refresh(0);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].folder_bytes.get(&folder), Some(&6144));
    let r = row(&app);
    app.queue_folder_size(0, r);
    wait_size(&mut app, 2560);
    let mut off = app.settings.clone();
    off.set_flag("LeftAutoFolderSize", false);
    app.apply_settings(off);
    wait(&mut app, &ctx);
    assert!(!app.panes[0].folder_sizes.contains_key(&folder));
    let r = row(&app);
    app.queue_folder_size(0, r);
    assert!(!app.panes[0].folder_sizes.contains_key(&folder));
    app.enqueue_folder_size(0, r);
    wait_size(&mut app, 2560);
    assert!(!app.panes[0].folder_auto_paths.contains(&folder));
    assert!(!native::folder_size_local_volume(Path::new(r"\\server\share\folder")));
    assert!(!native::folder_size_local_volume(Path::new(r"\\?\UNC\server\share\folder")));
    assert_eq!(fs::metadata(folder.join("nested/b.bin")).unwrap().len(), 512);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

fn wait_for_folder_idle(app: &mut App, ctx: &egui::Context, pane: usize) {
    let end = Instant::now() + Duration::from_secs(15);
    while !app.panes[pane].folder_auto_pending.is_empty() {
        assert!(Instant::now() < end, "folder validation timed out");
        app.poll(ctx);
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_folder_watch_change(app: &App, folder: &Path) {
    let watch = app.folder_size_cache[folder].watch.as_ref().expect("local folder change watch");
    let end = Instant::now() + Duration::from_secs(10);
    while watch.unchanged() {
        assert!(Instant::now() < end, "recursive notification not received for {folder:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn folder_cache_job_scope_updates_only_affected_totals() {
    let (root, ctx, mut app) = fixture("folder-operation-scope");
    let left = root.join("left");
    let a = left.join("a");
    let b = left.join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(a.join("original"), [1u8; 100]).unwrap();
    fs::write(b.join("other"), [2u8; 200]).unwrap();
    let input = root.join("right/input.bin");
    fs::write(&input, [3u8; 300]).unwrap();
    app.settings.set_flag("LeftAutoFolderSize", true);
    app.settings.set_flag("PlayCompletionSound", false);
    app.refresh(0);
    wait(&mut app, &ctx);
    let update = |app: &mut App, a_bytes, b_bytes| {
        for p in [&a, &b] {
            let row = app.panes[0].entries.iter().position(|e| &e.path == p).unwrap();
            app.queue_folder_size(0, row);
        }
        wait_for_folder_idle(app, &ctx, 0);
        assert_eq!(app.panes[0].folder_bytes.get(&a), Some(&a_bytes));
        assert_eq!(app.panes[0].folder_bytes.get(&b), Some(&b_bytes));
    };
    let run = |app: &mut App, operation| {
        app.start_job("fixture operation".into(), operation, super::file_operation::JobReport::Generic, None);
        let deadline = Instant::now() + Duration::from_secs(20);
        while app.job.as_ref().is_some_and(|job| !job.finished) {
            assert!(Instant::now() < deadline);
            app.poll_active_job_events();
            app.poll(&ctx);
            thread::sleep(Duration::from_millis(5));
        }
        app.poll(&ctx);
        wait(app, &ctx);
    };
    update(&mut app, 100, 200);
    let scans = app.panes[0].folder_auto_scans;
    run(&mut app, Operation::Copy { sources: vec![input.clone()], destination: a.clone(), move_files: false });
    assert_eq!(app.panes[0].folder_bytes.get(&a), Some(&100), "retain display during foreground copy");
    update(&mut app, 400, 200);
    assert_eq!(app.panes[0].folder_auto_scans, scans + 1, "unchanged sibling must not be recursively scanned");
    run(&mut app, Operation::Copy { sources: vec![a.join("input.bin")], destination: b.clone(), move_files: true });
    update(&mut app, 100, 500);
    run(&mut app, Operation::Delete { sources: vec![b.join("input.bin")], recycle: false });
    update(&mut app, 100, 200);
    assert_eq!(fs::metadata(input).unwrap().len(), 300);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_observation_handles_are_bounded_without_losing_totals() {
    let (root, _ctx, mut app) = fixture("folder-watch-budget");
    let folder = root.join("left/folder");
    let watch = Arc::new(native::FolderSizeWatch::open(&folder).unwrap());
    for i in 0..150 {
        app.cache_folder_size(root.join(format!("cached-{i}")), i, Some(watch.clone()), Instant::now(), native::FolderSizeMedia::Fast);
    }
    assert_eq!(app.folder_size_cache.len(), 150);
    assert!(app.folder_size_cache.values().filter(|o| o.watch.is_some()).count() <= 128);
    for i in 0..150 { assert_eq!(app.folder_size_cache[&root.join(format!("cached-{i}"))].bytes, i); }
    drop((watch, app));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn large_local_listing_worker_matches_synchronous_rows_and_widths() {
    let (root, ctx, mut app) = fixture("prepared-large-list");
    let left = root.join("left");
    for i in 0..4100 {
        fs::write(left.join(format!("item-{i:04}.{}", if i % 3 == 0 { "txt" } else { "log" })), vec![1u8; i % 23]).unwrap();
    }
    app.settings.set_flag("ShowDriveList", false);
    app.settings.set_text("LeftGroupMode", "Extension");
    app.settings.set_text("LeftSortMode", "Modified");
    app.settings.set_flag("LeftSortDescending", true);
    app.settings.set_flag("LeftAutoFolderSize", true);
    app.panes[0].filter = "*.txt".into();
    app.refresh(0);
    wait(&mut app, &ctx);
    assert!(app.smoke_prepared_lists >= 1);
    assert!(app.panes[0].prepared_field_max.is_some());
    let expected_paths: Vec<_> = app.panes[0].entries.iter().map(|e| e.path.clone()).collect();
    let expected_cells = app.panes[0].view.cells.clone();
    let expected_colors = app.panes[0].view.colors.clone();
    let expected_headers: Vec<_> = app.panes[0].view.rows.headers().map(str::to_owned).collect();
    let expected_fm = app.panes[0].prepared_field_max.as_ref().unwrap().2;
    let filter = ndr::parity::CompiledFilter::new("*.txt");
    let mut raw = fsops::list_dir(&left, app.panes[0].show_hidden).unwrap();
    raw.retain(|e| e.is_dir || filter.matches(&e.name));
    raw.insert(0, parent_item_for(&app.panes[0]).unwrap());
    app.panes[0].entries = raw;
    app.sort_pane(0);
    assert_eq!(app.panes[0].entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>(), expected_paths);
    assert_eq!(app.panes[0].view.cells, expected_cells);
    assert_eq!(app.panes[0].view.colors, expected_colors);
    assert_eq!(app.panes[0].view.rows.headers().map(str::to_owned).collect::<Vec<_>>(), expected_headers);
    let mut fm = None;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| { fm = Some(app.original_field_max(ui, 0)); });
    });
    assert_eq!(fm, Some(expected_fm));
    // Reject an obsolete ordering rather than derive an unsorted result from it.
    app.refresh_after_startup(0);
    app.settings.set_text("LeftSortMode", "None");
    app.settings.set_text("LeftGroupMode", "None");
    wait(&mut app, &ctx);
    let mut original = fsops::list_dir(&left, app.panes[0].show_hidden).unwrap();
    original.retain(|e| e.is_dir || filter.matches(&e.name));
    original.insert(0, parent_item_for(&app.panes[0]).unwrap());
    // The None mode still places the parent, folders and priority buckets first.
    original.sort_by_key(|e| if e.is_parent() { 0 } else if e.is_dir { 1 } else { 2 });
    assert_eq!(app.panes[0].entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>(),
        original.into_iter().map(|e| e.path).collect::<Vec<_>>());
    let stopped = AtomicBool::new(true);
    assert!(fsops::list_dir_with_cancel(&left, true, Some(&stopped)).is_err());
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_folder_sizes_expire_live_and_invalidate_both_panes() {
    let (root, ctx, mut app) = fixture("auto-folder-freshness");
    let left = root.join("left");
    let folder = left.join("deep");
    fs::create_dir_all(folder.join("child")).unwrap();
    let file = folder.join("child/data.bin");
    fs::write(&file, vec![1u8; 1024]).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    app.settings.set_flag("LeftAutoFolderSize", true);
    app.settings.set_flag("RightAutoFolderSize", true);
    let row = |app: &App, pane: usize| app.panes[pane].entries.iter().position(|e| e.path == folder).unwrap();
    let wait_size = |app: &mut App, bytes: u64| {
        let end = Instant::now() + Duration::from_secs(15);
        while app.panes[0].folder_bytes.get(&folder) != Some(&bytes) {
            assert!(Instant::now() < end, "automatic refresh timed out");
            app.poll(&ctx);
            thread::sleep(Duration::from_millis(5));
        }
    };
    let r = row(&app, 0);
    app.queue_folder_size(0, r);
    wait_size(&mut app, 1024);
    let original_generation = app.panes[0].generation;
    // A change inside a descendant does not require a root listing refresh.
    fs::write(&file, vec![2u8; 4096]).unwrap();
    wait_for_folder_watch_change(&app, &folder);
    let expired = Instant::now() - super::pane_chrome::FOLDER_SIZE_TTL - Duration::from_millis(5);
    app.panes[0].folder_auto_times.insert(folder.clone(), expired);
    app.queue_folder_size(0, r);
    assert_eq!(app.panes[0].folder_bytes.get(&folder), Some(&1024));
    wait_size(&mut app, 4096);
    assert_eq!(app.panes[0].generation, original_generation);
    app.navigate(1, left, true);
    wait(&mut app, &ctx);
    let right_row = row(&app, 1);
    app.queue_folder_size(1, right_row);
    assert_eq!(app.panes[1].folder_bytes.get(&folder), Some(&4096));
    app.refresh(0);
    assert!(!app.panes[1].folder_auto_times.contains_key(&folder));
    assert_eq!(app.panes[1].folder_bytes.get(&folder), Some(&4096));
    wait(&mut app, &ctx);
    app.queue_folder_size(0, row(&app, 0));
    assert_eq!(app.panes[0].folder_bytes.get(&folder), Some(&4096));
    app.queue_folder_size(1, right_row);
    wait_for_folder_idle(&mut app, &ctx, 0);
    wait_for_folder_idle(&mut app, &ctx, 1);
    assert_eq!(app.panes[1].folder_auto_scans, 0, "second pane reuses clean watch");
    // The same path's fallback cache must also retain its display during
    // revalidation if recursive notifications are unsupported.
    app.cache_folder_size(folder.clone(), 4096, None, Instant::now(), native::FolderSizeMedia::Unknown);
    app.folder_size_cache.get_mut(&folder).unwrap().observed_at = expired;
    app.panes[0].folder_auto_times.insert(folder.clone(), expired);
    app.folder_size_workers = true;
    app.queue_folder_size(0, row(&app, 0));
    assert_eq!(app.panes[0].folder_bytes.get(&folder), Some(&4096));
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scrolling_cancels_obsolete_automatic_jobs_but_not_manual_requests() {
    let (root, ctx, mut app) = fixture("auto-folder-scroll");
    let left = root.join("left");
    for i in 0..80 {
        fs::create_dir_all(left.join(format!("scroll-{i:03}"))).unwrap();
    }
    app.refresh(0);
    wait(&mut app, &ctx);
    app.settings.set_flag("LeftAutoFolderSize", true);
    // A deterministic queue-only fixture: no worker can race these assertions.
    app.folder_size_workers = true;
    let rows: Vec<_> = app.panes[0].entries.iter().enumerate()
        .filter_map(|(i, e)| e.name.starts_with("scroll-").then_some(i)).collect();
    assert_eq!(rows.len(), 80);
    app.enqueue_folder_size(0, rows[79]);
    let manual = app.panes[0].entries[rows[79]].path.clone();
    for &r in &rows[..60] { app.queue_folder_size(0, r); }
    assert_eq!(app.panes[0].folder_auto_pending.len(), 25);
    let abandoned = app.panes[0].entries[rows[0]].path.clone();
    let token = app.panes[0].folder_auto_pending[&abandoned].clone();
    app.panes[0].folder_auto_visible.clear();
    for &r in &rows[60..79] { app.queue_folder_size(0, r); }
    app.finish_visible_folder_size_frame(&ctx);
    assert!(token.load(Ordering::Relaxed));
    assert!(!app.panes[0].folder_sizes.contains_key(&abandoned));
    assert_eq!(app.panes[0].folder_sizes.get(&manual), Some(&None));
    for &r in &rows[60..79] { app.queue_folder_size(0, r); }
    app.finish_visible_folder_size_frame(&ctx);
    assert_eq!(app.panes[0].folder_auto_pending.len(), 19);
    let (queue, _, _) = &*app.folder_size_queue;
    {
        let queue = queue.lock().unwrap();
        assert_eq!(queue.len(), 20);
        assert_eq!(queue.front().unwrap().path, manual);
        assert!(!queue.front().unwrap().automatic);
        assert!(queue.iter().all(|job| !job.cancel.load(Ordering::Relaxed)));
    }
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn manual_folder_size_preempts_pending_auto_work_in_both_panes() {
    let (root, ctx, mut app) = fixture("folder-manual-priority");
    let left = root.join("left");
    for name in ["pending-a", "pending-b", "manual-a", "manual-b"] {
        fs::create_dir_all(left.join(name)).unwrap();
        fs::write(left.join(name).join("data"), b"fixture").unwrap();
    }
    app.refresh(0);
    app.navigate(1, left.clone(), true);
    wait(&mut app, &ctx);
    app.settings.set_flag("LeftAutoFolderSize", true);
    app.settings.set_flag("RightAutoFolderSize", true);
    // Keep this a deterministic scheduler test rather than racing the worker.
    app.folder_size_workers = true;
    let row = |app: &App, pane: usize, name: &str| app.panes[pane].entries.iter().position(|e| e.name == name).unwrap();
    let a = row(&app, 0, "pending-a");
    let b = row(&app, 1, "pending-b");
    let m1 = row(&app, 0, "manual-a");
    let m2 = row(&app, 1, "manual-b");
    app.enqueue_folder_size(0, m1);
    app.queue_folder_size(0, a);
    app.queue_folder_size(1, b);
    let a_path = left.join("pending-a");
    let b_path = left.join("pending-b");
    let a_token = app.panes[0].folder_auto_pending[&a_path].clone();
    let b_token = app.panes[1].folder_auto_pending[&b_path].clone();
    app.enqueue_folder_size(1, m2);
    assert!(a_token.load(Ordering::Relaxed) && b_token.load(Ordering::Relaxed));
    assert!(!app.panes[0].folder_sizes.contains_key(&a_path));
    assert!(!app.panes[1].folder_sizes.contains_key(&b_path));
    assert_eq!(app.panes[0].folder_sizes.get(&left.join("manual-a")), Some(&None));
    assert_eq!(app.panes[1].folder_sizes.get(&left.join("manual-b")), Some(&None));
    app.queue_folder_size(0, a);
    app.queue_folder_size(1, b);
    {
        let queue = app.folder_size_queue.0.lock().unwrap();
        assert_eq!(queue.len(), 4);
        assert!(queue.iter().take(2).all(|job| !job.automatic && !job.cancel.load(Ordering::Relaxed)));
        assert!(queue.iter().skip(2).all(|job| job.automatic && !job.cancel.load(Ordering::Relaxed)));
    }
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn refresh_keeps_marks_and_folder_sizes_like_the_original() {
    let (root, ctx, mut app) = fixture("refresh-keep");
    assert!(app.current_skin().is_some());
    let left = root.join("left");
    let row = |app: &App, name: &str| app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    let marked = |app: &App| app.panes[0].selected.iter().map(|&r| app.panes[0].entries[r].name.clone()).collect::<Vec<_>>();
    // Original FUN_00487ce8(1, 1): marks are restored by name, and counted folder sizes are restored too.
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
    // If the cursor item is gone, keep it at the old index.
    let old = app.panes[0].cursor;
    fs::remove_file(left.join("b.txt")).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].cursor, old);
    // The original miShowHiddenClick is (1, 0): marks are not kept.
    app.dispatch("toggleHidden", &ctx);
    wait(&mut app, &ctx);
    assert!(app.panes[0].selected.is_empty());
    // C# RefreshPaneAsync clears everything.
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
        // Treat the original ToolTipTimer (600ms) as elapsed.
        app.tip_since[0] = Some((row, Instant::now() - Duration::from_secs(1)));
        draw(app, &ctx);
        app.name_tip_shown[0]
    };
    let rect = show(&mut app, &long).expect("a shortened thumbnail name shows the full name");
    let thumb = app.settings.values.get("ThumbnailSize").and_then(|v| v.as_i64()).unwrap_or(120) as f32;
    assert!(rect.width() >= thumb + 8.0 - 0.5, "{rect:?}");
    assert!(rect.height() > 30.0, "the full name breaks into several lines: {rect:?}");
    assert_eq!(show(&mut app, "a.txt"), None, "a name that fits has no tooltip");
    // Original list constructor (0x7d4ef5): the list tooltip hides after 3 seconds (not the TNXToolTipWnd initial value of 5 seconds).
    let row = app.panes[0].entries.iter().position(|e| e.name == long).unwrap();
    app.panes[0].cursor = row;
    draw(&mut app, &ctx);
    app.tip_since[0] = Some((row, Instant::now() - Duration::from_millis(600 + super::panes::TIP_HIDE_MS + 100)));
    draw(&mut app, &ctx);
    assert_eq!((super::panes::TIP_HIDE_MS, app.name_tip_shown[0]), (3000, None), "hidden after three seconds");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn very_long_name_tooltip_wraps_all_characters_inside_viewport_with_readable_color() {
    let (root, ctx, _app) = fixture("long-filename-tip-layout");
    let long = format!("{}_한글테스트_abcdefghijklmnopqrstuvwxyz0123456789.txt", "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_".repeat(5));
    assert!(long.chars().count() > 200);
    for (width, height) in [(470.0, 260.0), (1400.0, 850.0)] {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            let (rect, galley) = super::panes::bounded_name_tip_layout(
                ctx, &long, egui::pos2(width - 20.0, height - 20.0), width * 4.0,
                0.0, 16.0, 0,
            ).expect("even a narrow viewport must fit a wrapped tooltip");
            let usable = ctx.content_rect().shrink(8.0);
            assert!(usable.contains_rect(rect), "tooltip must be inside the main window: {rect:?} / {usable:?}");
            assert!(rect.width() < width, "filename may not grow the window or create a huge popup");
            assert!(rect.height() > 27.0, "a long filename must wrap rather than draw a single invisible line");
            assert_eq!(galley.text(), long, "the entire name must be present in the rendered text");
            assert!(!galley.elided, "long name must not be truncated by the tooltip layout");
            let (r, g, b) = ndr::native::system_color(23);
            let foreground = Color32::from_rgb(r, g, b);
            assert!(galley.job.sections.iter().all(|section| section.format.color == foreground),
                "tooltip text must explicitly use COLOR_INFOTEXT, not the skin/list color");
            let drawn = super::panes::draw_bounded_name_tip(
                ctx, 0, &long, egui::pos2(width - 20.0, height - 20.0), width * 4.0, 0.0, 16.0, 0,
            ).expect("tooltip must paint into the existing viewport");
            assert_eq!(drawn, rect);
        });
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn selected_long_filename_tooltip_is_bounded_in_auto_names_details_and_thumbnails() {
    let (root, ctx, mut app) = fixture("long-name-every-view");
    let long = format!("{}.txt", "very_long_unbroken_file_name_0123456789_".repeat(5));
    fs::write(root.join("left").join(&long), b"test").unwrap();
    app.active = 0;
    for view in ["Auto", "Names", "Details", "Thumbnails"] {
        app.settings.set_text("LeftViewStyle", view);
        app.refresh(0);
        wait(&mut app, &ctx);
        let row = app.panes[0].entries.iter().position(|entry| entry.name == long).unwrap();
        app.panes[0].cursor = row;
        draw(&mut app, &ctx);
        app.tip_since[0] = Some((row, Instant::now() - Duration::from_millis(1000)));
        draw(&mut app, &ctx);
        let rect = app.name_tip_shown[0].unwrap_or_else(|| panic!("{view}: selected/truncated name must show tooltip"));
        let usable = ctx.content_rect().shrink(8.0);
        assert!(usable.contains_rect(rect), "{view}: tooltip clips/outgrows main window: {rect:?} / {usable:?}");
        assert!(rect.height() > 25.0, "{view}: a very long filename must wrap to readable rows: {rect:?}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn skin_status_text_is_a_brief_overlay_not_a_status_bar() {
    // The original window has no status bar: the skin theme does not shrink the list area but overlays above the function key row for 4 seconds.
    let (root, ctx, mut app) = fixture("status-overlay");
    assert!(app.current_skin().is_some());
    let raw = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let mut shown = None;
    app.status = "요청한 2개 항목을 복사했습니다.".into();
    let _ = ctx.run(raw(), |ctx| shown = app.skin_status_overlay(ctx, 20.0));
    let rect = shown.expect("the status text is shown");
    assert!(rect.bottom() <= 850.0 - 20.0, "above the function key bar: {rect:?}");
    // After 4 seconds it hides (the text stays).
    app.status_seen = Some((app.status.clone(), Instant::now() - Duration::from_secs(5)));
    let _ = ctx.run(raw(), |ctx| shown = app.skin_status_overlay(ctx, 20.0));
    assert!(shown.is_none());
    assert!(!app.status.is_empty());
    // New text shows again.
    app.status = "다른 글".into();
    let _ = ctx.run(raw(), |ctx| shown = app.skin_status_overlay(ctx, 20.0));
    assert!(shown.is_some());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn drive_list_cache_does_not_ask_the_drives_on_every_listing() {
    // A held list like the original TNXDriveList: within 5 seconds it does not ask again; invalidating it asks again right away.
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

/// Original FUN_004856d8: when there are marks, pressing in empty list space and dragging more than 8 pixels drags the marked items. It does not scroll the list by dragging.
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

/// Original TNXFramePanel: pressing in empty space inside a pane (left/right button, any view style) makes that pane the current pane.
/// update clears pane_rects before drawing the panes, so this test does the same.
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

/// The list right-click menu (original pmMain) does not close after being opened even when the pressed row scrolls out of view (full update path).
/// Fix: when the menu was attached to the row's response, egui closed the menu in a frame that did not draw that row, so it vanished at once.
#[test]
fn pane_menu_reopening_with_a_second_right_click_does_not_immediately_close() {
    let ctx = egui::Context::default();
    let menu_id = egui::Id::new(("ndr-list-menu", 0usize));
    let click = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    let draw = |at, events| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                super::skin_pane::pane_menu(ui, 0, at, |ui| {
                    let _ = ui.button("Copy");
                    let _ = ui.button("Rename");
                });
            });
        });
    };
    let first = egui::pos2(100.0, 120.0);
    let second = egui::pos2(450.0, 250.0);
    draw(Some(first), vec![egui::Event::PointerMoved(first), click(first, true), click(first, false)]);
    assert!(egui::Popup::is_id_open(&ctx, menu_id), "initial right click opens the popup");
    draw(None, vec![]);
    assert!(egui::Popup::is_id_open(&ctx, menu_id), "popup remains open when idle");
    // The original TPopupMenu repositions a displayed menu on another context click.
    // egui's CloseOnClick must not interpret that opening right-click as a dismissal.
    draw(Some(second), vec![egui::Event::PointerMoved(second), click(second, true), click(second, false)]);
    assert!(egui::Popup::is_id_open(&ctx, menu_id), "reopening right click must keep the menu visible");
    draw(None, vec![]);
    assert!(egui::Popup::is_id_open(&ctx, menu_id), "reopened menu stays visible without pointer activity");
    let outside = egui::pos2(900.0, 600.0);
    let primary = |pressed| egui::Event::PointerButton {
        pos: outside,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    draw(None, vec![egui::Event::PointerMoved(outside), primary(true), primary(false)]);
    assert!(!egui::Popup::is_id_open(&ctx, menu_id), "click outside still closes the menu normally");
}

#[test]
fn explorer_menu_waits_for_a_frame_without_the_file_popup() {
    let (root, ctx, mut app) = fixture("explorer-menu-handoff");
    let selected = root.join("left").join("a.txt");
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|entry| entry.path == selected).unwrap();
    let calls = std::cell::RefCell::new(Vec::new());
    let draw = |app: &mut App, open_at, dispatch| {
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0)));
        let mut drawn = false;
        let _ = ctx.run(egui::RawInput { screen_rect, ..Default::default() }, |ctx| {
            app.poll_shell_menu(ctx, |paths| {
                assert!(!egui::Popup::is_any_open(ctx));
                calls.borrow_mut().push(paths.to_vec());
                Ok(())
            });
            egui::CentralPanel::default().show(ctx, |ui| {
                drawn |= super::skin_pane::pane_menu(ui, 0, open_at, |ui| {
                    let _ = ui.button("Explorer menu");
                }).is_some();
                if dispatch {
                    app.dispatch("explorerMenu", ctx);
                }
            });
        });
        drawn
    };

    assert!(draw(&mut app, Some(egui::pos2(100.0, 120.0)), false));
    assert!(draw(&mut app, None, true), "the selection frame can still draw the old popup");
    assert!(!egui::Popup::is_any_open(&ctx), "dispatch closes the NDR popup immediately");
    assert!(!draw(&mut app, None, false), "the next frame clears the old popup");
    assert!(calls.borrow().is_empty(), "the Windows menu waits for the clear frame");
    assert!(!draw(&mut app, None, false));
    assert_eq!(*calls.borrow(), [vec![selected]]);
    assert!(!draw(&mut app, None, false), "closing the Windows menu does not reopen the NDR popup");
    assert_eq!(calls.borrow().len(), 1);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

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
        // Push the cursor to the end of the list so the pressed row is not drawn.
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
/// Original FUN_00484e8c key path (Apps key, MousePos -1, -1): shows pmMain below the current pane's cursor item cell,
/// and does not show it when there are no marks and the cursor item is "..".
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

/// Original pathclick (setting +0xf8): 0 means pressing empty space in the address bar edits the path, 1 means the address-bar menu (FUN_0048d10c) there,
/// 2 means a menu even when pressing a crumb (without going to that folder).
#[test]
fn path_click_mode_follows_the_original_path_label() {
    let (root, ctx, mut app) = fixture("pathclick");
    // The test clicks an *empty* portion of the path bar. With the fixture's
    // full temp-directory path, different Windows runner temp prefixes can
    // make the last breadcrumb reach that coordinate (pane.right() - 46),
    // turning the click into a real breadcrumb action. Use the drive root
    // for this empty-space test so the location is unambiguously blank.
    // Other breadcrumb tests retain realistic long, multi-segment paths.
    let drive_root = root.ancestors().last().expect("Windows drive root").to_path_buf();
    app.navigate(0, drive_root.clone(), false);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].path, drive_root);
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
    // Address bar = first row inside the frame (height 19): the leading ▼ (16) then the first segment; the label ends
    // before the right-hand [history 20][refresh 20] buttons (FUN_00481978, both alRight).
    let bar_y = pane.top() + 2.0 + super::skin_pane::ADDRESS_BAR_HEIGHT / 2.0;
    let empty = egui::pos2(pane.right() - 46.0, bar_y);
    click(&mut app, empty);
    assert!(app.panes[0].address_editing, "mode 0: an empty click starts editing");
    app.end_address_edit(0);
    run(&mut app, vec![]);
    app.settings.values.insert("PathClickMode".into(), serde_json::json!(1));
    click(&mut app, empty);
    assert!(!app.panes[0].address_editing, "mode 1: no editing");
    assert!(app.popup.is_some(), "mode 1: the address bar menu");
    app.popup = None;
    // 2: pressing a crumb (the first crumb) also shows a menu and does not go to that folder.
    app.settings.values.insert("PathClickMode".into(), serde_json::json!(2));
    let path = app.panes[0].path.clone();
    click(&mut app, egui::pos2(pane.left() + 2.0 + 16.0 + 6.0, bar_y));
    assert!(app.popup.is_some(), "mode 2: a segment click opens the menu");
    assert!(!app.panes[0].loading);
    assert_eq!(app.panes[0].path, path);
    fs::remove_dir_all(root).unwrap();
}

/// Original TNXPathLabel MouseDown (0x7c9d94): pressing a divider (the drive ▼ here) opens its DropDownMenu at once.
/// The release of that press keeps the menu open and is not an empty-space click (no path editing); a later click outside closes it.
#[test]
fn address_bar_dividers_open_their_menu_on_press() {
    let (root, ctx, mut app) = fixture("pathpress");
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
    let button = |at: egui::Pos2, pressed: bool| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    run(&mut app, vec![]);
    run(&mut app, vec![]);
    let pane = app.pane_rects[0];
    let bar_y = pane.top() + 2.0 + super::skin_pane::ADDRESS_BAR_HEIGHT / 2.0;
    let lead = egui::pos2(pane.left() + 2.0 + 8.0, bar_y);
    run(&mut app, vec![egui::Event::PointerMoved(lead), button(lead, true)]);
    assert!(app.skin_crumb_menu.as_ref().is_some_and(|(pane, _, drives, _)| *pane == 0 && *drives), "the press opens the drive menu");
    run(&mut app, vec![button(lead, false)]);
    run(&mut app, vec![]);
    assert!(app.skin_crumb_menu.is_some(), "the release of the opening press keeps the menu");
    assert!(!app.panes[0].address_editing, "nor does it start path editing");
    assert!(!app.skin_crumb_press);
    let away = egui::pos2(pane.center().x, pane.center().y);
    run(&mut app, vec![egui::Event::PointerMoved(away), button(away, true)]);
    run(&mut app, vec![button(away, false)]);
    run(&mut app, vec![]);
    assert!(app.skin_crumb_menu.is_none(), "a click outside closes it");
    // Press on the ▼, drag onto the first drive and release there: that drive is chosen (TrackPopupMenu press-drag-release).
    let before = app.panes[0].path.clone();
    run(&mut app, vec![egui::Event::PointerMoved(lead), button(lead, true)]);
    assert!(app.skin_crumb_menu.is_some());
    let first = egui::pos2(pane.left() + 2.0 + 30.0, pane.top() + 2.0 + super::skin_pane::ADDRESS_BAR_HEIGHT + 2.0 + 8.0);
    run(&mut app, vec![egui::Event::PointerMoved(first)]);
    run(&mut app, vec![button(first, false)]);
    assert!(app.skin_crumb_menu.is_none(), "releasing over an item chooses it");
    assert_ne!(app.panes[0].path, before, "the pane goes to the chosen drive");
    fs::remove_dir_all(root).unwrap();
}

/// A breadcrumb's trailing ">" must open a child-folder popup without
/// terminating the GUI, including when it is pressed and then released.
#[test]
fn breadcrumb_chevron_press_opens_and_closes_without_exiting() {
    let (root, ctx, mut app) = fixture("crumb-arrow-no-exit");
    let mut actual = root.join("left");
    fs::create_dir_all(actual.join("child")).unwrap();
    // Use the reported D:\NDR layout if present, otherwise the isolated
    // fixture; no files outside the fixture are ever changed.
    if Path::new(r"D:\NDR").is_dir() {
        actual = PathBuf::from(r"D:\NDR");
    }
    app.navigate(0, actual.clone(), false);
    wait(&mut app, &ctx);
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))),
            events, ..Default::default()
        };
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
    // Use the exact state produced by the chevron MouseDown handler.
    // Rendering must never exit/panic when the folder list is opened.
    let pane = app.pane_rects[0];
    let position = egui::pos2(pane.left() + 48.0,
        pane.top() + 2.0 + super::skin_pane::ADDRESS_BAR_HEIGHT);
    app.skin_crumb_menu = Some((0, actual.clone(), false, position));
    app.skin_crumb_press = false;
    run(&mut app, vec![]);
    assert_eq!(app.panes[0].path, actual, "opening a popup does not navigate");
    assert!(app.skin_crumb_menu.is_some(), "the folder popup stays visible without any navigation");
    run(&mut app, vec![egui::Event::Key {
        key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false,
        modifiers: Default::default(),
    }]);
    assert!(app.skin_crumb_menu.is_none(), "Escape closes the popup safely");
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

/// Regression: every path-segment chevron opens its own subfolder menu.
/// A previous variable shadowing error checked text `hot` AND chevron
/// `arrow_hot` simultaneously, so this was impossible and the release
/// instead entered address editing (and triggered an AccessKit panic).
#[test]
fn breadcrumb_segment_chevron_handles_real_mouse_down() {
    let (root, ctx, mut app) = fixture("crumb-segment-press");
    let run = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))),
            events, ..Default::default()
        };
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
    let root_segment = app.crumb_segments(0).first().unwrap().label.clone();
    let font = egui::FontId::proportional(super::BASE_TEXT_SIZE);
    let width = ctx.fonts_mut(|f| f.layout_no_wrap(root_segment, font, egui::Color32::WHITE).size().x.round() as i32);
    let arrow = egui::pos2(pane.left() + 2.0 + 16.0 + (width + 4) as f32 + 8.0,
        pane.top() + 2.0 + super::skin_pane::ADDRESS_BAR_HEIGHT / 2.0);
    let button = |pressed: bool| egui::Event::PointerButton {
        pos: arrow, button: egui::PointerButton::Primary, pressed,
        modifiers: Default::default(),
    };
    let path_before = app.panes[0].path.clone();
    run(&mut app, vec![egui::Event::PointerMoved(arrow), button(true)]);
    assert!(app.skin_crumb_menu.as_ref().is_some_and(|(pane, _, drives, _)|
        *pane == 0 && !*drives), "segment '>' must open child-folder menu on MouseDown");
    run(&mut app, vec![button(false)]);
    run(&mut app, vec![]);
    assert!(!app.panes[0].address_editing, "chevron must not enter path editing");
    assert_eq!(app.panes[0].path, path_before, "opening subfolders does not navigate");
    assert!(app.skin_crumb_menu.is_some(), "subfolder menu stays open until user chooses");
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn skin_drag_follows_the_original_list_mouse_move() {
    // Original FUN_004856d8 (list OnMouseMove): it must move more than 8 pixels horizontally or vertically to drag, and pressing in the mark cell does not drag.
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
    // Dragged items: if there are marks, the marked items regardless of the pressed item; otherwise the pressed item. ".." is not dragged.
    let left = app.panes[0].path.clone();
    let row = |app: &App, name: &str| app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    let (parent, a, b) = (row(&app, ".."), row(&app, "a.txt"), row(&app, "b.txt"));
    app.begin_file_drag(0, parent, false, egui::Modifiers::NONE);
    assert!(app.file_drag.is_none(), "\"..\" without marks is not dragged");
    app.panes[0].selected.insert(b);
    app.begin_file_drag(0, a, false, egui::Modifiers::NONE);
    assert_eq!(app.file_drag.as_ref().map(|d| d.paths.clone()), Some(vec![left.join("b.txt")]), "the marks, not the pressed item");
    // When a drop moves them, the dragged pane keeps the marks as the last selection and clears them (original Execute < 3 → CopySelection + ClearSelection).
    app.pane_rects[1] = egui::Rect::from_min_size(egui::pos2(700.0, 0.0), egui::vec2(700.0, 800.0));
    let at = egui::pos2(900.0, 400.0);
    let release = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::CTRL }];
    let _ = ctx.run(egui::RawInput { events: release, modifiers: egui::Modifiers::CTRL, ..Default::default() }, |ctx| app.drag_drop_frame(ctx));
    assert!(app.file_drag.is_none());
    // Original FUN_00488b64: even with Ctrl (copy), if "confirm drag and drop" is on it asks with DROPCOPY_CONFIRM.
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
    // Original FUN_00488700 (effect), FUN_00488b64 (drop), FUN_004856d8 (allowed effect at start).
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
    // Dropping in the same folder is effect 0: it neither asks nor creates a shortcut.
    app.handle_drop(0, vec![left.join("a.txt")], left.clone(), egui::Modifiers::NONE);
    assert!(app.msgboxes.is_empty() && app.original_dialogs.is_empty() && !left.join("a.txt.lnk").exists());
    // No key: on the same drive it asks about a move.
    app.handle_drop(1, vec![left.join("a.txt")], right.clone(), egui::Modifiers::NONE);
    assert_eq!(question(&app), Some(text(&app, "RES.DROPMOVE_CONFIRM")));
    app.answer_message_box(ndr::msgbox::IDNO, &ctx);
    assert!(left.join("a.txt").exists(), "No keeps the file");
    // When the drag starts with Ctrl, only copy is allowed: dropping without a key still copies.
    app.handle_drop_with(1, vec![left.join("a.txt")], right.clone(), egui::Modifiers::NONE, 1);
    assert_eq!(question(&app), Some(text(&app, "RES.DROPCOPY_CONFIRM")));
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    wait_job(&mut app);
    assert!(left.join("a.txt").exists() && right.join("a.txt").exists());
    // Starting with Shift means move only: dropping with Ctrl held still moves.
    app.handle_drop_with(1, vec![left.join("b.txt")], right.clone(), egui::Modifiers::CTRL, 2);
    assert_eq!(question(&app), Some(text(&app, "RES.DROPMOVE_CONFIRM")));
    app.answer_message_box(ndr::msgbox::IDYES, &ctx);
    wait_job(&mut app);
    assert!(!left.join("b.txt").exists() && right.join("b.txt").exists());
    // Ctrl+Shift = shortcut, does not ask.
    app.handle_drop(1, vec![left.join("c.log")], right.clone(), egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    assert!(app.msgboxes.is_empty());
    assert!(right.join("c.log.lnk").exists(), "\"name.ext.lnk\" shortcut");
    // Effect while dragging (DoDragDrop pointer): other pane = move (same drive), Ctrl = copy, same folder = cannot drop.
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
    // Original ShowToolTip format 3's DT_WORDBREAK: break only at spaces, and a word longer than the width stays on one line.
    let mut measure = |s: &str| s.chars().count() as f32;
    assert_eq!(super::panes::word_break_lines("a bb ccc", 4.0, &mut measure), vec!["a bb", "ccc"]);
    assert_eq!(super::panes::word_break_lines("abcdefgh ij", 4.0, &mut measure), vec!["abcdefgh", "ij"]);
    assert_eq!(super::panes::word_break_lines("photo.jpg", 4.0, &mut measure), vec!["photo.jpg"]);
}

#[test]
fn only_the_vertical_wheel_scrolls_like_the_original() {
    // The original WMMouseWheel does not block Shift, and WMMouseHWheel (the tilt wheel) moves nothing.
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
    // New tab: same location, empty history.
    app.dispatch("newTab", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].tab, app.panes[0].path.clone()), (2, 1, left.join("folder")));
    assert!(app.panes[0].back.is_empty());
    app.navigate(0, left.clone(), true);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].back, vec![left.join("folder")]);
    // Returning to the first tab restores that tab's location and history.
    app.tab_command(0, "tab:0", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].path.clone(), app.panes[0].back.clone()), (left.join("folder"), vec![left.clone()]));
    // Closing the current tab restores the remaining tab's location and history.
    app.dispatch("closeTab", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].path.clone(), app.panes[0].back.clone()), (1, left.clone(), vec![left.join("folder")]));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn saved_tabs_restore_like_csharp_restore_pane_tabs() {
    let saved: Vec<String> = ["C:\\a", "C:\\gone", "C:\\b", "C:\\a"].iter().map(|s| s.to_string()).collect();
    let exists = |p: &Path| p != Path::new("C:\\gone");
    // C#: drop tabs of missing folders and select the saved selected tab (original index 3 = the second C:\a).
    assert_eq!(super::restore_tabs(&saved, Some(3), Path::new("C:\\a"), false, exists), (vec![PathBuf::from("C:\\a"), PathBuf::from("C:\\b"), PathBuf::from("C:\\a")], 2));
    // If the selected tab's folder is gone, pick the current folder's tab; if absent, prepend it.
    assert_eq!(super::restore_tabs(&saved, Some(1), Path::new("C:\\b"), false, exists).1, 1);
    assert_eq!(super::restore_tabs(&saved, None, Path::new("C:\\c"), false, exists), (vec![PathBuf::from("C:\\c"), PathBuf::from("C:\\a"), PathBuf::from("C:\\b"), PathBuf::from("C:\\a")], 0));
    // A folder set from the command line or start folder comes before the saved selected tab.
    assert_eq!(super::restore_tabs(&saved, Some(3), Path::new("C:\\b"), true, exists).1, 1);
    // If no tabs remain, a single start-folder tab.
    assert_eq!(super::restore_tabs(&saved, Some(0), Path::new("D:\\x"), false, |_| false), (vec![PathBuf::from("D:\\x")], 0));
}

#[test]
fn window_bounds_are_saved_like_csharp_closing() {
    let (root, _ctx, mut app) = fixture("window-bounds");
    let outer = egui::Rect::from_min_size(egui::pos2(120.0, 80.0), egui::vec2(1216.0, 839.0));
    let inner = egui::Rect::from_min_size(egui::pos2(128.0, 111.0), egui::vec2(1200.0, 800.0));
    app.track_window(Some(outer), Some(inner), Some(false), Some(false));
    // When maximized, keep the normal window position/size and record only the maximized flag. While minimized, change nothing.
    app.track_window(Some(egui::Rect::from_min_size(egui::pos2(-8.0, -8.0), egui::vec2(1936.0, 1048.0))), Some(inner), Some(true), Some(false));
    app.track_window(Some(egui::Rect::from_min_size(egui::pos2(-32000.0, -32000.0), egui::vec2(160.0, 28.0))), None, Some(false), Some(true));
    app.save();
    let number = |key: &str| app.settings.values.get(key).and_then(|v| v.as_f64());
    assert_eq!((number("WindowLeft"), number("WindowTop"), number("WindowWidth"), number("WindowHeight")), (Some(120.0), Some(80.0), Some(1216.0), Some(839.0)), "outer size like the original TForm/C# Window");
    // The frame border and title bar size subtracted when reopening (a normal window).
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
    // A new Area only measures invisibly on the first frame, so it is seen on a later frame (an open list is one frame later still).
    for _ in 0..3 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::Area::new(menu).order(egui::Order::Foreground).fade_in(false).show(ctx, |ui| ui.painter().rect_filled(square, 0.0, Color32::WHITE));
        egui::Area::new(window).order(egui::Order::Middle).fade_in(false).show(ctx, |ui| ui.painter().rect_filled(square, 0.0, Color32::WHITE));
        // Like the original window of egui::Modal: a foreground layer but not a menu.
        super::keep_opaque(ctx, dialog);
        egui::Area::new(dialog).order(egui::Order::Foreground).fade_in(false).show(ctx, |ui| ui.painter().rect_filled(square, 0.0, Color32::WHITE));
        // An open list (ComboBox) is also a foreground layer but not a menu (the original and C# versions make only menus translucent): show_opaque marks the list layer.
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
    // Original SetLayeredWindowAttributes(menu window, 0xF0) = C# Opacity 0.94: only the menu layer is dimmed; the window layer, the original window (Modal), and the open list stay as-is.
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
    static GENERIC_FOLDER_ICON_QUERIES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static FLAKY_FOLDER_ICON_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[test]
fn archive_folder_icon_cache_never_uses_a_drive_root_or_virtual_entry_path() {
    let (root, ctx, mut app) = fixture("generic-folder-path");
    app.file_icons.clear();
    app.file_icon_failed_at.clear();
    GENERIC_FOLDER_ICON_QUERIES.with(|seen| seen.borrow_mut().clear());
    app.system.file_icon = |path, directory| {
        if directory {
            GENERIC_FOLDER_ICON_QUERIES.with(|seen| seen.borrow_mut().push(path.to_string_lossy().into_owned()));
        }
        Some((2, 2, if directory && path == Path::new("folder") { [230, 188, 84, 255] } else { [255, 0, 0, 255] }.repeat(4)))
    };
    let first = app.any_path_icon(&ctx, Path::new("C:\\"), true).expect("drive must use generic placeholder");
    let second = app.any_path_icon(&ctx, Path::new("assets"), true).expect("virtual archive folder must have an icon");
    let third = app.any_path_icon(&ctx, Path::new("D:\\NDR\\release\\Skins"), true).expect("local folder must use generic placeholder");
    assert_eq!(first.id(), second.id());
    assert_eq!(second.id(), third.id());
    assert_eq!(GENERIC_FOLDER_ICON_QUERIES.with(|seen| seen.borrow().clone()), ["folder"]);
    assert!(!app.file_icon_failed_at.contains_key("<DIR>"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn transient_missing_or_invisible_folder_icon_recovers_instead_of_being_cached_forever() {
    let (root, ctx, mut app) = fixture("icon-retry");
    FLAKY_FOLDER_ICON_CALLS.with(|calls| calls.set(0));
    app.file_icons.clear();
    app.file_icon_failed_at.clear();
    app.system.file_icon = |path, directory| {
        assert!(directory && path == Path::new("folder"));
        let n = FLAKY_FOLDER_ICON_CALLS.with(|calls| { let n = calls.get(); calls.set(n + 1); n });
        if n == 0 { None }
        else if n == 1 { Some((2, 2, [255, 0, 0, 0].repeat(4))) }
        else { Some((2, 2, [228, 188, 85, 255].repeat(4))) }
    };
    assert!(app.any_path_icon(&ctx, Path::new("assets"), true).is_none());
    assert!(app.file_icons.get("<DIR>").is_some_and(Option::is_none));
    assert_eq!(FLAKY_FOLDER_ICON_CALLS.with(|calls| calls.get()), 1);
    // A failure does not hammer shell handlers at redraw/frame rate.
    assert!(app.any_path_icon(&ctx, Path::new("build"), true).is_none());
    assert_eq!(FLAKY_FOLDER_ICON_CALLS.with(|calls| calls.get()), 1);
    app.file_icon_failed_at.insert("<DIR>".into(), Instant::now() - Duration::from_secs(5));
    assert!(app.any_path_icon(&ctx, Path::new("config"), true).is_none(), "fully transparent icon is not a success");
    assert_eq!(FLAKY_FOLDER_ICON_CALLS.with(|calls| calls.get()), 2);
    app.file_icon_failed_at.insert("<DIR>".into(), Instant::now() - Duration::from_secs(5));
    let restored = app.any_path_icon(&ctx, Path::new("config"), true).expect("folder icon recovers on retry");
    assert_eq!(restored.size(), [2, 2]);
    assert!(!app.file_icon_failed_at.contains_key("<DIR>"));
    assert_eq!(FLAKY_FOLDER_ICON_CALLS.with(|calls| calls.get()), 3);
    assert_eq!(app.any_path_icon(&ctx, Path::new("other"), true).unwrap().id(), restored.id());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn transient_local_overlay_failure_retries_and_replaces_the_generic_icon() {
    let (root, ctx, mut app) = fixture("overlay-retry");
    app.settings.set_flag("UseOverlayIcons", true);
    let row = app.panes[0].entries.iter().position(|entry| entry.name == "folder").unwrap();
    let path = app.panes[0].entries[row].path.clone();
    app.overlay_icons.insert(path.clone(), None);
    app.overlay_icon_failed_at.insert(path.clone(), Instant::now());
    assert_eq!(app.file_icon(&ctx, 0, row).unwrap().size(), [2, 2]);
    assert!(!app.overlay_pending.contains(&path), "a failed lookup should be throttled");
    app.overlay_icon_failed_at.insert(path.clone(), Instant::now() - Duration::from_secs(5));
    assert_eq!(app.file_icon(&ctx, 0, row).unwrap().size(), [2, 2]);
    assert!(app.overlay_pending.contains(&path), "a failed lookup should be retried once after backoff");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.overlay_icons.get(&path).is_some_and(Option::is_some) {
        assert!(Instant::now() < deadline, "overlay retry timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.file_icon(&ctx, 0, row).unwrap().size(), [3, 3]);
    assert!(!app.overlay_icon_failed_at.contains_key(&path));
    fs::remove_dir_all(root).unwrap();
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
    // Turn off overlay icons (UseOverlayIcons, on by default in the original) and look at each file's icon alone.
    app.settings.set_flag("UseOverlayIcons", false);
    draw(&mut app, &ctx);
    assert!(!app.settings.flag("UseOverlayIcons", true));
    // Original GetIconIndex (actual path): an executable uses its own file's icon (a fake 5x5 in the test). Until it arrives, the extension icon.
    let exe = app.panes[0].entries.iter().position(|e| e.name == "tool.exe").unwrap();
    let path = app.panes[0].entries[exe].path.clone();
    assert_eq!(app.file_icon(&ctx, 0, exe).map(|t| t.size()), Some([2, 2]));
    while !app.overlay_icons.contains_key(&path) {
        assert!(Instant::now() < end, "icon timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.file_icon(&ctx, 0, exe).map(|t| t.size()), Some([5, 5]));
    // Formats whose extension always has the same icon keep using the extension cache.
    let txt = app.panes[0].entries.iter().position(|e| e.name.ends_with(".txt")).unwrap();
    assert_eq!(app.file_icon(&ctx, 0, txt).map(|t| t.size()), Some([2, 2]));
    assert!(!app.overlay_icons.contains_key(&app.panes[0].entries[txt].path));
    // Even outside the fixed list, when HKCR says the icon is per file (DefaultIcon "%1" / IconHandler, a fake hook), read it by the actual path. Each extension is asked once.
    app.system.per_file_icon = |extension| {
        PER_FILE_ASKED.with(|a| a.borrow_mut().push(extension.to_owned()));
        extension == "msi"
    };
    assert!(app.per_file_icon_type(std::path::Path::new("C:\\x\\Setup.MSI")) && app.per_file_icon_type(std::path::Path::new("C:\\y\\other.msi")));
    assert!(!app.per_file_icon_type(std::path::Path::new("C:\\x\\a.doc")) && app.per_file_icon_type(std::path::Path::new("C:\\x\\b.LNK")));
    assert_eq!(PER_FILE_ASKED.with(|a| a.take()), ["msi", "doc"], "the fixed list is not asked; each extension once");
    assert!(ndr::native::per_file_icon_rule(Some("\"%1\""), false) && ndr::native::per_file_icon_rule(None, true));
    assert!(!ndr::native::per_file_icon_rule(Some("%SystemRoot%\\System32\\imageres.dll,-102"), false));
    // This PC's HKCR (read only): an executable is "%1", a text file is one icon per extension.
    assert!(ndr::native::extension_has_per_file_icon("exe") && !ndr::native::extension_has_per_file_icon("txt"));
    // A folder is read by its actual path only when it has the read-only/system attributes that make the shell read desktop.ini (a custom folder icon). A plain folder uses the "<DIR>" cache.
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
    // Original image thread FUN_007dd51c: a local ".." takes the parent folder's own shell icon (with overlays on); until then the folder icon.
    app.settings.set_flag("UseOverlayIcons", true);
    let up = app.panes[0].entries.iter().position(|e| e.is_parent()).expect("a subfolder lists \"..\"");
    let up_path = app.panes[0].entries[up].path.clone();
    assert_eq!(up_path.as_path(), root.as_path());
    assert_eq!(app.file_icon(&ctx, 0, up).map(|t| t.size()), Some([2, 2]));
    while !app.overlay_icons.contains_key(&up_path) {
        assert!(Instant::now() < end, "parent icon timeout");
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.file_icon(&ctx, 0, up).map(|t| t.size()), Some([3, 3]), "the overlay shell icon (fake 3x3)");
    let mut permissions = fs::metadata(&custom).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    let _ = fs::set_permissions(&custom, permissions);
    let _ = fs::remove_dir_all(root);
}

/// User-reported (2026-10-31) pane behavior: going back puts the cursor on the subfolder we left and shows that page; the wheel scrolls the list by page like the original;
/// dot files like ".gitconfig" show the whole name with no extension.
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
    // Dot files: name cell = whole name, extension cell empty. ".config.json" is ".config" + "json".
    app.compute_view(0);
    let cells = |app: &App, name: &str| app.panes[0].view.cells[row(app, name)].clone();
    assert_eq!((cells(&app, ".gitconfig")[0].as_ref(), cells(&app, ".gitconfig")[1].as_ref()), (".gitconfig", ""));
    assert_eq!((cells(&app, ".config.json")[0].as_ref(), cells(&app, ".config.json")[1].as_ref()), (".config", "json"));
    // Original GetAttrString: A/H/R/S into "____" (the folder dir000 has no attributes).
    assert_eq!(cells(&app, "dir000")[5].as_ref(), "____");
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
    // Wheel: by user request (2026-10-31 16:49) it scrolls the list like the original WMMouseWheel. The cursor and active pane stay the same, and
    // since it is by page (the default), one notch down = next page (as many as the visible columns), one notch up = previous page.
    app.active = 1;
    app.panes[0].cursor = 0;
    let at = app.pane_rects[0].center();
    let wheel = |y: f32| egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, y), modifiers: Default::default() };
    assert_eq!(app.panes[0].first_column, 0);
    run(&mut app, vec![egui::Event::PointerMoved(at), wheel(-1.0)]);
    run(&mut app, vec![]);
    let page = app.panes[0].first_column;
    assert!(page > 0, "one notch down scrolls a page");
    assert_eq!((app.panes[0].cursor, app.active), (0, 1), "the wheel does not move the cursor or activate the pane");
    run(&mut app, vec![wheel(1.0)]);
    run(&mut app, vec![]);
    assert_eq!((app.panes[0].first_column, app.panes[0].cursor), (0, 0), "one notch up scrolls back");
    // Enter a folder on a later page, then go back: cursor on that folder, and that page is shown (first column is not 0).
    app.active = 0;
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
    // Original history capacity 20: the places to go back to do not exceed 19 (20 including the current folder).
    for i in 0..25 {
        app.navigate(0, left.join(format!("dir{i:03}")), true);
    }
    wait(&mut app, &ctx);
    assert_eq!(app.history_list(0).len(), 20);
    assert_eq!(app.panes[0].back.first(), Some(&left.join("dir005")), "the oldest entries are dropped");
    let _ = fs::remove_dir_all(root);
}

/// Original shortcut.ini miDescription=Alt+Enter: Alt+Enter is "attach description" and there is no fullscreen toggle (reverted at the user's request).
#[test]
fn alt_enter_edits_the_description_like_the_original() {
    let (root, ctx, mut app) = fixture("fullscreen");
    let on_file = |app: &mut App| app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    on_file(&mut app);
    let alt_enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT };
    let raw = egui::RawInput { modifiers: egui::Modifiers::ALT, events: vec![alt_enter], screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let output = ctx.run(raw.clone(), |ctx| app.keyboard(ctx));
    let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(!commands.iter().any(|c| matches!(c, egui::ViewportCommand::Fullscreen(_))), "{commands:?}");
    // The Enter that ran the shortcut does not reach the window it opened in the same frame (it used to press OK at once).
    let (same_root, _, mut same_frame) = fixture("fullscreen-same-frame");
    on_file(&mut same_frame);
    let _ = ctx.run(raw, |ctx| {
        same_frame.keyboard(ctx);
        same_frame.original_dialog_ui(ctx);
    });
    assert_eq!(same_frame.original_dialogs.len(), 1, "the description box stays open");
    let _ = fs::remove_dir_all(same_root);
    let (expected_root, _, mut expected) = fixture("fullscreen-expected");
    on_file(&mut expected);
    expected.dispatch("description", &ctx);
    let kinds = |app: &App| (app.original_dialogs.len(), app.original_dialogs.last().map(|d| d.title.clone()), app.msgboxes.len());
    assert_eq!(kinds(&app), kinds(&expected), "Alt+Enter = miDescription");
    assert!(kinds(&app).0 + kinds(&app).2 > 0, "the description command opened something: {:?}", kinds(&app));
    let _ = fs::remove_dir_all(expected_root);
    let _ = fs::remove_dir_all(root);
}

/// Original AdjustPage (0x7daf24): when the list size changes (WMSize) or the same folder is re-read (FileListChanged), it re-fixes the horizontal position
/// so the cursor column becomes the last visible column (from that column if the cursor column is off to the left).
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
    // Put the cursor far back and reveal it, then move the cursor to the first visible column (position unchanged).
    app.panes[0].cursor = 500;
    app.panes[0].reveal = true;
    run(&mut app, 850.0);
    let first = app.panes[0].first_column;
    assert!(first > 0);
    app.panes[0].cursor = first * rows;
    run(&mut app, 850.0);
    assert_eq!(app.panes[0].first_column, first, "moving inside the page keeps the position");
    // When the list size changes (window height) the cursor column becomes the last visible column.
    run(&mut app, 650.0);
    run(&mut app, 650.0);
    let columns = app.panes[0].page.columns;
    let rows = app.panes[0].page.rows;
    let column = app.panes[0].cursor / rows;
    assert_eq!(app.panes[0].first_column, (column / columns) * columns, "page scrolling (default): WMSize → AdjustPage shows the cursor's page");
    // By line (ScrollStep 1): the cursor column becomes the last visible column.
    app.settings.values.insert("LeftScrollStep".into(), serde_json::json!(1));
    app.panes[0].first_column = column;
    run(&mut app, 650.0);
    run(&mut app, 600.0);
    run(&mut app, 600.0);
    let rows = app.panes[0].page.rows;
    let column = app.panes[0].cursor / rows;
    assert!(column >= first);
    assert_eq!(app.panes[0].first_column, (column + 1).saturating_sub(columns), "WMSize → AdjustPage");
    // Rereading the same folder gives the same result (refresh after putting the cursor column first).
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

/// Original FUN_00487ce8 and SetSortType (0x7dbb28): in Details, rereading the same folder keeps the old vertical position (SetScrollTop), changing the sort makes the cursor
/// item visible (SetItemIndex), and going to another folder shows the list from the first item.
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
    // Rereading the same folder keeps the old position.
    app.refresh(0);
    wait(&mut app, &ctx);
    run(&mut app);
    run(&mut app);
    assert_eq!(app.panes[0].page.top, top, "SetScrollTop(old top)");
    // Sorting descending: the cursor item (f250) moves to the front and is visible.
    assert!(app.sort_command("sortDescending"));
    run(&mut app);
    run(&mut app);
    let page = app.panes[0].page.clone();
    let cursor = app.panes[0].cursor;
    assert_eq!(app.panes[0].entries[cursor].name, "f250.txt");
    assert!(page.top <= cursor && cursor <= page.last, "SetSortDescend → SetItemIndex: {page:?} {cursor}");
    // Another folder: from the first item.
    app.navigate(0, left.join("sub"), true);
    wait(&mut app, &ctx);
    run(&mut app);
    run(&mut app);
    assert_eq!((app.panes[0].cursor, app.panes[0].page.top), (0, 0), "a new folder starts at the top");
    let _ = fs::remove_dir_all(root);
}


/// Original MouseDown (0x7d63c8): a middle-button press also does SetItemFocused (moves only the cursor, marks unchanged).
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

/// Original TNXShellChangeNotify (pane +0x374): when another program creates a file in the current folder, that folder is reread after Delay(1500 ms).
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

/// Original own operations (new folder FUN_00432378, rename FUN_004351cc, attributes FUN_0043657c, archive FUN_0043a7f8 ...): SetActive(0) -> operation ->
/// reread (FUN_00487ce8) -> SetActive(1) (new watch). So an own change does not cause another read 1.5 s later, while later outside changes are caught.
#[test]
fn own_changes_followed_by_a_refresh_do_not_relist_again() {
    let (root, ctx, mut app) = fixture("own-change");
    let left = app.panes[0].path.clone();
    app.watch_folders = true;
    app.poll(&ctx);
    std::thread::sleep(std::time::Duration::from_millis(300));
    let before = app.panes[0].generation;
    // Own operation: change then reread at once (same order as the original).
    fs::create_dir(left.join("made-by-us")).unwrap();
    app.refresh(0);
    let end = std::time::Instant::now() + std::time::Duration::from_millis(2500);
    while std::time::Instant::now() < end {
        app.poll(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(app.panes[0].generation, before + 1, "only our own refresh, no delayed second listing");
    assert!(app.panes[0].entries.iter().any(|e| e.name == "made-by-us"));
    // Later outside changes are caught by the new watch.
    fs::write(left.join("made-outside.txt"), b"x").unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while !app.panes[0].entries.iter().any(|e| e.name == "made-outside.txt") {
        assert!(std::time::Instant::now() < end, "the re-armed watch missed an outside change");
        app.poll(&ctx);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    app.watch_folders = false;
    app.poll(&ctx);
    let _ = fs::remove_dir_all(root);
}

/// Original TfMain DFM (pmFile ... pmHelp, Visible=False items removed, doubled and trailing separators merged or dropped): visible main menu items and their order.
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
        "miOpen miParameterOpen miOpenAllSelectedFiles - miCopyTo miMoveTo - miDelete miShred - miRename miAdvRename miChangeAttributes - miCalculateSelectedFolderSize - miProperties miDescription - miCreateEmptyFile miShortcutOnDesktop - miChecksum - miExit",
        "miFolderTree miFavorites - miNewFolder - miHistoryBack miHistoryForward - miToParent miToRoot miToLastDest - miNetwork - miSetWorkFolder+miGoToWorkFolder - miRefresh",
        "miEditCopy miEditCut miEditPaste miPasteAsShortcut - miCopyFullPath miCopyFileName miCopyPath - miSelect miSelectAll miInvertSelection miAdvSelect miAdvDeselect - miRestoreLastSel miSelectSameExt miSelectSameName - miFind miFindNext",
        "miLanguage miSkin - miDualView miShowFolders - miViewToolbar miViewFindResult miViewVertBar miViewButtonBar - miShowDriveButtons miViewFolderInfoBar miViewFileInfoBar - miNewTab - miListStyle miColumn - miSortBy miFilter - miShowHidden",
        "miToDesktop miToDocuments miToPictures miToMusic miToVideos miToDownloads - miControlPanel miAddRemoveProg - miRecycleBin miEmptyRecycleBin - miToTemp miEmptyTemp - miCommandLine",
        "miArcAdd - miArcExtract miArcExtractToCurr miArcExtractAuto",
        "miFTPQuickConnect - miFTPConnect miFTPDisconnect - miFTPGet miFTPPut - miMapNetworkDrive miUnmapNetworkDrive",
        // User request: Disk Cleanup (miCleanupDisk) was removed.
        "miTextViewer miTextEditor - miClearDescriptions - miSaveList - miSplitFile miJoinFile - miCompareFolder - miOptions",
        "miHelp miShortcut - miHomepage - miCheckUpdate - miAbout",
    ];
    let menus: Vec<String> = super::menu_data::MENUS.iter().map(|(_, _, _, items)| shown(items)).collect();
    assert_eq!(menus.len(), expected.len());
    let wrong: Vec<String> = menus.iter().zip(expected).filter(|(got, want)| got.as_str() != *want).map(|(got, want)| format!("\n got: {got}\nwant: {want}")).collect();
    assert!(wrong.is_empty(), "{}", wrong.concat());
}

/// Original GetItemAt (0x7dac1c): in Details (style 2) the row pitch is item height + LineSpace and the gap above a row belongs to that item; in column-flowing lists (styles 0/1)
/// the gap below an item belongs to it.
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

/// Original miShowFoldersClick (0x445258) -> FUN_0048da38: only on the current pane, a folder tree left of the list (width treewidth, initially 120) and a 3 px divider,
/// with the list to its right. Stored in that pane's setting (ShowLeft/RightFolderTree).
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

/// Original pane tree (TNXShellTreeView = VCL TTreeView, OnChange FUN_0048e320): with focus in the tree, Down/Up move the selected item
/// and each time the pane goes to that folder. Left collapses an expanded item and goes to the parent from a collapsed one. A letter goes to the next item starting with it.
/// Clicking the list returns focus to the list.
#[test]
fn cached_nested_folder_tree_renders_without_reloading_children() {
    use ndr::shell_tree::{ShellNode, Special};
    let (root, ctx, mut app) = fixture("tree-cached-subtrees");
    let outer = root.join("outer");
    let inner = outer.join("inner");
    fs::create_dir_all(&inner).unwrap();
    let make_node = |path: &Path, has_children| ShellNode {
        key: path.to_path_buf(), name: path.file_name().unwrap().to_string_lossy().into_owned(),
        path: Some(path.to_path_buf()), has_children, special: Special::None, ghosted: false,
    };
    let root_node = make_node(&root, true);
    let outer_node = make_node(&outer, true);
    let inner_node = make_node(&inner, false);
    app.shell_desktop = Some(root_node);
    let hidden = app.panes[0].show_hidden;
    app.tree_cache.insert((root.clone(), hidden), vec![outer_node]);
    app.tree_cache.insert((outer.clone(), hidden), vec![inner_node]);
    app.panes[0].tree_open.insert(root.clone());
    app.panes[0].tree_open.insert(outer.clone());
    app.panes[0].tree_synced = Some(app.panes[0].path.clone());
    let render = |app: &mut App| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO, egui::vec2(900.0, 650.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                app.pane_tree_ui(ui, 0, egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0), egui::vec2(280.0, 520.0)));
            });
        });
    };
    render(&mut app);
    let paths = |app: &App| app.panes[0].tree_rows.iter()
        .map(|(node, ..)| node.key.clone()).collect::<Vec<_>>();
    let expected = vec![root.clone(), outer.clone(), inner.clone()];
    assert_eq!(paths(&app), expected);
    assert!(app.tree_requests.is_empty(), "cached branches must not queue shell reads");
    let first = Arc::clone(&app.panes[0].tree_rows);
    render(&mut app);
    assert_eq!(paths(&app), expected, "subtree traversal remains stable across frames");
    assert!(Arc::ptr_eq(&first, &app.panes[0].tree_rows), "unchanged paint must share the original row buffer");
    assert!(app.tree_requests.is_empty());
    assert_eq!(app.panes[0].tree_row_positions.get(&ndr::shell_tree::key_identity(&inner)), Some(&2));

    // Closing and reopening changes the flattened rows even without a new
    // shell result. Hidden mode and new shell data also invalidate the cache.
    app.panes[0].tree_open.remove(&outer);
    render(&mut app);
    assert_eq!(paths(&app), vec![root.clone(), outer.clone()]);
    assert!(!Arc::ptr_eq(&first, &app.panes[0].tree_rows));
    app.panes[0].tree_open.insert(outer.clone());
    render(&mut app);
    assert_eq!(paths(&app), expected);
    let reopened = Arc::clone(&app.panes[0].tree_rows);
    let extra = outer.join("extra");
    app.tree_cache.insert((outer.clone(), hidden), vec![make_node(&inner, false), make_node(&extra, false)]);
    app.tree_cache_revision = app.tree_cache_revision.wrapping_add(1);
    render(&mut app);
    assert!(!Arc::ptr_eq(&reopened, &app.panes[0].tree_rows));
    assert_eq!(paths(&app), vec![root.clone(), outer.clone(), inner.clone(), extra]);
    let updated = Arc::clone(&app.panes[0].tree_rows);
    app.tree_cache.insert((root.clone(), !hidden), vec![make_node(&outer, true)]);
    app.tree_cache.insert((outer.clone(), !hidden), Vec::new());
    app.panes[0].show_hidden = !hidden;
    render(&mut app);
    assert!(!Arc::ptr_eq(&updated, &app.panes[0].tree_rows));
    assert_eq!(paths(&app), vec![root.clone(), outer.clone()]);
    assert!(!app.panes[0].tree_rows[1].2, "empty hidden-view branch has no expand glyph");
    assert!(app.tree_requests.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "explicit release benchmark for repeated large folder-tree rebuilds"]
fn benchmark_cached_tree_rows_during_repaints() {
    use ndr::shell_tree::{ShellNode, Special};
    let (root, _ctx, mut app) = fixture("tree-repaint-bench");
    let make_node = |key: PathBuf, has_children: bool| ShellNode {
        name: key.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        path: Some(key.clone()), key, has_children, special: Special::None, ghosted: false,
    };
    let hidden = app.panes[0].show_hidden;
    app.shell_desktop = Some(make_node(root.clone(), true));
    app.panes[0].tree_open.insert(root.clone());
    let parents: Vec<_> = (0..100).map(|i| root.join(format!("folder_{i:03}"))).collect();
    app.tree_cache.insert((root.clone(), hidden), parents.iter().cloned().map(|p| make_node(p, true)).collect());
    for parent in parents {
        let children = (0..50).map(|i| make_node(parent.join(format!("sub_{i:03}")), false)).collect();
        app.panes[0].tree_open.insert(parent.clone());
        app.tree_cache.insert((parent, hidden), children);
    }
    let expected = app.cached_pane_tree_rows(0);
    assert_eq!(expected.len(), 5101);
    let rounds = 120;
    let start = Instant::now();
    for _ in 0..rounds {
        app.panes[0].tree_rows_key = None;
        let rows = app.cached_pane_tree_rows(0);
        assert_eq!(rows.len(), expected.len());
        std::hint::black_box(rows);
    }
    let rebuild = start.elapsed();
    let first = Arc::clone(&app.panes[0].tree_rows);
    let start = Instant::now();
    for _ in 0..rounds {
        let rows = app.cached_pane_tree_rows(0);
        assert!(Arc::ptr_eq(&first, &rows));
        std::hint::black_box(rows);
    }
    let reused = start.elapsed();
    println!(
        "TREE_REPAINT rows=5101 rounds={rounds} rebuildMs={:.3} cachedMs={:.3} speedup={:.2}x",
        rebuild.as_secs_f64()*1000.0, reused.as_secs_f64()*1000.0,
        rebuild.as_secs_f64()/reused.as_secs_f64(),
    );
    assert!(app.tree_requests.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pane_tree_keys_follow_the_ttreeview() {
    use ndr::shell_tree::{ShellNode, Special};
    let (root, ctx, mut app) = fixture("tree-keys");
    let left = app.panes[0].path.clone();
    for name in ["alpha", "beta"] {
        fs::create_dir_all(left.join(name).join("inner")).unwrap();
    }
    let node = |path: &Path, has_children: bool| ShellNode { key: path.to_path_buf(), name: path.file_name().unwrap().to_string_lossy().into_owned(), path: Some(path.to_path_buf()), has_children, special: Special::None, ghosted: false };
    app.settings.set_flag("ShowLeftFolderTree", true);
    app.active = 0;
    app.tree_focus = Some(0);
    app.panes[0].tree_open.insert(left.clone());
    app.panes[0].tree_rows = vec![
        (node(&left, true), 0, true, true),
        (node(&left.join("alpha"), true), 1, true, false),
        (node(&left.join("beta"), true), 1, true, false),
    ].into();
    let press = |app: &mut App, key: egui::Key, text: Option<&str>| {
        let mut events = vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }];
        events.extend(text.map(|t| egui::Event::Text(t.into())));
        let raw = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| app.keyboard(ctx));
        wait(app, &ctx);
    };
    press(&mut app, egui::Key::ArrowDown, None);
    assert_eq!(app.panes[0].path, left.join("alpha"), "↓ = next item, OnChange moves the pane");
    // Letter b: the next "b..." item.
    app.panes[0].tree_synced = Some(app.panes[0].path.clone());
    press(&mut app, egui::Key::B, Some("b"));
    assert_eq!(app.panes[0].path, left.join("beta"));
    // Left (collapsed item): to the parent.
    app.panes[0].tree_synced = Some(app.panes[0].path.clone());
    press(&mut app, egui::Key::ArrowLeft, None);
    assert_eq!(app.panes[0].path, left, "← on a collapsed item selects its parent");
    // Left (expanded item): collapses it.
    press(&mut app, egui::Key::ArrowLeft, None);
    assert!(!app.panes[0].tree_open.iter().any(|k| ndr::shell_tree::same_key(k, &left)), "← on an open item collapses it");
    // Clicking the list moves focus to the list.
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


/// Original UpdateScrollBar, ArrowKey and AdjustPage (style 2): when the total cell width exceeds the list it can scroll horizontally, Right/Left keys move by 20,
/// and a list size change resets the horizontal position to 0.
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
    // A list size change (AdjustPage) resets it to 0.
    run(&mut app, vec![], 650.0);
    run(&mut app, vec![], 650.0);
    assert_eq!(app.panes[0].detail_x, 0.0);
    let _ = fs::remove_dir_all(root);
}

/// Also accepts the command ids (menu_data::ORIGINAL_ACTIONS) used when the original shortcut table binds keys to pmMain popup items or submenu names.
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


/// Non-shortcut keys of the original KeyDown (FUN_0048423c): '.' = parent folder (when the quick search text is empty), Ctrl+Tab / Ctrl+Shift+Tab = next/previous tab,
/// Shift+letter = that drive (with English input, or when hanshdrive is on).
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

/// Frame time of one frame on a large folder (20,000 files) (run by hand only: cargo test --release -- --ignored big_folder_frame_time).
#[test]
#[ignore]
fn big_folder_frame_time() {
    let (root, ctx, mut app) = fixture("big-folder");
    let big = root.join("big");
    fs::create_dir_all(&big).unwrap();
    for i in 0..20000 {
        fs::write(big.join(format!("file{i:05}.txt")), b"").unwrap();
    }
    let start = Instant::now();
    app.navigate(0, big.clone(), false);
    wait(&mut app, &ctx);
    eprintln!("listing 20k: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    assert_eq!(app.panes[0].entries.len(), 20001);
    for command in ["sort:Size", "sort:Name", "sortDescending", "sortDescending", "selectAll", "invertSelection"] {
        let start = Instant::now();
        app.dispatch(command, &ctx);
        wait(&mut app, &ctx);
        eprintln!("{command}: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    }
    let start = Instant::now();
    app.compute_view(0);
    eprintln!("compute_view: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    let mut entries = app.panes[0].entries.clone();
    let options = ndr::listing::SortOptions { folder_mode: "Name".into(), folder_descending: false, file_mode: "Size".into(), file_descending: false, logical: true };
    let start = Instant::now();
    ndr::listing::sort_entries(&mut entries, &options, &|_| 0);
    eprintln!("sort_entries: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    let times: Vec<std::time::SystemTime> = (0..20000u64).map(|i| std::time::UNIX_EPOCH + Duration::from_secs(1_600_000_000 + i * 3607)).collect();
    let now = std::time::SystemTime::now();
    let start = Instant::now();
    for &t in &times {
        let _ = ndr::listing::list_date_text(Some(t), "yyyy-MM-dd", "HH:mm", Some(["Today", "Yesterday", "%d days ago"]), now);
    }
    eprintln!("list_date_text x20k (spread): {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    let start = Instant::now();
    for &t in &times {
        let _ = chrono::DateTime::<chrono::Local>::from(t).date_naive();
    }
    eprintln!("chrono Local x20k: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
    let start = Instant::now();
    for e in &entries {
        let _ = ndr::parity::size_to_str(e.size, "Auto", true);
        let _ = ndr::listing::attr_string(e.attributes);
    }
    eprintln!("size+attr x20k: {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
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

/// Repeated refresh/filter/sort/navigation with real files, with a small
/// directory between the large listings so retained allocations are checked.
#[test]
#[ignore = "explicit Release soak test"]
fn batch78_repeated_navigation_and_selection_soak() {
    let (root,ctx,mut app)=fixture("batch78-soak");
    let large=root.join("large");let small=root.join("small");
    fs::create_dir_all(&large).unwrap();fs::create_dir_all(&small).unwrap();
    for i in 0..4000 {
        fs::write(large.join(format!("file{i:05}.{}",if i%2==0{"txt"}else{"log"})), [i as u8]).unwrap();
    }
    fs::write(small.join("only.txt"),b"x").unwrap();
    let start=Instant::now();
    for cycle in 0..50 {
        app.navigate(0,large.clone(),false);wait(&mut app,&ctx);
        app.quick_filter(0,"");wait(&mut app,&ctx);
        assert_eq!(app.panes[0].entries.len(),4001);
        app.apply_advanced_select(0,"*.txt;*.txt",true);
        assert_eq!(app.panes[0].selected.len(),2000);
        let marked: BTreeSet<_>=app.panes[0].selected.iter().map(|&i| app.panes[0].entries[i].name.clone()).collect();
        app.dispatch("sort:Size",&ctx);
        app.dispatch("sortDescending",&ctx);
        wait(&mut app,&ctx);
        assert_eq!(marked,app.panes[0].selected.iter().map(|&i|app.panes[0].entries[i].name.clone()).collect());
        app.quick_filter(0,"*.log");wait(&mut app,&ctx);
        assert_eq!(app.panes[0].entries.len(),2001);
        assert!(app.panes[0].entries.iter().all(|e|e.is_parent()||e.name.ends_with(".log")));
        assert_eq!(app.panes[0].view.cells.len(),2001);
        let raw=egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1400.0,850.0))),..Default::default()};
        app.settings.set_text("LeftViewStyle",["Auto","Details","Names","Thumbnails"][cycle%4]);
        let output=ctx.run(raw,|ctx|{egui::CentralPanel::default().show(ctx,|ui|{app.pane_ui(ui,0,ctx);});});
        let _=ctx.tessellate(output.shapes,output.pixels_per_point);
        app.navigate(0,small.clone(),false);wait(&mut app,&ctx);
        app.quick_filter(0,"");wait(&mut app,&ctx);
        let pane=&app.panes[0];
        assert_eq!(pane.entries.len(),2);
        assert_eq!(pane.view.cells.len(),2);
        assert!(pane.view.cells.capacity()<=1024);
        assert!(pane.view.colors.capacity()<=1024);
        assert!(pane.view.rows.capacity()<=1024);
        assert!(pane.error.is_none());
        if (cycle+1)%10==0 {eprintln!("BATCH78 cycles={} elapsed_s={:.3}",cycle+1,start.elapsed().as_secs_f64());}
    }
    assert_eq!(fs::read(large.join("file00000.txt")).unwrap(),[0]);
    assert_eq!(fs::read(small.join("only.txt")).unwrap(),b"x");
    assert_eq!(fs::read_dir(&large).unwrap().count(),4000);
    fs::remove_dir_all(root).unwrap();
}

/// Field widths are GDI widths (original Canvas.TextWidth); egui draws Latin text at most CELL_TEXT_SLACK wider, so it still fits its cell.
#[test]
fn field_widths_by_glyph_sum_match_the_laid_out_width() {
    let (root, ctx, app) = fixture("glyph-sum");
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let font = egui::FontId::new(app.list_font_size, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()));
            for text in ["report.txt", "AVAWAVAW To.pdf", "2026-10-31 13:52", "1,234,567", "WWWWAAAA.WWA", "NoteBurner YouTube Music Converter"] {
                let drawn = ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x);
                let measured = app.list_text_width(ui, text);
                assert!(measured + super::panes::CELL_TEXT_SLACK >= drawn, "{text}: measured {measured} vs drawn {drawn}");
                assert_eq!(Some(measured), app.gdi_list_widths(&[text]).map(|w| w[0]), "GDI width");
            }
        });
    });
    let _ = fs::remove_dir_all(root);
}

/// Original FUN_00483640: when a shortcut (.lnk) targets a folder, the pane moves to that folder instead of opening it in the shell.
#[test]
fn opening_a_folder_shortcut_moves_the_pane_into_the_target() {
    let (root, ctx, mut app) = fixture("folder-shortcut");
    let left = app.panes[0].path.clone();
    let target = root.join("target-folder");
    fs::create_dir_all(&target).unwrap();
    ndr::native::create_original_shortcut(&target, &left).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    let link = app.panes[0].entries.iter().position(|e| e.name.to_ascii_lowercase().ends_with(".lnk")).expect("the shortcut is listed");
    app.active = 0;
    app.panes[0].cursor = link;
    app.open_current();
    wait(&mut app, &ctx);
    // Windows ShellLink::GetPath may return an 8.3 short pathname on a fresh
    // runner (e.g. RUNNER~1) even though the shortcut opens the correct
    // directory. Compare the resolved filesystem targets, not spellings.
    let opened = fs::canonicalize(&app.panes[0].path).unwrap();
    let expected = fs::canonicalize(&target).unwrap();
    assert_eq!(opened.to_string_lossy().to_lowercase(), expected.to_string_lossy().to_lowercase());
    let _ = fs::remove_dir_all(root);
}


/// Original FUN_00489d20 (tab switch): if a tab's folder is gone, go to an existing parent; with no location at all, remove that tab.
#[test]
fn switching_to_a_tab_whose_folder_is_gone_follows_the_original() {
    let (root, ctx, mut app) = fixture("tab-gone");
    let left = app.panes[0].path.clone();
    let gone = left.join("folder").join("deep");
    fs::create_dir_all(&gone).unwrap();
    app.active = 0;
    app.navigate(0, gone.clone(), true);
    wait(&mut app, &ctx);
    app.dispatch("newTab", &ctx);
    wait(&mut app, &ctx);
    app.navigate(0, left.clone(), true);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].tab), (2, 1));
    fs::remove_dir_all(&gone).unwrap();
    app.tab_command(0, "tab:0", &ctx);
    wait(&mut app, &ctx);
    assert_eq!(app.panes[0].path, left.join("folder"), "the nearest existing parent of the vanished tab folder");
    let _ = fs::remove_dir_all(root);
}


/// User report: inside an archive ".." did not leave it and files inside could not be copied. ".." at the archive top rereads the folder containing
/// the archive and puts the cursor on the archive; files inside the archive are copied (extracted) to the other pane.
#[test]
fn leaving_an_archive_with_dotdot_relists_the_folder_and_inner_files_copy_out() {
    let (root, ctx, mut app) = fixture("archive-leave");
    let left = app.panes[0].path.clone();
    let right = app.panes[1].path.clone();
    let source = root.join("inside.txt");
    fs::write(&source, b"inner").unwrap();
    let zip = left.join("pack.zip");
    ndr::archive::create(&[source.clone()], &zip, None, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    app.refresh(0);
    wait(&mut app, &ctx);
    app.active = 0;
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "pack.zip").unwrap();
    app.open_current();
    wait(&mut app, &ctx);
    assert!(app.panes[0].archive.is_some());
    assert!(app.panes[0].entries.iter().any(|e| e.name == "inside.txt"), "{:?}", app.panes[0].entries.iter().map(|e| &e.name).collect::<Vec<_>>());
    // Copy (extract) the inner file to the other pane.
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "inside.txt").unwrap();
    app.copy_to_other(0, false, &ctx);
    let end = Instant::now() + Duration::from_secs(30);
    while !right.join("inside.txt").exists() {
        assert!(Instant::now() < end, "copy out of the archive timed out (status: {})", app.status);
        app.poll(&ctx);
        if !app.msgboxes.is_empty() {
            app.answer_message_box(6, &ctx);
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read(right.join("inside.txt")).unwrap(), b"inner");
    // ".." at the top: out of the archive, reread that folder and put the cursor on the archive.
    app.active = 0;
    app.panes[0].cursor = 0;
    assert!(app.panes[0].entries[0].is_parent());
    app.open_current();
    wait(&mut app, &ctx);
    assert!(app.panes[0].archive.is_none());
    assert_eq!(app.panes[0].path, left);
    assert_eq!(app.panes[0].entries.get(app.panes[0].cursor).map(|e| e.name.as_str()), Some("pack.zip"), "the cursor is on the archive we left");
    assert!(!app.panes[0].entries.iter().any(|e| e.name == "inside.txt"), "the archive listing is gone");
    let _ = fs::remove_dir_all(root);
}

/// Original pane tree drop target (TNXDropFileTarget of FUN_0048da38): hovering 1200 ms over a node with subfolders while dragging expands it
/// (timer +0x2f4 -> FUN_0048edf0), and a drop is FUN_00488b64(effect, node folder, 1 = skip confirmation), so "confirm drag and drop" is not asked.
#[test]
fn dropping_on_the_pane_tree_expands_on_hover_and_skips_the_confirmation() {
    let (root, ctx, mut app) = fixture("tree-drop");
    let (left, right) = (app.panes[0].path.clone(), app.panes[1].path.clone());
    assert!(app.settings.flag("ConfirmDragDrop", true), "the list drop would ask");
    let row = egui::Rect::from_min_size(egui::pos2(700.0, 100.0), egui::vec2(200.0, 18.0));
    app.pane_rects = [egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(650.0, 800.0)), egui::Rect::from_min_size(egui::pos2(650.0, 0.0), egui::vec2(650.0, 800.0))];
    app.tree_drop_rows = vec![(1, row, right.clone(), right.clone(), true)];
    app.file_drag = Some(super::drag_drop::FileDrag { paths: vec![left.join("a.txt")], secondary: false, source: 0, allowed: 7 });
    let start = Instant::now();
    assert!(!app.tree_drag_hover(Some(row.center()), start, &ctx), "not before the timer");
    assert!(!app.tree_drag_hover(Some(row.center()), start + Duration::from_millis(1100), &ctx));
    assert!(app.tree_drag_hover(Some(row.center()), start + Duration::from_millis(1250), &ctx), "expanded after 1200 ms");
    assert!(app.panes[1].tree_open.contains(&right));
    // Leaving the node clears the hover time (OnLeave / no node).
    assert!(!app.tree_drag_hover(Some(egui::pos2(10.0, 10.0)), start + Duration::from_millis(1300), &ctx));
    assert!(app.tree_drop_hover.is_none());
    // Drop: right away without a confirmation box (same drive, so a move by the original rule).
    app.file_drag = None;
    app.tree_drop(1, vec![left.join("a.txt")], right.clone(), egui::Modifiers::NONE, 7);
    assert!(app.msgboxes.is_empty(), "a tree drop does not ask ConfirmDragDrop");
    wait(&mut app, &ctx);
    let end = Instant::now() + Duration::from_secs(10);
    while !right.join("a.txt").exists() {
        assert!(Instant::now() < end, "the drop did not run (status: {})", app.status);
        app.poll(&ctx);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!left.join("a.txt").exists(), "same root: moved");
    let _ = fs::remove_dir_all(root);
}


/// Original miShowHiddenClick (0x4348e0) -> FUN_0048e564: changing the hidden view also shows or hides hidden folders in the pane folder tree
/// (+0x3f8 bit 4, the tree is rebuilt). The tree subfolder cache is kept per hidden view, so it does not mix with the other pane's setting.
#[test]
fn pane_tree_follows_the_hidden_toggle() {
    let (root, ctx, mut app) = fixture("tree-hidden");
    let left = app.panes[0].path.clone();
    fs::create_dir_all(left.join("visible-dir")).unwrap();
    fs::create_dir_all(left.join("hidden-dir")).unwrap();
    let status = std::process::Command::new("attrib").arg("+h").arg(left.join("hidden-dir")).status().unwrap();
    assert!(status.success());
    let draw = |app: &mut App| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 800.0));
                app.pane_tree_ui(ui, 0, rect);
            });
        });
    };
    let names = |app: &mut App, hidden: bool| -> Vec<String> {
        let end = Instant::now() + Duration::from_secs(20);
        // The test folder is below a hidden folder (AppData), so a tree with hidden items off does not connect from the Desktop. Request that folder's children directly.
        app.request_tree_children(0, &left);
        loop {
            draw(app);
            app.poll(&ctx);
            if let Some(children) = app.tree_cache.iter().find(|((key, h), _)| *h == hidden && ndr::shell_tree::same_key(key, &left)).map(|(_, c)| c.clone()) {
                return children.into_iter().map(|n| n.name).collect();
            }
            assert!(Instant::now() < end, "the tree did not read the folder (hidden {hidden})");
            thread::sleep(Duration::from_millis(20));
        }
    };
    app.panes[0].show_hidden = false;
    let shown = names(&mut app, false);
    assert!(shown.iter().any(|n| n == "visible-dir") && !shown.iter().any(|n| n == "hidden-dir"), "{shown:?}");
    app.active = 0;
    app.toggle_hidden();
    assert!(app.panes[0].show_hidden);
    let shown = names(&mut app, true);
    assert!(shown.iter().any(|n| n == "visible-dir") && shown.iter().any(|n| n == "hidden-dir"), "{shown:?}");
    // Original InitNode (0x7ed85c): the hidden folder is SFGAO_GHOSTED and is drawn with the dimmed icon (Cut); the visible one is not.
    let ghosted: Vec<(String, bool)> = app.tree_cache.iter().find(|((key, h), _)| *h && ndr::shell_tree::same_key(key, &left)).map(|(_, c)| c.iter().map(|n| (n.name.clone(), n.ghosted)).collect()).unwrap_or_default();
    assert!(ghosted.contains(&("hidden-dir".to_owned(), true)) && ghosted.contains(&("visible-dir".to_owned(), false)), "{ghosted:?}");
    // When the right pane (hidden off) reads the same folder in its tree, it does not use the left pane's (hidden on) result.
    assert!(!app.panes[1].show_hidden);
    let end = Instant::now() + Duration::from_secs(20);
    app.request_tree_children(1, &left);
    let right_side = loop {
        app.poll(&ctx);
        if let Some(children) = app.tree_cache.get(&(left.clone(), false)) {
            break children.iter().map(|n| n.name.clone()).collect::<Vec<_>>();
        }
        assert!(Instant::now() < end, "the right pane tree did not read the folder");
        thread::sleep(Duration::from_millis(20));
    };
    assert!(right_side.iter().any(|n| n == "visible-dir") && !right_side.iter().any(|n| n == "hidden-dir"), "{right_side:?}");
    let _ = std::process::Command::new("attrib").arg("-h").arg(left.join("hidden-dir")).status();
    let _ = fs::remove_dir_all(root);
}

/// Original mipOpenInNewTabClick (0x4453b4): folder -> a tab for that folder, ".." -> a tab for the parent, file -> a tab with that file path (FUN_00489d20 goes to the folder and
/// puts the cursor on the file); the new tab is appended and becomes current. Nothing happens when tabs are off.
#[test]
fn open_in_new_tab_follows_the_original() {
    let (root, ctx, mut app) = fixture("open-new-tab");
    let left = app.panes[0].path.clone();
    app.active = 0;
    let row = |app: &App, name: &str| app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    // Folder.
    app.panes[0].cursor = row(&app, "folder");
    app.dispatch("openInNewTab", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].tab), (2, 1));
    assert_eq!(app.panes[0].path, left.join("folder"));
    // "..": a tab for the parent (= left).
    app.panes[0].cursor = 0;
    assert!(app.panes[0].entries[0].is_parent());
    app.dispatch("openInNewTab", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].tab), (3, 2));
    assert_eq!(app.panes[0].path, left);
    // File: not run; the cursor goes to that file in a tab for its folder.
    app.panes[0].cursor = row(&app, "b.txt");
    app.dispatch("openInNewTab", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].tab), (4, 3));
    assert_eq!(app.panes[0].path, left);
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "b.txt");
    // Tabs off: nothing happens (not even opening in the current tab).
    app.settings.set_flag("UseTabs", false);
    app.panes[0].cursor = row(&app, "folder");
    app.dispatch("openInNewTab", &ctx);
    wait(&mut app, &ctx);
    assert_eq!((app.panes[0].tabs.len(), app.panes[0].path.clone()), (4, left.clone()));
    let _ = fs::remove_dir_all(root);
}


/// Original FUN_00442bc8 -> FUN_00442d80: when a drive appears or disappears, local panes that show the drive rows are rebuilt (the drive cache is cleared too).
/// Archive and FTP panes are left alone, and the quick search text is kept.
#[test]
fn drive_changes_relist_local_panes_with_drive_rows() {
    let (root, ctx, mut app) = fixture("drive-change");
    let left = app.panes[0].path.clone();
    let source = root.join("packed.txt");
    fs::write(&source, b"x").unwrap();
    let zip = left.join("pack.zip");
    ndr::archive::create(&[source], &zip, None, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    app.panes[1].archive = Some(ArchiveLocation { file: zip.clone(), prefix: String::new(), password: String::new() });
    app.refresh(1);
    wait(&mut app, &ctx);
    app.settings.set_flag("ShowDriveList", true);
    let (left_before, right_before) = (app.panes[0].generation, app.panes[1].generation);
    app.drive_cache = Some((Instant::now(), Vec::new()));
    app.typed_search[0].query = "ab".into();
    app.drives_changed();
    assert_eq!(app.panes[0].generation, left_before + 1, "the local pane with drive rows is listed again");
    assert_eq!(app.panes[1].generation, right_before, "the archive pane is left alone");
    assert!(app.drive_cache.is_none(), "drive buttons are read again");
    assert_eq!(app.typed_search[0].query, "ab", "the typed quick search text is kept");
    wait(&mut app, &ctx);
    // With drive rows off, a local folder pane is not reread (the My PC view is).
    app.settings.set_flag("ShowDriveList", false);
    let before = app.panes[0].generation;
    app.drives_changed();
    assert_eq!(app.panes[0].generation, before);
    let _ = fs::remove_dir_all(root);
}


/// Original TNXDropTarget (0x7b21a0 DragOver, 0x7b1e04 DoTargetScroll, 100 ms timer, margin 20): while dragging inside the bottom 15 px band of a list
/// it scrolls one step every 100 ms (not in the middle).
#[test]
fn dragging_near_the_list_edge_scrolls_like_the_drop_target() {
    let (root, ctx, mut app) = fixture("drag-scroll");
    let left = app.panes[0].path.clone();
    for i in 0..300 {
        fs::write(left.join(format!("f{i:03}.txt")), b"x").unwrap();
    }
    app.settings.set_text("LeftViewStyle", "Details");
    app.settings.values.insert("LeftScrollStep".into(), serde_json::json!(1));
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
    let rect = app.panes[0].list_rect.expect("the list area is recorded");
    let start = Instant::now();
    let middle = rect.center();
    assert!(!app.drag_edge_scroll(Some(middle), start, &ctx), "no scrolling in the middle");
    let bottom = egui::pos2(rect.center().x, rect.bottom() - 5.0);
    assert!(!app.drag_edge_scroll(Some(bottom), start, &ctx), "the timer starts first");
    assert!(!app.drag_edge_scroll(Some(bottom), start + Duration::from_millis(50), &ctx));
    assert!(app.drag_edge_scroll(Some(bottom), start + Duration::from_millis(100), &ctx), "one line after 100 ms");
    assert_eq!(app.panes[0].drag_scroll, (1, 0));
    let first_before = app.panes[0].page.top;
    run(&mut app);
    run(&mut app);
    assert_eq!(app.panes[0].drag_scroll, (0, 0), "the request is used");
    assert_eq!(app.panes[0].page.top, first_before + 1, "the list moved down one line");
    // Top edge (same pane, the timer keeps running): one step up.
    let top = egui::pos2(rect.center().x, rect.top() + 3.0);
    assert!(app.drag_edge_scroll(Some(top), start + Duration::from_millis(300), &ctx));
    assert_eq!(app.panes[0].drag_scroll, (-1, 0));
    // Bottom edge of the pane folder tree: scrolls the tree one row (the original tree effect 0x48e790 uses the same band).
    app.panes[0].drag_scroll = (0, 0);
    let tree = egui::Rect::from_min_size(rect.min, egui::vec2(150.0, 400.0));
    app.panes[0].tree_rect = Some(tree);
    let t = start + Duration::from_millis(1000);
    let near = egui::pos2(tree.center().x, tree.bottom() - 4.0);
    assert!(!app.drag_edge_scroll(Some(near), t, &ctx), "a new place starts the timer again");
    assert!(app.drag_edge_scroll(Some(near), t + Duration::from_millis(100), &ctx));
    assert_eq!((app.panes[0].tree_drag_scroll, app.panes[0].drag_scroll), (1, (0, 0)), "the tree scrolls, not the list");
    let _ = fs::remove_dir_all(root);
}

/// TNXCustomShellTreeView.Edit (0x7edf70): clicking the selected folder label again starts an in-place rename after the
/// double-click time; committing renames the folder and moves panes that showed it.
#[test]
fn pane_tree_label_edit_renames_the_folder() {
    let (root, ctx, mut app) = fixture("tree-rename");
    let left = root.join("left");
    let folder = left.join("folder");
    fs::create_dir_all(folder.join("inner")).unwrap();
    let node = ndr::shell_tree::ShellNode { key: folder.clone(), name: "folder".into(), path: Some(folder.clone()), has_children: true, special: ndr::shell_tree::Special::None, ghosted: false };
    assert!(super::views::tree_renamable(&node));
    assert!(!super::views::tree_renamable(&ndr::shell_tree::ShellNode { key: PathBuf::from("C:\\"), name: "C:".into(), path: Some(PathBuf::from("C:\\")), has_children: true, special: ndr::shell_tree::Special::None, ghosted: false }));
    app.panes[0].tree_rows = vec![(node, 1, true, false)].into();
    app.panes[0].tree_selected = Some(folder.clone());
    let start = Instant::now();
    app.panes[0].tree_label_click = Some((folder.clone(), start));
    assert!(app.tree_edit_due(0, start).is_some(), "waits for the double-click time");
    assert!(app.panes[0].tree_edit.is_none());
    assert!(app.tree_edit_due(0, start + Duration::from_secs(5)).is_none());
    assert_eq!(app.panes[0].tree_edit, Some((folder.clone(), "folder".into())));
    // A pane inside the renamed folder follows it.
    app.navigate(1, folder.join("inner"), false);
    wait(&mut app, &ctx);
    app.panes[0].tree_edit = Some((folder.clone(), "renamed".into()));
    app.tree_rename_commit(0);
    wait(&mut app, &ctx);
    assert!(left.join("renamed").join("inner").is_dir() && !folder.exists());
    assert!(app.panes[1].path.ends_with("renamed\\inner"), "{}", app.panes[1].path.display());
    // Choosing another node cancels a pending edit.
    app.panes[0].tree_label_click = Some((folder.clone(), start));
    app.panes[0].tree_selected = Some(left.clone());
    assert!(app.tree_edit_due(0, start + Duration::from_secs(5)).is_none() && app.panes[0].tree_edit.is_none());
    let _ = fs::remove_dir_all(root);
}


#[test]
fn cjk_fallback_ink_centers_match_at_multiple_sizes_and_dpi() {
    let windows = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| "C:/Windows".into());
    let Some(reference_bytes) = native::mapped_file(&windows.join("Fonts/malgun.ttf")) else {
        println!("SKIP missing Malgun reference");
        return;
    };
    let reference = super::cjk_ink_center(reference_bytes, 0).unwrap();
    let latin = native::mapped_file(&windows.join("Fonts/segoeui.ttf")).map(|b| (b, 0));
    // Test the same outlines the real renderer uses, not Windows font
    // metadata alone. These shared full-size characters isolate alignment
    // from the intentional placement of punctuation and small kana.
    let sample = "\u{4e2d}\u{6587}\u{65e5}\u{672c}\u{76ee}\u{5b57}\u{4ef6}\u{540d}\u{8a00}\u{8a9e}\u{6848}\u{8cc7}";
    let mut checked = 0;
    for (locale, file) in [("ja", "YuGothM.ttc"), ("zh-CN", "msyh.ttc"), ("zh-TW", "msjh.ttc")] {
        let Some(bytes) = native::mapped_file(&windows.join("Fonts").join(file)) else {
            println!("SKIP missing {file}");
            continue;
        };
        let uncorrected = super::cjk_ink_center(bytes, 0).unwrap();
        let tweak = super::cjk_vertical_tweak(bytes, reference);
        assert!((uncorrected + tweak.y_offset_factor - reference).abs() < 0.015,
            "{locale}: metric normalization outside safety limit");
        println!("CJK {locale} raw={uncorrected:.4} reference={reference:.4} offset={:.4}", tweak.y_offset_factor);
        use ab_glyph::{Font, FontRef};
        let face = FontRef::try_from_slice_and_index(bytes, 0).unwrap();
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let ctx = egui::Context::default();
            super::set_app_fonts(&ctx, latin, false, 1.0, locale);
            ctx.set_pixels_per_point(scale);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()),
                    egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())] {
                    for size in [12.0, 14.0, 20.0] {
                        let font = egui::FontId::new(size, family.clone());
                        ctx.fonts_mut(|fonts| {
                            let g = fonts.layout_no_wrap(sample.to_owned(), font.clone(), Color32::WHITE);
                            let row = &g.rows[0];
                            let mut centers = Vec::new();
                            for glyph in &row.glyphs {
                                if face.glyph_id(glyph.chr).0 == 0 { continue; }
                                assert!(fonts.has_glyph(&font, glyph.chr), "{locale} missing {}", glyph.chr);
                                let target_ascent = size * face.ascent_unscaled() / face.units_per_em().unwrap();
                                assert!((glyph.font_impl_ascent-target_ascent).abs() < 0.8, "{locale} wrong fallback U+{:04X} {family:?} size={size} dpi={scale}: actual={} expected={target_ascent} impl_height={} font_height={}", glyph.chr as u32, glyph.font_impl_ascent,glyph.font_impl_height,glyph.font_height);
                                let center = glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y * 0.5 - row.height() * 0.5;
                                centers.push(center);
                                let top = glyph.pos.y + glyph.uv_rect.offset.y;
                                let bottom = top + glyph.uv_rect.size.y;
                                assert!(top >= -1.1 && bottom <= row.height()+1.1, "{locale} {size} {scale} {} outside row {top}..{bottom}, height {}", glyph.chr,row.height());
                            }
                            assert!(centers.len() >= 8);
                            centers.sort_by(f32::total_cmp);
                            let mid = centers[centers.len()/2];
                            assert!((mid-reference*size).abs() <= 1.5, "{locale} {family:?} size={size} dpi={scale} ink={mid}, expected={}", reference*size);
                            checked += centers.len();
                        });
                    }
                }
            });
        }
    }
    println!("CJK glyph positions checked: {checked}");
}

#[test]
fn outgoing_drag_reset_retires_only_initiating_button_before_new_input() {
    let (root, ctx, mut app) = fixture("native-drag-reset");
    let start = egui::pos2(50.0, 50.0);
    let primary = egui::PointerButton::Primary;
    let secondary = egui::PointerButton::Secondary;
    let button_event = |button, pressed| egui::Event::PointerButton { pos: start, button, pressed, modifiers: Default::default() };
    let _ = ctx.run(egui::RawInput { events:vec![egui::Event::PointerMoved(start), button_event(primary,true), button_event(secondary,true)], ..Default::default() }, |_| {});
    app.native_drag_reset = Some(primary);
    let mut raw = egui::RawInput { events: vec![egui::Event::PointerMoved(start)], ..Default::default() };
    eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);
    assert!(matches!(raw.events.first(), Some(egui::Event::PointerButton{button: egui::PointerButton::Primary,pressed:false,..})));
    assert!(matches!(raw.events.get(1), Some(egui::Event::PointerGone)));
    assert!(app.native_drag_reset.is_none());
    let _ = ctx.run(raw, |ctx| assert!(!ctx.input(|i| i.pointer.primary_down())));
    assert!(ctx.input(|i| i.pointer.secondary_down()));
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn default_theme_list_colors_match_nexus_and_old_cyan_profile_is_normalized() {
    let (root, _ctx, mut app) = fixture("nexus-default-palette");
    app.settings.set_flag("UseNexusFileColorMap", false);
    app.apply_theme("GitHub");
    let a = app.panes[0].view.colors.clone();
    let colors = ["NXFORE","NXFOLDER","NXDRIVE","NXREADONLY","NXHIDDEN","NXDESC","NXMB","NXGB","NXSIZE","NXTIME","NXATTR","NXSELECT"];
    let palette: Vec<_> = colors.iter().map(|c| app.basic_list_color(c)).collect();
    app.apply_theme("Nexus");
    assert_eq!(a, app.panes[0].view.colors);
    assert_eq!(palette,colors.iter().map(|c|app.basic_list_color(c)).collect::<Vec<_>>());
    app.settings.set_text("Theme","Cyan");
    app.settings.normalize();
    assert_eq!(app.settings.text("Theme",""), ndr::styles::DEFAULT_THEME);
    assert_eq!(app.skin_names(), ["GitHub","Nexus","Classic"]);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_folder_display_never_reverts_or_publishes_partial_totals() {
    let (root, ctx, mut app) = fixture("stable-folder-label");
    let path = root.join("left/folder");
    app.settings.set_flag("LeftAutoFolderSize", true);
    app.folder_size_workers = true; // Deterministic asynchronous result injection.
    let mut trace = FolderLabelTrace::default();
    let schedule = |app: &mut App| {
        let p = &mut app.panes[0];
        let token = Arc::new(AtomicBool::new(false));
        p.folder_auto_paths.insert(path.clone());
        p.folder_auto_visible.insert(path.clone());
        p.folder_sizes.entry(path.clone()).or_insert(None);
        p.folder_auto_pending.insert(path.clone(), token.clone());
        token
    };
    let publish = |app: &mut App, cancel: Arc<AtomicBool>, bytes, _text: &str, complete, stable| {
        app.async_tx.send(AsyncResult::FolderSizeRow {
            pane:0, path:path.clone(), bytes,
            automatic:true, complete, stable, watch:None, scanned:true,
            work:ndr::folder_cache::ScanWork::default(), audit_at:Instant::now(),
            media:native::FolderSizeMedia::Fast, cancel,
        }).unwrap();
        app.poll(&ctx);
    };
    let assert_display = |app: &App, expected: Option<&str>| {
        assert_eq!(app.panes[0].folder_size_display(&path).as_deref(), expected);
    };

    // A cold folder with inaccessible descendants must not publish a
    // partial count or even mark partial bytes as a completed total.
    let token = schedule(&mut app);
    publish(&mut app, token, 1024, "\u{2248}1.00 KB", false, true);
    assert_display(&app, None);
    assert!(!app.panes[0].folder_bytes.contains_key(&path));
    assert!(app.panes[0].folder_auto_times[&path] >= Instant::now()+Duration::from_secs(10));
    trace.sample(&app.panes[0],Duration::ZERO);

    // A previous-session verified snapshot has the same label as a new
    // successful measurement, but is never counted as fresh folder_bytes.
    let token = schedule(&mut app);
    app.async_tx.send(AsyncResult::FolderSizeHint {
        pane:0, path:path.clone(), bytes:3072, cancel:token.clone(),
    }).unwrap();
    app.poll(&ctx);
    assert_display(&app, Some("3.00 KB"));
    assert!(!app.panes[0].folder_bytes.contains_key(&path));
    trace.sample(&app.panes[0],Duration::ZERO);
    publish(&mut app, token, 2048, "\u{2248}2.00 KB", false, true);
    assert_display(&app, Some("3.00 KB"));
    assert!(!app.panes[0].folder_bytes.contains_key(&path));
    trace.sample(&app.panes[0],Duration::ZERO);

    let token = schedule(&mut app);
    publish(&mut app, token, 3072, "3.00 KB", true, true);
    assert_eq!(app.panes[0].folder_bytes.get(&path), Some(&3072));
    trace.sample(&app.panes[0],Duration::ZERO);
    // Repeated incomplete, unstable and cancelled results cannot
    // change a known size even for one sampled frame.
    for _ in 0..12 {
        let token = schedule(&mut app);
        trace.sample(&app.panes[0],Duration::ZERO); // pending
        publish(&mut app, token, 1024, "\u{2248}1.00 KB", false, true);
        assert_display(&app, Some("3.00 KB"));
        trace.sample(&app.panes[0],Duration::ZERO);
        let token = schedule(&mut app);
        publish(&mut app, token, 4096, "4.00 KB", true, false);
        assert_display(&app, Some("3.00 KB"));
        trace.sample(&app.panes[0],Duration::ZERO);
        let token = schedule(&mut app);
        token.store(true, Ordering::Relaxed);
        publish(&mut app, token, 8192, "8.00 KB", true, true);
        assert_display(&app, Some("3.00 KB"));
        trace.sample(&app.panes[0],Duration::ZERO);
        app.panes[0].cancel_automatic_folder_jobs();
        app.panes[0].folder_auto_visible.insert(path.clone());
        trace.sample(&app.panes[0],Duration::ZERO);
    }
    assert_eq!(app.panes[0].folder_bytes.get(&path), Some(&3072));

    // A temporary read failure leaves the original listing visible.
    // Previously this path cleared the old folder_sizes map entirely.
    let generation = app.panes[0].generation;
    app.apply_listing(Listing {pane:0,generation,result:Err("temporary listing failure".into()),
        prepared:None,cache_revision:None,hardlinks:false,partial:None,timing_us:[0;6],skipped_device_names:0,descriptions:HashMap::new(),drive_infos:HashMap::new()},&mut Vec::new());
    assert_display(&app, Some("3.00 KB"));
    app.panes[0].folder_auto_visible.insert(path.clone());
    trace.sample(&app.panes[0],Duration::ZERO);
    app.refresh(0);
    assert_display(&app, Some("3.00 KB"));
    wait(&mut app,&ctx);
    assert_display(&app, Some("3.00 KB"));
    app.panes[0].folder_auto_visible.insert(path.clone());
    trace.sample(&app.panes[0],Duration::ZERO);

    // The sole visible change is a newly verified different total.
    let token = schedule(&mut app);
    publish(&mut app, token, 6144, "6.00 KB", true, true);
    assert_display(&app, Some("6.00 KB"));
    assert_eq!(app.panes[0].folder_bytes.get(&path), Some(&6144));
    trace.sample(&app.panes[0],Duration::ZERO);
    assert_eq!((trace.first_values, trace.markers_after_value, trace.approximate_samples, trace.value_changes),(1,0,0,1));
    assert!(trace.samples >= 50);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_folder_cache_eviction_never_drops_later_visible_rows() {
    let (root,ctx,mut app) = fixture("stable-folder-eviction");
    let now=Instant::now();
    let first=root.join("left/folder");
    let late=root.join("left/later-visible");
    {
        let pane=&mut app.panes[0];
        for n in 0..514 {
            let path=match n {0=>late.clone(),513=>first.clone(),_=>root.join(format!("left/folder-{n:03}"))};
            pane.folder_auto_paths.insert(path.clone());
            pane.folder_bytes.insert(path.clone(),1024);
            pane.folder_sizes.insert(path.clone(),Some("1.00 KB".into()));
            pane.folder_auto_times.insert(path,now+Duration::from_millis(n));
        }
        pane.folder_auto_visible.insert(first.clone());
    }
    // Simulate the first painted row hitting its cache before the later
    // visible row has been encountered by the renderer.
    app.record_folder_size_time(0,&first,now+Duration::from_secs(2));
    assert_eq!(app.panes[0].folder_size_display(&late).as_deref(),Some("1.00 KB"));
    app.panes[0].folder_auto_visible.insert(late.clone());
    app.finish_visible_folder_size_frame(&ctx);
    assert_eq!(app.panes[0].folder_auto_paths.len(),512);
    assert_eq!(app.panes[0].folder_sizes.len(),512);
    assert_eq!(app.panes[0].folder_size_display(&late).as_deref(),Some("1.00 KB"));
    assert_eq!(app.panes[0].folder_size_display(&first).as_deref(),Some("1.00 KB"));
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_folder_replays_clean_cached_value_while_io_busy() {
    let (root,ctx,mut app)=fixture("stable-folder-busy-cache");
    let folder=root.join("left/folder");
    app.settings.set_flag("LeftAutoFolderSize",true);
    let row=app.panes[0].entries.iter().position(|e|e.path==folder).unwrap();
    app.cache_folder_size(folder.clone(), 4096, native::FolderSizeWatch::open(&folder).map(Arc::new),
        Instant::now(),native::FolderSizeMedia::Fast);
    // Exercise enqueue's foreground guard directly through real app job state.
    let cancel=Arc::new(AtomicBool::new(false));
    let (_tx,rx)=mpsc::channel();
    app.job=Some(Job::new(rx,cancel,"fixture".into()));
    app.queue_folder_size(0,row);
    assert!(app.folder_auto_busy);
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(),Some("4.00 KB"));
    assert_eq!(app.panes[0].folder_auto_pending.len(),0);
    assert!(!app.folder_size_workers);
    app.job=None;
    app.poll(&ctx);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bounded_thumbnail_queue_reuses_keys_and_evicts_only_old_entries() {
    use super::views::ThumbnailKey;
    let (root,ctx,mut app) = fixture("bounded-thumbnails");
    let mut entry=app.panes[0].entries.iter().find(|e|e.name=="a.txt").unwrap().clone();
    let (tx,rx)=mpsc::sync_channel(16);
    app.thumbnail_tx=Some(tx);
    let first=ThumbnailKey::new(&entry,120);
    assert!(app.thumbnail_for(&ctx,&entry,120).is_none());
    for _ in 0..30 { assert!(app.thumbnail_for(&ctx,&entry,120).is_none()); }
    assert_eq!(rx.try_iter().count(),1);
    for i in 1..40 {
        entry.path=root.join(format!("image-{i}.png"));
        app.thumbnail_for(&ctx,&entry,120);
    }
    assert_eq!(app.image_requests.len(),16);
    assert_eq!(rx.try_iter().count(),15);
    let token=app.image_requests[&first].clone();
    app.finish_thumbnail(&ctx,first.clone(),token,Ok((16,16,vec![255;16*16*4])));
    assert!(app.images.contains_key(&first));
    assert_eq!(app.image_bytes,16*16*4);
    // A result with no matching pending job must be ignored.
    let unknown=ThumbnailKey::new(&entry,256);
    app.finish_thumbnail(&ctx,unknown.clone(),Arc::new(AtomicBool::new(false)),Ok((16,16,vec![255;16*16*4])));
    assert!(!app.images.contains_key(&unknown));
    app.image_requests.clear();
    // 520 lightweight positive results plus the first entry. The old
    // implementation dropped the entire cache at the 512nd insertion.
    for i in 0..520 {
        entry.path=root.join(format!("cached-{i}.png"));
        let key=ThumbnailKey::new(&entry,120);
        let token=Arc::new(AtomicBool::new(false));app.image_requests.insert(key.clone(),token.clone());
        app.finish_thumbnail(&ctx,key,token,Ok((16,16,vec![255;16*16*4])));
    }
    assert_eq!(app.images.len(),512);
    assert_eq!(app.image_bytes,512*16*16*4);
    let current=ThumbnailKey::new(&entry,120);
    assert!(app.images.contains_key(&current));
    assert!(!app.images.contains_key(&first));
    let changed=ThumbnailKey::new(&entry,256);
    assert_ne!(current,changed);
    entry.size+=1;
    assert_ne!(current,ThumbnailKey::new(&entry,120));
    // Pixel budget independently bounds oversized provider responses.
    for i in 0..10 {
        entry.path=root.join(format!("large-{i}.png"));
        let key=ThumbnailKey::new(&entry,256);
        let token=Arc::new(AtomicBool::new(false));app.image_requests.insert(key.clone(),token.clone());
        app.finish_thumbnail(&ctx,key,token,Ok((1024,1024,vec![255;1024*1024*4])));
    }
    assert!(app.image_bytes<=32*1024*1024);
    assert!(app.images.len()<=512);
    assert!(app.image_requests.is_empty());
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn thumbnail_scroll_cancels_old_queue_and_rejects_late_same_key_result(){
    use super::views::ThumbnailKey;
    let (root,ctx,mut app)=fixture("thumbnail-cancel-scroll");
    let entry=app.panes[0].entries.iter().find(|e|e.name=="a.txt").unwrap().clone();
    let (tx,rx)=mpsc::sync_channel(16);app.thumbnail_tx=Some(tx);
    let key=ThumbnailKey::new(&entry,120);
    app.thumbnail_for(&ctx,&entry,120);
    let old=rx.try_recv().unwrap();
    assert_eq!(old.key,key);
    app.thumbnail_visible.clear();app.retire_offscreen_thumbnails();
    assert!(old.cancel.load(Ordering::Relaxed));assert!(app.image_requests.is_empty());
    app.thumbnail_for(&ctx,&entry,120);
    let new=rx.try_recv().unwrap();assert_eq!(new.key,key);
    app.finish_thumbnail(&ctx,key.clone(),old.cancel,Ok((16,16,vec![255;1024])));
    assert!(Arc::ptr_eq(&app.image_requests[&key],&new.cancel));
    assert!(!app.images.contains_key(&key));
    app.finish_thumbnail(&ctx,key.clone(),new.cancel,Ok((16,16,vec![255;1024])));
    assert!(app.images.contains_key(&key));
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlay_cache_evicts_one_old_entry_without_resetting_all_others() {
    let (root,_ctx,mut app)=fixture("overlay-lru");
    let now=Instant::now();
    for i in 0..512 {
        let path=root.join(format!("icon-{i}"));
        app.overlay_icons.insert(path.clone(),None);
        app.overlay_icon_failed_at.insert(path.clone(),now);
        app.overlay_last_used.insert(path,now+Duration::from_millis(i));
    }
    let incoming=root.join("new-icon");
    app.trim_overlay_icons(&incoming);
    assert_eq!(app.overlay_icons.len(),511);
    assert!(!app.overlay_icons.contains_key(&root.join("icon-0")));
    assert!(app.overlay_icons.contains_key(&root.join("icon-511")));
    assert_eq!(app.overlay_icon_failed_at.len(),511);
    assert_eq!(app.overlay_last_used.len(),511);
    app.trim_overlay_icons(&root.join("icon-511"));
    assert_eq!(app.overlay_icons.len(),511);
    drop(app);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn local_listing_lane_discards_superseded_pending_work() {
    let mut lane=super::panes::ListingLane::default();
    let (started_tx,started_rx)=mpsc::channel();
    let (release_tx,release_rx)=mpsc::channel();
    let (done_tx,done_rx)=mpsc::channel();
    lane.submit(Box::new(move||{
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    }));
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    for i in 0..64 {
        let tx=done_tx.clone();
        lane.submit(Box::new(move||{tx.send(i).unwrap();}));
    }
    release_tx.send(()).unwrap();
    assert_eq!(done_rx.recv_timeout(Duration::from_secs(2)).unwrap(),63);
    assert!(done_rx.recv_timeout(Duration::from_millis(80)).is_err());
}

#[test]
fn metadata_partial_matches_full_layout_and_preserves_marks(){
    let (root,ctx,mut app)=fixture("partial-layout");
    let left=root.join("left");
    let file=left.join("partial-test.bin");
    fs::write(&file,vec![1;100]).unwrap();
    // Same-file length changes keep the name sort stable.
    for i in 0..4096{fs::write(left.join(format!("partial-{i:04}.bin")),vec![2;100]).unwrap();}
    app.settings.set_flag("ShowDriveList",false);
    app.settings.set_text("LeftSortMode","Name");
    app.settings.set_text("LeftGroupMode","None");
    app.refresh(0);wait(&mut app,&ctx);
    let plan=panes::LocalListingPlan::capture(&app,0);
    assert!(plan.supports_partial());
    let row=app.panes[0].entries.iter().position(|e|e.path==file).unwrap();
    app.panes[0].cursor=row;
    app.panes[0].selected.insert(row);
    let cells_ptr=app.panes[0].view.cells.as_ptr();
    let entries_ptr=app.panes[0].entries.as_ptr();
    let revision=app.panes[0].thumb_content_revision;
    let old_time=fs::metadata(&file).unwrap().modified().unwrap();
    let old_bytes=app.panes[0].summary.as_ref().unwrap().bytes;
    app.panes[0].local_snapshot=Some(panes::DisplayedSnapshot{
        revision:1,content_revision:revision,plan:plan.clone(),day:chrono::Local::now().date_naive()});
    fs::write(&file,vec![3;101]).unwrap();
    filetime::set_file_mtime(&file,filetime::FileTime::from_system_time(old_time)).unwrap();
    let changed=fsops::list_dir(&left,false).unwrap().into_iter().filter(|e|e.path==file).collect::<Vec<_>>();
    assert_eq!(changed.len(),1);
    let descriptions=app.panes[0].descriptions.clone();
    let drive_infos=app.panes[0].drive_infos.clone();
    let partial=plan.prepare_partial(&left,&changed,&descriptions,&drive_infos,&AtomicBool::new(false),1).unwrap();
    let generation=app.panes[0].generation;
    let listing=Listing{pane:0,generation,result:Ok(changed),prepared:None,cache_revision:Some(2),hardlinks:false,
        partial:Some(partial),timing_us:[0;6],skipped_device_names:0,descriptions,drive_infos};
    assert!(app.apply_partial_listing_inner(listing));
    assert_eq!(app.panes[0].entries.as_ptr(),entries_ptr);
    assert_eq!(app.panes[0].view.cells.as_ptr(),cells_ptr);
    assert_eq!(app.panes[0].entries[row].size,101);
    assert_eq!(app.panes[0].summary.as_ref().unwrap().bytes,old_bytes+1);
    assert_eq!(app.panes[0].cursor,row);
    assert!(app.panes[0].selected.contains(&row));
    let cells=app.panes[0].view.cells.clone();
    let field=app.panes[0].prepared_field_max.as_ref().unwrap().2;
    app.refresh(0);wait(&mut app,&ctx);
    assert_eq!(app.panes[0].view.cells,cells);
    assert_eq!(app.panes[0].prepared_field_max.as_ref().unwrap().2,field);
    drop(app);fs::remove_dir_all(root).unwrap();
}


#[test]
fn structural_partial_insert_rename_remove_matches_full_view_and_selection(){
    let (root,ctx,mut app)=fixture("structural-partial");
    let left=root.join("left");
    for i in 0..4096{fs::write(left.join(format!("ordinary-{i:05}.txt")),vec![1;100]).unwrap();}
    fs::write(left.join("very-long-name-that-keeps-the-column-maximum.txt"),vec![1;10000]).unwrap();
    app.settings.set_flag("ShowDriveList",false);
    app.settings.set_text("LeftSortMode","Name");
    app.settings.set_text("LeftGroupMode","None");
    app.settings.set_text("SizeUnit","B");
    app.refresh(0);wait(&mut app,&ctx);
    let keep=left.join("ordinary-02000.txt");
    let row=app.panes[0].entries.iter().position(|e|e.path==keep).unwrap();
    app.panes[0].cursor=row;app.panes[0].selected.insert(row);
    let original_len=app.panes[0].entries.len();
    let mut revision=1;
    for (new,removed) in [(Some("aa.txt"),None),(Some("ab.txt"),Some("aa.txt")),(None,Some("ab.txt"))]{
        let name_cell=app.panes[0].view.cells[app.panes[0].cursor][0].clone();
        match (new,removed){
            (Some(n),None)=>fs::write(left.join(n),b"small").unwrap(),
            (Some(n),Some(o))=>fs::rename(left.join(o),left.join(n)).unwrap(),
            (None,Some(o))=>fs::remove_file(left.join(o)).unwrap(),
            _=>unreachable!(),
        }
        let plan=panes::LocalListingPlan::capture(&app,0);
        app.panes[0].local_snapshot=Some(panes::DisplayedSnapshot{revision,
            content_revision:app.panes[0].thumb_content_revision,plan:plan.clone(),day:chrono::Local::now().date_naive()});
        let changed:Vec<_>=fsops::list_dir(&left,false).unwrap().into_iter().filter(|e|new==Some(e.name.as_str())).collect();
        let descriptions=app.panes[0].descriptions.clone();let drive_infos=app.panes[0].drive_infos.clone();
        let mut partial=plan.prepare_partial(&left,&changed,&descriptions,&drive_infos,&AtomicBool::new(false),revision).unwrap();
        partial.structural=true;partial.removed=removed.into_iter().map(std::ffi::OsString::from).collect();
        revision+=1;
        let listing=Listing{pane:0,generation:app.panes[0].generation,result:Ok(changed),prepared:None,
            cache_revision:Some(revision),hardlinks:false,partial:Some(partial),timing_us:[0;6],
            skipped_device_names:0,descriptions,drive_infos};
        assert!(app.apply_partial_listing_inner(listing),"structural edit {new:?}/{removed:?}");
        let pane=&app.panes[0];let row=pane.entries.iter().position(|e|e.path==keep).unwrap();
        assert_eq!(pane.cursor,row);assert!(pane.selected.contains(&row));assert!(Arc::ptr_eq(&name_cell,&pane.view.cells[row][0]));
        assert_eq!(pane.entries.len(),original_len+usize::from(new.is_some()));
        assert_eq!(pane.view.cells.len(),pane.entries.len());
        if let Some(n)=new{assert!(pane.entries.iter().any(|e|e.name==n));}
        if let Some(n)=removed{assert!(!pane.entries.iter().any(|e|e.name==n));}
        let cells=pane.view.cells.clone();
        let field=pane.prepared_field_max.as_ref().unwrap().2;
        let order:Vec<_>=pane.entries.iter().map(|e|(e.path.clone(),e.size,e.modified,e.attributes)).collect();
        let bytes=pane.summary.as_ref().unwrap().bytes;
        app.refresh(0);wait(&mut app,&ctx);
        assert_eq!(app.panes[0].view.cells,cells);
        assert_eq!(app.panes[0].prepared_field_max.as_ref().unwrap().2,field);
        assert_eq!(app.panes[0].entries.iter().map(|e|(e.path.clone(),e.size,e.modified,e.attributes)).collect::<Vec<_>>(),order);
        assert_eq!(app.panes[0].summary.as_ref().unwrap().bytes,bytes);
        // A full refresh intentionally rebuilds strings; record the next
        // unchanged allocation before the following edit.
        if new.is_some(){
            let row=app.panes[0].entries.iter().position(|e|e.path==keep).unwrap();
            app.panes[0].cursor=row;app.panes[0].selected.insert(row);
        }
    }
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn twin_preparation_waits_for_large_owner_without_rebuilding_or_stale_retention(){
    use super::panes::test_large_twin_rendezvous;
    let (root,_ctx,app)=fixture("twin-rendezvous");
    test_large_twin_rendezvous(&app);
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn sorted_partial_edits_match_full_order_in_extension_size_modified_and_attributes(){
    let (root,ctx,mut app)=fixture("ordered-partial-modes");
    let left=root.join("left");
    for i in 0..4096{fs::write(left.join(format!("ordinary-{i:05}.txt")),vec![1u8;100+i%10]).unwrap();}
    fs::write(left.join("very-long-name-that-keeps-the-column-maximum.verylongextension"),vec![1u8;10000]).unwrap();
    app.settings.set_flag("ShowDriveList",false);
    app.settings.set_text("LeftGroupMode","None");app.settings.set_text("SizeUnit","B");
    for mode in ["Extension","Size","Modified","Attributes"]{
        for desc in [false,true]{
            app.settings.set_text("LeftSortMode",mode);app.settings.set_flag("LeftSortDescending",desc);
            app.refresh(0);wait(&mut app,&ctx);
            let keep=left.join("ordinary-00128.txt");
            let row=app.panes[0].entries.iter().position(|e|e.path==keep).unwrap();
            app.panes[0].cursor=row;app.panes[0].selected.insert(row);
            let text=app.panes[0].view.cells[row][0].clone();
            let new=left.join("new-small.bin");fs::write(&new,b"12345").unwrap();
            let changed_path=if matches!(mode,"Size"|"Modified"){
                fs::remove_file(&new).unwrap();
                fs::write(&keep,[1u8;90]).unwrap();keep.clone()
            }else{new.clone()};
            let plan=panes::LocalListingPlan::capture(&app,0);
            app.panes[0].local_snapshot=Some(panes::DisplayedSnapshot{revision:1,
                content_revision:app.panes[0].thumb_content_revision,plan:plan.clone(),day:chrono::Local::now().date_naive()});
            let changed:Vec<_>=fsops::list_dir(&left,false).unwrap().into_iter().filter(|e|e.path==changed_path).collect();
            let descriptions=app.panes[0].descriptions.clone();let drive_infos=app.panes[0].drive_infos.clone();
            let mut partial=plan.prepare_partial(&left,&changed,&descriptions,&drive_infos,&AtomicBool::new(false),1).unwrap();
            partial.structural=changed_path==new;
            let listing=Listing{pane:0,generation:app.panes[0].generation,result:Ok(changed),prepared:None,
                cache_revision:Some(2),hardlinks:false,partial:Some(partial),timing_us:[0;6],skipped_device_names:0,descriptions,drive_infos};
            assert!(app.apply_partial_listing_inner(listing),"{mode}/{desc}");
            let pane=&app.panes[0];let row=pane.entries.iter().position(|e|e.path==keep).unwrap();
            assert_eq!(pane.cursor,row);assert!(pane.selected.contains(&row));
            if changed_path!=keep{assert!(Arc::ptr_eq(&text,&pane.view.cells[row][0]));}
            let cells=pane.view.cells.clone();let field=pane.prepared_field_max.as_ref().unwrap().2;
            let order:Vec<_>=pane.entries.iter().map(|e|(e.path.clone(),e.size,e.modified,e.attributes)).collect();
            app.refresh(0);wait(&mut app,&ctx);
            assert_eq!(app.panes[0].view.cells,cells,"{mode}/{desc}");
            assert_eq!(app.panes[0].prepared_field_max.as_ref().unwrap().2,field);
            assert_eq!(app.panes[0].entries.iter().map(|e|(e.path.clone(),e.size,e.modified,e.attributes)).collect::<Vec<_>>(),order);
            if new.exists(){fs::remove_file(new).unwrap();}
        }
    }
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn equal_overlay_pixels_share_texture_and_release_only_last_owner() {
    let (root,ctx,mut app)=fixture("overlay-shared-pixels");
    let a=app.shared_overlay_texture(&ctx,&root.join("a"),16,16,vec![255;1024]);
    let b=app.shared_overlay_texture(&ctx,&root.join("b"),16,16,vec![255;1024]);
    let c=app.shared_overlay_texture(&ctx,&root.join("c"),16,16,vec![127;1024]);
    assert_eq!(a.id(),b.id());assert_ne!(a.id(),c.id());
    assert_eq!(app.overlay_texture_pool.len(),2);
    app.release_overlay_texture(a.id());assert!(app.ghost_icons.contains_key(&b.id()));
    app.release_overlay_texture(b.id());assert!(!app.ghost_icons.contains_key(&b.id()));
    app.release_overlay_texture(c.id());assert!(app.overlay_texture_pool.is_empty());
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn queued_thumbnail_checks_real_file_version() {
    use std::os::windows::fs::MetadataExt;
    let (root,_ctx,app)=fixture("thumbnail-current-version");
    let path=root.join("left/a.txt");
    let entry=app.panes[0].entries.iter().find(|e|e.path==path).unwrap().clone();
    let key=super::views::ThumbnailKey::new(&entry,120);
    assert!(key.still_current());
    fs::write(&path,b"new and longer version").unwrap();assert!(!key.still_current());
    let mut replacement=entry;let m=fs::metadata(&path).unwrap();replacement.size=m.len();replacement.modified=m.modified().ok();replacement.attributes=m.file_attributes();
    let key=super::views::ThumbnailKey::new(&replacement,120);assert!(key.still_current());
    fs::remove_file(&path).unwrap();assert!(!key.still_current());
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_size_pending_job_and_cooldown_survive_same_location_watch_relists() {
    let (root,ctx,mut app)=fixture("folder-relist-pending");
    let folder=root.join("left/folder");
    fs::write(folder.join("actual.bin"),vec![7;4096]).unwrap();
    app.settings.set_flag("LeftAutoFolderSize",true);
    app.folder_size_workers=true; // deterministic delayed delivery
    let row=app.panes[0].entries.iter().position(|e|e.path==folder).unwrap();
    app.queue_folder_size(0,row);
    let generation=app.panes[0].generation;
    let token=app.panes[0].folder_auto_pending[&folder].clone();
    for n in 0..4 {
        fs::write(root.join("left").join(format!("unrelated-{n}.txt")),b"x").unwrap();
        app.refresh_after_folder_change(0);
        wait(&mut app,&ctx);
        assert!(Arc::ptr_eq(&app.panes[0].folder_auto_pending[&folder],&token));
        assert!(!token.load(Ordering::Relaxed));
    }
    assert!(app.panes[0].generation>generation);
    app.async_tx.send(AsyncResult::FolderSizeRow {
        pane:0,path:folder.clone(),bytes:4096,
        automatic:true,complete:true,watch:native::FolderSizeWatch::open(&folder).map(Arc::new),stable:true,scanned:true,
        work:ndr::folder_cache::ScanWork::default(),audit_at:Instant::now(),
        media:native::FolderSizeMedia::Fast,cancel:token.clone()
    }).unwrap();
    app.poll(&ctx);
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(),Some("4.00 KB"));
    let due=app.panes[0].folder_auto_times[&folder];
    let metadata=|app:&App|app.panes[0].entries.iter().find(|e|e.path==folder).map(|e|(e.attributes,e.modified)).unwrap();
    let before=metadata(&app);
    app.refresh_after_folder_change(0);wait(&mut app,&ctx);
    if before==metadata(&app){
        assert_eq!(app.panes[0].folder_auto_times.get(&folder),Some(&due));
    }else{
        // Windows can publish the directory's own modified timestamp
        // after the child write. A changed root schedules only a metadata
        // verification; it must not remove the measured display.
        assert!(!app.panes[0].folder_auto_times.contains_key(&folder));
        assert!(app.folder_size_cache[&folder].watch.as_ref().unwrap().reusable(&folder));
    }
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(),Some("4.00 KB"));
    assert_eq!(app.panes[0].folder_auto_scans,1);
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn dirty_folder_observation_replays_size_on_return_and_updates_after_scan() {
    let (root,ctx,mut app)=fixture("folder-dirty-return");
    let left=root.join("left");let folder=left.join("folder");let file=folder.join("actual.bin");
    fs::write(&file,vec![1;4096]).unwrap();
    app.settings.set_flag("LeftAutoFolderSize",true);
    let row=app.panes[0].entries.iter().position(|e|e.path==folder).unwrap();
    app.queue_folder_size(0,row);wait_for_folder_idle(&mut app,&ctx,0);
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(),Some("4.00 KB"));
    app.navigate(0,root.join("right"),true);wait(&mut app,&ctx);
    fs::write(&file,vec![2;8192]).unwrap();wait_for_folder_watch_change(&app,&folder);
    app.navigate(0,left,true);wait(&mut app,&ctx);
    let row=app.panes[0].entries.iter().position(|e|e.path==folder).unwrap();
    app.queue_folder_size(0,row);
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(),Some("4.00 KB"));
    assert!(app.panes[0].folder_auto_pending.contains_key(&folder));
    wait_for_folder_idle(&mut app,&ctx,0);
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(),Some("8.00 KB"));
    let scans=app.panes[0].folder_auto_scans;
    let mut draft=app.settings.clone();draft.set_text("SizeUnit","B");
    app.apply_settings(draft);wait(&mut app,&ctx);
    let expected=ndr::parity::size_to_str(8192,"B",app.settings.flag("UseThousandsSeparator",true));
    assert_eq!(app.panes[0].folder_size_display(&folder).as_deref(),Some(expected.as_str()));
    app.queue_folder_size(0,row);wait_for_folder_idle(&mut app,&ctx,0);
    assert_eq!(app.panes[0].folder_auto_scans,scans,"formatting and clean relist do not rescan descendants");
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn about_uses_live_process_ram_and_cpu_without_semantic_version() {
    let (root,ctx,mut app)=fixture("about-live-process");
    app.about_command();
    let first=app.msgboxes[0].text.clone();
    assert!(first.contains("RAM: ")&&first.contains(" MiB")&&first.contains("CPU: "));
    assert!(!first.contains(env!("CARGO_PKG_VERSION")));
    thread::sleep(Duration::from_millis(1100));
    let _=ctx.run(egui::RawInput::default(),|ctx|app.msgbox_ui(ctx));
    let text=&app.msgboxes[0].text;
    let cpu=text.lines().find_map(|l|l.strip_prefix("CPU: ")).unwrap().strip_suffix('%').unwrap().parse::<f64>().unwrap();
    let ram=text.lines().find_map(|l|l.strip_prefix("RAM: ")).unwrap().strip_suffix(" MiB").unwrap().parse::<f64>().unwrap();
    assert!(ram>0.0&&(0.0..=100.0).contains(&cpu),"{text}");
    app.answer_message_box(1,&ctx);
    assert!(app.msgboxes.is_empty());
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn manual_folder_measurement_survives_relist_and_keeps_automatic_tracking(){
    let (root,ctx,mut app)=fixture("manual-folder-tracked");
    let folder=root.join("left/folder");let file=folder.join("actual.bin");
    fs::write(&file,vec![1;1024]).unwrap();
    app.settings.set_flag("LeftAutoFolderSize",true);
    let row=app.panes[0].entries.iter().position(|e|e.path==folder).unwrap();
    app.panes[0].cursor=row;app.active=0;
    app.calculate_folder_size();
    let end=Instant::now()+Duration::from_secs(20);
    while !app.panes[0].folder_manual_pending.is_empty(){
        assert!(Instant::now()<end);
        app.poll(&ctx);thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.panes[0].folder_bytes.get(&folder),Some(&1024));
    assert!(app.panes[0].folder_auto_paths.contains(&folder));
    assert!(app.folder_size_cache[&folder].watch.is_some());
    fs::write(&file,vec![2;8192]).unwrap();
    wait_for_folder_watch_change(&app,&folder);
    app.refresh_after_folder_change(0);wait(&mut app,&ctx);
    let row=app.panes[0].entries.iter().position(|e|e.path==folder).unwrap();
    assert_eq!(app.panes[0].folder_size_display(&folder),Some("1.00 KB"));
    app.panes[0].folder_auto_times.remove(&folder);
    app.queue_folder_size(0,row);wait_for_folder_idle(&mut app,&ctx,0);
    assert_eq!(app.panes[0].folder_bytes.get(&folder),Some(&8192));
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn manual_pending_and_exact_bytes_are_not_lost_on_same_folder_refresh(){
    let (root,ctx,mut app)=fixture("manual-folder-pending-relist");
    let folder=root.join("left/folder");
    app.folder_size_workers=true;
    let row=app.panes[0].entries.iter().position(|e|e.path==folder).unwrap();
    app.enqueue_folder_size(0,row);
    let token=app.panes[0].folder_manual_pending[&folder].clone();
    for n in 0..3{
        fs::write(root.join("left").join(format!("change-{n}.txt")),b"x").unwrap();
        app.refresh_after_folder_change(0);wait(&mut app,&ctx);
        assert!(Arc::ptr_eq(&token,&app.panes[0].folder_manual_pending[&folder]));
        assert!(app.panes[0].folder_manual_pending.contains_key(&folder));
    }
    let publish=|a:&mut App,bytes,complete|{
        a.async_tx.send(AsyncResult::FolderSizeRow {pane:0,path:folder.clone(),bytes,automatic:false,complete,
            stable:true,watch:None,scanned:true,work:Default::default(),audit_at:Instant::now(),
            media:native::FolderSizeMedia::Fast,cancel:token.clone()}).unwrap();a.poll(&ctx);
    };
    publish(&mut app,1024,false);
    assert!(!app.panes[0].folder_bytes.contains_key(&folder));
    assert!(app.panes[0].folder_size_display(&folder).unwrap().starts_with('\u{2265}'));
    let newer=Arc::new(AtomicBool::new(false));
    app.panes[0].folder_manual_pending.insert(folder.clone(),newer.clone());
    publish(&mut app,99999,true); // completed old job cannot overwrite a new token
    assert!(!app.panes[0].folder_bytes.contains_key(&folder));
    assert!(Arc::ptr_eq(&newer,&app.panes[0].folder_manual_pending[&folder]));
    app.panes[0].folder_manual_pending.insert(folder.clone(),token.clone());publish(&mut app,4096,true);
    assert_eq!(app.panes[0].folder_bytes.get(&folder),Some(&4096));
    app.panes[0].folder_manual_pending.insert(folder.clone(),token.clone());publish(&mut app,1024,false);
    assert_eq!(app.panes[0].folder_bytes.get(&folder),Some(&4096));
    assert_eq!(app.panes[0].folder_size_display(&folder),Some("4.00 KB"));
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn folder_disk_hint_reformats_without_restarting_pending_scan(){
    let (root,ctx,mut app)=fixture("folder-hint-format");
    let folder=root.join("left/folder");
    app.settings.set_flag("LeftAutoFolderSize",true);
    app.folder_size_workers=true;
    let row=app.panes[0].entries.iter().position(|e|e.path==folder).unwrap();
    app.queue_folder_size(0,row);
    let token=app.panes[0].folder_auto_pending[&folder].clone();
    app.async_tx.send(AsyncResult::FolderSizeHint {pane:0,path:folder.clone(),bytes:3072,cancel:token.clone()}).unwrap();
    app.poll(&ctx);
    assert_eq!(app.panes[0].folder_size_display(&folder),Some("3.00 KB"));
    assert!(!app.panes[0].folder_bytes.contains_key(&folder));
    let mut settings=app.settings.clone();settings.set_text("SizeUnit","B");
    app.apply_settings(settings);wait(&mut app,&ctx);
    assert!(Arc::ptr_eq(&token,&app.panes[0].folder_auto_pending[&folder]));
    let expected=ndr::parity::size_to_str(3072,"B",app.settings.flag("UseThousandsSeparator",true));
    assert_eq!(app.panes[0].folder_size_display(&folder),Some(expected.as_str()));
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn auto_folder_admission_interleaves_two_columns_and_prefers_missing_sizes(){
    let (root,ctx,mut app)=fixture("folder-fair-columns");
    let left=root.join("left");
    for n in 0..80{fs::create_dir(left.join(format!("dir-{n:03}"))).unwrap();}
    app.settings.set_flag("LeftAutoFolderSize",true);
    app.settings.set_text("LeftSortMode","Name");
    app.refresh(0);wait(&mut app,&ctx);
    app.folder_size_workers=true; // inspect real admission order without I/O
    let rows:Vec<_>=(0..80).map(|n|app.panes[0].entries.iter().position(|e|e.name==format!("dir-{n:03}")).unwrap()).collect();
    // Every first-column item has a measured value and an expired check.
    for &row in &rows[..40]{
        let path=app.panes[0].entries[row].path.clone();
        app.panes[0].folder_auto_paths.insert(path.clone());
        app.panes[0].folder_bytes.insert(path.clone(),1024);
        app.panes[0].folder_sizes.insert(path,Some("1.00 KB".into()));
    }
    for col in 0..2{for line in 0..40{
        app.note_visible_folder_size(0,rows[col*40+line],line*2+col);
    }}
    app.finish_visible_folder_size_frame(&ctx);
    assert_eq!(app.panes[0].folder_auto_visible.len(),80);
    let jobs=app.folder_size_queue.0.lock().unwrap();
    assert_eq!(jobs.len(),25);
    for (i,job) in jobs.iter().enumerate(){
        assert_eq!(job.path,left.join(format!("dir-{:03}",40+i)));
    }
    drop(jobs);drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn navigation_replays_both_complete_and_hint_sizes_before_admission(){
    let (root,ctx,mut app)=fixture("folder-return-before-admission");
    let left=root.join("left");let right=root.join("right");
    let folder=left.join("folder");let hint=left.join("hint");
    fs::create_dir(&hint).unwrap();app.refresh(0);wait(&mut app,&ctx);
    app.settings.set_flag("LeftAutoFolderSize",true);
    app.cache_folder_size(folder.clone(),8192,native::FolderSizeWatch::open(&folder).map(Arc::new),Instant::now(),native::FolderSizeMedia::Fast);
    app.folder_size_hints.insert(hint.clone(),(4096,Instant::now()));
    app.navigate(0,right,true);wait(&mut app,&ctx);
    app.navigate(0,left,true);wait(&mut app,&ctx);
    app.folder_auto_busy=true;
    for path in [&folder,&hint]{
        let row=app.panes[0].entries.iter().position(|e|&e.path==path).unwrap();
        app.note_visible_folder_size(0,row,row);
    }
    assert_eq!(app.panes[0].folder_size_display(&folder),Some("8.00 KB"));
    assert_eq!(app.panes[0].folder_size_display(&hint),Some("4.00 KB"));
    assert_eq!(app.panes[0].folder_hint_bytes.get(&hint),Some(&4096));
    assert!(!app.panes[0].folder_bytes.contains_key(&hint));
    assert!(app.panes[0].folder_auto_pending.is_empty());
    app.finish_visible_folder_size_frame(&ctx);
    assert!(app.panes[0].folder_auto_pending.is_empty());
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn narrow_auto_folder_size_cells_never_cross_column_boundary(){
    let (root,ctx,mut app)=fixture("narrow-auto-size-column");
    app.settings.set_flag("LeftAutoFolderSize",true);
    app.settings.set_text("SizeUnit","B");
    let _=ctx.run(egui::RawInput::default(),|ctx|{
        egui::CentralPanel::default().show(ctx,|ui|{
            for width in [90.0,120.0,180.0,220.0,345.0,600.0]{
                let (name,fields)=app.original_auto_fields(ui,0,width);
                assert!(name+fields.iter().flatten().sum::<f32>()<=width+0.01,"{width}: {name} {fields:?}");
                assert!(fields[2].is_some(),"size missing: {width}");
            }
        });
    });
    drop(app);fs::remove_dir_all(root).unwrap();
}

#[test]
fn dropping_app_without_on_exit_retires_both_folder_workers(){
    let (root,ctx,mut app)=fixture("folder-worker-owner-drop");
    let folder=root.join("left/folder");
    fs::write(folder.join("data.bin"),[1u8;1024]).unwrap();
    app.settings.set_flag("LeftAutoFolderSize",true);
    let row=app.panes[0].entries.iter().position(|e|e.path==folder).unwrap();
    app.queue_folder_size(0,row);
    assert_eq!(app.folder_size_done.len(),2);
    let shutdown=app.folder_size_shutdown.clone();
    let queue=app.folder_size_queue.clone();
    let done=std::mem::take(&mut app.folder_size_done);
    drop(app);
    assert!(shutdown.load(Ordering::Relaxed));
    for lane in done{lane.recv_timeout(Duration::from_secs(3)).unwrap();}
    assert!(queue.0.lock().unwrap().is_empty());
    assert!(queue.2.lock().unwrap().is_empty());
    drop(ctx);fs::remove_dir_all(root).unwrap();
}

#[test]
fn watcher_loading_indicator_never_moves_the_list_or_scroll_geometry() {
    let (root,ctx,mut app)=fixture("stable-loading-geometry");
    for side in 0..2 {
        for style in ["Details","Auto","Names","Thumbnails"] {
            app.settings.set_text(&App::side_key(side,"ViewStyle"),style);
            let draw=|app:&mut App, loading:bool|{
                app.panes[side].loading=loading;
                let input=egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(800.0,600.0))),..Default::default()};
                let _=ctx.run(input,|ctx|egui::CentralPanel::default().show(ctx,|ui|app.pane_ui(ui,side,ctx)).inner);
                app.panes[side].list_rect.unwrap()
            };
            // Let egui scrollbars and measurements settle.
            for _ in 0..4 { draw(&mut app,false); }
            let stable=draw(&mut app,false);
            app.panes[side].adjust=None;
            for loading in [true,false,true,false,true,false] {
                let rect=draw(&mut app,loading);
                assert_eq!(rect,stable,"side={side} view={style} loading={loading}");
                assert_eq!(app.panes[side].list_shape.as_ref().unwrap().0,stable.size());
            }
        }
    }
    app.panes[0].loading=false;app.panes[1].loading=false;
    drop(app);let _=fs::remove_dir_all(root);
}

#[test]
fn completion_notice_overlay_is_lifted_compact_and_stacks_without_overlap() {
    let (root,ctx,mut app)=fixture("completion-notice-layout");
    app.settings.set_flag("ShowCompletionToast",true);
    app.settings.set_flag("ShowFunctionBar",true);
    app.completion_toasts.push_at("Completed","Copied 4 files",super::completion_toast::NoticeKind::Success,Instant::now());
    app.completion_toasts.push_at("Second","",super::completion_toast::NoticeKind::Warning,Instant::now());
    let raw=||egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(800.0,600.0))),..Default::default()};
    for _ in 0..4 {let _=ctx.run(raw(),|ctx|app.completion_toasts_ui(ctx));}
    let older=ctx.memory(|m|m.area_rect(egui::Id::new(("ndr_completion_toast",0usize)))).unwrap();
    let newer=ctx.memory(|m|m.area_rect(egui::Id::new(("ndr_completion_toast",1usize)))).unwrap();
    assert!((newer.bottom()-553.0).abs()<=1.0,"{newer:?}");
    assert!((newer.top()-older.bottom()-9.0).abs()<=1.0,"{older:?} {newer:?}");
    assert!(older.height()<69.0 && newer.height()<older.height(),"{older:?} {newer:?}");
    assert_eq!(older.right(),newer.right());
    drop(app);let _=fs::remove_dir_all(root);
}
