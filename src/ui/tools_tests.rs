//! C# ChecksumAsync, MergeTextFilesAsync, OpenWithAsync, RunCommandAsync, OriginalFindAsync, Map/UnmapNetworkDriveAsync,
//! ToggleExplorerIntegrationAsync, ImportLegacySettingsAsync, InstallSelectedArchive, SelectDestinationFolder/RememberDestination를
//! Rust UI에서 확인한다. 한국어와 영어만. 격리 폴더만 쓰고, 프로그램 실행·네트워크 드라이브·레지스트리·폴더/파일 대화 상자는 가짜 함수다.
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
/// "deny"로 시작하는 이름은 ERROR_ACCESS_DENIED(5), "fail"은 다른 오류(1).
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
/// 여러 항목 속성 창(원본 FUN_00458da8): 경로마다 "properties*"로 남긴다.
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

fn fixture(label: &str, language: &str) -> (PathBuf, egui::Context, App) {
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
    app.system = super::tools_dialog::SystemHooks { network_drives: one_network_drive, add_connection: connect, cancel_connection: disconnect, explorer_enabled: explorer_off, set_explorer, run_verb, properties: record_properties, network_path_error: |_| 0, connect_share: |_| 1223, shortcut_folder: |_| None, create_shortcut: |_, _| Ok(()), clipboard_files: Vec::new, temp_folder: fake_temp_folder, special_folder: |_| None, ready_drives: fake_drives, volume_label: fake_volume_label, set_volume_label: fake_set_volume_label, disk_info: |_| None, drive_item: fake_drive_item, file_icon: |_, _| Some((1, 1, vec![255, 255, 255, 255])), overlay_icon: |_| None, path_icon: |_| None, per_file_icon: |_| false, font_file: |_| None, large_icon: |_| None, compact_path: |p, _| p.to_owned(), wheel_lines: || 3, play_wave: super::tools_dialog::record_wave };
    wait_until(&mut app, &ctx, |app| app.panes.iter().all(|p| !p.loading));
    (root, ctx, app)
}
/// 시간이 다 되면 부른 줄을 알린다(#[track_caller]) — 어느 기다림이 멈췄는지 바로 보이게.
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
    // 찾기: TfGetText(TITLE_FIND, MSG_FIND, 빈 글).
    app.dispatch("find", &ctx);
    let (title, message, value, _) = prompt(&app);
    assert_eq!((title, message, value), (app.language_pack.text("RES.TITLE_FIND", "?"), app.language_pack.text("RES.MSG_FIND", "?"), String::new()));
    app.answer_dialog(DialogResult::Text(Some("gam".into())), &ctx);
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "gamma.log");
    // 명령 실행: 기록이 있는 입력 상자.
    app.settings.set_strings("CommandHistory", vec!["old".into()]);
    app.dispatch("command", &ctx);
    let (title, message, _, history) = prompt(&app);
    assert_eq!((title, message, history), (app.language_pack.text("RES.TITLE_CMD", "?"), app.language_pack.text("RES.MSG_CMD", "?"), Some(vec!["old".to_owned()])), "original miCommandLineClick");
    app.answer_dialog(DialogResult::Text(Some("someprog.exe a b".into())), &ctx);
    assert_eq!(launched(), [("someprog.exe".to_owned(), "a b".to_owned(), left.clone())], "ShellExecuteEx(name, arguments, current folder)");
    assert_eq!(app.settings.strings("CommandHistory"), ["someprog.exe a b", "old"]);
    // 실행 파일 기록: 이름이 같은 후보 하나면 그 경로를 인수 없이.
    let tool = left.join("folder1").join("tool.exe");
    fs::write(&tool, b"").unwrap();
    app.settings.set_strings("ExecutableHistory", vec![format!("tool={}", tool.display()), "ghost=C:\\nowhere\\ghost.exe".into()]);
    app.run_command("TOOL --ignored".into());
    assert_eq!(launched(), [(tool.to_string_lossy().into_owned(), String::new(), left.clone())]);
    app.run_command("ghost".into());
    assert_eq!(launched(), [("ghost".to_owned(), String::new(), left.clone())], "a missing cached path is no candidate");
    // 후보가 여럿이면 고르는 메뉴.
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
    // 다른 프로그램으로 열기: 프로그램(처음 값 TextViewerPath) → 인수 템플릿.
    app.navigate(0, left.clone(), true);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    // 원본 mipOpenWithClick(0x43da54, 스킨 테마): 커서 파일을 rundll32 shell32.dll,OpenAs_RunDLL 로.
    app.panes[0].cursor = row(&app, "alpha.txt");
    app.dispatch("openWith", &ctx);
    assert_eq!(launched(), [("rundll32.exe".to_owned(), format!("shell32.dll,OpenAs_RunDLL {}", left.join("alpha.txt").display()), left.clone())]);
    fs::remove_dir_all(root).unwrap();
}

