//! C# RemoteWindow, RemoteSitesWindow, RemoteSiteEditorWindow, RemotePermissionsWindow, RunRemoteCommandAsync를 Rust UI에서 확인한다.
//! 한국어와 영어만. 서버 연결은 가짜 함수(설정만 적어 두고 실패를 돌려준다), 실제 전송은 맨 아래 무시된 시험이 이 시험이 띄운
//! 127.0.0.1 전용 pyftpdlib 서버(격리 폴더)로만 한다.
use super::original_dialogs::{DialogKind, DialogResult};
use super::remote_window::{self as rw, RemoteAction};
use super::remote_sites::SitesAction;
use super::*;
use ndr::remote::{Client, Config, RemoteEntry};
use std::fs;

static REMOTE_LOCK: Mutex<()> = Mutex::new(());
static CONNECTS: Mutex<Vec<Config>> = Mutex::new(Vec::new());

fn connects() -> Vec<Config> {
    std::mem::take(&mut *CONNECTS.lock().unwrap_or_else(|e| e.into_inner()))
}
/// 가짜 연결: 설정을 적어 두고, 호스트에 따라 서버 키 확인 또는 실패를 돌려준다.
fn fake_connect(config: &Config) -> Result<Client, String> {
    CONNECTS.lock().unwrap_or_else(|e| e.into_inner()).push(config.clone());
    if config.host == "key.invalid" && config.fingerprint.is_empty() {
        return Err(serde_json::json!({"kind": "unknown_host_key", "host": config.host, "port": config.port, "candidate": "SHA256:abc", "message": "verify"}).to_string());
    }
    Err(if config.host == "key.invalid" { "auth failed after key".into() } else { "530 Login incorrect.".into() })
}
fn no_files(_: &str, _: Option<&Path>) -> Vec<PathBuf> {
    vec![]
}

/// 원본 TfFTPAttributes FormKeyDown(0x427334): End(숫자 칸 eAttr에 초점이 없을 때) = mrAll(모두).
#[test]
fn ftp_attributes_end_key_applies_to_all() {
    let (root, ctx, app) = fixture("ftp-attr-end", "ko");
    let mut form = rw::PermissionForm::new(0o644, "a.txt (1 / 2)", "/a.txt");
    let mut result = None;
    let input = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::End, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }], ..Default::default() };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| result = rw::permission_ui(ui, &mut form, &app.language_pack, false, false, false));
    });
    assert!(matches!(result, Some(Some(rw::PermissionAction::ApplyAllRemaining))));
    let _ = fs::remove_dir_all(root);
}

fn fixture(label: &str, language: &str) -> (PathBuf, egui::Context, App) {
    let root = std::env::temp_dir().join(format!("ndr-remote-{}-{}-{label}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let (left, right) = (root.join("left"), root.join("right"));
    fs::create_dir_all(left.join("folder1")).unwrap();
    fs::create_dir_all(&right).unwrap();
    fs::write(left.join("alpha.txt"), b"alpha").unwrap();
    let ctx = egui::Context::default();
    let options = Options { profile: Some(root.join("profile")), left: Some(left), right: Some(right), language: Some(language.into()), ..Default::default() };
    let mut app = App::from_context(&ctx, options);
    app.clipboard_writer = |_| Ok(());
    app.shell_runner = |_, _, _| Ok(());
    app.save_dialog = |_, _, _, _, _, _| None;
    app.open_dialog = |_, _, _, _| None;
    app.folder_picker = |_, _| None;
    app.remote_hooks = rw::RemoteHooks { connect: fake_connect, open_files: no_files, pane_connect: |_| Err("no network in tests".into()) };
    wait_until(&mut app, &ctx, |app| app.panes.iter().all(|p| !p.loading));
    connects();
    (root, ctx, app)
}
fn wait_until(app: &mut App, ctx: &egui::Context, done: impl Fn(&mut App) -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    while !done(app) {
        assert!(Instant::now() < end, "timeout");
        app.poll(ctx);
        thread::sleep(Duration::from_millis(5));
    }
}
fn frame(app: &mut App, ctx: &egui::Context) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        app.original_dialog_ui(ctx);
        app.msgbox_ui(ctx);
    });
}
fn boxed(app: &App) -> (String, String) {
    let first = app.msgboxes.first().expect("a message box");
    (first.title.clone(), first.text.clone())
}
fn form(app: &mut App) -> &mut rw::RemoteForm {
    app.remote_form().expect("remote window")
}
fn wait_local(app: &mut App, ctx: &egui::Context) {
    wait_until(app, ctx, |app| app.remote_form().is_some_and(|f| !f.local_loading));
}
fn select_local(app: &mut App, name: &str) {
    let f = form(app);
    let index = f.local_items.iter().position(|r| r.name == name).unwrap_or_else(|| panic!("{name} not listed"));
    f.local_grid.selected = [index].into_iter().collect();
    f.local_grid.cursor = Some(index);
}

