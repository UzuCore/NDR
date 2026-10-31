//! Checks C# RemoteWindow, RemoteSitesWindow, RemoteSiteEditorWindow, RemotePermissionsWindow and RunRemoteCommandAsync in the Rust UI.
//! Korean and English only. Server connections use a fake function (records the settings and returns failure); real transfers run only in the ignored test at the bottom,
//! against a 127.0.0.1-only pyftpdlib server (isolated folder) started by that test.
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
/// Fake connect: records the settings and, depending on the host, returns a server key confirmation or a failure.
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

/// Original TfFTPAttributes FormKeyDown (0x427334): End (when the number field eAttr has no focus) = mrAll (all).
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
    // Local: open folder, back/forward, up, "Computer" = drive list.
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
    // Download and upload before connecting.
    app.remote_action(RemoteAction::Download, &ctx);
    assert_eq!(boxed(&app), (String::new(), "먼저 원격 서버에 연결하세요.".to_owned()));
    app.answer_message_box(1, &ctx);
    app.remote_action(RemoteAction::UploadSelected, &ctx);
    assert_eq!(boxed(&app).1, "먼저 원격 서버에 연결하세요.");
    app.answer_message_box(1, &ctx);
    // SFTP checks host, user and port.
    app.remote_action(RemoteAction::Protocol(2), &ctx);
    assert_eq!(form(&mut app).port, "22");
    form(&mut app).host = "sftp.invalid".into();
    app.remote_action(RemoteAction::Connect, &ctx);
    assert_eq!(boxed(&app), ("원격 연결".to_owned(), "호스트, 사용자 이름, 올바른 포트를 입력하세요.".to_owned()));
    app.answer_message_box(1, &ctx);
    assert!(connects().is_empty());
    // Quick FTP connect + add to the site list: the input is saved as a site before connecting.
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
    // Even with FTP chosen, port 22 quick-connects as SFTP; port 0 does not connect.
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
    // Out-of-range ports are not truncated to 16 bits and do not connect.
    form(&mut app).port = "99999".into();
    app.remote_action(RemoteAction::Connect, &ctx);
    assert!(!app.msgboxes.is_empty(), "an out-of-range FTP port is refused at once");
    app.answer_message_box(1, &ctx);
    assert!(connects().is_empty());
    // SFTP server key: an unknown key asks, and yes reconnects with that key.
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
    // RemotePermissionsWindow: bits <-> number; checkboxes follow only for three digits below 778.
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
    // Remote command: without sites, the RunSafely warning.
    app.dispatch("remoteCommand", &ctx);
    assert_eq!(boxed(&app), ("NDR".to_owned(), "Save a remote site from the Network menu first.".to_owned()));
    app.answer_message_box(1, &ctx);
    app.dispatch("sites", &ctx);
    assert_eq!(app.original_dialogs.last().unwrap().title, "FTP Site Manager");
    // Add: TfGetText(FTPSITE_ADD) -> a new row (FTP, 21, passive mode).
    app.sites_action(SitesAction::Add, &ctx);
    let dialog = app.original_dialogs.last().unwrap();
    assert!(matches!(&dialog.kind, DialogKind::Prompt { message, .. } if message == "Type a FTP site name."));
    app.answer_dialog(DialogResult::Text(Some("alpha".into())), &ctx);
    {
        let f = app.sites_form().unwrap();
        assert_eq!((f.sites.len(), f.selected, f.protocol.as_str(), f.port.as_str(), f.passive), (1, Some(0), "FTP", "21", true));
    }
    // An empty name or address gives FTPSITE_INVALIDINFO (TfMsgBox, asterisk icon).
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
    // Advanced settings (editor window): switching to SFTP sets port 22; the saved key stays only for the same SFTP host and port.
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
    // Connect: saves and opens the remote window with that site, connecting at once.
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
    // Delete: FTPSITE_DELCONFIRM yes/no.
    app.dispatch("sites", &ctx);
    app.sites_action(SitesAction::Delete, &ctx);
    assert_eq!(boxed(&app).1, "Are you sure you want to delete a selected site?");
    app.answer_message_box(7, &ctx);
    assert_eq!(app.sites_form().unwrap().sites.len(), 1);
    app.sites_action(SitesAction::Cancel, &ctx);
    // Remote command: choose a site -> command (initially pwd) -> a connection failure shows a warning box.
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