/// 목록 스레드에서 불리므로 스레드 지역 값을 쓰지 않는다: 늘 "Test", 고정 디스크, 여유 2 KB / 전체 8 KB.
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
    // 원본 TfChecksum(하위 메뉴 MD5): 초점 파일 하나, 제목 "MD5", 처음엔 CALCULATING, 끝나면 소문자 해시와 켜진 복사 단추.
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
    // Enter = 초점 받은 복사 단추.
    frame(&mut app, &ctx, vec![enter()]);
    assert_eq!(COPIED.with(|c| c.borrow().clone()), md5);
    assert_eq!(app.original_dialogs.len(), 1, "copying keeps the window open");
    COPIED.with(|c| c.borrow_mut().clear());
    app.copy_checksum();
    assert_eq!(COPIED.with(|c| c.borrow().clone()), md5);
    app.clipboard_writer = |_| Ok(());
    app.answer_dialog(DialogResult::Checksum, &ctx);
    assert!(app.original_dialogs.is_empty() && app.text.is_none());
    // CRC는 IntToHex(8자, 대문자). 폴더에 초점이 있으면 아무 일도 없다.
    app.dispatch("checksum:Crc32", &ctx);
    wait_until(&mut app, &ctx, |app| matches!(&app.original_dialogs[0].kind, DialogKind::Checksum { ready: true, .. }));
    let crc32 = ndr::tools::checksum(&left.join("alpha.txt"), "crc32").unwrap();
    assert!(matches!(&app.original_dialogs[0].kind, DialogKind::Checksum { value, .. } if *value == crc32 && value.len() == 8));
    app.answer_dialog(DialogResult::Checksum, &ctx);
    app.panes[0].cursor = row(&app, "folder1");
    app.dispatch("checksum:Sha1", &ctx);
    assert!(app.original_dialogs.is_empty());
    // C# 판의 SHA-256은 글 창 그대로.
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
    // 텍스트 파일 합치기: 둘 이상, 저장 경로, 같은 경로 검사, 구분 고르기.
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
    // 다른 폴더로 복사: 대상 선택 창(작업 폴더, 자주 쓴 대상), RememberDestination.
    let mut work = vec![right.to_string_lossy().into_owned()];
    work.resize(5, String::new());
    app.settings.set_strings("WorkingFolders", work);
    app.panes[0].selected = [row(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("copyToFolder", &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, format!("{}{}", app.language_pack.text("RES.COPY", "?"), app.language_pack.text("RES.SELECTDEST", "?")), "original COPY + SELECTDEST");
    frame(&mut app, &ctx, vec![]);
    app.destination_action(super::destination_dialog::DestinationAction::Work(1), &ctx);
    assert_eq!(boxed(&app), ("Copy - Select a destination".to_owned(), "Work folder 2 is not set or is unavailable.".to_owned()));
    // 상자가 위에 있어도 대상 선택 창은 그 뒤에 그대로 그려진다(예전에는 사라졌다).
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
    // 찾아보기와 기록에서 지우기.
    app.panes[0].cursor = row(&app, "gamma.log");
    app.dispatch("copyToFolder", &ctx);
    app.destination_action(super::destination_dialog::DestinationAction::Remove(right.to_string_lossy().into_owned()), &ctx);
    assert_eq!(boxed(&app).1, app.language_pack.text("RES.SELDEST_DELMUFOLDER", "?"));
    app.answer_message_box(6, &ctx);
    assert!(app.settings.values["MostUsedDestinations"].as_array().unwrap().is_empty());
    PICKED.with(|p| *p.borrow_mut() = Some(root.join("left").join("folder1")));
    app.destination_action(super::destination_dialog::DestinationAction::Browse, &ctx);
    wait_job(&mut app, &ctx);
    assert!(left.join("folder1").join("gamma.log").is_file());
    fs::remove_dir_all(root).unwrap();
}

thread_local! {
    static NETWORK_PICKS: std::cell::RefCell<Vec<(String, Option<PathBuf>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 원본 miNetwork 폴더 고르기 대신: 제목과 처음 폴더를 적고 처음 폴더의 "net"을 고른다.
fn fake_network_picker(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
    NETWORK_PICKS.with(|p| p.borrow_mut().push((title.to_owned(), initial.map(Path::to_path_buf))));
    initial.map(|p| if p.ends_with("net") { p.to_path_buf() } else { p.join("net") })
}

#[test]
fn network_drives_explorer_import_and_install_follow_the_csharp_flows() {
    let (root, ctx, mut app) = fixture("system", "ko");
    let used: Vec<String> = native::drives().iter().map(|d| d.to_string_lossy().to_lowercase()).collect();
    let letter = ('m'..='y').find(|c| !used.iter().any(|d| d.starts_with(&format!("{c}:")))).expect("a free drive letter");
    // 네트워크 드라이브 연결: 문자 → 공유 경로 → 예/아니요/취소 → (아니요) 계정 입력 창.
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
    // 연결 해제.
    app.dispatch("unmapDrive", &ctx);
    assert_eq!(choice(&app), ("네트워크 드라이브 연결 해제".to_owned(), vec!["Z:".to_owned()]));
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    assert_eq!(CANCELLED.with(|c| c.borrow().clone()), ["Z:"]);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.system.network_drives = no_network_drives;
    app.dispatch("unmapDrive", &ctx);
    assert_eq!(boxed(&app).1, "연결된 네트워크 드라이브가 없습니다.");
    app.answer_message_box(1, &ctx);
    // 원본 miNetwork(SHBrowseForFolderW, 뿌리 = 네트워크, 제목 [RES] NETWORK): 지금 폴더를 처음 선택으로 열고, 고르면 그 폴더로 간다.
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
    // 탐색기 메뉴: 없으면 추가를, 있으면 제거를 묻는다.
    app.dispatch("shellIntegration", &ctx);
    assert_eq!(boxed(&app), ("탐색기 통합".to_owned(), "현재 계정의 Windows 탐색기 폴더 메뉴에 NDR을 추가할까요?".to_owned()));
    app.answer_message_box(6, &ctx);
    assert_eq!(app.status, "탐색기 메뉴에 NDR을 추가했습니다.");
    app.system.explorer_enabled = explorer_on;
    app.dispatch("shellIntegration", &ctx);
    assert_eq!(boxed(&app).1, "NDR의 탐색기 폴더 메뉴 등록을 제거할까요?");
    app.answer_message_box(7, &ctx);
    assert_eq!(EXPLORER.with(|e| e.borrow().clone()), [true], "No changes nothing");
    // 압축 파일 설치(셸 동사 install).
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
    // 레거시 INI 가져오기: 설정 바꾸기, 옆의 toolbar.xml(있던 것은 보관), 시작 위치.
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

/// 원본 rbFormatClick(PathRenameExtensionW). 압축 파일은 늘 지금 폴더에 만든다(고친 것: 적은 경로는 쓰지 않고 이름만).
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

/// VCL TLabel FocusControl: 압축하기 창의 "비밀번호(&P):"는 Alt+P로 비밀번호 칸에 초점을 준다.
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

/// 외부 압축 프로그램은 쓰지 않는다: RAR·ARJ(동봉 7-Zip이 못 만듦)는 고를 수 없다.
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

/// 원본 cbFileName에 이름을 넣는다.
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
    // 압축하기 창: 하위 폴더 포함(폴더가 있으면 처음 켬), 분할 크기 검사.
    app.dispatch("createArchive", &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "압축하기");
    let form = archive_form(&app);
    assert!(form.recursive);
    assert_eq!(form.formats.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), ["zip", "7z", "tar", "tar.gz", "tar.bz2"], "plain names, no ALZ/WIM");
    assert_eq!(form.preset_labels, ["최고", "보통", "빠르게"]);
    assert_eq!(form.volume, "분할 안함");
    frame(&mut app, &ctx, vec![]);
    app.edit_dialog_kind(|kind| if let DialogKind::ArchiveCreate(form) = kind { form.volume = "열 MB".into(); });
    app.submit_archive_create(0, &ctx);
    assert_eq!(boxed(&app), ("압축하기".to_owned(), app.language_pack.text("formAddToArchive.Message.InvalidVolume", "Enter a valid volume size such as 10 MB, or 0 for no splitting.")));
    app.answer_message_box(1, &ctx);
    let pack = left.join("pack.zip");
    // 원본 TfAddToArc cbFileName: 기본 이름(폴더 이름 + 확장자). 저장 상자 없이 지금 폴더에 그 이름으로 만든다.
    assert_eq!(archive_form(&app).file_name, "left.zip");
    let _ = &right;
    SAVE_TARGET.with(|t| *t.borrow_mut() = None);
    let named = pack.to_string_lossy().into_owned();
    app.edit_dialog_kind(|kind| if let DialogKind::ArchiveCreate(form) = kind { form.volume = "분할 안함".into(); form.set_format(0); form.file_name = "pack.zip".into(); });
    app.submit_archive_create(0, &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(app.status, "ZIP 압축 파일을 만들었습니다.");
    assert_eq!(archive_keys(&pack), ["alpha.txt", "folder1/inner.txt"]);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "pack.zip", "the cursor goes to the new archive");
    assert_eq!(app.settings.text("DefaultArchiveFormatExtension", ""), "zip", "RememberArchiveDialogChoices");
    // 같은 이름으로 다시 만들면 묶음 교체를 묻는다(아니요 = 그대로). 새 목록은 ".."에서 시작하므로 전처럼 folder1에 커서를 둔다.
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "folder1").unwrap();
    app.dispatch("createArchive", &ctx);
    name_archive(&mut app, &named);
    app.submit_archive_create(0, &ctx);
    assert_eq!(boxed(&app).0, "압축 파일 교체");
    assert!(boxed(&app).1.starts_with("기존 압축 파일 묶음 1개를 새 결과로 교체할까요?\r\n\r\npack.zip"));
    app.answer_message_box(7, &ctx);
    assert!(app.job.as_ref().is_none_or(|j| j.finished));
    // 압축 파일에 추가: 있던 항목은 남는다. 형식이 다르면 알린다.
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
    // 압축 파일로 이동: 확인 뒤 원본을 지운다(이 시험은 휴지통을 쓰지 않는다).
    app.panes[0].selected = [row(&app, "gamma.log")].into_iter().collect();
    app.dispatch("archiveMove", &ctx);
    name_archive(&mut app, &named);
    app.submit_archive_create(0, &ctx);
    assert_eq!(boxed(&app), ("압축 후 이동".to_owned(), "압축 파일을 만든 뒤 원본을 휴지통으로 이동할까요?".to_owned()));
    app.answer_message_box(6, &ctx);
    wait_job(&mut app, &ctx);
    assert!(!left.join("gamma.log").exists());
    assert_eq!(archive_keys(&pack), ["alpha.txt", "beta.txt", "folder1/inner.txt", "gamma.log"]);
    // 새 폴더에 풀기: pack, pack(2).
    app.panes[0].selected.clear();
    app.panes[0].cursor = row(&app, "pack.zip");
    app.dispatch("extractNewFolder", &ctx);
    wait_job(&mut app, &ctx);
    assert_eq!(app.status, format!("압축을 풀었습니다: {}", left.display()));
    assert!(left.join("pack").join("folder1").join("inner.txt").is_file());
    app.panes[0].cursor = row(&app, "pack.zip");
    app.dispatch("extractNewFolder", &ctx);
    wait_job(&mut app, &ctx);
    assert!(PathBuf::from(format!("{}\\pack(2)", left.display())).join("gamma.log").is_file());
    // 자동: 맨 위가 폴더 하나면 그대로, 아니면 새 폴더.
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
    // 압축 파일 안: 다른 패널로 복사 = 그 항목 풀기, 이동은 안내, 속성.
    app.panes[0].archive = Some(ArchiveLocation { file: pack.clone(), prefix: String::new(), password: String::new() });
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.panes[0].cursor = row(&app, "folder1");
    app.dispatch("copyToOther", &ctx);
    if !app.msgboxes.is_empty() {
        app.answer_message_box(6, &ctx);
    }
    wait_job(&mut app, &ctx);
    assert!(right.join("folder1").join("inner.txt").is_file() && !right.join("alpha.txt").exists());
    app.panes[0].cursor = row(&app, "folder1");
    app.dispatch("moveToOther", &ctx);
    if app.msgboxes.first().is_some_and(|m| m.text.contains("%s") || m.title == app.language_pack.text("RES.TITLE_FILEOPERATION", "?")) {
        app.answer_message_box(6, &ctx);
    }
    assert_eq!(boxed(&app).1, "압축 파일 안의 항목은 이동할 수 없습니다. 복사를 사용하세요.");
    app.answer_message_box(1, &ctx);
    app.dispatch("archiveProperties", &ctx);
    wait_until(&mut app, &ctx, |app| app.text.is_some());
    let text = app.text.take().unwrap();
    assert_eq!(text.title, "압축 파일 속성");
    assert!(text.text.starts_with("파일: pack.zip\r\n형식: ZIP\r\n") && text.text.contains("파일 항목: 4\r\n폴더 항목: 1\r\n"), "{}", text.text);
    app.dispatch("extractCurrent", &ctx);
    assert!(app.original_dialogs.is_empty() && app.job.as_ref().is_none_or(|j| j.finished), "inside an archive the current-folder mode does nothing");
    fs::remove_dir_all(root).unwrap();
}

// ---- 디스크 복사(C# DiskCopyWindow). 장치 함수는 모두 가짜이고, 이미지 파일은 격리 폴더에만 쓴다. ----
use super::disk_copy_dialog::{DiskAction, DiskHooks, DiskMode, DiskStage, physical_text};
use ndr::disk::{PhysicalDisk, RemovableDrive};

/// 가짜 장치 함수는 일꾼 스레드에서 불리므로 기록은 전역이다. 디스크 테스트끼리는 이 잠금으로 차례를 지킨다.
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
        // 처음: 직접 복사, 원본 = 첫 드라이브, 대상 = 두 번째.
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
    // 직접 복사: 경고 확인(아니요면 아무것도 하지 않음, 예면 복사 → 창이 닫힘).
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
    // 이미지 만들기: 찾아보기(저장 대화 상자: 문서 폴더, E-.img, .img 형식, 덮어쓰기 확인) → 확인 없이 시작.
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
    // 취소: 만들기 중에는 취소 단추와 창 닫기(취소 요청)가 된다. 창은 남고 상태 글이 바뀐다.
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
    // 한 드라이브 복사(미디어 교체): 임시 이미지 → 교체 확인. 아니요면 쓰지 않고 임시 이미지를 지운다.
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
        // 예 → 같은 디스크 번호에 쓰기(검증 확인란 따름). 실패하면 복구용 이미지를 남기고 복원 방식으로 바꾼다.
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
        // 남은 이미지로 전체 디스크 복원: 확인 → 검증 없이 쓰기 → 창이 닫힘.
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
    // 전체 디스크 복원: 찾아보기(열기 대화 상자) → 확인 → 쓰기 실패 → "복원 또는 드라이브 복사" 오류, 창은 남는다.
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
    // 볼륨 복원 확인 글, 볼륨 이미지 만들기 실패("The image could not be created").
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
    // C# ExecuteFunctionKeyMacroAsync: 프로그램 매크로 → 패널 매크로 → %p면 InputDialog(TITLE_EDITPARAM, MSG_EDITPARAM, 인수) →
    // 빈 프로그램 무시, 있는 폴더는 열기, 아니면 ShellExecuteEx(프로그램, 인수 글, 활성 패널 폴더).
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
    // 취소면 실행하지 않는다.
    app.run_function_key_button("F9", &ctx);
    app.answer_dialog(DialogResult::Text(None), &ctx);
    assert!(launched().is_empty(), "Cancel runs nothing");
    // 옛 FunctionKeys: [이름] 떼고, 인수는 패널 매크로를 푼 한 줄 그대로.
    app.run_function_key_button("F7", &ctx);
    assert!(app.original_dialogs.is_empty(), "no %p, no prompt");
    let path = format!("{}\\", left.display());
    assert_eq!(launched(), [("viewer.exe".to_owned(), format!("\"alpha.txt\" {path}"), left.clone())]);
    // 있는 폴더는 활성 패널에서 연다.
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
fn shortcut_dir(_: bool) -> Option<PathBuf> {
    SHORTCUT_DIR.with(|d| d.borrow().clone())
}
fn record_link(target: &Path, folder: &Path) -> Result<(), String> {
    LINKS.with(|l| l.borrow_mut().push((target.to_path_buf(), folder.to_path_buf())));
    Ok(())
}
fn links() -> Vec<(PathBuf, PathBuf)> {
    LINKS.with(|l| std::mem::take(&mut *l.borrow_mut()))
}
fn clip() -> Vec<PathBuf> {
    CLIP.with(|c| c.borrow().clone())
}

#[test]
fn system_commands_follow_csharp() {
    let (root, ctx, mut app) = fixture("system", "en");
    let (left, right) = (root.join("left"), root.join("right"));
    app.system.shortcut_folder = shortcut_dir;
    app.system.create_shortcut = record_link;
    app.system.clipboard_files = clip;
    let index = |app: &App, name: &str| app.panes[0].entries.iter().position(|e| e.name == name).unwrap();
    // CreateOriginalShortcuts: no target folder -> nothing; otherwise marked items in list order, then the selection is cleared.
    app.panes[0].selected = [index(&app, "beta.txt"), index(&app, "alpha.txt")].into_iter().collect();
    app.dispatch("shortcut", &ctx);
    assert!(links().is_empty());
    let desk = root.join("desk");
    fs::create_dir_all(&desk).unwrap();
    SHORTCUT_DIR.with(|d| *d.borrow_mut() = Some(desk.clone()));
    app.dispatch("quickLaunchShortcut", &ctx);
    assert_eq!(links(), vec![(left.join("alpha.txt"), desk.clone()), (left.join("beta.txt"), desk.clone())]);
    assert!(app.panes[0].selected.is_empty());
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
    // 원본 miPropertiesClick(스킨 테마): 표시한 항목 전부의 여러 파일 속성 창.
    app.panes[0].selected = [index(&app, "alpha.txt"), index(&app, "beta.txt")].into_iter().collect();
    app.dispatch("properties", &ctx);
    let mut recorded = VERBS.with(|v| v.borrow().clone());
    recorded.sort();
    assert_eq!(recorded, vec![(left.join("alpha.txt"), "properties*".to_owned()), (left.join("beta.txt"), "properties*".to_owned())]);
    app.panes[0].selected.clear();
    VERBS.with(|v| v.borrow_mut().clear());
    // 원본 miFileInfoClick(0x4452bc, 스킨 테마): https://ext.xiles.app/?ext=확장자.
    app.dispatch("fileInfo", &ctx);
    assert_eq!(launched().first().map(|(program, _, _)| program.clone()), Some("https://ext.xiles.app/?ext=txt".to_owned()));
    // ViewTextAsync: the configured viewer gets the quoted file path in the file's folder; with none, the NDR text window.
    app.settings.set_text("TextViewerPath", "viewer.exe");
    app.dispatch("textView", &ctx);
    assert_eq!(launched(), vec![("viewer.exe".to_owned(), format!("\"{}\"", left.join("alpha.txt").display()), left.clone())]);
    // 원본 FUN_00430304(스킨 테마): 설정이 비면 notepad.exe, 커서가 폴더면 아무것도 하지 않는다.
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
    let _ = fs::remove_dir_all(&root);
}

/// C# OpenItemAsync(압축 패널) + ExtractExternalWithPasswordPromptAsync: 항목을 임시로 풀어 열고, 암호가 틀리면 묻는다.
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
    // 원본 FUN_004399b4: TfGetText(TITLE_PASSWORD, MSG_PASSWORD, 빈 글), 가린 입력.
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


/// 압축하기 형식에 ALZ·EGG는 없다(사용자 요청, 풀기만 된다). zip·7z 밖의 형식은 암호를 넣을 수 없다.
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

/// 원본 0x483c79: playwav(처음 값 켜짐)면 .wav를 셸로 열지 않고 PlaySoundW(NULL) 뒤 PlaySoundW(파일)로 재생한다.
/// 원본 0x43d9fc: 완료 소리는 프로그램 폴더의 complete.wav.
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
    // 완료 소리.
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

/// 원본 TNXScrollStep: 2(처음 값) = 쪽 단위(한 칸에 보이는 줄 수), 1 = 줄 단위(Mouse.WheelScrollLines 줄).
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
    // 원본 SetScrollTop(쪽 단위): 위치는 쪽 경계, 0..쪽 수 − 1.
    assert_eq!(App::page_snap(90.0, 100.0, 5), 100.0, "nearest page boundary");
    assert_eq!(App::page_snap(-50.0, 100.0, 5), 0.0);
    assert_eq!(App::page_snap(900.0, 100.0, 5), 400.0, "the last page starts on a boundary");
    let _ = fs::remove_dir_all(root);
}