#[test]
fn remote_window_follows_the_csharp_connection_and_local_flows() {
    let _serial = REMOTE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (root, ctx, mut app) = fixture("ko", "ko");
    let left = root.join("left");
    app.dispatch("remote", &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "NDR 원격 연결");
    wait_local(&mut app, &ctx);
    {
        let f = form(&mut app);
        assert_eq!((f.protocol, f.port.as_str(), f.status.as_str(), f.remote_path.as_str()), (1, "21", "호스트에 연결하세요.", "/"), "FTPS is the first protocol");
        assert_eq!(f.local_path, left.to_string_lossy());
        let names: Vec<&str> = f.local_items.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["..", "folder1", "alpha.txt"], "parent, folders, files");
        assert_eq!(f.local_items[0].kind, "상위 폴더");
        assert!(f.quick_options_visible(), "quick-connect options for FTP/FTPS without a site");
    }
    frame(&mut app, &ctx);
    // 로컬: 폴더 열기, 뒤로/앞으로, 위로, "Computer" = 드라이브 목록.
    select_local(&mut app, "folder1");
    app.remote_action(RemoteAction::LocalOpen, &ctx);
    wait_local(&mut app, &ctx);
    assert_eq!(form(&mut app).local_path, left.join("folder1").to_string_lossy());
    app.remote_action(RemoteAction::LocalBack, &ctx);
    wait_local(&mut app, &ctx);
    assert_eq!(form(&mut app).local_path, left.to_string_lossy());
    app.remote_action(RemoteAction::LocalForward, &ctx);
    wait_local(&mut app, &ctx);
    assert_eq!(form(&mut app).local_path, left.join("folder1").to_string_lossy());
    app.remote_action(RemoteAction::LocalUp, &ctx);
    wait_local(&mut app, &ctx);
    assert_eq!(form(&mut app).local_path, left.to_string_lossy());
    form(&mut app).local_path_box = "computer".into();
    app.remote_action(RemoteAction::LocalGo, &ctx);
    wait_local(&mut app, &ctx);
    {
        let f = form(&mut app);
        assert_eq!((f.local_path.as_str(), f.local_path_box.as_str()), (rw::DRIVES, "Computer"));
        assert!(!f.local_items.is_empty() && f.local_items.iter().all(|r| r.is_drive));
    }
    form(&mut app).local_path_box = root.join("missing").to_string_lossy().into_owned();
    app.remote_action(RemoteAction::LocalGo, &ctx);
    assert_eq!(boxed(&app).0, "로컬 폴더");
    app.answer_message_box(1, &ctx);
    assert_eq!(form(&mut app).local_path_box, "Computer", "a missing folder keeps the current path");
    app.remote_action(RemoteAction::LocalBack, &ctx);
    wait_local(&mut app, &ctx);
    // 연결 전 내려받기·올리기.
    app.remote_action(RemoteAction::Download, &ctx);
    assert_eq!(boxed(&app), (String::new(), "먼저 원격 서버에 연결하세요.".to_owned()));
    app.answer_message_box(1, &ctx);
    app.remote_action(RemoteAction::UploadSelected, &ctx);
    assert_eq!(boxed(&app).1, "먼저 원격 서버에 연결하세요.");
    app.answer_message_box(1, &ctx);
    // SFTP는 호스트·사용자·포트를 검사한다.
    app.remote_action(RemoteAction::Protocol(2), &ctx);
    assert_eq!(form(&mut app).port, "22");
    form(&mut app).host = "sftp.invalid".into();
    app.remote_action(RemoteAction::Connect, &ctx);
    assert_eq!(boxed(&app), ("원격 연결".to_owned(), "호스트, 사용자 이름, 올바른 포트를 입력하세요.".to_owned()));
    app.answer_message_box(1, &ctx);
    assert!(connects().is_empty());
    // 빠른 FTP 연결 + 사이트 목록에 추가: 연결 전에 입력값을 사이트로 저장한다.
    app.remote_action(RemoteAction::Protocol(0), &ctx);
    {
        let f = form(&mut app);
        (f.host, f.port, f.user, f.password, f.add_to_sites, f.passive) = ("ftp://deny.invalid".into(), "2121".into(), "me".into(), "pw".into(), true, false);
    }
    app.remote_action(RemoteAction::Connect, &ctx);
    assert_eq!(app.status, "FTP 사이트를 저장했습니다: ftp://deny.invalid");
    let sites = rw::load_sites(&app.settings);
    assert_eq!(sites.len(), 1);
    assert_eq!((sites[0].name.as_str(), sites[0].host.as_str(), sites[0].port, sites[0].remote_directory.as_str(), sites[0].passive, sites[0].encoding_name.as_str()), ("ftp://deny.invalid", "ftp://deny.invalid", 2121, "", false, "UTF-8"));
    assert_eq!(native::unprotect_secret(&sites[0].encrypted_password).unwrap(), "pw");
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    assert_eq!(boxed(&app), ("원격 연결 실패".to_owned(), "530 Login incorrect.".to_owned()));
    app.answer_message_box(1, &ctx);
    assert_eq!(form(&mut app).status, "연결하지 못했습니다.");
    let config = connects().pop().unwrap();
    assert_eq!((config.protocol.as_str(), config.host.as_str(), config.port, config.user.as_str(), config.passive), ("FTP", "deny.invalid", 2121, "me", false), "PrepareHost strips ftp://");
    // FTP를 골라도 포트가 22면 SFTP로 빠른 연결, 포트 0은 연결하지 않는다.
    form(&mut app).add_to_sites = false;
    form(&mut app).port = "22".into();
    app.remote_action(RemoteAction::Connect, &ctx);
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    app.answer_message_box(1, &ctx);
    assert_eq!(connects().pop().unwrap().protocol, "SFTP");
    form(&mut app).port = "0".into();
    app.remote_action(RemoteAction::Connect, &ctx);
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    assert_eq!(boxed(&app), ("원격 연결 실패".to_owned(), "포트 0에는 연결할 수 없습니다.".to_owned()));
    app.answer_message_box(1, &ctx);
    assert!(connects().is_empty());
    // 범위 밖 포트는 16비트로 잘라 연결하지 않는다.
    form(&mut app).port = "99999".into();
    app.remote_action(RemoteAction::Connect, &ctx);
    assert!(!app.msgboxes.is_empty(), "an out-of-range FTP port is refused at once");
    app.answer_message_box(1, &ctx);
    assert!(connects().is_empty());
    // SFTP 서버 키: 처음 보는 키면 묻고, 예면 그 키로 다시 연결한다.
    app.remote_action(RemoteAction::Protocol(2), &ctx);
    {
        let f = form(&mut app);
        (f.host, f.user) = ("key.invalid".into(), "u".into());
    }
    app.remote_action(RemoteAction::Connect, &ctx);
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    assert_eq!(boxed(&app), ("SFTP 서버 확인".to_owned(), "호스트: key.invalid:22\r\nSHA-256: SHA256:abc\r\n\r\n이 키를 신뢰하고 연결할까요?".to_owned()));
    app.answer_message_box(6, &ctx);
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    assert_eq!(boxed(&app).1, "auth failed after key");
    app.answer_message_box(1, &ctx);
    let tries = connects();
    assert_eq!(tries.iter().map(|c| c.fingerprint.as_str()).collect::<Vec<_>>(), ["", "SHA256:abc"]);
    app.remote_action(RemoteAction::Close, &ctx);
    assert!(app.original_dialogs.is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn remote_helpers_follow_nexusfile_rules() {
    assert_eq!(rw::prepare_host("ftp://host/x"), "host/x");
    assert_eq!(rw::prepare_host("xftp://host"), "/host", "the native copies from position 7 wherever ftp:// appears");
    assert_eq!(rw::prepare_host("host"), "host");
    assert_eq!((rw::parse_port(" 2121"), rw::parse_port("abc"), rw::parse_port("-5")), (2121, 21, -5));
    assert_eq!((rw::parent_path("/a/b/"), rw::parent_path("/a"), rw::parent_path("/")), ("/a".to_owned(), "/".to_owned(), "/".to_owned()));
    let entries = [
        RemoteEntry { name: "b.txt".into(), size: 2048, modified: Some(0), mode: Some(0o644), ..Default::default() },
        RemoteEntry { name: "A".into(), is_dir: true, ..Default::default() },
    ];
    let rows = rw::remote_rows("/srv", &entries, true);
    assert_eq!(rows.iter().map(|r| (r.name.as_str(), r.kind.as_str(), r.remote_path.as_str())).collect::<Vec<_>>(), [("..", "상위 폴더", ""), ("A", "폴더", "/srv/A"), ("b.txt", "파일", "/srv/b.txt")]);
    assert_eq!((rows[2].size_text.as_str(), rows[2].mode), ("2.0 KB", Some(0o644)));
    assert!(rw::remote_rows("/", &entries, false).iter().all(|r| !r.is_parent), "no parent row at /");
    // RemotePermissionsWindow: 비트 ↔ 숫자, 세 자리이고 778보다 작을 때만 확인란을 맞춘다.
    let mut permissions = rw::PermissionForm::new(0o4754, "a", "/a");
    assert_eq!((permissions.mode_text.as_str(), permissions.mode()), ("754", 0o754));
    permissions.mode_text = "640".into();
    permissions.text_changed();
    assert_eq!(permissions.mode(), 0o640);
    permissions.mode_text = "799".into();
    permissions.text_changed();
    assert_eq!(permissions.mode(), 0o640, "778 and above are ignored");
    permissions.mode_text = "689".into();
    permissions.text_changed();
    assert_eq!(permissions.mode(), 0o640, "digits 8 and 9 are not octal and are ignored");
    permissions.mode_text = "6x".into();
    permissions.text_changed();
    assert_eq!(permissions.mode_text, "6");
    assert_eq!(rw::numeric_value(0o755), 755);
}

#[test]
fn remote_sites_editor_and_command_follow_the_csharp_flows() {
    let _serial = REMOTE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (root, ctx, mut app) = fixture("sites", "en");
    // 원격 명령: 사이트가 없으면 RunSafely 경고.
    app.dispatch("remoteCommand", &ctx);
    assert_eq!(boxed(&app), ("NDR".to_owned(), "Save a remote site from the Network menu first.".to_owned()));
    app.answer_message_box(1, &ctx);
    app.dispatch("sites", &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "FTP Site Manager");
    // 추가: TfGetText(FTPSITE_ADD) → 새 줄(FTP, 21, 수동 모드).
    app.sites_action(SitesAction::Add, &ctx);
    let dialog = app.original_dialogs.last().unwrap();
    assert!(matches!(&dialog.kind, DialogKind::Prompt { message, .. } if message == "Type a FTP site name."));
    app.answer_dialog(DialogResult::Text(Some("alpha".into())), &ctx);
    {
        let f = app.sites_form().unwrap();
        assert_eq!((f.sites.len(), f.selected, f.protocol.as_str(), f.port.as_str(), f.passive), (1, Some(0), "FTP", "21", true));
    }
    // 이름이나 주소가 비면 FTPSITE_INVALIDINFO(TfMsgBox, 별표 아이콘).
    app.sites_action(SitesAction::Save, &ctx);
    assert_eq!(boxed(&app), ("FTP Site Manager".to_owned(), "Site name or address is empty.".to_owned()));
    assert_eq!(app.msgboxes[0].flags, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONASTERISK);
    app.answer_message_box(1, &ctx);
    {
        let f = app.sites_form().unwrap();
        (f.host, f.user, f.password, f.port) = ("ftp.example.invalid".into(), "me".into(), "secret".into(), "2121".into());
    }
    app.sites_action(SitesAction::Add, &ctx);
    app.answer_dialog(DialogResult::Text(Some("ALPHA".into())), &ctx);
    assert_eq!(boxed(&app).1, "A site has same name already exists.");
    app.answer_message_box(1, &ctx);
    // 추가 설정(편집 창): SFTP로 바꾸면 포트 22, 저장한 키는 같은 SFTP·호스트·포트일 때만 남는다.
    app.sites_action(SitesAction::Advanced, &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "Remote site");
    if let Some(DialogKind::SiteEditor(editor)) = app.original_dialogs.last_mut().map(|d| &mut d.kind) {
        assert_eq!((editor.host.as_str(), editor.port.as_str(), editor.password.as_str()), ("ftp.example.invalid", "2121", "secret"));
        editor.protocol = "SFTP".into();
        editor.port = "22".into();
        editor.fingerprint = "SHA256:old".into();
        editor.encoding = " ".into();
        editor.key_path = root.join("missing.key").to_string_lossy().into_owned();
    }
    app.site_editor_action(super::remote_sites::EditorAction::Save, &ctx);
    assert_eq!(boxed(&app), (String::new(), "The private key file was not found.".to_owned()));
    app.answer_message_box(1, &ctx);
    if let Some(DialogKind::SiteEditor(editor)) = app.original_dialogs.last_mut().map(|d| &mut d.kind) {
        editor.key_path.clear();
    }
    app.site_editor_action(super::remote_sites::EditorAction::Save, &ctx);
    {
        let f = app.sites_form().unwrap();
        let site = &f.sites[0];
        assert_eq!((site.protocol.as_str(), site.port, site.encoding_name.as_str(), site.known_host_fingerprint.as_str()), ("SFTP", 22, "UTF-8", ""), "a changed protocol forgets the host key");
        assert_eq!((f.protocol.as_str(), f.port.as_str()), ("SFTP", "22"));
    }
    app.sites_action(SitesAction::Save, &ctx);
    assert!(app.original_dialogs.is_empty());
    let sites = rw::load_sites(&app.settings);
    assert_eq!((sites.len(), sites[0].name.as_str(), sites[0].user.as_str()), (1, "alpha", "me"));
    // 연결: 저장하고 그 사이트로 원격 창을 열어 바로 연결한다.
    app.dispatch("sites", &ctx);
    app.sites_action(SitesAction::Connect, &ctx);
    let dialog = app.original_dialogs.last().unwrap();
    assert!(matches!(&dialog.kind, DialogKind::Remote(f) if f.protocol == 2 && f.host == "ftp.example.invalid" && f.user == "me" && f.password == "secret" && !f.quick_options_visible()));
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    assert_eq!(boxed(&app), ("Remote connection failed".to_owned(), "530 Login incorrect.".to_owned()));
    app.answer_message_box(1, &ctx);
    let config = connects().pop().unwrap();
    assert_eq!((config.protocol.as_str(), config.port, config.name.as_str()), ("SFTP", 22, "alpha"));
    app.remote_action(RemoteAction::Close, &ctx);
    // 삭제: FTPSITE_DELCONFIRM 예/아니요.
    app.dispatch("sites", &ctx);
    app.sites_action(SitesAction::Delete, &ctx);
    assert_eq!(boxed(&app).1, "Are you sure you want to delete a selected site?");
    app.answer_message_box(7, &ctx);
    assert_eq!(app.sites_form().unwrap().sites.len(), 1);
    app.sites_action(SitesAction::Cancel, &ctx);
    // 원격 명령: 사이트 고르기 → 명령(처음 값 pwd) → 연결 실패는 경고 상자.
    app.dispatch("remoteCommand", &ctx);
    let dialog = app.original_dialogs.last().unwrap();
    assert!(matches!(&dialog.kind, DialogKind::Choice(c) if c.options == ["alpha — SFTP ftp.example.invalid:22"]));
    app.answer_dialog(DialogResult::Choice(Some(0)), &ctx);
    let dialog = app.original_dialogs.last().unwrap();
    assert_eq!(dialog.title, "Remote command");
    assert!(matches!(&dialog.kind, DialogKind::Prompt { message, value, .. } if message == "Command to run on alpha (SFTP)" && value == "pwd"));
    app.answer_dialog(DialogResult::Text(Some("ls -l".into())), &ctx);
    wait_until(&mut app, &ctx, |app| !app.msgboxes.is_empty());
    assert_eq!(boxed(&app), ("NDR".to_owned(), "530 Login incorrect.".to_owned()));
    fs::remove_dir_all(root).unwrap();
}

/// 시험 전용 FTP 서버(127.0.0.1, 격리 폴더, 사용자 user/pass).
const FTP_SERVER: &str = r#"
import sys
from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import FTPHandler
from pyftpdlib.servers import FTPServer
authorizer = DummyAuthorizer()
authorizer.add_user("user", "pass", sys.argv[1], perm="elradfmwMT")
FTPHandler.authorizer = authorizer
server = FTPServer(("127.0.0.1", 0), FTPHandler)
print(server.address[1], flush=True)
server.serve_forever()
"#;

#[test]
#[ignore = "owned 127.0.0.1 FTP server; requires Python and pyftpdlib"]
fn remote_window_transfers_against_an_owned_ftp_server() {
    use std::io::BufRead;
    let _serial = REMOTE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (root, ctx, mut app) = fixture("live", "ko");
    app.remote_hooks.connect = Client::connect;
    let served = root.join("served");
    fs::create_dir_all(served.join("docs")).unwrap();
    fs::write(served.join("docs").join("readme.txt"), b"hello").unwrap();
    let mut child = std::process::Command::new("python").args(["-u", "-c", FTP_SERVER]).arg(&served).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let port = line.trim().to_owned();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.dispatch("remote", &ctx);
        wait_local(&mut app, &ctx);
        app.remote_action(RemoteAction::Protocol(0), &ctx);
        {
            let f = form(&mut app);
            (f.host, f.port, f.user, f.password) = ("127.0.0.1".into(), port.clone(), "user".into(), "pass".into());
        }
        app.remote_action(RemoteAction::Connect, &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.listing_valid) || !app.msgboxes.is_empty());
        assert!(app.msgboxes.is_empty(), "{:?}", app.msgboxes.first().map(|m| m.text.clone()));
        assert_eq!(app.settings.strings("RecentRemoteHosts"), ["127.0.0.1"]);
        {
            let f = form(&mut app);
            assert_eq!(f.remote_path, "/");
            assert_eq!(f.status, "/ · 1개 항목");
            assert_eq!(f.items.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["docs"]);
            f.remote_grid.selected = [0].into_iter().collect();
            f.remote_grid.cursor = Some(0);
            f.focus_remote = true;
        }
        // 폴더 열기 → 뒤로.
        app.remote_action(RemoteAction::RemoteOpen, &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none() && f.remote_path == "/docs" && f.listing_valid));
        assert_eq!(form(&mut app).items.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["..", "readme.txt"]);
        // 내려받기: 같은 이름이 없으면 묻지 않고 받는다.
        {
            let f = form(&mut app);
            f.remote_grid.selected = [1].into_iter().collect();
        }
        app.remote_action(RemoteAction::Download, &ctx);
        assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::RemoteTransfer(_))));
        frame(&mut app, &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none()) && !app.original_dialogs.iter().any(|d| matches!(d.kind, DialogKind::RemoteTransfer(_))));
        let left = root.join("left");
        assert_eq!(fs::read(left.join("readme.txt")).unwrap(), b"hello");
        assert_eq!(form(&mut app).status, "1개 항목을 다운로드했습니다.");
        // 다시 받으면 이름 충돌(예/아니요/취소), 아니요면 건너뛴다.
        app.remote_action(RemoteAction::Download, &ctx);
        assert_eq!(boxed(&app), ("이름 충돌".to_owned(), "로컬 폴더에 같은 이름의 파일 1개가 있습니다. 덮어쓸까요?".to_owned()));
        app.answer_message_box(7, &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none()) && !app.original_dialogs.iter().any(|d| matches!(d.kind, DialogKind::RemoteTransfer(_))));
        assert_eq!(form(&mut app).status, "1개 항목을 다운로드했습니다.", "skipped items count as completed");
        // 올리기(▲): 로컬 alpha.txt.
        wait_local(&mut app, &ctx);
        select_local(&mut app, "alpha.txt");
        app.remote_action(RemoteAction::UploadSelected, &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none() && f.listing_valid) && !app.original_dialogs.iter().any(|d| matches!(d.kind, DialogKind::RemoteTransfer(_))));
        assert_eq!(fs::read(served.join("docs").join("alpha.txt")).unwrap(), b"alpha");
        assert_eq!(form(&mut app).status, "1개 파일을 업로드했습니다.");
        // 새 원격 폴더, 이름 바꾸기, 삭제.
        app.remote_action(RemoteAction::NewFolder, &ctx);
        app.answer_dialog(DialogResult::Text(Some("made".into())), &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none() && f.items.iter().any(|r| r.name == "made")));
        assert!(served.join("docs").join("made").is_dir());
        let index = form(&mut app).items.iter().position(|r| r.name == "made").unwrap();
        {
            let f = form(&mut app);
            f.remote_grid.selected = [index].into_iter().collect();
            f.remote_grid.cursor = Some(index);
        }
        app.remote_action(RemoteAction::Rename, &ctx);
        app.answer_dialog(DialogResult::Text(Some("renamed".into())), &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none() && f.items.iter().any(|r| r.name == "renamed")));
        let index = form(&mut app).items.iter().position(|r| r.name == "renamed").unwrap();
        form(&mut app).remote_grid.selected = [index].into_iter().collect();
        app.remote_action(RemoteAction::Delete, &ctx);
        assert_eq!(boxed(&app), ("원격 삭제".to_owned(), "원격 항목 1개를 서버에서 삭제할까요?".to_owned()));
        app.answer_message_box(6, &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none() && f.listing_valid && !f.items.iter().any(|r| r.name == "renamed")));
        assert!(!served.join("docs").join("renamed").exists());
        // 권한: 권한 창(확인) → "1/1개 항목 권한 변경, 건너뜀 0개".
        let index = form(&mut app).items.iter().position(|r| r.name == "alpha.txt").unwrap();
        form(&mut app).remote_grid.selected = [index].into_iter().collect();
        app.remote_action(RemoteAction::Permissions, &ctx);
        if !app.msgboxes.is_empty() {
            assert_eq!(boxed(&app).1, "서버가 이 항목의 기존 권한을 알려주지 않습니다. 기본값 644로 편집할까요?");
            app.answer_message_box(6, &ctx);
        }
        assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::RemotePermissions(_))));
        frame(&mut app, &ctx);
        app.answer_dialog(DialogResult::RemotePermissions(Some((rw::PermissionAction::ApplyCurrent, 0o644))), &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none() && f.listing_valid) && app.msgboxes.is_empty());
        assert_eq!(form(&mut app).status, "1/1개 항목 권한 변경, 건너뜀 0개");
        // 닫기: 내려받은 것이 있으므로 연 패널을 그 로컬 폴더로.
        app.remote_action(RemoteAction::Close, &ctx);
        assert!(app.original_dialogs.is_empty());
    }));
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_dir_all(&root);
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}