/// Test-only FTP server (127.0.0.1, isolated folder, user user/pass).
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
        // Open folder -> back.
        app.remote_action(RemoteAction::RemoteOpen, &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none() && f.remote_path == "/docs" && f.listing_valid));
        assert_eq!(form(&mut app).items.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["..", "readme.txt"]);
        // Download: with no existing name it downloads without asking.
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
        // Downloading again is a name conflict (yes/no/cancel); no skips.
        app.remote_action(RemoteAction::Download, &ctx);
        assert_eq!(boxed(&app), ("이름 충돌".to_owned(), "로컬 폴더에 같은 이름의 파일 1개가 있습니다. 덮어쓸까요?".to_owned()));
        app.answer_message_box(7, &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none()) && !app.original_dialogs.iter().any(|d| matches!(d.kind, DialogKind::RemoteTransfer(_))));
        assert_eq!(form(&mut app).status, "1개 항목을 다운로드했습니다.", "skipped items count as completed");
        // Upload (▲): local alpha.txt.
        wait_local(&mut app, &ctx);
        select_local(&mut app, "alpha.txt");
        app.remote_action(RemoteAction::UploadSelected, &ctx);
        wait_until(&mut app, &ctx, |app| app.remote_form().is_some_and(|f| f.busy.is_none() && f.listing_valid) && !app.original_dialogs.iter().any(|d| matches!(d.kind, DialogKind::RemoteTransfer(_))));
        assert_eq!(fs::read(served.join("docs").join("alpha.txt")).unwrap(), b"alpha");
        assert_eq!(form(&mut app).status, "1개 파일을 업로드했습니다.");
        // New remote folder, rename, delete.
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
        // Permissions: permission window (OK) -> "changed permissions of 1/1 items, skipped 0".
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
        // Close: something was downloaded, so the opening pane goes to that local folder.
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


/// In-memory FTP for tests (path -> (is folder, content)). Used only by the pane FTP mode tests.
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