/// C# OpenItemAsync: 압축 안에서 연 항목이 동봉 7-Zip이 읽는 압축 형식이면(예: .iso) 셸로 열지 않고 그 안으로 들어간다.
#[test]
fn nested_archives_of_any_known_format_open_as_panes_like_csharp() {
    let (root, ctx, mut app) = fixture("nested-iso", "en");
    let iso = root.join("left").join("disc.iso");
    fs::write(&iso, b"not really an image").unwrap();
    app.archive_entries_opened(0, root.join("outer.zip"), None, vec![iso.clone()], vec![], true);
    assert_eq!(app.panes[0].archive.as_ref().map(|l| l.file.clone()), Some(iso));
    assert!(launched().is_empty(), "not handed to the shell");
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    // ALZ·EGG는 동봉 7-Zip(UzuCore 7zip+α)이 읽으므로 패널로 연다.
    for name in ["old.alz", "old.egg"] {
        let nested = root.join("left").join(name);
        fs::write(&nested, b"x").unwrap();
        app.archive_entries_opened(1, root.join("outer.zip"), None, vec![nested.clone()], vec![], true);
        assert_eq!(app.panes[1].archive.as_ref().map(|l| l.file.clone()), Some(nested));
        wait_until(&mut app, &ctx, |app| !app.panes[1].loading);
        app.panes[1].archive = None;
    }
    assert!(launched().is_empty(), "ALZ/EGG not handed to the shell");
    // 동봉 7-Zip이 못 읽는 ACE: C# TryGetExternalArchiveProgram 길. 외부 프로그램이 없으면 셸로 열지 않고 C# 오류 글(NDR 경고 상자).
    let ace = root.join("left").join("old.ace");
    fs::write(&ace, b"x").unwrap();
    app.archive_entries_opened(1, root.join("outer.zip"), None, vec![ace.clone()], vec![], true);
    assert!(app.panes[1].archive.is_none());
    assert!(launched().is_empty(), "ACE is not handed to the shell");
    assert!(app.msgboxes.last().is_some_and(|m| m.text.contains(".ace")), "{:?}", app.msgboxes.last().map(|m| m.text.clone()));
    let _ = fs::remove_dir_all(root);
}