/// 시험용 메모리 FTP(경로 → (폴더인가, 내용)). 패널 FTP 모드 시험만 쓴다.
static FAKE_FTP: Mutex<std::collections::BTreeMap<String, (bool, Vec<u8>)>> = Mutex::new(std::collections::BTreeMap::new());
static FAKE_FTP_CONFIGS: Mutex<Vec<Config>> = Mutex::new(Vec::new());
static FAKE_CHMODS: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());

struct FakeFtp;
impl super::ftp_pane::PaneFs for FakeFtp {
    fn pwd(&mut self) -> Result<String, String> {
        log::trace!(target: "suppaftp::sync_ftp", "CC OUT: PWD");
        Ok("/".into())
    }
    fn change_directory(&mut self, _current: &str, path: &str) -> Result<(String, Option<String>), String> {
        Ok((path.to_owned(), None))
    }
    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>, String> {
        log::trace!(target: "suppaftp::sync_ftp", "CC OUT: LIST");
        let store = FAKE_FTP.lock().unwrap();
        let prefix = if path.ends_with('/') { path.to_owned() } else { format!("{path}/") };
        Ok(store
            .iter()
            .filter_map(|(key, (dir, data))| {
                let rest = key.strip_prefix(&prefix)?;
                (!rest.is_empty() && !rest.contains('/')).then(|| RemoteEntry { name: rest.to_owned(), is_dir: *dir, size: data.len() as u64, mode: Some(if *dir { 0o755 } else { 0o644 }), ..Default::default() })
            })
            .collect())
    }
    fn download(&mut self, remote: &str, local: &Path, _cancel: &AtomicBool, _resolve: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, progress: &mut dyn FnMut(ndr::remote::TransferProgress)) -> Result<(), String> {
        let (dir, data) = FAKE_FTP.lock().unwrap().get(remote).cloned().ok_or("no such remote file")?;
        if dir {
            return Err("folders are not used in this test".into());
        }
        fs::write(local, &data).map_err(|e| e.to_string())?;
        progress(ndr::remote::TransferProgress { path: remote.to_owned(), bytes: data.len() as u64, files: 1 });
        Ok(())
    }
    fn upload(&mut self, local: &Path, remote: &str, _cancel: &AtomicBool, _resolve: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, _progress: &mut dyn FnMut(ndr::remote::TransferProgress)) -> Result<(), String> {
        let data = fs::read(local).map_err(|e| e.to_string())?;
        FAKE_FTP.lock().unwrap().insert(remote.to_owned(), (false, data));
        Ok(())
    }
    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        let mut store = FAKE_FTP.lock().unwrap();
        if store.contains_key(path) {
            return Err("550 already exists".into());
        }
        store.insert(path.to_owned(), (true, Vec::new()));
        Ok(())
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        let mut store = FAKE_FTP.lock().unwrap();
        let moved: Vec<(String, (bool, Vec<u8>))> = store.iter().filter(|(k, _)| *k == from || k.starts_with(&format!("{from}/"))).map(|(k, v)| (k.clone(), v.clone())).collect();
        if moved.is_empty() {
            return Err("550 not found".into());
        }
        for (key, value) in moved {
            store.remove(&key);
            store.insert(format!("{to}{}", &key[from.len()..]), value);
        }
        Ok(())
    }
    fn delete(&mut self, path: &str, _is_dir: bool, _cancel: &AtomicBool) -> Result<(), String> {
        FAKE_FTP.lock().unwrap().retain(|k, _| k != path && !k.starts_with(&format!("{path}/")));
        Ok(())
    }
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), String> {
        FAKE_CHMODS.lock().unwrap().push((path.to_owned(), mode));
        Ok(())
    }
    fn command(&mut self, command: &str) -> Result<String, String> {
        Ok(format!("200 {command} ok\r\n"))
    }
    fn close(&mut self) {}
}
fn fake_pane_connect(config: &Config) -> Result<Box<dyn super::ftp_pane::PaneFs>, String> {
    FAKE_FTP_CONFIGS.lock().unwrap().push(config.clone());
    Ok(Box::new(FakeFtp))
}