/// Original pane FTP mode: the site menu (FUN_00440734 "&1. name") turns the current pane into FTP, open folder, copy to the other pane = download/upload
/// (FUN_0042d518), local commands blocked on the FTP pane, disconnect (FUN_0043e8d4) returns to a local folder, and with both panes local, upload shows FTP_PUTCRITERIA.
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
    // Original FUN_0045bb0c: without the hidden view, remote names starting with '.' are left out; with it, they get the hidden attribute.
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
    // Original FUN_0048c548 (FTP mode): ">" after the host segment lists the remote subfolders (enumerator FUN_0045c948, sorted);
    // the leading ▼ adds nothing, so no drive list appears on an FTP pane. Choosing an item opens that remote folder.
    {
        FAKE_FTP.lock().unwrap().insert("/Docs".into(), (true, Vec::new()));
        let segments = app.crumb_segments(0);
        assert_eq!(segments[0].folder.as_deref(), Some(Path::new("/")));
        let children = app.crumb_children(0, Path::new("/"), false);
        assert_eq!(children.iter().map(|(p, n)| (p.to_string_lossy().into_owned(), n.as_str())).collect::<Vec<_>>(), [("/Docs".to_owned(), "Docs"), ("/pub".to_owned(), "pub")]);
        assert!(app.crumb_children(0, Path::new(""), true).is_empty(), "no drive list on an FTP pane");
        FAKE_FTP.lock().unwrap().remove("/Docs");
        app.go_crumb(0, super::pane_chrome::CrumbTarget::Remote("/pub".into()));
        wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
        assert_eq!(names(&app, 0), ["..", "a.txt"]);
        app.go_crumb(0, super::pane_chrome::CrumbTarget::Remote("/".into()));
        wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
        assert_eq!(names(&app, 0), ["pub", "readme.txt"]);
    }
    // Original FUN_004408f8: for non-SFTP connections, an FTP log box below the pane (FUN_0048d7ac, height 60) records control lines.
    let log_lines = |app: &App| app.panes[0].remote.as_ref().and_then(|r| r.log.as_ref()).map(|l| l.lock().unwrap().lines.clone()).unwrap_or_default();
    assert_eq!(app.panes[0].ftp_log_height, 60.0);
    assert!(app.ftp_log_reserve(0, 0.0) > 60.0);
    assert_eq!(log_lines(&app).first().map(String::as_str), Some(" > PWD"));
    assert_eq!(log_lines(&app).last().map(String::as_str), Some("/ > LIST"));
    // Open folder.
    app.panes[0].cursor = 0;
    app.open_current();
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(names(&app, 0), ["..", "a.txt"], "a remote subfolder starts with \"..\"");
    assert_eq!(app.address_text(0), "ftp://ftp.example/pub");
    assert_eq!(log_lines(&app).last().map(String::as_str), Some("/pub > LIST"), "sent lines start with the current remote folder");
    // Original miShredClick (0x431c60) -> FUN_0042c734: the FTP mode branch shows the FTP delete confirmation (no Recycle Bin button) regardless of the mode.
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    app.dispatch("shred", &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::DeleteWarning { show_delete: false, .. })), "shred goes to the FTP delete");
    app.answer_dialog(DialogResult::DeleteWarning(super::original_dialogs::DeleteWarningChoice::No), &ctx);
    assert!(app.job.is_none() && app.original_dialogs.is_empty() && app.msgboxes.is_empty());
    // Commands that only work on local files are still blocked.
    app.dispatch("compare", &ctx);
    assert!(app.job.is_none() && app.original_dialogs.is_empty() && app.msgboxes.is_empty());
    // FTP branch of original miNewFolderClick: TfGetText(TITLE_NEWFOLDER) -> FTP new folder -> reread with the cursor on that name.
    app.dispatch("newFolder", &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::Prompt { .. })));
    assert_eq!(app.original_dialogs.last().unwrap().title, app.language_pack.text("RES.TITLE_NEWFOLDER", "?"));
    app.answer_dialog(DialogResult::Text(Some("made".into())), &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading && app.panes[0].entries.iter().any(|e| e.name == "made"));
    assert!(FAKE_FTP.lock().unwrap().get("/pub/made").is_some_and(|(dir, _)| *dir));
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "made", "the cursor goes to the new folder");
    // FTP branch of original miRenameClick: TfRename (name and extension fields) -> FTP rename.
    app.panes[0].selected.clear();
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    app.dispatch("rename", &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::Rename { name, extension, .. }) if name == "a" && extension == "txt"));
    app.answer_dialog(DialogResult::Rename(Some(("b".into(), "txt".into()))), &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading && app.panes[0].entries.iter().any(|e| e.name == "b.txt"));
    assert!(FAKE_FTP.lock().unwrap().contains_key("/pub/b.txt") && !FAKE_FTP.lock().unwrap().contains_key("/pub/a.txt"));
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "b.txt");
    // Original FUN_00436b98: TfFTPAttributes ("name (1 / 1)", listed permissions 644) -> change FTP permissions -> reread.
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
    // FTP branch of original FUN_0042c734: delete confirmation without the Recycle Bin -> FTP delete.
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "made").unwrap();
    app.dispatch("deleteRecycle", &ctx);
    assert!(matches!(app.original_dialogs.last().map(|d| &d.kind), Some(DialogKind::DeleteWarning { show_delete: false, .. })), "no recycle bin on FTP");
    app.answer_dialog(DialogResult::DeleteWarning(super::original_dialogs::DeleteWarningChoice::Yes), &ctx);
    wait_until(&mut app, &ctx, |app| app.job.as_ref().is_none_or(|j| j.finished) && !app.panes[0].loading && !app.panes[0].entries.iter().any(|e| e.name == "made"));
    assert!(!FAKE_FTP.lock().unwrap().contains_key("/pub/made"));
    // Rename back so the step below (download a.txt) works unchanged.
    FAKE_FTP.lock().unwrap().remove("/pub/b.txt");
    FAKE_FTP.lock().unwrap().insert("/pub/a.txt".into(), (false, b"hello".to_vec()));
    app.refresh(0);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    app.job = None;
    // Copy to the other pane = download (to the right local folder, no destination prompt).
    let right = app.panes[1].path.clone();
    app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "a.txt").unwrap();
    app.dispatch("copyToOther", &ctx);
    // Original FTP engine: no progress window, progress shows only in the result area below and the main window stays usable.
    assert!(app.job.as_ref().is_some_and(|j| j.progress.is_none()), "no NDR progress window for panel FTP transfers");
    wait_until(&mut app, &ctx, |app| app.job.as_ref().is_none_or(|j| j.finished) && app.panes.iter().all(|p| !p.loading));
    assert_eq!(fs::read(right.join("a.txt")).unwrap(), b"hello");
    // Original FUN_0043f800 shows the result area (pBottom), removes rows as items finish (FUN_0043f04c event 2),
    // and FUN_0044033c hides the result area when the queue is empty.
    assert!(!app.show_results);
    assert_eq!(app.menu_checked("toggleResults"), Some(false));
    assert!(app.ftp_results.lock().unwrap().is_empty());
    // Failed rows stay and the result area stays too.
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
    // Without a transfer, pause and cancel do nothing.
    app.ftp_pause_toggle();
    assert!(!app.ftp_pause.load(std::sync::atomic::Ordering::Relaxed));
    // Local -> FTP copy = upload (to the FTP pane's current folder).
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
    // When the upload ends, finished rows are removed (the new transfer cleared the failed row) and the result area hides.
    assert!(app.ftp_results.lock().unwrap().is_empty());
    assert!(!app.show_results);
    app.dispatch("toggleResults", &ctx);
    assert!(app.show_results);
    app.dispatch("toggleResults", &ctx);
    assert!(!app.show_results);
    // Parent folder, disconnect.
    app.active = 0;
    app.dispatch("goParent", &ctx);
    wait_until(&mut app, &ctx, |app| !app.panes[0].loading);
    assert_eq!(app.address_text(0), "ftp://ftp.example/");
    assert_eq!(app.panes[0].entries[app.panes[0].cursor].name, "pub", "the cursor is on the folder we came from");
    // Original bSyncAddressClick: does nothing when sent from the FTP side.
    let right_before = app.panes[1].path.clone();
    app.vertical_bar_click("bSyncPathToRight", &ctx);
    assert_eq!(app.panes[1].path, right_before);
    assert!(app.panes[0].remote.is_some());
    // Original miNewTabClick/pmiNewTabClick: a new tab in FTP mode is "c:\".
    assert_eq!(app.new_tab_location(0, app.panes[0].tab), std::path::PathBuf::from("c:\\"));
    // FTP branch of original miCommandLineClick: the input goes out as an FTP control command.
    app.run_command("NOOP".into());
    wait_until(&mut app, &ctx, |app| app.status == "200 NOOP ok");
    assert!(app.panes[0].remote.is_some());
    // Original FUN_0043c602: folder comparison only when both panes are local (even when started from the local side).
    app.active = 1;
    let before = app.original_dialogs.len();
    app.dispatch("compare", &ctx);
    assert!(!app.original_dialogs[before..].iter().any(|d| matches!(d.kind, DialogKind::FolderCompare(_))));
    app.original_dialogs.truncate(before);
    app.msgboxes.clear();
    app.active = 0;
    // Original FUN_0043d5ec (swap paths): only when both panes are local.
    let right_path = app.panes[1].path.clone();
    assert!(app.original_internal("EXCHANGEPATH", &ctx));
    assert_eq!(app.panes[1].path, right_path);
    assert!(app.panes[0].remote.is_some());
    // Original miUnmapNetworkDriveClick: opens in FTP mode too (the fake system has no mapped drives, so a notice box).
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
    // With both panes local, upload shows the original FTP_PUTCRITERIA.
    app.dispatch("ftpUpload", &ctx);
    assert_eq!(app.msgboxes.first().map(|m| m.text.clone()), Some(app.language_pack.text("RES.FTP_PUTCRITERIA", "?")));
    let _ = fs::remove_dir_all(&root);
}