/// 동봉 7-Zip이 못 읽는 압축: 외부 압축 프로그램은 쓰지 않으므로 설정이 남아 있어도 열지 않고 알린다.
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

/// 원본 0x439593: 다음 볼륨이 없으면 TfMsgBox(TITLE_ERROR, ARC_CHANGEVOL %s, MB_OKCANCEL | MB_ICONEXCLAMATION). 확인 = 다시 시도, 취소 = 끝.
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

/// C# ApplySingleInstanceModeAsync: 환경 설정의 "한 번에 하나만 실행"은 바로 적용된다. 다른 NDR이 이미 잡고 있으면 이전 값으로 되돌리고 알린다.
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


/// 원본 주소 줄(TNXPathLabel.SetPath) 조각과 패널 테두리 색(검정 스킨: lframelocal 305B91, lframearc 505000, lframeinactive 3C3C3C).
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
    assert!(segments[0].label.ends_with('\\') && segments.iter().all(|s| s.folder.is_some()));
    app.active = 0;
    assert_eq!(app.skin_frame_stroke(&skin, 0), egui::Stroke::new(1.0, Color32::from_rgb(0x30, 0x5B, 0x91)));
    assert_eq!(app.skin_frame_stroke(&skin, 1), egui::Stroke::new(1.0, Color32::from_rgb(0x1C, 0x36, 0x57)), "inactive: 60% lframelocal + 40% lback (000000), FUN_00483300");
    app.panes[0].archive = Some(ArchiveLocation { file: folder.join("x.zip"), prefix: "inner/".into(), password: String::new() });
    assert_eq!(app.skin_frame_stroke(&skin, 0).color, Color32::from_rgb(0x50, 0x50, 0x00));
    let labels: Vec<String> = app.crumb_segments(0).into_iter().map(|s| s.label).collect();
    assert_eq!(&labels[labels.len() - 3..], ["left", "x.zip", "inner"], "archive folders, the archive name, the inner folders");
    let _ = fs::remove_dir_all(root);
}