/// 원본 패널 FTP 모드: 사이트 메뉴(FUN_00440734 "&1. 이름")로 지금 패널을 FTP로, 폴더 열기, 다른 패널로 복사 = 내려받기/올리기
/// (FUN_0042d518), FTP 패널에서 로컬 명령 막기, 끊기(FUN_0043e8d4)로 로컬 폴더, 양쪽 로컬이면 올리기는 FTP_PUTCRITERIA.
#[test]
fn ftp_pane_mode_connects_lists_transfers_and_disconnects_like_the_original() {
    let _guard = REMOTE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (root, ctx, mut app) = fixture("ftp-pane", "ko");
    app.remote_hooks.pane_connect = fake_pane_connect;
    {
        let mut store = FAKE_FTP.lock().unwrap();
        store.clear();
        store.insert("/pub".into(), (true, Vec::new()));
        store.insert("/pub/a.txt".into(), (false, b"hello".to_vec()));
        store.insert("/readme.txt".into(), (false, b"r".to_vec()));
        store.insert("/.profile".into(), (false, b"p".to_vec()));
    }
    let site = rw::SiteProfile { name: "home".into(), host: "ftp.example".into(), user: "u".into(), encrypted_password: native::protect_secret("pw").unwrap_or_default(), remote_directory: String::new(), ..Default::default() };
    rw::store_sites(&mut app.settings, &[site]);
    assert_eq!(app.ftp_site_items(), vec![(0, "&1. home".to_owned())]);
    assert_eq!(super::ftp_pane::site_menu_caption(10, "k"), "&A. k");
    assert_eq!(super::ftp_pane::site_menu_caption(35, "z"), "36. z");
    let left = app.panes[0].path.clone();
    app.active = 0;
    app.dispatch("ftpConnect:0", &ctx);
    wait_until(&mut app, &ctx, |app| app.panes[0].remote.is_some() && !app.panes[0].loading);
    let config = FAKE_FTP_CONFIGS.lock().unwrap().pop().expect("connected");
    assert_eq!((config.host.as_str(), config.user.as_str(), config.password.as_str()), ("ftp.example", "u", "pw"));
    let names = |app: &App, i: usize| app.panes[i].entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&app, 0), ["pub", "readme.txt"]);
    // 원본 FUN_0045bb0c: 숨김 보기가 아니면 '.'으로 시작하는 원격 이름을 넣지 않고, 숨김 보기면 숨김 속성으로 넣는다.
    app.panes[0].show_hidden = true;
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(names(&app, 0), ["pub", ".profile", "readme.txt"]);
    let profile = app.panes[0].entries.iter().find(|e| e.name == ".profile").unwrap();
    assert!(profile.hidden && profile.attributes & 0x2 != 0);
    app.panes[0].show_hidden = false;
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(names(&app, 0), ["pub", "readme.txt"]);
    assert_eq!(app.address_text(0), "ftp://ftp.example/");
    assert!(app.panes[0].disk.is_none(), "the original hides the disk info in FTP mode");
    // 원본 FUN_004408f8: SFTP가 아니면 패널 아래 FTP 기록 칸(FUN_0048d7ac, 높이 60)을 만들고 제어 줄을 적는다.
    let log_lines = |app: &App| app.panes[0].remote.as_ref().and_then(|r| r.log.as_ref()).map(|l| l.lock().unwrap().lines.clone()).unwrap_or_default();
    assert_eq!(app.panes[0].ftp_log_height, 60.0);
    assert!(app.ftp_log_reserve(0, 0.0) > 60.0);
    assert_eq!(log_lines(&app).first().map(String::as_str), Some(" > PWD"));
    assert_eq!(log_lines(&app).last().map(String::as_str), Some("/ > LIST"));
    // 폴더 열기.
    app.panes[0].cursor = 0;
    app.open_current();
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(names(&app, 0), ["..", "a.txt"], "a remote subfolder starts with \"..\"");
    assert_eq!(app.address_text(0), "ftp://ftp.example/pub");
    assert_eq!(log_lines(&app).last().map(String::as_str), Some("/pub > LIST"), "sent lines start with the current remote folder");
    // 원본 miShredClick(0x431c60) → FUN_0042c734: FTP 모드 갈래는 방식과 상관없이 FTP 삭제 확인(휴지통 단추 없음)을 띄운다.
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    app.dispatch("shred", &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::DeleteWarning { show_delete: false, .. })), "shred goes to the FTP delete");
    app.answer_dialog(DialogResult::DeleteWarning(super::original_dialogs::DeleteWarningChoice::No), &ctx);
    assert!(app.job.is_none() && app.original_dialogs.is_empty() && app.msgboxes.is_empty());
    // 로컬 파일만 다루는 명령은 여전히 막는다.
    app.dispatch("compare", &ctx);
    assert!(app.job.is_none() && app.original_dialogs.is_empty() && app.msgboxes.is_empty());
    // 원본 miNewFolderClick의 FTP 갈래: TfGetText(TITLE_NEWFOLDER) → FTP 새 폴더 → 새로 읽고 그 이름에 커서.
    app.dispatch("newFolder", &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::Prompt { .. })));
    assert_eq!(app.original_dialogs.last().unwrap().title, app.language_pack.text("RES.TITLE_NEWFOLDER", "?"));
    app.answer_dialog(DialogResult::Text(Some("made".into())), &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading && app.panes[0].entries.iter().any(|e| e.name == "made"));
    assert!(FAKE_FTP.lock().unwrap().get("/pub/made").is_some_and(|(dir, _)| *dir));
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "made", "the cursor goes to the new folder");
    // 원본 miRenameClick의 FTP 갈래: TfRename(이름·확장자 칸) → FTP 이름 바꾸기.
    app.panes[0].selected.clear();
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    app.dispatch("rename", &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::Rename { name, extension, .. }) if name == "a" && extension == "txt"));
    app.answer_dialog(DialogResult::Rename(Some(("b".into(), "txt".into()))), &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading && app.panes[0].entries.iter().any(|e| e.name == "b.txt"));
    assert!(FAKE_FTP.lock().unwrap().contains_key("/pub/b.txt") && !FAKE_FTP.lock().unwrap().contains_key("/pub/a.txt"));
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "b.txt");
    // 원본 FUN_00436b98: TfFTPAttributes("이름 (1 / 1)", 목록의 권한 644) → FTP 권한 바꾸기 → 새로 읽기.
    FAKE_CHMODS.lock().unwrap().clear();
    app.dispatch("attributes", &ctx);
    match app.original_dialogs.last().map(|d| (&d.title, &d.kind)) {
        Some((title, DialogKind::RemotePermissions(form))) => {
            assert_eq!(*title, app.language_pack.text("formFTPAttributes.Form.Caption", "?"));
            assert_eq!((form.name.as_str(), form.path.as_str(), form.bits), ("b.txt (1 / 1)", "/pub/b.txt", [[true, true, false], [true, false, false], [true, false, false]]));
        }
        _ => panic!("expected the FTP attributes window"),
    }
    app.answer_dialog(DialogResult::RemotePermissions(Some((rw::PermissionAction::ApplyCurrent, 0o600))), &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading && !FAKE_CHMODS.lock().unwrap().is_empty() && app.panes[0].remote.as_ref().is_some_and(|r| !r.busy.load(std::sync::atomic::Ordering::Relaxed)));
    assert_eq!(FAKE_CHMODS.lock().unwrap().clone(), vec![("/pub/b.txt".to_owned(), 0o600)]);
    // 원본 FUN_0042c734의 FTP 갈래: 휴지통 없이 삭제 확인 → FTP 삭제.
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "made").unwrap();
    app.dispatch("deleteRecycle", &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::DeleteWarning { show_delete: false, .. })), "no recycle bin on FTP");
    app.answer_dialog(DialogResult::DeleteWarning(super::original_dialogs::DeleteWarningChoice::Yes), &ctx);
    wait_until(&mut app, &ctx, |app| app.job.as_ref().is_none_or(|j| j.finished) && !app.panes[0].loading && !app.panes[0].entries.iter().any(|e| e.name == "made"));
    assert!(!FAKE_FTP.lock().unwrap().contains_key("/pub/made"));
    // 이름을 되돌려 아래 단계(a.txt 내려받기)를 그대로 쓴다.
    FAKE_FTP.lock().unwrap().remove("/pub/b.txt");
    FAKE_FTP.lock().unwrap().insert("/pub/a.txt".into(), (false, b"hello".to_vec()));
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.job = None;
    // 다른 패널로 복사 = 내려받기(오른쪽 로컬 폴더로, 대상 묻지 않음).
    let right = app.panes[1].path.clone();
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    app.dispatch("copyToOther", &ctx);
    // 원본 FTP 엔진: 진행 창 없이 아래 결과 칸에만 진행을 보이고 메인 창은 그대로 쓸 수 있다.
    assert!(app.job.as_ref().is_some_and(|j| j.progress.is_none()), "no NDR progress window for panel FTP transfers");
    wait_until(&mut app, &ctx, |app| app.job.as_ref().is_none_or(|j| j.finished) && app.panes.iter().all(|p| !p.loading));
    assert_eq!(fs::read(right.join("a.txt")).unwrap(), b"hello");
    // 원본 FUN_0043f800은 결과창(pBottom)을 보이고, 항목이 끝나면 줄을 지우며(FUN_0043f04c 이벤트 2),
    // 대기열이 비면 FUN_0044033c가 결과창을 숨긴다.
    assert!(!app.show_results);
    assert_eq!(app.menu_checked("toggleResults"), Some(false));
    assert!(app.ftp_results.lock().unwrap().is_empty());
    // 실패한 줄은 남고 결과창도 그대로다.
    app.show_results = true;
    app.ftp_results.lock().unwrap().extend([
        super::ftp_pane::FtpResultRow { upload: false, local: "x".into(), remote: "/pub/x".into(), size: Some(5), done: 5, state: super::ftp_pane::RowState::Done },
        super::ftp_pane::FtpResultRow { upload: false, local: "y".into(), remote: "/pub/y".into(), size: Some(5), done: 0, state: super::ftp_pane::RowState::Failed("550".into()) },
    ]);
    app.ftp_queue_done();
    assert!(app.show_results);
    let rows = app.ftp_results.lock().unwrap().clone();
    assert_eq!((rows.len(), rows[0].remote.as_str(), rows[0].progress_text(true)), (1, "/pub/y", "550".to_owned()));
    assert_eq!(super::ftp_pane::FtpResultRow { state: super::ftp_pane::RowState::Done, ..rows[0].clone() }.progress_text(true), "100%");
    // 전송 중이 아니면 잠시 중단·취소는 아무것도 하지 않는다.
    app.ftp_pause_toggle();
    assert!(!app.ftp_pause.load(std::sync::atomic::Ordering::Relaxed));
    // 로컬 → FTP 복사 = 올리기(FTP 패널의 지금 폴더로).
    fs::write(right.join("up.txt"), b"up").unwrap();
    app.refresh(1);
    wait_until(&mut app, &ctx, |app| !app.panes[1].loading);
    app.active = 1;
    app.panes[1].cursor = app.panes[1].entries.iter().position(|e| e.name == "up.txt").unwrap();
    app.panes[1].selected.clear();
    app.dispatch("copyToOther", &ctx);
    wait_until(&mut app, &ctx, |app| app.job.as_ref().is_none_or(|j| j.finished) && app.panes.iter().all(|p| !p.loading));
    assert_eq!(FAKE_FTP.lock().unwrap().get("/pub/up.txt").map(|(_, d)| d.clone()), Some(b"up".to_vec()));
    assert!(names(&app, 0).contains(&"up.txt".to_owned()), "the FTP pane is listed again");
    // 올리기가 끝나면 끝난 줄이 빠지고(실패한 줄은 새 전송이 비웠다) 결과창이 숨는다.
    assert!(app.ftp_results.lock().unwrap().is_empty());
    assert!(!app.show_results);
    app.dispatch("toggleResults", &ctx);
    assert!(app.show_results);
    app.dispatch("toggleResults", &ctx);
    assert!(!app.show_results);
    // 상위 폴더, 끊기.
    app.active = 0;
    app.dispatch("goParent", &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.address_text(0), "ftp://ftp.example/");
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "pub", "the cursor is on the folder we came from");
    // 원본 bSyncAddressClick: FTP 쪽에서 보내면 아무것도 안 한다.
    let right_before = app.panes[1].path.clone();
    app.vertical_bar_click("bSyncPathToRight", &ctx);
    assert_eq!(app.panes[1].path, right_before);
    assert!(app.panes[0].remote.is_some());
    // 원본 miNewTabClick/pmiNewTabClick: FTP 모드의 새 탭은 "c:\".
    assert_eq!(app.new_tab_location(0, app.panes[0].tab), std::path::PathBuf::from("c:\\"));
    // 원본 miCommandLineClick의 FTP 갈래: 입력을 FTP 제어 명령으로 보낸다.
    app.run_command("NOOP".into());
    wait_until(&mut app, &ctx, |app| app.status == "200 NOOP ok");
    assert!(app.panes[0].remote.is_some());
    // 원본 FUN_0043c602: 폴더 비교는 두 패널 모두 로컬일 때만(로컬 쪽에서 시작해도).
    app.active = 1;
    let before = app.original_dialogs.len();
    app.dispatch("compare", &ctx);
    assert!(!app.original_dialogs[before..].iter().any(|d| matches!(d.kind, DialogKind::FolderCompare(_))));
    app.original_dialogs.truncate(before);
    app.msgboxes.clear();
    app.active = 0;
    // 원본 FUN_0043d5ec(경로 바꾸기): 두 패널 모두 로컬일 때만.
    let right_path = app.panes[1].path.clone();
    assert!(app.original_internal("EXCHANGEPATH", &ctx));
    assert_eq!(app.panes[1].path, right_path);
    assert!(app.panes[0].remote.is_some());
    // 원본 miUnmapNetworkDriveClick: FTP 모드에서도 연다(가짜 시스템에는 연결된 드라이브가 없어 알림 상자).
    let (boxes, dialogs) = (app.msgboxes.len(), app.original_dialogs.len());
    app.system.network_drives = Vec::new;
    app.dispatch("unmapDrive", &ctx);
    assert!(app.msgboxes.len() + app.original_dialogs.len() > boxes + dialogs);
    app.msgboxes.clear();
    app.original_dialogs.truncate(dialogs);
    app.dispatch("ftpDisconnect", &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert!(app.panes[0].remote.is_none());
    assert_eq!(app.panes[0].path, left);
    assert!(names(&app, 0).contains(&"alpha.txt".to_owned()), "back to the local folder");
    // 양쪽 로컬이면 올리기는 원본 FTP_PUTCRITERIA.
    app.dispatch("ftpUpload", &ctx);
    assert_eq!(app.msgboxes.first().map(|m| m.text.clone()), Some(app.language_pack.text("RES.FTP_PUTCRITERIA", "?")));
    let _ = fs::remove_dir_all(&root);
}