/// Original FUN_0043e9f8 (TfFTPQuickConnect): port 22 means SFTP, "add to site list" saves the site, recent addresses are recorded.
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
    // Cancel does nothing.
    app.active = 0;
    app.dispatch("ftpQuickConnect", &ctx);
    app.answer_dialog(DialogResult::FtpQuickConnect(None), &ctx);
    assert!(app.panes[0].remote.is_none() && app.original_dialogs.is_empty());
    let _ = fs::remove_dir_all(&root);
}
#[test]
fn quick_connect_site_save_failure_shows_the_csharp_warning() {
    let (root, _ctx, mut app) = fixture("site-save-failure", "en");
    // A profile whose settings could not be read is not overwritten, so saving fails (the disk is not touched).
    app.profile_load_error = true;
    app.status.clear();
    app.save_quick_connect_site(rw::SiteProfile { name: "Example".into(), host: "ftp.example.invalid".into(), ..Default::default() });
    // C# SaveQuickConnectSite catch: MessageBox(error, SaveFailedTitle, warning). The success status text ("Saved FTP site") is not written.
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
    let info = ndr::remote::ConflictInfo { remote: &folder, local_dir: true, remote_is_source: true, can_resume: true, new_name: Default::default() };
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
        let info = ndr::remote::ConflictInfo { remote: &file, local_dir: false, remote_is_source: true, can_resume: true, new_name: Default::default() };
        super::ftp_pane::ask_remote_conflict(&events, &cancel, "/pub/big.bin", &local_text, &info)
    });
    let Ok(Event::Conflict { source, destination, allow_resume, reply }) = received.recv_timeout(Duration::from_secs(5)) else { panic!("conflict dialog expected") };
    let form = super::file_operation::ConflictForm::read(&source, &destination, allow_resume);
    assert_eq!((form.source_size, form.destination_size), (Some(1000), Some(10)));
    assert_eq!(form.source_modified, Some(std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000)));
    assert!(form.can_resume(), "a smaller local file can be resumed");
    assert!(form.remote, "the remote side marks the window as an FTP conflict");
    assert!(super::file_operation::conflict_buttons(&form).iter().all(|(b, _)| *b != super::file_operation::ConflictButton::ReplaceFolder));
    reply.send(ConflictChoice::Resume).unwrap();
    assert_eq!(worker.join().unwrap(), ConflictChoice::Resume);
    let _ = fs::remove_dir_all(root);
}
/// FUN_00467658: Rename asks for the name; a cancelled name prompt asks the conflict again; the typed name is handed back.
#[test]
fn remote_conflict_rename_asks_for_the_new_name() {
    let (events, received) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let cancel = AtomicBool::new(false);
        let file = RemoteEntry { name: "a.txt".into(), size: 5, ..Default::default() };
        let info = ndr::remote::ConflictInfo { remote: &file, local_dir: false, remote_is_source: false, can_resume: false, new_name: Default::default() };
        let choice = super::ftp_pane::ask_remote_conflict(&events, &cancel, "C:\\local\\a.txt", "/pub/a.txt", &info);
        (choice, info.new_name.take())
    });
    let next = || received.recv_timeout(Duration::from_secs(5)).expect("event");
    let Event::Conflict { reply, .. } = next() else { panic!("conflict expected") };
    reply.send(ConflictChoice::Rename).unwrap();
    let Event::RenameTarget { name, reply } = next() else { panic!("name prompt expected") };
    assert_eq!(name, "a.txt");
    reply.send(None).unwrap();
    let Event::Conflict { reply, .. } = next() else { panic!("a cancelled name prompt asks again") };
    reply.send(ConflictChoice::Rename).unwrap();
    let Event::RenameTarget { reply, .. } = next() else { panic!("name prompt expected") };
    reply.send(Some("b.txt".into())).unwrap();
    assert_eq!(worker.join().unwrap(), (ConflictChoice::Rename, Some("b.txt".into())));
}