/// 원본 TNXTabSet(스킨 테마): CalcTabWidth 너비 규칙, 첫 탭 맞춤, 탭이 하나면 줄을 그리지 않음, 탭 그림·글 색.
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
    // 탭 목록 단추(왼쪽 끝 12 × 24)를 누르면 "1. alpha"·"2. beta". 메뉴가 열린 동안 "1"을 치면 첫 탭(원본 "&1. ").
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

/// 원본 +0x344: 숨김 파일을 보이는 패널은 폴더 정보 뒤에 빨간 "H".
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

/// 원본 확장자 정보 줄(라벨 0x350): 글은 비어 있고, 누르면 FUN_0048bd50이 초점 파일의 https://ext.xiles.app/?ext=%s를 연다.
#[test]
fn skin_extension_info_line_opens_the_extension_page() {
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
    let extension = Path::new(&app.panes[0].entries[file].name).extension().unwrap().to_string_lossy().into_owned();
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
    assert_eq!(OPENED.with(|o| o.borrow().clone()), [format!("https://ext.xiles.app/?ext={extension}")]);
    let _ = fs::remove_dir_all(root);
}

/// 원본 폴더 정보 표시줄(FUN_00482700, 20, lfolderback·lfoldertext·lfoldersubtext1)과 파일 정보 표시줄(FUN_00482d44, 20/40, lfileback·lfiletext).
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

/// 원본 DrawListHeader(0x7e2274): 칸 이름 NXFORE×0.4+NXBACK×0.6, 정렬 칸 ×0.6과 맨 위 ▲(오름차순), 세로선 NXLINE×0.3.
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
            let _ = app.skin_list_header(ui, 0, &columns, &widths, egui::FontId::proportional(12.0));
        });
    });
    let titles: Vec<(String, f32, Color32)> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Text(t) => Some((t.galley.text().to_owned(), t.pos.x, t.fallback_color)), _ => None }).collect();
    assert_eq!(titles.first().map(|t| (t.1, t.2)), Some((5.0, Color32::from_gray(0x73))), "the Name column sorts by default: {titles:?}");
    assert!(titles.iter().skip(1).all(|t| t.2 == Color32::from_gray(0x4D)), "{titles:?}");
    let marks: Vec<egui::Rect> = output.shapes.iter().filter_map(|c| match &c.shape { egui::Shape::Rect(r) if r.fill == Color32::from_gray(0x3A) => Some(r.rect), _ => None }).collect();
    assert_eq!(marks.len(), 3, "a 5-3-1 triangle");
    assert_eq!(marks.iter().map(|r| (r.top(), r.width())).collect::<Vec<_>>(), [(2.0, 5.0), (1.0, 3.0), (0.0, 1.0)], "ascending: wide row at the bottom");
    let _ = fs::remove_dir_all(root);
}

/// 원본 드라이브 단추 줄(FUN_004825d8·FUN_00483108): 22 높이, ldrivesback 바탕, 단추는 36씩, 글 ldrivestext.
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
    // 원본 TNXImageButton 글 배치 1: 그림 x = (36 − (간격 2 + 글 너비 + 그림 너비)) div 2, 글 x = 그림 x + 그림 너비 + 2(아이콘을 끄면 그림 너비 0).
    for (i, l) in labels.iter().enumerate() {
        assert_eq!(l.1, 36.0 * i as f32 + ((36.0 - (2.0 + l.2.round())) / 2.0).trunc() + 2.0, "{l:?}");
    }
    assert!(labels.iter().all(|l| l.3 == Color32::from_gray(0x99)), "ldrivestext (Black 999999): {labels:?}");
    let _ = fs::remove_dir_all(root);
}