/// 원본 FUN_0043e9f8(TfFTPQuickConnect): 포트 22면 SFTP, "사이트 목록에 추가"면 사이트 저장, 최근 주소 기록.
#[test]
fn ftp_quick_connect_follows_the_original_tfftpquickconnect() {
    let _guard = REMOTE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (root, ctx, mut app) = fixture("ftp-quick", "ko");
    app.remote_hooks.pane_connect = fake_pane_connect;
    FAKE_FTP.lock().unwrap().clear();
    FAKE_FTP_CONFIGS.lock().unwrap().clear();
    app.active = 1;
    app.dispatch("ftpQuickConnect", &ctx);
    let mut form = match app.original_dialogs.last().map(|d| &d.kind) {
        Some(DialogKind::FtpQuickConnect(form)) => (**form).clone(),
        _ => panic!("expected the quick connect dialog"),
    };
    assert_eq!((form.port.as_str(), form.passive, form.pane), ("21", true, 1));
    frame(&mut app, &ctx);
    form.host = "sftp.example".into();
    form.port = "22".into();
    form.user = "me".into();
    form.password = "secret".into();
    form.add_to_sites = true;
    app.answer_dialog(DialogResult::FtpQuickConnect(Some(form)), &ctx);
    wait_until(&mut app, &ctx, |app| app.panes[1].remote.is_some() && !app.panes[1].loading);
    let config = FAKE_FTP_CONFIGS.lock().unwrap().pop().expect("connected");
    assert_eq!((config.protocol.as_str(), config.host.as_str(), config.port, config.user.as_str(), config.password.as_str()), ("SFTP", "sftp.example", 22, "me", "secret"));
    assert_eq!(app.ftp_site_items(), vec![(0, "&1. sftp.example".to_owned())]);
    assert_eq!(app.settings.strings("RecentRemoteHosts"), ["sftp.example"]);
    assert_eq!(app.address_text(1), "ftp://sftp.example/");
    // 취소는 아무것도 하지 않는다.
    app.active = 0;
    app.dispatch("ftpQuickConnect", &ctx);
    app.answer_dialog(DialogResult::FtpQuickConnect(None), &ctx);
    assert!(app.panes[0].remote.is_none() && app.original_dialogs.is_empty());
    let _ = fs::remove_dir_all(&root);
}
#[test]
fn quick_connect_site_save_failure_shows_the_csharp_warning() {
    let (root, _ctx, mut app) = fixture("site-save-failure", "en");
    // 설정을 읽지 못한 프로필은 덮어쓰지 않으므로 저장이 실패한다(디스크는 건드리지 않음).
    app.profile_load_error = true;
    app.status.clear();
    app.save_quick_connect_site(rw::SiteProfile { name: "Example".into(), host: "ftp.example.invalid".into(), ..Default::default() });
    // C# SaveQuickConnectSite catch: MessageBox(오류, SaveFailedTitle, 경고). 성공 상태 글("Saved FTP site")은 쓰지 않는다.
    let title = app.language_pack.text("NDRRemoteSites.SaveFailedTitle", "FTP site save failed");
    let warning = app.msgboxes.last().expect("save failure warning");
    assert_eq!(warning.title, title);
    assert!(!app.status.contains("Example"), "status: {}", app.status);
    let _ = fs::remove_dir_all(&root);
}