/// 원본 TNXScrollBar(UpdateThumbWidth·RecalcThumbPos): 엄지 길이 = 쓸 길이 × 보이는 양 / 전체(최소 막대 두께·20), 위치 = 단추 + 비례.
#[test]
fn skin_scrollbar_thumb_follows_the_original_tnxscrollbar() {
    use super::skin_pane::scrollbar_thumb;
    assert_eq!(scrollbar_thumb(300.0, 12.0, 16.0, 0.0, 1000.0, 250.0), (16.0, 67.0), "268 × 250 / 1000, just under the top button");
    assert_eq!(scrollbar_thumb(300.0, 12.0, 16.0, 750.0, 1000.0, 250.0), (217.0, 67.0), "the end: 16 + 268 − 67");
    assert_eq!(scrollbar_thumb(300.0, 12.0, 16.0, 375.0, 1000.0, 250.0).0, 116.0, "half way: 16 + floor(201 / 2)");
    assert_eq!(scrollbar_thumb(300.0, 12.0, 16.0, 0.0, 100_000.0, 250.0).1, 20.0, "never shorter than 20");
    assert_eq!(scrollbar_thumb(40.0, 12.0, 16.0, 0.0, 1000.0, 250.0), (16.0, 8.0), "never longer than the track between the buttons");
}

/// 원본 DrawFocus(0x7d7f54)·GraphUtil HLS·InitSelMarkBitmap·크기 칸 나누기.
/// 원본 가로 TNXScrollBar: 세로 그림을 90° 돌려 그린다(FUN_007cdfc0 시계 방향, FUN_007ce0e4 반시계 방향).
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
    // 시계 방향: 그림 왼쪽 아래 → 왼쪽 위, 왼쪽 위 → 오른쪽 위(위쪽 끝이 오른쪽으로). 반시계 방향: 오른쪽 위 → 왼쪽 위.
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
    // 이름표 200(쓸 폭 184, 첫 몫 = 첫 글 + 20), "..." 10.
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
    // NXSELECT FFFFFF(밝기 240 → 아래 190 = CA), NXBACK 000000. 섞는 값은 원본 single 상수(0.9 = 0.89999998 → 229).
    let (select, back) = (gray(0xFF), gray(0x00));
    assert_eq!(thumbnail_focus(select, back, true, false, false, true), (gray(0x66), gray(0x51), gray(0x99)), "cursor, focused: 0.4 and line 0.6");
    assert_eq!(thumbnail_focus(select, back, true, true, false, true).2, gray(0xE5), "marked cursor line 0.9");
    assert_eq!(thumbnail_focus(select, back, false, true, false, true), (gray(0x33), gray(0x28), gray(0x66)), "marked only: 0.2 and line 0.4");
    assert_eq!(thumbnail_focus(select, back, true, true, false, false).2, gray(0xB2), "unfocused marked cursor line 0.7");
    assert_eq!(thumbnail_focus(select, back, false, true, false, false), (gray(26), gray(20), gray(77)), "unfocused marked: 0.1 and line 0.3");
    assert_eq!(thumbnail_focus(select, back, false, false, true, true), (gray(128), gray(101), gray(255)), "drop target: 0.5 and line 1.0");
    // 어두운 NXSELECT는 위를 밝힌다(밝기 < 50 → +50).
    let (top, bottom, _) = thumbnail_focus(gray(0x10), back, true, false, false, true);
    assert!(top.r() > bottom.r(), "{top:?} {bottom:?}");
    // 자리: 칸 150 × 200, 크기 120, 글 20 → 왼쪽 (150 − 132) div 2 + 1 = 10, 위 1, 130 × 150.
    let cell = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(150.0, 200.0));
    assert_eq!(thumbnail_focus_rect(cell, 120.0, 20.0), egui::Rect::from_min_size(egui::pos2(10.0, 1.0), egui::vec2(130.0, 150.0)));
    // 둥근 사각형(반지름 4): 그러데이션 채우기와 1화소 테두리.
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

/// 원본 bbFunctions(스킨 테마): F2~F8 일곱 칸(같은 너비), 글쇠 글은 bbkeytext(검정 E2A31D), 배경 btnbar_back.bmp 반복.
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
    assert!(keys.iter().enumerate().all(|(i, k)| (k.1.x - (100.0 * i as f32 + 5.0)).abs() < 0.01), "cells of 700 / 7, text at + 5: {keys:?}");
    assert!(keys.iter().all(|k| k.2 == Color32::from_rgb(0xE2, 0xA3, 0x1D)), "bbkeytext");
    let tiles = output.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Mesh(m) if m.texture_id == tile.id())).count();
    assert!(tiles >= 700 / 8, "{tiles}");
    let _ = fs::remove_dir_all(root);
}

/// 원본 FUN_00479970: "[이름]명령"은 [ ] 안, "#내부 명령"은 RES FUNC_<대문자>, 프로그램은 확장자를 뺀 파일 이름.
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
    assert_eq!(original_function_key_caption("C:\\Tools\\my tool.exe", &resource), "my tool");
    assert_eq!(original_function_key_caption("C:\\Tools\\v1.0 beta\\run", &resource), "run");
    assert_eq!(original_function_key_caption("  ", &resource), "");
}

/// 원본 FUN_00444710(스킨 테마): 단추 35×35를 위 tbtopoffset(검정 7)에, 왼쪽 4부터 폭 + 4씩, 구분선 칸 15,
/// align=right 항목은 (너비 − 6 − Σ(폭 + 4))부터. 배경은 toolbar_back.bmp를 가로로 이어 그린다.
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
    // 원본 default\toolbar.xml: 뒤로·앞으로 | 바탕 화면·문서·사진·음악·동영상·즐겨찾기 | 휴지통(오른쪽).
    let commands: Vec<(&str, bool)> = app.toolbar.items.iter().filter(|i| i.kind == ndr::commands::ToolbarKind::Command).map(|i| (i.icon.rsplit('\\').next().unwrap_or(""), i.align_right)).collect();
    assert_eq!(commands, [("tb_back.png", false), ("tb_forward.png", false), ("tb_desktop.png", false), ("tb_documents.png", false), ("tb_pictures.png", false), ("tb_music.png", false), ("tb_videos.png", false), ("tb_favorites.png", false), ("tb_recyclebin.png", true)]);
    assert_eq!(bounds(back.id()).first().copied(), Some(egui::Rect::from_min_size(egui::pos2(4.0, 7.0), egui::vec2(35.0, 35.0))));
    let tiles = bounds(tile.id());
    assert!(tiles.len() >= 1000 / 8 && tiles.iter().all(|r| r.height() == 51.0), "{}", tiles.len());
    // 원본 TNXImageButton(그림 칸 하나): 포인터를 올려도 그대로, 누른 동안 색 × 6 / 10(FUN_007c39e4).
    let at = egui::pos2(20.0, 20.0);
    let frame = |app: &mut App, events: Vec<egui::Event>| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0))), events, ..Default::default() };
        ctx.run(raw, |ctx| {
            egui::TopBottomPanel::top("toolbar").exact_height(51.0).frame(egui::Frame::NONE).show(ctx, |ui| app.skin_toolbar(ui, ctx, &skin));
        })
    };
    let back_color = |output: &egui::FullOutput| output.shapes.iter().find_map(|c| match &c.shape { egui::Shape::Mesh(m) if m.texture_id == back.id() => m.vertices.first().map(|v| v.color), _ => None });
    // 원본 Back 단추(+0xd70): 기록이 없으면 Enabled = False → 상수 알파 0x82.
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

/// 원본 FUN_00444710: Favorites 단추는 DropDownMenu = pmFavorites. 누르면 단추 왼쪽 아래에 즐겨찾기 메뉴가 열린다.
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
    // 왼쪽 묶음: 뒤로 4, 앞으로 43, 구분선 82(15), 바탕 화면 101 … 즐겨찾기 296(너비 35, 위 7).
    let at = egui::pos2(310.0, 20.0);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(&mut app, vec![egui::Event::PointerMoved(at)]);
    frame(&mut app, vec![button(true)]);
    frame(&mut app, vec![button(false)]);
    let popup = app.popup.as_ref().expect("the favorites menu");
    assert_eq!(popup.position, Some(egui::pos2(296.0, 42.0)), "under the button");
    assert!(app.original_dialogs.is_empty(), "not the organize dialog");
    let _ = fs::remove_dir_all(root);
}

/// 원본 TNXLabel 마크업 &lt;b&gt;: "bold" 묶음(Segoe UI Bold)은 보통 꼴보다 넓다.
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

/// 원본 pmMain(DFM): 스킨 테마의 목록 오른쪽 단추 메뉴에는 원본 항목만 있다. C# 테마는 C# 판의 열기·다른 패널로·경로 복사 줄을 둔다.
/// 원본 pVertBar 단추(TNXPngButton, vertbarbtns.png 160×16 = 16×16 칸 10개): 칸은 16×16 그대로 그린다(납작하지 않게).
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
    let _ = fs::remove_dir_all(root);
}