/// FUN_00467f0c: folders merge without a prompt; a file conflict shows the remote size and time and offers resume.
#[test]
fn remote_conflicts_merge_folders_and_describe_the_remote_file() {
    let (events, received) = mpsc::channel();
    let cancel = AtomicBool::new(false);
    let folder = RemoteEntry { name: "dir".into(), is_dir: true, ..Default::default() };
    let info = ndr::remote::ConflictInfo { remote: &folder, local_dir: true, remote_is_source: true, can_resume: true };
    assert_eq!(super::ftp_pane::ask_remote_conflict(&events, &cancel, "/dir", "C:\\local\\dir", &info), ConflictChoice::Overwrite);
    assert!(received.try_recv().is_err(), "no dialog for two folders");
    let root = std::env::temp_dir().join(format!("ndr-remote-conflict-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let local = root.join("big.bin");
    fs::write(&local, vec![0u8; 10]).unwrap();
    let file = RemoteEntry { name: "big.bin".into(), size: 1000, modified: Some(1_700_000_000), ..Default::default() };
    let local_text = local.to_string_lossy().into_owned();
    let worker = std::thread::spawn(move || {
        let cancel = AtomicBool::new(false);
        let info = ndr::remote::ConflictInfo { remote: &file, local_dir: false, remote_is_source: true, can_resume: true };
        super::ftp_pane::ask_remote_conflict(&events, &cancel, "/pub/big.bin", &local_text, &info)
    });
    let Ok(Event::Conflict { source, destination, allow_resume, reply }) = received.recv_timeout(Duration::from_secs(5)) else { panic!("conflict dialog expected") };
    let form = super::file_operation::ConflictForm::read(&source, &destination, allow_resume);
    assert_eq!((form.source_size, form.destination_size), (Some(1000), Some(10)));
    assert_eq!(form.source_modified, Some(std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000)));
    assert!(form.can_resume(), "a smaller local file can be resumed");
    reply.send(ConflictChoice::Resume).unwrap();
    assert_eq!(worker.join().unwrap(), ConflictChoice::Resume);
    let _ = fs::remove_dir_all(root);
}