#[test]
fn skin_menus_use_the_original_menu_colours() {
    // 원본 MenuDrawItem(0x42e518)·메뉴 줄(0x430e74): 스킨 [Menu] 색(검정: mback 141414, mtext B4B4B4, mhighlight E6E6E6, mover 000000,
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
    let (bar_back, bar) = super::skin_pane::skin_menu_bar_style(&skin);
    let mut style = egui::Style::default();
    bar.apply(&mut style);
    assert_eq!(bar_back, Color32::from_rgb(0x2A, 0x2A, 0x2A));
    assert_eq!((style.visuals.widgets.inactive.fg_stroke.color, style.visuals.widgets.hovered.weak_bg_fill, style.visuals.widgets.hovered.fg_stroke.color), (Color32::from_rgb(0x9F, 0x9F, 0x9F), Color32::from_rgb(0x33, 0x33, 0x33), Color32::WHITE));
    // 단축키 칸: mrback 000000, mrtext C8C8C8, mrover 5577EE.
    assert_eq!(super::skin_pane::skin_menu_shortcut_colors(&skin), (Color32::BLACK, Color32::from_rgb(0xC8, 0xC8, 0xC8), Color32::from_rgb(0x55, 0x77, 0xEE)));
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

/// 원본 도구 모음 UserCommand(FUN_0044381c, 내부 명령 아님)와 Container(pmToolBarPopup FUN_00444e60 → FUN_00444e04).
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
        super::favorites_dialog::PopupItem::Header(text) => text.clone(),
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
    // 시험용 가짜 임시 폴더(실제 GetTempPathW 폴더는 건드리지 않는다).
    let temp = root.join("fake-temp");
    fs::create_dir_all(temp.join("sub")).unwrap();
    fs::write(temp.join("a.txt"), "12345").unwrap();
    fs::write(temp.join("sub").join("b.txt"), "123").unwrap();
    let locked = temp.join("readonly.txt");
    fs::write(&locked, "1").unwrap();
    let mut permissions = fs::metadata(&locked).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&locked, permissions).unwrap();
    // GetTempPathW가 실패하면(None) 아무것도 하지 않는다.
    TEMP_FOLDER.with(|t| *t.borrow_mut() = None);
    app.dispatch("emptyTemp", &ctx);
    assert!(app.msgboxes.is_empty() && app.original_dialogs.is_empty());
    TEMP_FOLDER.with(|t| *t.borrow_mut() = Some(temp.clone()));
    app.dispatch("emptyTemp", &ctx);
    // 셈은 따로 돈다: 그동안 화면은 멈추지 않고, 끝나면 묻는다.
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    let (title, text) = app.msgboxes.first().map(|m| (m.title.clone(), m.text.clone())).expect("the MSG_EMPTYTEMP question");
    assert_eq!(title, app.language_pack.text("RES.TITLE_EMPTYTEMP", "?"));
    let expected = app.language_pack.text("RES.MSG_EMPTYTEMP", "?").replacen("%d", "3", 1).replacen("%s", &ndr::native::format_byte_size(9), 1);
    assert_eq!(text, expected);
    assert!(app.original_dialogs.is_empty(), "no cleanup scan window (C# flow) any more");
    // 아니요: 그대로.
    app.answer_message_box(ndr::msgbox::IDNO, &ctx);
    assert!(temp.join("a.txt").exists());
    // 예: 안의 항목만 지우고(읽기 전용도 묻지 않고) 폴더는 남긴다.
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
    // 원본 FUN_00459ba8: 루트에서는 위로 가지 않는다.
    assert!(app.current_skin().is_some());
    app.navigate(0, drive_root.clone(), false);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.parent();
    assert_eq!(app.panes[0].path, drive_root, "the original stays at a drive root");
    // 원본 FUN_00486474: "목록에 디스크 드라이브 목록 보기"가 켜져 있으면 폴더 목록 끝에 드라이브 항목.
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
    // 원본 FUN_004351cc 드라이브 갈래: TfGetText(창 제목, MSG_DRIVELABEL, 지금 볼륨 이름) → SetVolumeLabelW.
    app.active = 0;
    app.panes[0].cursor = first_drive;
    app.dispatch("rename", &ctx);
    let (title, message, value, _) = prompt(&app);
    assert_eq!((title.as_str(), message, value.as_str()), ("NDR", app.language_pack.text("RES.MSG_DRIVELABEL", "?"), "OLD"));
    app.answer_dialog(DialogResult::Text(Some("NEW".into())), &ctx);
    assert_eq!(fake_volume_label(&left).as_deref(), Some("NEW"));
    // ERROR_ACCESS_DENIED는 원본 TfMessage(목록 가운데 잠깐 뜨는 알림, 3초), 다른 실패와 빈 이름은 조용히.
    app.dispatch("rename", &ctx);
    app.answer_dialog(DialogResult::Text(Some("deny-me".into())), &ctx);
    assert_eq!(app.pane_message.as_ref().map(|(t, i, _)| (t.clone(), *i)), Some((app.language_pack.text("RES.ACCESS_DENIED", "?"), 0)));
    assert!(app.status.is_empty(), "not on the status line");
    // 원본 Timer1(3000ms): 3초가 지나면 닫힌다.
    app.pane_message.as_mut().unwrap().2 = Instant::now() - Duration::from_millis(3100);
    app.pane_message_ui(&ctx);
    assert!(app.pane_message.is_none());
    app.dispatch("rename", &ctx);
    app.answer_dialog(DialogResult::Text(Some("fail-me".into())), &ctx);
    app.dispatch("rename", &ctx);
    app.answer_dialog(DialogResult::Text(Some(String::new())), &ctx);
    assert!(app.status.is_empty() && app.msgboxes.is_empty() && app.pane_message.is_none());
    assert_eq!(fake_volume_label(&left).as_deref(), Some("NEW"));
    // 준비되지 않은 드라이브(볼륨 정보 없음)는 아무것도 묻지 않는다.
    app.panes[0].cursor = first_drive + 1;
    app.dispatch("rename", &ctx);
    assert!(app.original_dialogs.is_empty());
    // 드라이브 항목을 열면 그 루트로.
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
    // 원본 열 머리글 FIELD_*("종류"는 C# 열: C# 테마에만). 스킨 테마는 원본 여섯 칸.
    let [name, ext, size, date, attr, desc] = ["FIELD_NAME", "FIELD_EXT", "FIELD_SIZE", "FIELD_DATEMODIFIED", "FIELD_ATTR", "FIELD_DESC"].map(|key| app.language_pack.text(&format!("RES.{key}"), "?"));
    assert!(app.current_skin().is_some(), "the default theme is the Black skin");
    let titles: Vec<String> = app.detail_columns(true).into_iter().map(|(_, title, _)| title).collect();
    assert_eq!(titles, [name.clone(), ext.clone(), size.clone(), date.clone(), attr.clone(), desc.clone()]);
    // 원본 DrawListItem 설명 칸: 설명이 없고 showfiletype이면 파일 종류 글.
    app.compute_view(0);
    let file = app.panes[0].entries.iter().position(|e| !e.is_dir && !e.drive).expect("a file");
    let cells = app.panes[0].view.cells[file].clone();
    assert_eq!(cells[DetailColumn::Description.index()], cells[DetailColumn::Kind.index()]);
    // 원본 GetList: 종류 글은 셸 이름(SHGetFileInfoW SHGFI_TYPENAME).
    let entry = &app.panes[0].entries[file];
    assert_eq!(cells[DetailColumn::Kind.index()], native::shell_type_name(ndr::parity::dotnet_extension(&entry.name), false));
    // 원본 FormCloseQuery(0x42a798): 작업 중이면 STILL_HAVE_OPERATION(TITLE_INFO, 0x2124). 아니요 = 그대로, 예 = 멈추고 닫기.
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
    // 원본 BACKHINT/FORWARDHINT: 단추가 열 경로를 넣은 도움말. 기록이 없으면 단추 글.
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
    // 원본 FUN_0042f27c 도움말: 빈 칸 = WORKFOLDER_SETHINT.
    assert_eq!(app.work_folder_hint(2), app.language_pack.text("RES.WORKFOLDER_SETHINT", "?"));
    // 원본 FUN_0042fc84: 빈 칸 누르기 = 묻지 않고 지금 폴더로 지정.
    app.work_folder_click(2);
    assert!(app.msgboxes.is_empty());
    assert_eq!(app.settings.strings("WorkingFolders")[2], left.to_string_lossy());
    assert_eq!(app.work_folder_hint(2), format!("{}\n{}", left.display(), app.language_pack.text("RES.WORKFOLDER_UNSETHINT", "?")));
    // 지정된 칸 누르기 = 그 폴더로.
    app.navigate(0, left.join("folder1"), true);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.work_folder_click(2);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.panes[0].path, left);
    assert_eq!(app.settings.strings("WorkingFolders")[2], left.to_string_lossy(), "opening does not overwrite");
    // 원본 FUN_0042fe60: 오른쪽 단추 = 비우기.
    app.work_folder_clear(2);
    assert!(app.settings.strings("WorkingFolders")[2].is_empty());
    // 원본 pVertBar 단추: 모두 선택 해제, 경로 맞추기(bSyncAddressClick).
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
    // 나눔 막대(C# PaneDivider LeftPaneWidth %, 20~80; 원본 pmiFrameWidthSame = 50%).
    assert!((app.left_pane_ratio() - 0.5).abs() < 1e-6);
    app.set_left_pane_width(30.0);
    assert!((app.left_pane_ratio() - 0.3).abs() < 1e-6);
    app.set_left_pane_width(95.0);
    assert!((app.left_pane_ratio() - 0.8).abs() < 1e-6, "clamped to 80%");
    assert_eq!(app.settings.values.get("RightPaneWidth").and_then(|v| v.as_f64()), Some(20.0));
    app.set_left_pane_width(50.0);
    // 막대 그리기(스킨 띠 그림)도 해 본다. 원본 iVertThumb: 막대 아래(높이 − 25 칸) 가운데에 vertthumb.png 원래 크기.
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
    assert!((drawn.center().x - bar.center().x).abs() < 0.01 && (drawn.center().y - (bar.max.y - 25.0 + 11.0)).abs() < 0.01, "{drawn:?} in {bar:?}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn control_panel_and_add_remove_programs_follow_original() {
    let (root, ctx, mut app) = fixture("system-menu", "ko");
    launched();
    VERBS.with(|v| v.borrow_mut().clear());
    // 원본 miControlPanelClick: control.exe. miAddRemovePorgClick(0x432fc8): rundll32.exe "shell32.dll,Control_RunDLL appwiz.cpl".
    app.dispatch("controlPanel", &ctx);
    assert_eq!(VERBS.with(|v| v.borrow().clone()), vec![(PathBuf::from("control.exe"), "open".to_owned())]);
    app.dispatch("addRemovePrograms", &ctx);
    assert_eq!(launched(), vec![("rundll32.exe".to_owned(), "shell32.dll,Control_RunDLL appwiz.cpl".to_owned(), PathBuf::new())]);
    let _ = fs::remove_dir_all(root);
}
