//! C# RemoteWindow(NDIR 원격 연결, 1060×720) + RemoteTransferProgressWindow + RemotePermissionsWindow + NexusFileFtpHostService.
//! 연결 줄(프로토콜 FTP/FTPS(처음)/SFTP, 호스트(최근 호스트 목록), 포트, 사용자, 비밀번호, 연결/연결 끊기, 빠른 연결이면 "FTP 사이트 목록에 추가"·
//! 수동 모드), 로컬(뒤로/앞으로/위/경로/이동/새로 고침, "Computer" = 드라이브 목록)과 원격(뒤로/위/경로/이동/새로 고침) 목록, 가운데 ▲▼(ShowVerticalBar),
//! 아래 상태 글과 내려받기/올리기/폴더 선택 업로드…/닫기. 원격 목록 오른쪽 단추 메뉴 = 새 원격 폴더, 이름 바꾸기, 삭제, 권한 바꾸기.
//! 전송은 진행 창(일시 중지/계속, 자세히 보기/간단히 보기, 중단). 창을 닫으면 연결을 끊고, 내려받은 것이 있으면 연 패널을 그 로컬 폴더로 옮긴다.
//! 차이(HANDOFF): 전송은 Rust remote 모듈(임시 이름에 받은 뒤 바꾸기, 이름 검사)로 하고, 진행 막대는 파일 크기를 모르면 움직이는 막대다.
//! FTP 원격 명령은 Rust의 안전한 제어 명령 목록만 보낸다. 영어 화면은 창 글이 모두 영어다(C#은 일부 한국어).
use super::grid::{self, Column, GridState, GridView};
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndir::fsops::ConflictChoice;
use ndir::msgbox::{IDCANCEL, IDYES, MB_ICONEXCLAMATION, MB_OK, MB_YESNO, MB_YESNOCANCEL};
use ndir::remote::{Client, Config, RemoteEntry};
use ndir::shortcut_bindings::{self as bindings, Modifiers as KeyMods};
use serde_json::{Value, json};
use std::collections::VecDeque;

/// C# FileSystemService의 컴퓨터(드라이브 목록) 경로.
pub(super) const DRIVES: &str = "::drives::";
/// PaneState.HistoryCapacity.
const HISTORY_CAPACITY: usize = 20;

pub(super) fn t(ko: bool, korean: &'static str, english: &'static str) -> &'static str {
    if ko { korean } else { english }
}

/// .NET "N0".
pub(super) fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, c) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// C# RemoteSiteProfile(설정 RemoteSites의 한 항목).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SiteProfile {
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: i64,
    pub user: String,
    pub encrypted_password: String,
    pub remote_directory: String,
    pub passive: bool,
    pub encoding_name: String,
    pub private_key_path: String,
    pub encrypted_key_passphrase: String,
    pub known_host_fingerprint: String,
}

impl Default for SiteProfile {
    fn default() -> Self {
        Self {
            name: String::new(),
            protocol: "FTP".into(),
            host: String::new(),
            port: 21,
            user: String::new(),
            encrypted_password: String::new(),
            remote_directory: "/".into(),
            passive: true,
            encoding_name: "UTF-8".into(),
            private_key_path: String::new(),
            encrypted_key_passphrase: String::new(),
            known_host_fingerprint: String::new(),
        }
    }
}

impl SiteProfile {
    pub(super) fn from_json(value: &Value) -> Self {
        let d = Self::default();
        let text = |key: &str, fallback: &str| value.get(key).and_then(Value::as_str).unwrap_or(fallback).to_owned();
        Self {
            name: text("Name", &d.name),
            protocol: text("Protocol", &d.protocol),
            host: text("Host", &d.host),
            port: value.get("Port").and_then(Value::as_i64).unwrap_or(d.port),
            user: text("User", &d.user),
            encrypted_password: text("EncryptedPassword", ""),
            remote_directory: text("RemoteDirectory", &d.remote_directory),
            passive: value.get("Passive").and_then(Value::as_bool).unwrap_or(true),
            encoding_name: text("EncodingName", &d.encoding_name),
            private_key_path: text("PrivateKeyPath", ""),
            encrypted_key_passphrase: text("EncryptedKeyPassphrase", ""),
            known_host_fingerprint: text("KnownHostFingerprint", ""),
        }
    }

    pub(super) fn to_json(&self) -> Value {
        json!({
            "Name": self.name, "Protocol": self.protocol, "Host": self.host, "Port": self.port, "User": self.user,
            "EncryptedPassword": self.encrypted_password, "RemoteDirectory": self.remote_directory, "Passive": self.passive,
            "EncodingName": self.encoding_name, "PrivateKeyPath": self.private_key_path,
            "EncryptedKeyPassphrase": self.encrypted_key_passphrase, "KnownHostFingerprint": self.known_host_fingerprint,
        })
    }
}

pub(super) fn load_sites(settings: &ndir::settings::Settings) -> Vec<SiteProfile> {
    settings.values.get("RemoteSites").and_then(Value::as_array).map(|a| a.iter().map(SiteProfile::from_json).collect()).unwrap_or_default()
}

pub(super) fn store_sites(settings: &mut ndir::settings::Settings, sites: &[SiteProfile]) {
    settings.values.insert("RemoteSites".into(), Value::Array(sites.iter().map(SiteProfile::to_json).collect()));
}

/// NexusFileFtpHostService.PrepareHost: "ftp://"가 어디든 있으면 UTF-16 위치 6부터 최대 256자(URL은 풀지 않는다).
pub(super) fn prepare_host(host: &str) -> String {
    if !host.contains("ftp://") {
        return host.to_owned();
    }
    let units: Vec<u16> = host.encode_utf16().collect();
    let length = 256.min(units.len().saturating_sub(6));
    String::from_utf16_lossy(&units[6..6 + length])
}

/// NexusFileFtpHostService.ParsePort: StrToIntExW(글, 0), 못 읽으면 21.
pub(super) fn parse_port(text: &str) -> i64 {
    native::str_to_int_ex(text).map_or(21, i64::from)
}

/// QuickConnectProtocol: FTP를 고르고 포트가 정확히 22면 SFTP.
fn quick_connect_protocol(selected: &str, port: i64) -> String {
    if selected == "FTP" && port == 22 { "SFTP".into() } else { selected.into() }
}

/// RemoteWindow.ParentPath.
pub(super) fn parent_path(path: &str) -> String {
    let clean = path.trim_end_matches('/');
    match clean.rfind('/') {
        Some(at) if at > 0 => clean[..at].to_owned(),
        _ => "/".into(),
    }
}

/// SafeLocalName: 마지막 이름 부분만, "", ".", ".."는 거부.
fn safe_local_name(name: &str) -> Result<String, String> {
    let safe = name.rsplit(['/', '\\']).next().unwrap_or("");
    if safe.is_empty() || safe == "." || safe == ".." {
        return Err("원격 파일 이름이 안전하지 않습니다.".into());
    }
    Ok(safe.to_owned())
}

/// egui 글쇠 → WPF Key 이름(ShortcutBindingService 비교용).
pub(super) fn wpf_key(key: egui::Key) -> Option<String> {
    use egui::Key;
    Some(match key {
        Key::ArrowLeft => "Left".into(),
        Key::ArrowRight => "Right".into(),
        Key::ArrowUp => "Up".into(),
        Key::ArrowDown => "Down".into(),
        Key::Backspace => "Back".into(),
        Key::Minus => "OemMinus".into(),
        Key::Plus | Key::Equals => "OemPlus".into(),
        Key::Comma => "OemComma".into(),
        Key::Period => "OemPeriod".into(),
        _ => {
            let name = key.name();
            if name.len() == 1 && name.as_bytes()[0].is_ascii_digit() { format!("D{name}") } else { name.to_owned() }
        }
    })
}

/// 이번 장면에 눌린 글쇠 가운데 정의(id)에 맞는 것이 있는가.
pub(super) fn pressed(ctx: &egui::Context, settings: &ndir::settings::Settings, id: &str) -> bool {
    ctx.input(|i| {
        i.events.iter().any(|event| match event {
            egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => {
                let mods = KeyMods { ctrl: modifiers.ctrl, alt: modifiers.alt, shift: modifiers.shift };
                wpf_key(*key).is_some_and(|name| bindings::matches(settings, id, &name, mods))
            }
            _ => false,
        })
    })
}

/// 로컬 목록 한 줄(C# FileItem 중 이 창이 쓰는 것).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LocalRow {
    pub name: String,
    pub kind: String,
    pub size_text: String,
    pub modified_text: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_parent: bool,
    pub is_drive: bool,
}

/// C# RemoteEntry.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RemoteRow {
    pub name: String,
    pub kind: String,
    pub size_text: String,
    pub modified_text: String,
    pub remote_path: String,
    pub mode: Option<u32>,
    pub size: Option<u64>,
    pub is_dir: bool,
    pub is_parent: bool,
}

/// C# FileSystemService.ListLocal(경로, 숨김, 설명 없음): 드라이브 목록 또는 ".." + 폴더 + 파일.
pub(super) fn list_local(path: &str, show_hidden: bool, ko: bool) -> Result<Vec<LocalRow>, String> {
    if path == DRIVES {
        return Ok(native::drives()
            .into_iter()
            .filter(|root| root.is_dir())
            .map(|root| LocalRow {
                name: root.to_string_lossy().into_owned(),
                kind: native::drive_type_name(&root).into(),
                size_text: String::new(),
                modified_text: String::new(),
                path: root,
                is_dir: true,
                is_parent: false,
                is_drive: true,
            })
            .collect());
    }
    let full = std::path::absolute(path).map_err(|e| e.to_string())?;
    if !full.is_dir() {
        return Err(format!("{}: {}", t(ko, "폴더를 찾을 수 없습니다", "Folder not found"), full.display()));
    }
    let mut rows = Vec::new();
    if let Some(parent) = full.parent() {
        rows.push(LocalRow {
            name: "..".into(),
            kind: t(ko, "상위 폴더", "Parent folder").into(),
            size_text: String::new(),
            modified_text: String::new(),
            path: parent.to_path_buf(),
            is_dir: true,
            is_parent: true,
            is_drive: false,
        });
    }
    let now = std::time::SystemTime::now();
    let entries = ndir::fsops::list_dir(&full, show_hidden)?;
    let row = |e: &Entry| LocalRow {
        name: e.name.clone(),
        kind: ndir::listing::type_name(&e.name, e.is_dir, ko),
        size_text: if e.is_dir { String::new() } else { ndir::parity::format_size(e.size, "Auto", true) },
        modified_text: ndir::listing::date_text(e.modified, "yyyy-MM-dd", "HH:mm", false, ko, now),
        path: e.path.clone(),
        is_dir: e.is_dir,
        is_parent: false,
        is_drive: false,
    };
    rows.extend(entries.iter().filter(|e| e.is_dir).map(row));
    rows.extend(entries.iter().filter(|e| !e.is_dir).map(row));
    Ok(rows)
}

/// C# LoadListingAsync의 줄: 폴더 먼저, 이름(대소문자 무시) 순, "/"가 아니면 맨 위에 "..".
pub(super) fn remote_rows(directory: &str, entries: &[RemoteEntry], ko: bool) -> Vec<RemoteRow> {
    let now = std::time::SystemTime::now();
    let mut rows: Vec<RemoteRow> = entries
        .iter()
        .map(|e| RemoteRow {
            name: e.name.clone(),
            kind: if e.is_dir { t(ko, "폴더", "Folder") } else { t(ko, "파일", "File") }.into(),
            size_text: if e.is_dir { String::new() } else { ndir::parity::format_size(e.size, "Auto", true) },
            modified_text: e
                .modified
                .map(|s| ndir::listing::date_text(Some(std::time::UNIX_EPOCH + Duration::from_secs(s.max(0) as u64)), "yyyy-MM-dd", "HH:mm", false, ko, now))
                .unwrap_or_default(),
            remote_path: format!("{}/{}", directory.trim_end_matches('/'), e.name),
            mode: e.mode,
            size: (!e.is_dir).then_some(e.size),
            is_dir: e.is_dir,
            is_parent: false,
        })
        .collect();
    rows.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    if directory != "/" {
        rows.insert(0, RemoteRow {
            name: "..".into(),
            kind: t(ko, "상위 폴더", "Parent folder").into(),
            size_text: String::new(),
            modified_text: String::new(),
            remote_path: String::new(),
            mode: None,
            size: None,
            is_dir: true,
            is_parent: true,
        });
    }
    rows
}

/// 바깥(서버·파일 대화 상자)을 건드리는 함수들(테스트에서 바꿔 끼운다).
#[derive(Clone, Copy)]
pub(super) struct RemoteHooks {
    pub connect: fn(&Config) -> Result<Client, String>,
    /// 여러 파일 열기 대화 상자(제목, 처음 폴더).
    pub open_files: fn(&str, Option<&Path>) -> Vec<PathBuf>,
    /// 원본 패널 FTP 모드의 연결(ftp_pane::PaneFs). 시험에서는 가짜 원격 폴더.
    pub pane_connect: fn(&Config) -> Result<Box<dyn super::ftp_pane::PaneFs>, String>,
}

impl Default for RemoteHooks {
    fn default() -> Self {
        Self { connect: Client::connect, open_files: ndir::file_dialogs::open_files, pane_connect: super::ftp_pane::connect_client }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Busy {
    Connecting,
    Listing,
    Navigating,
    Working,
}

/// 원격 목록 오른쪽 단추 메뉴의 동작 뒤에 할 일.
#[derive(Clone, Debug)]
enum RemoteOp {
    Refresh,
    /// 삭제 뒤: 다시 읽고 남은 항목을 고른다.
    Delete { remaining: HashSet<String> },
    /// 권한 뒤: 다시 읽고 남은 항목을 고르고 상태 글.
    Permissions { remaining: HashSet<String>, status: String, failures: Vec<String> },
}

#[derive(Clone, Debug)]
enum TransferKind {
    Download { destination: String },
    Upload,
    UploadFolder { name: String },
}

/// ▲(선택한 로컬 항목 올리기)는 폴더마다 폴더 업로드, 그 뒤 파일들을 차례로 한다.
#[derive(Clone, Debug)]
enum UploadStep {
    Folder(PathBuf),
    Files(Vec<PathBuf>),
}

enum RemoteEvent {
    LocalListed { version: u64, path: String, history: Option<usize>, result: Result<Vec<LocalRow>, String> },
    Connected { session: u64, client: Client, path: String },
    ConnectFailed { session: u64, error: String, config: Box<Config> },
    Listed { session: u64, listing: u64, path: String, result: Result<Vec<RemoteEntry>, String> },
    Navigated { session: u64, previous: String, remember: bool, result: Result<(String, Option<String>), String> },
    Done { session: u64, op: RemoteOp, result: Result<(), String>, caption: String },
    Progress { path: String, bytes: u64, total: Option<u64>, completed: usize },
    TransferDone { session: u64, kind: TransferKind, result: Result<(), String>, cancelled: bool, completed: usize, transferred: usize, warnings: usize },
}

pub(super) struct RemoteForm {
    ko: bool,
    pub protocol: usize,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub recent_hosts: Vec<String>,
    pub add_to_sites: bool,
    pub passive: bool,
    pub profile: Option<SiteProfile>,
    /// 연 패널(닫을 때 내려받은 로컬 폴더로 옮긴다).
    pub pane: usize,
    client: Option<Arc<Mutex<Client>>>,
    pub connected: bool,
    pub busy: Option<Busy>,
    session: u64,
    listing_version: u64,
    local_version: u64,
    pub remote_path: String,
    pub remote_path_box: String,
    pub remote_history: Vec<String>,
    pub items: Vec<RemoteRow>,
    pub remote_grid: GridState,
    pub listing_valid: bool,
    listed_path: Option<String>,
    pending_status: Option<String>,
    pending_select: Option<HashSet<String>>,
    pub local_path: String,
    pub local_path_box: String,
    pub local_items: Vec<LocalRow>,
    pub local_grid: GridState,
    pub local_history: Vec<String>,
    pub local_index: Option<usize>,
    pub local_loading: bool,
    show_hidden: bool,
    show_vertical_bar: bool,
    pub focus_remote: bool,
    pub status: String,
    pub downloaded: bool,
    tx: Sender<RemoteEvent>,
    rx: Receiver<RemoteEvent>,
    uploads: VecDeque<UploadStep>,
    /// 이번 연결에서 신뢰한다고 답한 SFTP 서버 키.
    accepted_fingerprint: Option<String>,
}

pub(super) const PROTOCOLS: [&str; 3] = ["FTP", "FTPS", "SFTP"];

impl RemoteForm {
    fn protocol_name(&self) -> &'static str {
        PROTOCOLS[self.protocol.min(2)]
    }
    /// 빠른 연결 선택(사이트 목록에 추가, 수동 모드, 알림)이 보이는가.
    pub(super) fn quick_options_visible(&self) -> bool {
        self.profile.is_none() && self.protocol < 2
    }
    pub(super) fn selected_remote(&self) -> Vec<RemoteRow> {
        self.remote_grid.selected_in_order().into_iter().filter_map(|i| self.items.get(i)).filter(|r| !r.is_parent).cloned().collect()
    }
    pub(super) fn selected_local(&self) -> Vec<LocalRow> {
        self.local_grid.selected_in_order().into_iter().filter_map(|i| self.local_items.get(i)).filter(|r| !r.is_parent).cloned().collect()
    }
    /// SelectedItem(커서 줄, 없으면 첫 선택 줄).
    fn current_remote(&self) -> Option<RemoteRow> {
        let index = self.remote_grid.cursor.filter(|c| self.remote_grid.selected.contains(c)).or_else(|| self.remote_grid.selected_in_order().first().copied())?;
        self.items.get(index).cloned()
    }
    fn current_local(&self) -> Option<LocalRow> {
        let index = self.local_grid.cursor.filter(|c| self.local_grid.selected.contains(c)).or_else(|| self.local_grid.selected_in_order().first().copied())?;
        self.local_items.get(index).cloned()
    }
    fn local_box_text(&self) -> String {
        if self.local_path == DRIVES { "Computer".into() } else { self.local_path.clone() }
    }
}

/// 창 단추·글쇠.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RemoteAction {
    Protocol(usize),
    Connect,
    Disconnect,
    LocalBack,
    LocalForward,
    LocalUp,
    LocalGo,
    LocalRefresh,
    LocalOpen,
    RemoteBack,
    RemoteUp,
    RemoteGo,
    RemoteRefresh,
    RemoteOpen,
    UploadSelected,
    Download,
    Upload,
    UploadFolder,
    Close,
    NewFolder,
    Rename,
    Delete,
    Permissions,
}

/// 창 그리기(이름표는 부르는 쪽이 언어 파일에서 넘긴다).
pub(super) struct RemoteLabels {
    pub connect: String,
    pub disconnect: String,
    pub download: String,
    pub upload: String,
    pub add_to_sites: String,
    pub passive: String,
    pub password_notice: String,
    pub back: String,
    pub forward: String,
    pub menu: [String; 4],
    /// 원본 TfFTPQuickConnect lURL·lPort·lUser·lPassword("A&ddress:" 등, & 없이).
    pub fields: [String; 4],
    /// C# ApplyLegacyVerticalTransferIcons: 스킨 [VertBar] vbbtnimgs·vbbtnoverimgs(vertbarbtns.png·_over) 띠. 0칸 = 올리기, 1칸 = 내려받기.
    pub transfer_icons: [Option<egui::TextureHandle>; 2],
}

pub(super) fn remote_ui(ui: &mut egui::Ui, form: &mut RemoteForm, labels: &RemoteLabels, settings: &ndir::settings::Settings) -> Option<RemoteAction> {
    let ko = form.ko;
    let mut action = None;
    if form.busy.is_some() || form.local_loading {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    ui.set_width(1024.0);
    // 연결 줄.
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("remote-protocol").width(90.0).selected_text(form.protocol_name()).show_opaque(ui, |ui| {
                for (index, name) in PROTOCOLS.iter().enumerate() {
                    if ui.selectable_label(form.protocol == index, *name).clicked() && form.protocol != index {
                        action = Some(RemoteAction::Protocol(index));
                    }
                }
            });
            ui.label(&labels.fields[0]);
            ui.add(egui::TextEdit::singleline(&mut form.host).desired_width(176.0).hint_text(t(ko, "호스트 이름 또는 IP", "Host name or IP")));
            if !form.recent_hosts.is_empty() {
                ui.menu_button("▼", |ui| {
                    for host in form.recent_hosts.clone() {
                        if ui.button(&host).clicked() {
                            form.host = host;
                            ui.close();
                        }
                    }
                });
            }
            ui.label(&labels.fields[1]);
            ui.add(egui::TextEdit::singleline(&mut form.port).desired_width(52.0).hint_text(t(ko, "포트", "Port")));
            ui.label(&labels.fields[2]);
            ui.add(egui::TextEdit::singleline(&mut form.user).desired_width(140.0).hint_text(t(ko, "사용자 이름", "User name")));
            ui.label(&labels.fields[3]);
            ui.add(egui::TextEdit::singleline(&mut form.password).password(true).desired_width(150.0).hint_text(t(ko, "비밀번호 (저장하지 않음)", "Password (not saved)")));
            if ui.add_enabled(form.busy != Some(Busy::Connecting), egui::Button::new(&labels.connect)).clicked() {
                action = Some(RemoteAction::Connect);
            }
            if ui.add_enabled(form.connected, egui::Button::new(&labels.disconnect)).clicked() {
                action = Some(RemoteAction::Disconnect);
            }
            if form.quick_options_visible() {
                ui.checkbox(&mut form.add_to_sites, &labels.add_to_sites);
                ui.checkbox(&mut form.passive, &labels.passive);
                ui.label(RichText::new(&labels.password_notice).small().weak());
            }
        });
    });
    ui.add_space(6.0);
    let half = 488.0;
    // 경로 줄.
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(half, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let back = form.local_index.is_some_and(|i| i > 0) && !form.local_loading;
            let forward = form.local_index.is_some_and(|i| i + 1 < form.local_history.len()) && !form.local_loading;
            if ui.add_enabled(back, egui::Button::new("←")).on_hover_text(&labels.back).clicked() {
                action = Some(RemoteAction::LocalBack);
            }
            if ui.add_enabled(forward, egui::Button::new("→")).on_hover_text(&labels.forward).clicked() {
                action = Some(RemoteAction::LocalForward);
            }
            if ui.button("↑").on_hover_text(t(ko, "상위 폴더", "Parent folder")).clicked() {
                action = Some(RemoteAction::LocalUp);
            }
            let response = ui.add(egui::TextEdit::singleline(&mut form.local_path_box).desired_width(half - 190.0));
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                action = Some(RemoteAction::LocalGo);
            }
            if ui.button(t(ko, "이동", "Go")).clicked() {
                action = Some(RemoteAction::LocalGo);
            }
            if ui.button("↻").on_hover_text(t(ko, "로컬 폴더 새로 고침", "Refresh local folder")).clicked() {
                action = Some(RemoteAction::LocalRefresh);
            }
        });
        ui.allocate_ui_with_layout(egui::vec2(36.0, 26.0), egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
            ui.label(RichText::new("⇄").weak());
        });
        ui.allocate_ui_with_layout(egui::vec2(half, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if ui.button("←").clicked() {
                action = Some(RemoteAction::RemoteBack);
            }
            if ui.button("↑").clicked() {
                action = Some(RemoteAction::RemoteUp);
            }
            let response = ui.add(egui::TextEdit::singleline(&mut form.remote_path_box).desired_width(half - 150.0));
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                action = Some(RemoteAction::RemoteGo);
            }
            if ui.button(t(ko, "이동", "Go")).clicked() {
                action = Some(RemoteAction::RemoteGo);
            }
            if ui.button("↻").on_hover_text(t(ko, "원격 폴더 새로 고침", "Refresh remote folder")).clicked() {
                action = Some(RemoteAction::RemoteRefresh);
            }
        });
    });
    ui.add_space(6.0);
    // 목록 둘.
    let height = 470.0;
    let typing = ui.ctx().wants_keyboard_input();
    let local_columns = [
        Column::text(t(ko, "이름", "Name"), half * 0.38),
        Column::text(t(ko, "종류", "Type"), half * 0.19),
        Column::right(t(ko, "크기", "Size"), half * 0.19),
        Column::text(t(ko, "수정한 날짜", "Date modified"), half * 0.24),
    ];
    let remote_columns = [
        Column::text(t(ko, "이름", "Name"), half * 0.27),
        Column::text(t(ko, "종류", "Type"), half * 0.13),
        Column::right(t(ko, "크기", "Size"), half * 0.13),
        Column::text(t(ko, "수정한 날짜", "Date modified"), half * 0.19),
        Column::text(t(ko, "원격 경로", "Remote path"), half * 0.28),
    ];
    let menu_labels = labels.menu.clone();
    let menu = move |ui: &mut egui::Ui| -> Option<usize> { menu_labels.iter().position(|label| ui.button(label).clicked()) };
    ui.horizontal_top(|ui| {
        ui.allocate_ui(egui::vec2(half, height + 26.0), |ui| {
            let view = GridView { id: "remote-local", columns: &local_columns, height, enabled: !form.local_loading, sortable: false, multi_select: true, keyboard: !form.focus_remote && !typing, context_menu: None, row_color: None, background: None };
            let rows = &form.local_items;
            let events = grid::show(ui, &view, &mut form.local_grid, &|row, column| {
                let r = &rows[row];
                match column {
                    0 => r.name.clone(),
                    1 => r.kind.clone(),
                    2 => r.size_text.clone(),
                    _ => r.modified_text.clone(),
                }
            }, &|_| false);
            if events.clicked {
                form.focus_remote = false;
            }
            if events.double_clicked.is_some() {
                form.focus_remote = false;
                action = Some(RemoteAction::LocalOpen);
            }
        });
        ui.allocate_ui_with_layout(egui::vec2(36.0, height), egui::Layout::top_down(egui::Align::Center), |ui| {
            if form.show_vertical_bar {
                ui.add_space(height / 2.0 - 40.0);
                // C# ApplyLegacyVerticalTransferIcons: 스킨 띠의 0칸(올리기)·1칸(내려받기), 포인터가 올라가면 over 띠. 도움말 = miFTPPut·miFTPGet.
                let [normal, over] = &labels.transfer_icons;
                let transfer_button = |ui: &mut egui::Ui, index: usize, glyph: &str, tip: &str| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                    let texture = if ui.rect_contains_pointer(rect) { over.as_ref().or(normal.as_ref()) } else { normal.as_ref() };
                    // egui Image는 띠 전체 비로 맞춘 뒤 uv를 써서 칸이 납작해지므로, 단추 틀 위에 칸(16×16)을 바로 그린다.
                    let response = match texture {
                        Some(_) => ui.put(rect, egui::Button::new("").min_size(rect.size())),
                        None => ui.put(rect, egui::Button::new(glyph)),
                    };
                    if let Some(texture) = texture {
                        let uv = egui::Rect::from_min_max(egui::pos2(index as f32 / 10.0, 0.0), egui::pos2((index + 1) as f32 / 10.0, 1.0));
                        ui.painter().image(texture.id(), egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0)), uv, Color32::WHITE);
                    }
                    response.on_hover_text(tip).clicked()
                };
                if transfer_button(ui, 0, "▲", &labels.upload) {
                    action = Some(RemoteAction::UploadSelected);
                }
                ui.add_space(8.0);
                if transfer_button(ui, 1, "▼", &labels.download) {
                    action = Some(RemoteAction::Download);
                }
            }
        });
        ui.allocate_ui(egui::vec2(half, height + 26.0), |ui| {
            let view = GridView { id: "remote-remote", columns: &remote_columns, height, enabled: true, sortable: false, multi_select: true, keyboard: form.focus_remote && !typing, context_menu: Some(&menu), row_color: None, background: None };
            let rows = &form.items;
            let events = grid::show(ui, &view, &mut form.remote_grid, &|row, column| {
                let r = &rows[row];
                match column {
                    0 => r.name.clone(),
                    1 => r.kind.clone(),
                    2 => r.size_text.clone(),
                    3 => r.modified_text.clone(),
                    _ => r.remote_path.clone(),
                }
            }, &|_| false);
            if events.clicked || events.menu.is_some() {
                form.focus_remote = true;
            }
            if events.double_clicked.is_some() {
                form.focus_remote = true;
                action = Some(RemoteAction::RemoteOpen);
            }
            if let Some(choice) = events.menu {
                action = Some([RemoteAction::NewFolder, RemoteAction::Rename, RemoteAction::Delete, RemoteAction::Permissions][choice.min(3)]);
            }
        });
    });
    // 목록 글쇠(C# LocalGrid/RemoteGrid PreviewKeyDown).
    if !typing && action.is_none() {
        let ctx = ui.ctx().clone();
        if form.focus_remote {
            if form.connected && form.busy.is_none() {
                if pressed(&ctx, settings, "open") && form.current_remote().is_some_and(|r| r.is_dir) {
                    action = Some(RemoteAction::RemoteOpen);
                } else if pressed(&ctx, settings, "refreshF5") || pressed(&ctx, settings, "refreshCtrlR") {
                    action = Some(RemoteAction::RemoteRefresh);
                } else if pressed(&ctx, settings, "legacyChangeAttributes") && !form.selected_remote().is_empty() {
                    action = Some(RemoteAction::Permissions);
                }
            }
        } else if !form.local_loading {
            if pressed(&ctx, settings, "historyBack") {
                action = Some(RemoteAction::LocalBack);
            } else if pressed(&ctx, settings, "historyForward") {
                action = Some(RemoteAction::LocalForward);
            } else if pressed(&ctx, settings, "open") && form.current_local().is_some() {
                action = Some(RemoteAction::LocalOpen);
            } else if pressed(&ctx, settings, "refreshF5") || pressed(&ctx, settings, "refreshCtrlR") {
                action = Some(RemoteAction::LocalRefresh);
            }
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(&form.status).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(t(ko, "닫기", "Close")).clicked() {
                action = Some(RemoteAction::Close);
            }
            if ui.button(t(ko, "폴더 선택 업로드…", "Upload folder…")).clicked() {
                action = Some(RemoteAction::UploadFolder);
            }
            if ui.button(&labels.upload).clicked() {
                action = Some(RemoteAction::Upload);
            }
            if ui.button(&labels.download).clicked() {
                action = Some(RemoteAction::Download);
            }
        });
    });
    action
}

/// RemoteTransferProgressWindow(600×250, 자세히 보기면 400).
pub(super) struct TransferForm {
    ko: bool,
    pub status: String,
    /// 0..1, 전체 크기를 모르면 None(움직이는 막대).
    pub progress: Option<f32>,
    pub details: Vec<String>,
    current_detail: Option<String>,
    pub show_details: bool,
    pub paused: bool,
    pub cancelling: bool,
    pause: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransferAction {
    Pause,
    Details,
    Cancel,
}

/// C# FormatBytes: N1 단위.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let text = format!("{value:.1}");
    let (integer, fraction) = text.split_once('.').unwrap_or((&text, "0"));
    format!("{}.{fraction} {}", grouped(integer.parse().unwrap_or(0)), UNITS[unit])
}

impl TransferForm {
    fn new(ko: bool, pause: Arc<AtomicBool>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            ko,
            status: t(ko, "전송을 준비하고 있습니다.", "Preparing the transfer.").into(),
            progress: None,
            details: Vec::new(),
            current_detail: None,
            show_details: false,
            paused: false,
            cancelling: false,
            pause,
            cancel,
        }
    }

    /// RenderLatestProgress.
    pub(super) fn show_progress(&mut self, path: &str, bytes: u64, total: Option<u64>, completed: usize) {
        let ko = self.ko;
        let count = format!("{} {}{}", t(ko, "완료", "Completed"), grouped(completed as u64), t(ko, "개", ""));
        let size = match total {
            Some(total) if total > 0 => format!(" · {} / {}", format_bytes(bytes), format_bytes(total)),
            _ if bytes > 0 => format!(" · {}", format_bytes(bytes)),
            _ => String::new(),
        };
        if !self.cancelling {
            self.status = format!("{path}\n{count}{size}");
        }
        self.progress = total.filter(|t| *t > 0).map(|total| (bytes as f64 / total as f64).clamp(0.0, 1.0) as f32);
        if self.current_detail.as_deref() != Some(path) {
            self.current_detail = Some(path.to_owned());
            self.details.push(path.to_owned());
            if self.details.len() > 500 {
                self.details.remove(0);
            }
        }
        let progress = match self.progress {
            Some(p) => format!("{}%", (p as f64 * 100.0).round()),
            None => format_bytes(bytes),
        };
        if let Some(last) = self.details.last_mut() {
            *last = format!("{path} — {progress}");
        }
    }

    /// RequestCancel.
    fn request_cancel(&mut self) {
        if self.cancelling {
            return;
        }
        self.cancelling = true;
        self.status = t(self.ko, "전송을 중단하고 있습니다.", "Stopping the transfer.").into();
        self.cancel.store(true, Ordering::SeqCst);
    }
}

pub(super) fn transfer_ui(ui: &mut egui::Ui, form: &mut TransferForm) -> Option<TransferAction> {
    let ko = form.ko;
    let mut action = None;
    ui.ctx().request_repaint_after(Duration::from_millis(100));
    ui.set_width(560.0);
    ui.add(egui::Label::new(form.status.as_str()).wrap());
    ui.add_space(12.0);
    match form.progress {
        Some(p) => ui.add(egui::ProgressBar::new(p).desired_height(10.0)),
        None => ui.add(egui::ProgressBar::new(0.0).animate(true).desired_height(10.0)),
    };
    if form.show_details {
        ui.add_space(12.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("remote-transfer-details").max_height(140.0).stick_to_bottom(true).show(ui, |ui| {
                for line in &form.details {
                    ui.label(line);
                }
            });
        });
    }
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        if ui.add_enabled(!form.cancelling, egui::Button::new(t(ko, "중단", "Abort")).min_size(egui::vec2(88.0, 26.0))).clicked() {
            action = Some(TransferAction::Cancel);
        }
        let details = if form.show_details { t(ko, "간단히 보기", "Fewer details") } else { t(ko, "자세히 보기", "More details") };
        if ui.add(egui::Button::new(details).min_size(egui::vec2(88.0, 26.0))).clicked() {
            action = Some(TransferAction::Details);
        }
        let pause = if form.paused { t(ko, "계속", "Resume") } else { t(ko, "일시 중지", "Pause") };
        if ui.add_enabled(!form.cancelling, egui::Button::new(pause).min_size(egui::vec2(88.0, 26.0))).clicked() {
            action = Some(TransferAction::Pause);
        }
    });
    action
}

/// RemotePermissionsWindow(590×420).
#[derive(Clone, Debug)]
pub(super) struct PermissionForm {
    pub name: String,
    pub path: String,
    /// 소유자·그룹·나머지 × 읽기·쓰기·실행.
    pub bits: [[bool; 3]; 3],
    pub mode_text: String,
}

/// RemotePermissionAction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PermissionAction {
    ApplyCurrent,
    ApplyAllRemaining,
    SkipCurrent,
}

/// GetNumericValue: 권한 비트를 "644" 같은 10진 숫자로.
pub(super) fn numeric_value(mode: u32) -> u32 {
    ((mode >> 6) & 7) * 100 + ((mode >> 3) & 7) * 10 + (mode & 7)
}

impl PermissionForm {
    pub(super) fn new(mode: u32, name: &str, path: &str) -> Self {
        let mut form = Self { name: name.into(), path: path.into(), bits: [[false; 3]; 3], mode_text: String::new() };
        form.set_mode(mode & 0o777);
        form
    }
    fn set_mode(&mut self, mode: u32) {
        for group in 0..3 {
            for permission in 0..3 {
                self.bits[group][permission] = mode & (0o400 >> (group * 3 + permission)) != 0;
            }
        }
        self.mode_text = numeric_value(mode).to_string();
    }
    pub(super) fn mode(&self) -> u32 {
        let mut mode = 0;
        for group in 0..3 {
            for permission in 0..3 {
                if self.bits[group][permission] {
                    mode |= 0o400 >> (group * 3 + permission);
                }
            }
        }
        mode
    }
    /// Mode_TextChanged: 세 자리이고 778보다 작으면 확인란을 맞춘다(각 자리 & (4 >> 칸)).
    pub(super) fn text_changed(&mut self) {
        self.mode_text.retain(|c| c.is_ascii_digit());
        self.mode_text.truncate(3);
        if self.mode_text.len() != 3 {
            return;
        }
        let Ok(value) = self.mode_text.parse::<u32>() else { return };
        if value >= 778 {
            return;
        }
        let digits = [value / 100, value / 10 % 10, value % 10];
        for group in 0..3 {
            for permission in 0..3 {
                self.bits[group][permission] = digits[group] & (4 >> permission) != 0;
            }
        }
    }
}

/// 이름표: [formFTPAttributes].
pub(super) fn permission_ui(ui: &mut egui::Ui, form: &mut PermissionForm, language: &compat::Language, enter: bool, shift_enter: bool, escape: bool) -> Option<Option<PermissionAction>> {
    let text = |key: &str, fallback: &str| language.text(&format!("formFTPAttributes.{key}"), fallback);
    let mut result = None;
    // 원본 TfFTPAttributes(ClientWidth 355 × ClientHeight 242): lFileName(8,12 굵게), gbOwner(8,39)·gbGroup(123,39)·gbUser(237,39) 110×110
    // (확인란 위 28/51/74), lEdit(10,167) eAttr(131,164 57×21 세 자리), TntBevel1(8,197 339), btnAll(10,207) btnSkip(90,207)
    // btnOk(192,207) btnCancel(272,207) 75×25.
    let (area, _) = ui.allocate_exact_size(egui::vec2(355.0, 242.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    super::original_dialogs::put_left(ui, at(8.0, 12.0, 339.0, 15.0), egui::Label::new(RichText::new(&form.name).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()))).truncate()).on_hover_text(if form.path.trim().is_empty() { &form.name } else { &form.path });
    let groups = [(8.0, 8.0, "GroupBox.gbOwner", "Owner"), (123.0, 6.0, "GroupBox.gbGroup", "Group"), (237.0, 7.0, "GroupBox.gbUser", "Public")];
    let names = [
        [("Caption.chkORead", "Read(&Q)"), ("Caption.chkOWrite", "Write(&A)"), ("Caption.chkOExecute", "Execute(&Z)")],
        [("Caption.chkGRead", "Read(&W)"), ("Caption.chkGWrite", "Write(&S)"), ("Caption.chkGExecute", "Execute(&X)")],
        [("Caption.chkURead", "Read(&E)"), ("Caption.chkUWrite", "Write(&D)"), ("Caption.chkUExecute", "Execute(&C)")],
    ];
    let mut changed = false;
    for (group, (left, inset, key, fallback)) in groups.into_iter().enumerate() {
        super::original_dialogs::group_box(ui, at(left, 39.0, 110.0, 110.0), &text(key, fallback));
        for permission in 0..3 {
            let (key, fallback) = names[group][permission];
            let rect = at(left + inset, 39.0 + 28.0 + 23.0 * permission as f32, 98.0, 17.0);
            changed |= super::original_dialogs::put_left(ui, rect, egui::Checkbox::new(&mut form.bits[group][permission], text(key, fallback))).changed();
        }
    }
    if changed {
        form.mode_text = numeric_value(form.mode()).to_string();
    }
    super::original_dialogs::put_left(ui, at(10.0, 167.0, 118.0, 15.0), egui::Label::new(text("Caption.lEdit", "Numeric Value(&T):")).truncate());
    let attr = ui.put(at(131.0, 164.0, 57.0, 21.0), egui::TextEdit::singleline(&mut form.mode_text).char_limit(3));
    if attr.changed() {
        form.text_changed();
    }
    // 원본 FormKeyDown(0x427334, KeyPreview): End(0x23)이고 eAttr(+0x414)에 초점이 없으면 ModalResult = mrAll(btnAll).
    let end = !attr.has_focus() && ui.input(|i| i.key_pressed(egui::Key::End));
    ui.painter().hline(area.min.x + 8.0..=area.min.x + 347.0, area.min.y + 197.0, ui.visuals().widgets.noninteractive.bg_stroke);
    let buttons = [
        (10.0, "Caption.btnAll", "All", Some(PermissionAction::ApplyAllRemaining)),
        (90.0, "Caption.btnSkip", "Skip", Some(PermissionAction::SkipCurrent)),
        (192.0, "Caption.btnOk", "OK", Some(PermissionAction::ApplyCurrent)),
        (272.0, "Caption.btnCancel", "Cancel", None),
    ];
    for (left, key, fallback, action) in buttons {
        let response = ui.put(at(left, 207.0, 75.0, 25.0), egui::Button::new(text(key, fallback)));
        let response = if key == "Caption.btnAll" { response.on_hover_text("Shift+Enter") } else { response };
        if response.clicked() {
            result = Some(action);
        }
    }
    if result.is_none() {
        // Esc = 건너뛰기, Shift+Enter·End = 모두(원본 FormKeyDown), Enter = 확인(IsDefault).
        if escape {
            result = Some(Some(PermissionAction::SkipCurrent));
        } else if shift_enter || end {
            result = Some(Some(PermissionAction::ApplyAllRemaining));
        } else if enter {
            result = Some(Some(PermissionAction::ApplyCurrent));
        }
    }
    result
}

/// 권한 바꾸기 진행 상태(C# ChangeRemotePermissionsAsync의 지역 변수).
struct PermissionRun {
    entries: Vec<RemoteRow>,
    index: usize,
    apply_all: Option<u32>,
    default_confirmed: bool,
    plan: Vec<(RemoteRow, u32)>,
    skipped: usize,
}

fn spawn_event(tx: &Sender<RemoteEvent>, job: impl FnOnce() -> RemoteEvent + Send + 'static) {
    let tx = tx.clone();
    thread::spawn(move || {
        let _ = tx.send(job());
    });
}

/// 중단을 눌렀으면 "Cancelled"(Rust remote 모듈과 같은 글).
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) { Err("Cancelled".into()) } else { Ok(()) }
}

/// 일시 중지면 기다린다(중단하면 바로 돌아온다).
fn wait_if_paused(pause: &AtomicBool, cancel: &AtomicBool) {
    while pause.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(50));
    }
}

impl App {
    pub(super) fn remote_form(&mut self) -> Option<&mut RemoteForm> {
        self.original_dialogs.iter_mut().rev().find_map(|d| match &mut d.kind {
            DialogKind::Remote(form) => Some(form.as_mut()),
            _ => None,
        })
    }

    pub(super) fn transfer_form(&mut self) -> Option<&mut TransferForm> {
        self.original_dialogs.iter_mut().rev().find_map(|d| match &mut d.kind {
            DialogKind::RemoteTransfer(form) => Some(form.as_mut()),
            _ => None,
        })
    }

    pub(super) fn remote_labels(&self) -> RemoteLabels {
        let menu = |key: &str, fallback: &str| self.language_pack.text(&format!("formMain.Menu.{key}"), fallback);
        let text = |id: &str, fallback: &str| self.language_pack.text(id, fallback);
        RemoteLabels {
            connect: menu("miFTPConnect", "Connect"),
            disconnect: menu("miFTPDisconnect", "Disconnect"),
            download: menu("miFTPGet", "Download"),
            upload: menu("miFTPPut", "Upload"),
            add_to_sites: text("formFTPQuickConnect.Caption.chkAddToSites", "Add to the FTP site list"),
            passive: text("formFTPQuickConnect.Caption.chkPassive", "Passive Mode"),
            password_notice: text("NDIRRemoteSites.PasswordStoredWhenSaved", "Password saved when adding a site"),
            back: menu("miHistoryBack", "Back"),
            forward: menu("miHistoryForward", "Forward"),
            menu: [
                text("formFolderTree.Menu.miCreateNewFolder", "New remote folder"),
                text("formFolderTree.Menu.miRename", "Rename"),
                text("formFolderTree.Menu.miDelete", "Delete"),
                text("formFTPAttributes.Form.Caption", "Change remote permissions"),
            ],
            fields: [
                text("formFTPQuickConnect.Caption.lURL", "A&ddress:").replace('&', ""),
                text("formFTPQuickConnect.Caption.lPort", "&Port:").replace('&', ""),
                text("formFTPQuickConnect.Caption.lUser", "&User:").replace('&', ""),
                text("formFTPQuickConnect.Caption.lPassword", "&Password:").replace('&', ""),
            ],
            transfer_icons: [None, None],
        }
    }

    /// C# OpenRemoteWindow(profile, initialHost): 활성 패널의 폴더(압축 안이면 압축 파일 폴더)에서 연다.
    pub(super) fn open_remote_window(&mut self, profile: Option<SiteProfile>, initial_host: Option<String>) {
        let ko = self.language == "ko";
        let pane = self.active;
        let p = &self.panes[pane];
        let destination = if let Some(archive) = &p.archive {
            archive.file.parent().map(Path::to_path_buf).unwrap_or_default()
        } else {
            p.path.clone()
        };
        let local = if destination.is_dir() { destination } else { native::documents_folder().unwrap_or_default() };
        let show_hidden = self.settings.flag(if pane == 0 { "LeftShowHiddenFiles" } else { "RightShowHiddenFiles" }, false);
        let mut recent: Vec<String> = Vec::new();
        for host in self.settings.strings("RecentRemoteHosts") {
            if !host.trim().is_empty() && !recent.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
                recent.push(host);
            }
        }
        let (tx, rx) = mpsc::channel();
        let mut form = RemoteForm {
            ko,
            protocol: 1,
            host: String::new(),
            port: "21".into(),
            user: String::new(),
            password: String::new(),
            recent_hosts: recent,
            add_to_sites: false,
            passive: profile.as_ref().is_none_or(|p| p.passive),
            profile: None,
            pane,
            client: None,
            connected: false,
            busy: None,
            session: 0,
            listing_version: 0,
            local_version: 0,
            remote_path: "/".into(),
            remote_path_box: "/".into(),
            remote_history: Vec::new(),
            items: Vec::new(),
            remote_grid: GridState::new(0),
            listing_valid: false,
            listed_path: None,
            pending_status: None,
            pending_select: None,
            local_path: local.to_string_lossy().into_owned(),
            local_path_box: String::new(),
            local_items: Vec::new(),
            local_grid: GridState::new(0),
            local_history: Vec::new(),
            local_index: None,
            local_loading: false,
            show_hidden,
            show_vertical_bar: self.settings.flag("ShowVerticalBar", true),
            focus_remote: false,
            status: t(ko, "호스트에 연결하세요.", "Connect to a host.").into(),
            downloaded: false,
            tx,
            rx,
            uploads: VecDeque::new(),
            accepted_fingerprint: None,
        };
        if let Some(site) = &profile {
            form.protocol = match site.protocol.to_ascii_uppercase().as_str() {
                "FTP" => 0,
                "FTPS" => 1,
                _ => 2,
            };
            form.host = site.host.clone();
            form.port = site.port.to_string();
            form.user = site.user.clone();
            form.password = native::unprotect_secret(&site.encrypted_password).unwrap_or_default();
            form.remote_path = if site.remote_directory.trim().is_empty() { "/".into() } else { site.remote_directory.clone() };
            form.remote_path_box = form.remote_path.clone();
        } else if let Some(host) = initial_host {
            form.protocol = 0;
            form.host = host;
        }
        let connect_now = profile.is_some();
        form.profile = profile;
        let title = t(ko, "NDIR 원격 연결", "NDIR Remote Connection").to_owned();
        self.push_original_dialog(title, DialogKind::Remote(Box::new(form)), |_, _, _| {});
        let path = self.remote_form().map(|f| f.local_path.clone()).unwrap_or_default();
        self.remote_read_local(path, None);
        if connect_now {
            self.remote_connect();
        }
    }


    pub(super) fn remote_action(&mut self, action: RemoteAction, ctx: &egui::Context) {
        match action {
            RemoteAction::Protocol(index) => {
                let Some(form) = self.remote_form() else { return };
                form.protocol = index;
                form.port = if index == 2 { "22" } else { "21" }.into();
                if form.connected || form.busy == Some(Busy::Connecting) {
                    self.remote_disconnect();
                }
            }
            RemoteAction::Connect => self.remote_connect(),
            RemoteAction::Disconnect => self.remote_disconnect(),
            RemoteAction::LocalBack | RemoteAction::LocalForward => {
                let forward = action == RemoteAction::LocalForward;
                let Some(form) = self.remote_form() else { return };
                if form.local_loading {
                    return;
                }
                let Some(index) = form.local_index.and_then(|i| if forward { i.checked_add(1) } else { i.checked_sub(1) }).filter(|i| *i < form.local_history.len()) else { return };
                let path = form.local_history[index].clone();
                self.remote_read_local(path, Some(index));
            }
            RemoteAction::LocalUp => {
                let Some(form) = self.remote_form() else { return };
                if form.local_path == DRIVES {
                    return;
                }
                let full = std::path::absolute(&form.local_path).unwrap_or_else(|_| PathBuf::from(&form.local_path));
                let next = match full.parent() {
                    None => DRIVES.to_owned(),
                    Some(parent) => parent.to_string_lossy().into_owned(),
                };
                self.remote_navigate_local(next);
            }
            RemoteAction::LocalGo => {
                let Some(form) = self.remote_form() else { return };
                let path = form.local_path_box.trim().to_owned();
                self.remote_navigate_local(if path.eq_ignore_ascii_case("Computer") { DRIVES.into() } else { path });
            }
            RemoteAction::LocalRefresh => {
                let Some(form) = self.remote_form() else { return };
                let path = form.local_path.clone();
                self.remote_read_local(path, None);
            }
            RemoteAction::LocalOpen => self.remote_open_local(),
            RemoteAction::RemoteBack => {
                let Some(form) = self.remote_form() else { return };
                if !form.connected || form.busy.is_some() {
                    return;
                }
                let target = form.remote_history.pop().unwrap_or_else(|| "/".into());
                self.remote_navigate(target, false);
            }
            RemoteAction::RemoteUp => {
                let Some(form) = self.remote_form() else { return };
                let target = parent_path(&form.remote_path);
                self.remote_navigate(target, true);
            }
            RemoteAction::RemoteGo => {
                let Some(form) = self.remote_form() else { return };
                let target = form.remote_path_box.clone();
                self.remote_navigate(target, true);
            }
            RemoteAction::RemoteRefresh => {
                if self.remote_form().is_some_and(|f| f.busy.is_none()) {
                    self.remote_load_listing();
                }
            }
            RemoteAction::RemoteOpen => {
                let Some(form) = self.remote_form() else { return };
                let Some(entry) = form.current_remote() else { return };
                if entry.is_parent {
                    let target = parent_path(&form.remote_path);
                    self.remote_navigate(target, true);
                } else if entry.is_dir {
                    self.remote_navigate(entry.remote_path, true);
                }
            }
            RemoteAction::UploadSelected => self.remote_upload_selected_local(),
            RemoteAction::Download => self.remote_download(),
            RemoteAction::Upload => self.remote_upload_files(None),
            RemoteAction::UploadFolder => self.remote_upload_folder(None),
            RemoteAction::Close => self.remote_close(ctx),
            RemoteAction::NewFolder => self.remote_new_folder(),
            RemoteAction::Rename => self.remote_rename(),
            RemoteAction::Delete => self.remote_delete(),
            RemoteAction::Permissions => self.remote_permissions(),
        }
    }

    fn remote_box(&mut self, caption: &str, text: &str) {
        self.system_box(caption, text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
    }

    /// MessageBox.Show(this, 글): 캡션·아이콘 없음.
    fn remote_notice(&mut self, korean: &str, english: &str) {
        let text = self.tr(korean, english).into_owned();
        self.system_box("", &text, MB_OK, |_, _, _| {});
    }

    /// C# ReadLocalPathAsync.
    fn remote_read_local(&mut self, path: String, history: Option<usize>) {
        let Some(form) = self.remote_form() else { return };
        form.local_version += 1;
        form.local_loading = true;
        let (version, show_hidden, ko) = (form.local_version, form.show_hidden, form.ko);
        spawn_event(&form.tx, move || {
            let result = list_local(&path, show_hidden, ko);
            RemoteEvent::LocalListed { version, path, history, result }
        });
    }

    /// C# NavigateLocalAsync: 환경 변수를 풀고 전체 경로로, 없는 폴더면 알림.
    fn remote_navigate_local(&mut self, path: String) {
        if path == DRIVES {
            return self.remote_read_local(path, None);
        }
        let full = std::path::absolute(native::expand_environment(&path)).map_err(|e| e.to_string()).and_then(|full| {
            if full.is_dir() { Ok(full) } else { Err(format!("Local folder not found: {}", full.display())) }
        });
        match full {
            Ok(full) => self.remote_read_local(full.to_string_lossy().into_owned(), None),
            Err(error) => {
                let Some(form) = self.remote_form() else { return };
                form.local_path_box = form.local_box_text();
                form.status = format!("{}: {error}", t(form.ko, "로컬 폴더를 열지 못했습니다", "Local folder could not be opened"));
                let caption = self.tr("로컬 폴더", "Local folder").into_owned();
                self.remote_box(&caption, &error);
            }
        }
    }

    /// C# OpenSelectedLocalAsync: 폴더는 들어가고 파일은 연다(작업 폴더 = 지금 로컬 폴더).
    fn remote_open_local(&mut self) {
        let Some(form) = self.remote_form() else { return };
        if form.local_loading {
            return;
        }
        let Some(item) = form.current_local() else { return };
        if item.is_dir || item.is_parent {
            return self.remote_navigate_local(item.path.to_string_lossy().into_owned());
        }
        let directory = PathBuf::from(&form.local_path);
        if let Err(error) = (self.shell_runner)(&item.path, "", &directory) {
            let caption = self.tr("파일 열기", "Open file").into_owned();
            self.remote_box(&caption, &error);
        }
    }

    /// C# DisconnectAsync.
    fn remote_disconnect(&mut self) {
        let Some(form) = self.remote_form() else { return };
        form.session += 1;
        form.connected = false;
        form.busy = None;
        form.listing_valid = false;
        form.listed_path = None;
        form.pending_status = None;
        form.pending_select = None;
        form.items.clear();
        form.remote_grid = GridState::new(0);
        form.status = t(form.ko, "호스트에 연결하세요.", "Connect to a host.").into();
        if let Some(client) = form.client.take() {
            thread::spawn(move || {
                if let Ok(mut client) = client.lock() {
                    let _ = client.close();
                }
            });
        }
    }

    /// C# ConnectAsync/ConnectCoreAsync.
    fn remote_connect(&mut self) {
        let Some(form) = self.remote_form() else { return };
        if form.busy == Some(Busy::Connecting) {
            return;
        }
        let accepted = form.accepted_fingerprint.take();
        self.remote_disconnect();
        let ko = self.language == "ko";
        let connect = self.remote_hooks.connect;
        let Some(form) = self.remote_form() else { return };
        let protocol = form.protocol_name();
        let host = prepare_host(&form.host);
        let port = if protocol == "SFTP" {
            match form.port.trim().parse::<i64>() {
                Ok(port) if (1..=65535).contains(&port) && !host.is_empty() && !form.user.is_empty() => port,
                _ => return self.remote_box_plain(t(ko, "호스트, 사용자 이름, 올바른 포트를 입력하세요.", "Enter a host, a user name and a valid port."), t(ko, "원격 연결", "Remote connection")),
            }
        } else {
            parse_port(&form.port)
        };
        if host.is_empty() {
            return self.remote_box_plain(t(ko, "호스트, 사용자 이름, 올바른 포트를 입력하세요.", "Enter a host, a user name and a valid port."), t(ko, "원격 연결", "Remote connection"));
        }
        let connection_protocol = if form.profile.is_none() { quick_connect_protocol(protocol, port) } else { protocol.to_owned() };
        form.status = format!("{protocol} {}", t(ko, "서버에 연결하는 중…", "connecting to the server…"));
        let mut quick_site = None;
        if form.profile.is_none() && form.add_to_sites && protocol != "SFTP" {
            // 원본은 연결 전에 입력값을 사이트로 쓴다(인증이 실패해도). 호스트는 입력 그대로, 처음 폴더는 빈 값.
            quick_site = Some(SiteProfile {
                name: form.host.clone(),
                protocol: connection_protocol.clone(),
                host: form.host.clone(),
                port,
                user: form.user.clone(),
                encrypted_password: native::protect_secret(&form.password).unwrap_or_default(),
                remote_directory: String::new(),
                passive: form.passive,
                encoding_name: "UTF-8".into(),
                ..Default::default()
            });
        }
        let profile = form.profile.clone();
        let fingerprint = accepted.or_else(|| profile.as_ref().map(|p| p.known_host_fingerprint.clone()).filter(|f| !f.is_empty())).unwrap_or_default();
        let private_key = profile.as_ref().map(|p| PathBuf::from(&p.private_key_path)).filter(|p| !p.as_os_str().is_empty() && p.is_file()).unwrap_or_default();
        let config = Config {
            name: profile.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
            protocol: connection_protocol.clone(),
            host: host.clone(),
            // FtpTransportPort: unchecked((ushort)port).
            port: port as u16,
            user: form.user.clone(),
            password: form.password.clone(),
            key_passphrase: if private_key.as_os_str().is_empty() { String::new() } else { profile.as_ref().map(|p| native::unprotect_secret(&p.encrypted_key_passphrase).unwrap_or_default()).unwrap_or_default() },
            private_key,
            fingerprint,
            passive: profile.as_ref().map_or(form.passive, |p| p.passive),
            encoding_name: profile.as_ref().map(|p| p.encoding_name.clone()).unwrap_or_default(),
        };
        let initial = profile.as_ref().map(|p| p.remote_directory.clone()).unwrap_or_default();
        form.busy = Some(Busy::Connecting);
        let session = form.session;
        let sftp = connection_protocol == "SFTP";
        let tx = form.tx.clone();
        if let Some(site) = quick_site {
            self.save_quick_connect_site(site);
        }
        thread::spawn(move || {
            let event = if !sftp && config.port == 0 {
                RemoteEvent::ConnectFailed { session, error: "포트 0에는 연결할 수 없습니다.".into(), config: Box::new(config) }
            } else {
                match connect(&config).and_then(|mut client| {
                    let path = if sftp {
                        let home = client.pwd()?;
                        if initial.is_empty() { home } else { client.change_directory(&home, &initial)?.0 }
                    } else {
                        let directory = if initial.is_empty() { "." } else { initial.as_str() };
                        client.command(&format!("CWD {directory}"))?;
                        client.pwd()?
                    };
                    Ok((client, path))
                }) {
                    Ok((client, path)) => RemoteEvent::Connected { session, client, path },
                    Err(error) => RemoteEvent::ConnectFailed { session, error, config: Box::new(config) },
                }
            };
            let _ = tx.send(event);
        });
    }

    fn remote_box_plain(&mut self, text: &str, caption: &str) {
        self.system_box(caption, text, MB_OK, |_, _, _| {});
    }

    /// C# SaveQuickConnectSite: 목록 끝에 더하고(같은 이름도 따로) 저장, 상태 줄.
    pub(super) fn save_quick_connect_site(&mut self, site: SiteProfile) {
        let mut sites = load_sites(&self.settings);
        let name = site.name.clone();
        sites.push(site);
        store_sites(&mut self.settings, &sites);
        // C# SaveQuickConnectSite: 저장에 실패하면 경고 상자(제목 SaveFailedTitle, 내용 = 오류), 성공하면 상태 줄.
        match self.try_save() {
            Ok(()) => self.status = self.language_pack.text("NDIRRemoteSites.QuickConnectSaved", "Saved FTP site: {0}").replace("{0}", &name),
            Err(error) => {
                let title = self.language_pack.text("NDIRRemoteSites.SaveFailedTitle", "FTP site save failed");
                self.system_box(&title, &error, ndir::msgbox::MB_OK | ndir::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
            }
        }
    }

    /// C# RecordRemoteHost: 같은 것(대소문자 무시)을 빼고 맨 앞에, 20개까지.
    fn record_remote_host(&mut self, host: &str) {
        let value = host.trim();
        if value.is_empty() {
            return;
        }
        let mut hosts = self.settings.strings("RecentRemoteHosts");
        hosts.retain(|h| !h.eq_ignore_ascii_case(value));
        hosts.insert(0, value.to_owned());
        hosts.truncate(20);
        self.settings.set_strings("RecentRemoteHosts", hosts);
        // C# RecordRemoteHost: 저장에 실패하면 상태 줄에 HostHistorySaveFailed.
        if let Err(error) = self.try_save() {
            self.status = self.language_pack.text("NDIRRemoteSites.HostHistorySaveFailed", "Could not save recent remote host history: {0}").replace("{0}", &error);
        }
    }

    /// C# LoadListingAsync.
    fn remote_load_listing(&mut self) {
        let Some(form) = self.remote_form() else { return };
        if !form.connected {
            return;
        }
        let Some(client) = form.client.clone() else { return };
        form.listing_version += 1;
        if form.busy.is_none() {
            form.busy = Some(Busy::Listing);
        }
        form.listing_valid = false;
        let directory = form.remote_path.clone();
        form.status = format!("{directory} {}", t(form.ko, "읽는 중…", "reading…"));
        let (session, listing) = (form.session, form.listing_version);
        spawn_event(&form.tx, move || {
            let result = client.lock().map_err(|_| "Connection lock failed".to_owned()).and_then(|mut c| c.list(&directory));
            RemoteEvent::Listed { session, listing, path: directory, result }
        });
    }

    /// C# NavigateAsync.
    fn remote_navigate(&mut self, path: String, remember: bool) {
        let Some(form) = self.remote_form() else { return };
        if !form.connected || form.busy.is_some() {
            return;
        }
        let Some(client) = form.client.clone() else { return };
        form.busy = Some(Busy::Navigating);
        let previous = form.remote_path.clone();
        let session = form.session;
        spawn_event(&form.tx, move || {
            let result = client.lock().map_err(|_| "Connection lock failed".to_owned()).and_then(|mut c| c.change_directory(&previous, &path));
            RemoteEvent::Navigated { session, previous, remember, result }
        });
    }

    /// 창의 일꾼 알림(App::poll에서 매번).
    pub(super) fn poll_remote_window(&mut self, ctx: &egui::Context) {
        let Some(form) = self.remote_form() else { return };
        let events: Vec<RemoteEvent> = form.rx.try_iter().collect();
        for event in events {
            self.remote_event(event, ctx);
        }
    }

    fn remote_event(&mut self, event: RemoteEvent, ctx: &egui::Context) {
        let ko = self.language == "ko";
        match event {
            RemoteEvent::LocalListed { version, path, history, result } => {
                let Some(form) = self.remote_form() else { return };
                if version != form.local_version {
                    return;
                }
                form.local_loading = false;
                match result {
                    Ok(rows) => {
                        form.local_path = path.clone();
                        if let Some(index) = history {
                            form.local_index = Some(index);
                        } else if form.local_index.is_none_or(|i| !form.local_history[i].eq_ignore_ascii_case(&path)) {
                            let keep = form.local_index.map_or(0, |i| i + 1);
                            form.local_history.truncate(keep);
                            form.local_history.push(path);
                            if form.local_history.len() > HISTORY_CAPACITY {
                                form.local_history.remove(0);
                            }
                            form.local_index = Some(form.local_history.len() - 1);
                        }
                        form.local_grid = GridState::new(rows.len());
                        form.local_items = rows;
                        form.local_path_box = form.local_box_text();
                    }
                    Err(error) => {
                        form.local_path_box = form.local_box_text();
                        form.status = format!("{}: {error}", t(ko, "로컬 폴더를 읽지 못했습니다", "Local folder could not be read"));
                    }
                }
            }
            RemoteEvent::Connected { session, client, path } => {
                let Some(form) = self.remote_form() else { return };
                if session != form.session {
                    thread::spawn(move || {
                        let mut client = client;
                        let _ = client.close();
                    });
                    return;
                }
                form.client = Some(Arc::new(Mutex::new(client)));
                form.connected = true;
                form.busy = None;
                form.remote_path = path.clone();
                form.remote_path_box = path;
                form.remote_history.clear();
                form.status = format!("{} {} · {}{}", form.protocol_name(), t(ko, "연결됨", "connected"), form.local_path, t(ko, "로 다운로드", " is the download folder"));
                let host = prepare_host(&form.host);
                self.record_remote_host(&host);
                self.remote_load_listing();
            }
            RemoteEvent::ConnectFailed { session, error, config } => {
                let Some(form) = self.remote_form() else { return };
                if session != form.session {
                    return;
                }
                form.busy = None;
                let challenge = serde_json::from_str::<Value>(&error).ok().filter(|v| matches!(v.get("kind").and_then(Value::as_str), Some("unknown_host_key" | "host_key_mismatch")));
                if let Some(challenge) = challenge {
                    let changed = challenge["kind"] == "host_key_mismatch";
                    let candidate = challenge["candidate"].as_str().unwrap_or_default().to_owned();
                    let title = self.language_pack.text(if changed { "NDIRSFTPHostKey.ChangedTitle" } else { "NDIRSFTPHostKey.FirstUseTitle" }, if changed { "SFTP server key changed" } else { "SFTP server verification" });
                    let host_label = self.language_pack.text("NDIRSFTPHostKey.HostLabel", "Host");
                    let fingerprint_label = self.language_pack.text("NDIRSFTPHostKey.FingerprintLabel", "SHA-256");
                    let question = self.language_pack.text("NDIRSFTPHostKey.TrustQuestion", "Trust this key and continue?");
                    let prompt = format!("{host_label}: {}:{}\r\n{fingerprint_label}: {candidate}\r\n\r\n{question}", config.host, config.port);
                    self.system_box(&title, &prompt, MB_YESNO | MB_ICONEXCLAMATION, move |app, answer, _| app.remote_host_key_answered(answer == IDYES, candidate));
                    return;
                }
                self.remote_connect_failed(&error);
            }
            RemoteEvent::Listed { session, listing, path, result } => {
                let Some(form) = self.remote_form() else { return };
                if session != form.session || listing != form.listing_version || !form.connected || path != form.remote_path {
                    return;
                }
                form.busy = None;
                match result {
                    Ok(entries) => {
                        let rows = remote_rows(&path, &entries, ko);
                        form.remote_grid = GridState::new(rows.len());
                        if let Some(select) = form.pending_select.take() {
                            for (index, row) in rows.iter().enumerate() {
                                if select.contains(&row.remote_path) {
                                    form.remote_grid.selected.insert(index);
                                }
                            }
                        }
                        form.items = rows;
                        form.listing_valid = true;
                        form.listed_path = Some(path.clone());
                        form.status = format!("{path} · {}{}", grouped(entries.len() as u64), t(ko, "개 항목", " items"));
                        if let Some(status) = form.pending_status.take() {
                            form.status = status;
                        }
                    }
                    Err(error) => {
                        if form.listed_path.as_deref() != Some(path.as_str()) {
                            form.items.clear();
                            form.remote_grid = GridState::new(0);
                        }
                        form.status = form.pending_status.take().unwrap_or_else(|| t(ko, "원격 폴더를 읽지 못했습니다.", "The remote folder could not be read.").into());
                        form.pending_select = None;
                        let caption = t(ko, "원격 폴더", "Remote folder");
                        self.remote_box(caption, &error);
                    }
                }
            }
            RemoteEvent::Navigated { session, previous, remember, result } => {
                let Some(form) = self.remote_form() else { return };
                if session != form.session || !form.connected {
                    return;
                }
                form.busy = None;
                match result {
                    Ok((next, rejected)) => {
                        if remember && next != previous {
                            form.remote_history.push(previous);
                        }
                        form.remote_path = next.clone();
                        form.remote_path_box = next;
                        form.pending_status = rejected;
                        self.remote_load_listing();
                        if let Some(form) = self.remote_form() {
                            form.busy = Some(Busy::Navigating);
                        }
                    }
                    Err(error) => {
                        form.remote_path_box = form.remote_path.clone();
                        form.status = t(ko, "원격 폴더로 이동하지 못했습니다.", "Could not open the remote folder.").into();
                        let caption = t(ko, "원격 폴더", "Remote folder");
                        self.remote_box(caption, &error);
                    }
                }
            }
            RemoteEvent::Done { session, op, result, caption } => {
                let Some(form) = self.remote_form() else { return };
                if session != form.session {
                    return;
                }
                form.busy = None;
                match op {
                    RemoteOp::Refresh => {}
                    RemoteOp::Delete { remaining } => form.pending_select = Some(remaining),
                    RemoteOp::Permissions { remaining, status, failures } => {
                        form.pending_select = Some(remaining);
                        form.pending_status = Some(status);
                        if !failures.is_empty() {
                            let mut summary = failures.iter().take(10).cloned().collect::<Vec<_>>().join("\r\n");
                            if failures.len() > 10 {
                                summary.push_str(&format!("\r\n{} {}{}", t(ko, "외", "and"), grouped(failures.len() as u64 - 10), t(ko, "개 항목", " more items")));
                            }
                            let caption = t(ko, "권한을 변경하지 못한 항목", "Items whose permissions were not changed");
                            self.remote_box(caption, &summary);
                        }
                    }
                }
                if let Err(error) = result {
                    self.remote_box(&caption, &error);
                }
                self.remote_load_listing();
            }
            RemoteEvent::Progress { path, bytes, total, completed } => {
                if let Some(form) = self.transfer_form() {
                    form.show_progress(&path, bytes, total, completed);
                }
            }
            RemoteEvent::TransferDone { session, kind, result, cancelled, completed, transferred, warnings } => {
                if self.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::RemoteTransfer(_))) {
                    self.answer_dialog(DialogResult::RemoteTransfer, ctx);
                } else {
                    self.original_dialogs.retain(|d| !matches!(d.kind, DialogKind::RemoteTransfer(_)));
                }
                self.remote_transfer_done(session, kind, result, cancelled, completed, transferred, warnings);
            }
        }
    }

    fn remote_connect_failed(&mut self, error: &str) {
        self.remote_disconnect();
        let ko = self.language == "ko";
        if let Some(form) = self.remote_form() {
            form.status = t(ko, "연결하지 못했습니다.", "Could not connect.").into();
        }
        self.remote_box(t(ko, "원격 연결 실패", "Remote connection failed"), error);
    }

    /// SFTP HostKeyReceived 상자의 답: 예면 이 키로 다시 연결(사이트면 KnownHostFingerprint에 저장), 아니요면 연결 실패.
    fn remote_host_key_answered(&mut self, trust: bool, fingerprint: String) {
        if !trust {
            return self.remote_connect_failed("Key exchange negotiation failed.");
        }
        let Some(form) = self.remote_form() else { return };
        let mut saved = None;
        if let Some(profile) = form.profile.as_mut() {
            let before = profile.clone();
            profile.known_host_fingerprint = fingerprint.clone();
            saved = Some((before, profile.clone()));
        }
        form.accepted_fingerprint = Some(fingerprint);
        if let Some((before, after)) = saved {
            // C#: 창을 닫은 뒤 그 사이트가 RemoteSites에 있으면 지문을 저장한다.
            let mut sites = load_sites(&self.settings);
            if let Some(site) = sites.iter_mut().find(|s| **s == before) {
                *site = after;
                store_sites(&mut self.settings, &sites);
                self.save();
            }
        }
        self.remote_connect();
    }

    /// 닫기(×·Esc·닫기 단추): 연결을 끊고 창을 닫는다. 내려받았으면 연 패널을 그 로컬 폴더로.
    fn remote_close(&mut self, ctx: &egui::Context) {
        self.remote_disconnect();
        let Some(form) = self.remote_form() else { return };
        let (downloaded, local, pane) = (form.downloaded, form.local_path.clone(), form.pane);
        if self.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::Remote(_))) {
            self.answer_dialog(DialogResult::Remote, ctx);
        } else {
            self.original_dialogs.retain(|d| !matches!(d.kind, DialogKind::Remote(_)));
        }
        if downloaded && local != DRIVES && Path::new(&local).is_dir() {
            self.navigate(pane, PathBuf::from(local), true);
        }
    }

    /// 전송 진행 창을 열고 일꾼을 돌린다.
    fn remote_start_transfer(&mut self, title: String, kind: TransferKind, job: impl FnOnce(&mut Client, &AtomicBool, &AtomicBool, &dyn Fn(String, u64, Option<u64>, usize)) -> (Result<(), String>, usize, usize) + Send + 'static) {
        let ko = self.language == "ko";
        let Some(form) = self.remote_form() else { return };
        let Some(client) = form.client.clone() else { return };
        let pause = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let (worker_pause, worker_cancel) = (pause.clone(), cancel.clone());
        let tx = form.tx.clone();
        let session = form.session;
        form.busy = Some(Busy::Working);
        thread::spawn(move || {
            let progress_tx = tx.clone();
            let report = move |path: String, bytes: u64, total: Option<u64>, completed: usize| {
                let _ = progress_tx.send(RemoteEvent::Progress { path, bytes, total, completed });
            };
            let (result, completed, transferred, warnings) = match client.lock() {
                Ok(mut client) => {
                    let (result, completed, transferred) = job(&mut client, &worker_pause, &worker_cancel, &report);
                    (result, completed, transferred, client.take_warnings().len())
                }
                Err(_) => (Err("Connection lock failed".into()), 0, 0, 0),
            };
            let cancelled = worker_cancel.load(Ordering::SeqCst) && result.as_ref().err().is_none_or(|e| e == "Cancelled");
            let result = if cancelled { Ok(()) } else { result };
            let _ = tx.send(RemoteEvent::TransferDone { session, kind, result, cancelled, completed, transferred, warnings });
        });
        self.push_original_dialog(title, DialogKind::RemoteTransfer(Box::new(TransferForm::new(ko, pause, cancel))), |_, _, _| {});
    }

    pub(super) fn transfer_action(&mut self, action: TransferAction) {
        let Some(form) = self.transfer_form() else { return };
        match action {
            TransferAction::Pause => {
                form.paused = !form.paused;
                form.pause.store(form.paused, Ordering::SeqCst);
            }
            TransferAction::Details => form.show_details = !form.show_details,
            TransferAction::Cancel => form.request_cancel(),
        }
    }

    /// 진행 창을 닫으려 하면(×·Esc) 중단을 요청한다(OnClosing).
    pub(super) fn transfer_close_requested(&mut self) {
        if let Some(form) = self.transfer_form() {
            form.request_cancel();
        }
    }

    /// 전송이 끝난 뒤(C# Download/Upload/UploadFolder의 끝).
    #[allow(clippy::too_many_arguments)]
    fn remote_transfer_done(&mut self, session: u64, kind: TransferKind, result: Result<(), String>, cancelled: bool, completed: usize, transferred: usize, warnings: usize) {
        let ko = self.language == "ko";
        let Some(form) = self.remote_form() else { return };
        if session != form.session {
            return;
        }
        form.busy = None;
        let suffix = if warnings == 0 { String::new() } else { format!(" · {} {}{}", t(ko, "시간 보존 실패", "timestamps not preserved:"), grouped(warnings as u64), t(ko, "개", "")) };
        let done = grouped(completed as u64);
        let error = result.err();
        let (failed, stopped, success, caption) = match &kind {
            TransferKind::Download { .. } => (
                format!("{} {done}{}{suffix}", t(ko, "다운로드 실패. 완료", "Download failed. Completed"), t(ko, "개", "")),
                format!("{} {done}{}{suffix}", t(ko, "다운로드를 중단했습니다. 완료", "Download stopped. Completed"), t(ko, "개", "")),
                format!("{done}{}{suffix}", t(ko, "개 항목을 다운로드했습니다.", " items downloaded.")),
                t(ko, "다운로드 실패", "Download failed"),
            ),
            TransferKind::Upload => (
                format!("{} {done}{}{suffix}", t(ko, "업로드 실패. 완료", "Upload failed. Completed"), t(ko, "개", "")),
                format!("{} {done}{}{suffix}", t(ko, "업로드를 중단했습니다. 완료", "Upload stopped. Completed"), t(ko, "개", "")),
                format!("{done}{}{suffix}", t(ko, "개 파일을 업로드했습니다.", " files uploaded.")),
                t(ko, "업로드 실패", "Upload failed"),
            ),
            TransferKind::UploadFolder { name } => (
                format!("{} {done}{}{suffix}", t(ko, "폴더 업로드 실패. 완료", "Folder upload failed. Completed"), t(ko, "개", "")),
                format!("{} {done}{}{suffix}", t(ko, "폴더 업로드를 중단했습니다. 완료", "Folder upload stopped. Completed"), t(ko, "개", "")),
                format!("{}: {name}{suffix}", t(ko, "폴더를 업로드했습니다", "Folder uploaded")),
                t(ko, "폴더 업로드 실패", "Folder upload failed"),
            ),
        };
        let status = if error.is_some() { failed } else if cancelled { stopped } else { success };
        form.status = status.clone();
        let ok = error.is_none() && !cancelled;
        if !ok {
            form.uploads.clear();
        }
        match &kind {
            TransferKind::Download { destination } => {
                form.downloaded |= transferred > 0;
                let reload = transferred > 0 && !form.local_loading && *destination == form.local_path;
                if let Some(error) = &error {
                    self.remote_box(caption, error);
                }
                if reload {
                    let path = destination.clone();
                    self.remote_read_local(path, None);
                }
            }
            TransferKind::Upload | TransferKind::UploadFolder { .. } => {
                if let Some(error) = &error {
                    self.remote_box(caption, error);
                }
                if transferred > 0 {
                    if let Some(form) = self.remote_form() {
                        form.pending_status = Some(status);
                    }
                    self.remote_load_listing();
                }
            }
        }
        if ok {
            self.remote_next_upload();
        }
    }

    /// C# UploadSelectedLocalAsync: 폴더는 하나씩 폴더 업로드, 그 뒤 파일들.
    fn remote_upload_selected_local(&mut self) {
        let Some(form) = self.remote_form() else { return };
        if form.busy.is_some() {
            return;
        }
        if !form.connected {
            return self.remote_notice("먼저 원격 서버에 연결하세요.", "Connect to a remote server first.");
        }
        if !form.listing_valid {
            return;
        }
        let selected = form.selected_local();
        if selected.is_empty() {
            return self.remote_notice("업로드할 로컬 파일이나 폴더를 선택하세요.", "Select local files or folders to upload.");
        }
        if selected.iter().any(|item| item.is_dir && item.path.parent().is_none()) {
            return self.remote_notice("드라이브 루트 대신 드라이브 안의 폴더를 선택하세요.", "Select a folder inside the drive instead of the drive root.");
        }
        let files: Vec<PathBuf> = selected.iter().filter(|i| !i.is_dir).map(|i| i.path.clone()).collect();
        form.uploads = selected.iter().filter(|i| i.is_dir).map(|i| UploadStep::Folder(i.path.clone())).collect();
        if !files.is_empty() {
            form.uploads.push_back(UploadStep::Files(files));
        }
        self.remote_next_upload();
    }

    fn remote_next_upload(&mut self) {
        let Some(step) = self.remote_form().and_then(|f| f.uploads.pop_front()) else { return };
        match step {
            UploadStep::Folder(path) => self.remote_upload_folder(Some(path)),
            UploadStep::Files(files) => self.remote_upload_files(Some(files)),
        }
    }

    /// C# DownloadSelectedAsync.
    fn remote_download(&mut self) {
        let ko = self.language == "ko";
        let Some(form) = self.remote_form() else { return };
        if !form.connected {
            return self.remote_notice("먼저 원격 서버에 연결하세요.", "Connect to a remote server first.");
        }
        if form.busy.is_some() {
            return;
        }
        let destination = form.local_path.clone();
        if destination == DRIVES {
            return self.remote_notice("로컬 드라이브를 먼저 선택하세요.", "Select a local drive first.");
        }
        let selected = form.selected_remote();
        if selected.is_empty() {
            return self.remote_notice("다운로드할 파일을 선택하세요.", "Select the files to download.");
        }
        let collisions = selected.iter().filter(|e| safe_local_name(&e.name).is_ok_and(|n| std::fs::symlink_metadata(Path::new(&destination).join(n)).is_ok())).count();
        if collisions == 0 {
            return self.remote_begin_download(destination, selected, false);
        }
        let text = format!("{}{collisions}{}", t(ko, "로컬 폴더에 같은 이름의 파일 ", "The local folder already has "), t(ko, "개가 있습니다. 덮어쓸까요?", " items with the same names. Overwrite them?"));
        let caption = t(ko, "이름 충돌", "Name conflict");
        self.system_box(caption, &text, MB_YESNOCANCEL | MB_ICONEXCLAMATION, move |app, answer, _| {
            if answer != IDCANCEL {
                app.remote_begin_download(destination, selected, answer == IDYES);
            }
        });
    }

    fn remote_begin_download(&mut self, destination: String, selected: Vec<RemoteRow>, overwrite: bool) {
        let ko = self.language == "ko";
        if let Err(error) = std::fs::create_dir_all(&destination) {
            return self.remote_box(t(ko, "다운로드 실패", "Download failed"), &error.to_string());
        }
        let title = t(ko, "원격 다운로드", "Remote download").to_owned();
        let kind = TransferKind::Download { destination: destination.clone() };
        self.remote_start_transfer(title, kind, move |client, pause, cancel, report| {
            let (mut completed, mut transferred) = (0usize, 0usize);
            let result = (|| {
                for entry in &selected {
                    wait_if_paused(pause, cancel);
                    check(cancel)?;
                    let local = Path::new(&destination).join(safe_local_name(&entry.name)?);
                    if std::fs::symlink_metadata(&local).is_ok() && !overwrite {
                        completed += 1;
                        report(format!("{}: {}", t(ko, "건너뜀", "Skipped"), entry.name), 0, None, completed);
                        continue;
                    }
                    report(entry.remote_path.clone(), 0, entry.size, completed);
                    let base = completed;
                    let mut files = 0;
                    client.download_tree_with_conflicts(
                        &entry.remote_path,
                        &local,
                        cancel,
                        &mut |_, _| if overwrite { ConflictChoice::Overwrite } else { ConflictChoice::Skip },
                        &mut |p| {
                            wait_if_paused(pause, cancel);
                            files = p.files as usize;
                            report(p.path.clone(), p.bytes, None, base + files);
                        },
                    )?;
                    completed = base + files.max(1);
                    transferred += 1;
                    report(entry.remote_path.clone(), entry.size.unwrap_or(0), entry.size, completed);
                }
                Ok(())
            })();
            (result, completed, transferred)
        });
    }

    /// C# UploadFilesAsync(requestedPaths).
    fn remote_upload_files(&mut self, requested: Option<Vec<PathBuf>>) {
        let ko = self.language == "ko";
        let Some(form) = self.remote_form() else { return };
        if form.busy.is_some() {
            return;
        }
        if !form.connected {
            return self.remote_notice("먼저 원격 서버에 연결하세요.", "Connect to a remote server first.");
        }
        if !form.listing_valid {
            return;
        }
        let paths: Vec<PathBuf> = match requested {
            Some(paths) => {
                let mut unique: Vec<PathBuf> = Vec::new();
                for path in paths.into_iter().filter(|p| p.is_file()) {
                    if !unique.iter().any(|u| u.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy())) {
                        unique.push(path);
                    }
                }
                unique
            }
            None => {
                let initial = if form.local_path == DRIVES { native::documents_folder().unwrap_or_default() } else { PathBuf::from(&form.local_path) };
                (self.remote_hooks.open_files)(t(ko, "업로드할 파일 선택", "Select files to upload"), Some(&initial))
            }
        };
        if paths.is_empty() {
            return;
        }
        let Some(form) = self.remote_form() else { return };
        if !form.connected || form.busy.is_some() || !form.listing_valid {
            return;
        }
        let remote_directory = form.remote_path.clone();
        let collisions: Vec<PathBuf> = paths
            .iter()
            .filter(|file| {
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                form.items.iter().any(|e| !e.is_dir && e.name.eq_ignore_ascii_case(&name))
            })
            .cloned()
            .collect();
        if collisions.is_empty() {
            return self.remote_begin_upload(remote_directory, paths);
        }
        let text = format!("{}{}{}", t(ko, "원격 폴더에 같은 이름의 파일 ", "The remote folder already has "), grouped(collisions.len() as u64), t(ko, "개가 있습니다. 덮어쓸까요?", " files with the same names. Overwrite them?"));
        let caption = t(ko, "원격 이름 충돌", "Remote name conflict");
        self.system_box(caption, &text, MB_YESNOCANCEL | MB_ICONEXCLAMATION, move |app, answer, _| {
            if answer == IDCANCEL {
                if let Some(form) = app.remote_form() {
                    form.uploads.clear();
                }
                return;
            }
            let list = if answer == IDYES { paths } else { paths.into_iter().filter(|p| !collisions.contains(p)).collect() };
            app.remote_begin_upload(remote_directory, list);
        });
    }

    fn remote_begin_upload(&mut self, remote_directory: String, files: Vec<PathBuf>) {
        let ko = self.language == "ko";
        let title = t(ko, "원격 업로드", "Remote upload").to_owned();
        self.remote_start_transfer(title, TransferKind::Upload, move |client, pause, cancel, report| {
            let (mut completed, mut transferred) = (0usize, 0usize);
            let result = (|| {
                for file in &files {
                    wait_if_paused(pause, cancel);
                    check(cancel)?;
                    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let remote = format!("{}/{name}", remote_directory.trim_end_matches('/'));
                    let size = std::fs::metadata(file).map(|m| m.len()).ok();
                    report(remote.clone(), 0, size, completed);
                    client.upload_tree_with_conflicts(file, &remote, cancel, &mut |_, _| ConflictChoice::Overwrite, &mut |p| {
                        wait_if_paused(pause, cancel);
                        report(p.path.clone(), p.bytes, size, completed);
                    })?;
                    completed += 1;
                    transferred += 1;
                    report(remote, size.unwrap_or(0), size, completed);
                }
                Ok(())
            })();
            (result, completed, transferred)
        });
    }

    /// C# UploadFolderAsync(selectedLocalRoot).
    fn remote_upload_folder(&mut self, selected: Option<PathBuf>) {
        let ko = self.language == "ko";
        let Some(form) = self.remote_form() else { return };
        if form.busy.is_some() {
            return;
        }
        if !form.connected {
            return self.remote_notice("먼저 원격 서버에 연결하세요.", "Connect to a remote server first.");
        }
        if !form.listing_valid {
            return;
        }
        let root = match selected {
            Some(root) => root,
            None => {
                let initial = if form.local_path == DRIVES { native::documents_folder().unwrap_or_default() } else { PathBuf::from(&form.local_path) };
                match (self.folder_picker)(t(ko, "원격 서버에 올릴 폴더 선택", "Select a folder to upload"), Some(&initial)) {
                    Some(root) => root,
                    None => return,
                }
            }
        };
        let Some(form) = self.remote_form() else { return };
        if !form.connected || form.busy.is_some() || !form.listing_valid {
            return;
        }
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let remote_root = format!("{}/{name}", form.remote_path.trim_end_matches('/'));
        let title = t(ko, "원격 폴더 업로드", "Remote folder upload").to_owned();
        self.remote_start_transfer(title, TransferKind::UploadFolder { name }, move |client, pause, cancel, report| {
            let mut completed = 0usize;
            report(remote_root.clone(), 0, None, 0);
            let result = client.upload_tree_with_conflicts(&root, &remote_root, cancel, &mut |_, _| ConflictChoice::Overwrite, &mut |p| {
                wait_if_paused(pause, cancel);
                completed = p.files as usize;
                report(p.path.clone(), p.bytes, None, completed);
            });
            // 원격 폴더를 만들었거나 파일을 하나라도 올렸으면 목록을 다시 읽는다.
            let transferred = usize::from(result.is_ok() || completed > 0);
            (result, completed.max(transferred), transferred)
        });
    }

    /// C# CreateRemoteFolderAsync.
    fn remote_new_folder(&mut self) {
        let ko = self.language == "ko";
        self.prompt(t(ko, "새 원격 폴더", "New remote folder"), t(ko, "폴더 이름", "Folder name"), "NewFolder", None, false, move |app, value, _| {
            let Some(name) = value.filter(|n| !n.trim().is_empty() && !n.contains('/') && !n.contains('\\')) else { return };
            let caption = app.remote_labels().menu[0].clone();
            app.remote_run(caption, RemoteOp::Refresh, move |client, directory| client.mkdir(&format!("{}/{name}", directory.trim_end_matches('/'))));
        });
    }

    /// 원격 동작 하나를 일꾼에서(끝나면 다시 읽기). caption = 실패 상자 캡션(메뉴 이름).
    fn remote_run(&mut self, caption: String, op: RemoteOp, job: impl FnOnce(&mut Client, &str) -> Result<(), String> + Send + 'static) {
        let Some(form) = self.remote_form() else { return };
        let Some(client) = form.client.clone() else { return };
        if !form.connected {
            return;
        }
        form.busy = Some(Busy::Working);
        let (session, directory) = (form.session, form.remote_path.clone());
        spawn_event(&form.tx, move || {
            let result = client.lock().map_err(|_| "Connection lock failed".to_owned()).and_then(|mut c| job(&mut c, &directory));
            RemoteEvent::Done { session, op, result, caption }
        });
    }

    /// C# RenameRemoteAsync.
    fn remote_rename(&mut self) {
        let ko = self.language == "ko";
        let caption = self.remote_labels().menu[1].clone();
        let Some(form) = self.remote_form() else { return };
        let Some(entry) = form.current_remote().filter(|e| !e.is_parent) else {
            return self.remote_box(&caption, t(ko, "이름을 바꿀 원격 항목 하나를 선택하세요.", "Select one remote item to rename."));
        };
        self.prompt(t(ko, "원격 이름 변경", "Rename remote item"), t(ko, "새 이름", "New name"), &entry.name.clone(), None, false, move |app, value, _| {
            let Some(name) = value.filter(|n| !n.trim().is_empty() && !n.contains('/') && !n.contains('\\')) else { return };
            let target = format!("{}/{name}", parent_path(&entry.remote_path).trim_end_matches('/'));
            let source = entry.remote_path.clone();
            app.remote_run(caption, RemoteOp::Refresh, move |client, _| client.rename(&source, &target));
        });
    }

    /// C# DeleteRemoteAsync.
    fn remote_delete(&mut self) {
        let ko = self.language == "ko";
        let caption = self.remote_labels().menu[2].clone();
        let Some(form) = self.remote_form() else { return };
        if form.busy.is_some() {
            return;
        }
        let selected = form.selected_remote();
        if selected.is_empty() {
            return self.remote_box(&caption, t(ko, "삭제할 원격 파일이나 폴더를 선택하세요.", "Select the remote files or folders to delete."));
        }
        let text = format!("{}{}{}", t(ko, "원격 항목 ", "Delete "), grouped(selected.len() as u64), t(ko, "개를 서버에서 삭제할까요?", " remote items from the server?"));
        self.system_box(t(ko, "원격 삭제", "Remote delete"), &text, MB_YESNO | MB_ICONEXCLAMATION, move |app, answer, _| {
            if answer != IDYES {
                return;
            }
            let remaining: HashSet<String> = selected.iter().map(|e| e.remote_path.clone()).collect();
            let Some(form) = app.remote_form() else { return };
            let Some(client) = form.client.clone() else { return };
            form.busy = Some(Busy::Working);
            let session = form.session;
            spawn_event(&form.tx, move || {
                let mut remaining = remaining;
                let cancel = AtomicBool::new(false);
                let result = client.lock().map_err(|_| "Connection lock failed".to_owned()).and_then(|mut client| {
                    for entry in &selected {
                        if entry.is_dir { client.delete_tree(&entry.remote_path, &cancel)? } else { client.delete(&entry.remote_path, false)? }
                        remaining.remove(&entry.remote_path);
                    }
                    Ok(())
                });
                RemoteEvent::Done { session, op: RemoteOp::Delete { remaining }, result, caption }
            });
        });
    }

    /// C# ChangeRemotePermissionsAsync: 항목마다 권한 창(모두/건너뛰기/확인/취소), 알 수 없으면 한 번 "기본값 644" 확인.
    fn remote_permissions(&mut self) {
        let ko = self.language == "ko";
        let caption = self.remote_labels().menu[3].clone();
        let Some(form) = self.remote_form() else { return };
        let selected = form.selected_remote();
        if selected.is_empty() {
            return self.remote_box(&caption, t(ko, "원격 권한을 변경할 항목을 선택하세요.", "Select the remote items whose permissions to change."));
        }
        if !form.connected {
            return self.remote_box(&caption, t(ko, "먼저 원격 서버에 연결하세요.", "Connect to a remote server first."));
        }
        self.permission_step(PermissionRun { entries: selected, index: 0, apply_all: None, default_confirmed: false, plan: Vec::new(), skipped: 0 }, false);
    }

    fn permission_step(&mut self, mut run: PermissionRun, stopped: bool) {
        let ko = self.language == "ko";
        if stopped || run.index >= run.entries.len() {
            return self.permission_finish(run, stopped);
        }
        let entry = run.entries[run.index].clone();
        if let Some(mode) = run.apply_all {
            run.plan.push((entry, mode));
            run.index += 1;
            return self.permission_step(run, false);
        }
        let mode = match entry.mode {
            Some(mode) => mode & 0o777,
            None if !run.default_confirmed => {
                let text = t(ko, "서버가 이 항목의 기존 권한을 알려주지 않습니다. 기본값 644로 편집할까요?", "The server did not report this item's permissions. Edit it starting from 644?");
                self.system_box(t(ko, "원격 권한", "Remote permissions"), text, MB_YESNO | MB_ICONEXCLAMATION, move |app, answer, _| {
                    if answer != IDYES {
                        return app.permission_finish(run, true);
                    }
                    run.default_confirmed = true;
                    app.permission_editor(run, 0o644);
                });
                return;
            }
            None => 0o644,
        };
        self.permission_editor(run, mode);
    }

    fn permission_editor(&mut self, run: PermissionRun, mode: u32) {
        let entry = &run.entries[run.index];
        let form = PermissionForm::new(mode, &entry.name, &entry.remote_path);
        let title = self.language_pack.text("formFTPAttributes.Form.Caption", "FTP Change Attributes");
        self.push_original_dialog(title, DialogKind::RemotePermissions(Box::new(form)), move |app, result, _| {
            let mut run = run;
            let DialogResult::RemotePermissions(choice) = result else { return };
            match choice {
                None => app.permission_finish(run, true),
                Some((PermissionAction::SkipCurrent, _)) => {
                    run.skipped += 1;
                    run.index += 1;
                    app.permission_step(run, false);
                }
                Some((action, mode)) => {
                    let entry = run.entries[run.index].clone();
                    run.plan.push((entry, mode));
                    if action == PermissionAction::ApplyAllRemaining {
                        run.apply_all = Some(mode);
                    }
                    run.index += 1;
                    app.permission_step(run, false);
                }
            }
        });
    }

    /// 고른 권한을 적용하고 다시 읽은 뒤 "N/M개 항목 권한 변경, 건너뜀 K개( (중단))".
    fn permission_finish(&mut self, run: PermissionRun, stopped: bool) {
        let ko = self.language == "ko";
        if run.plan.is_empty() && run.skipped == 0 {
            return;
        }
        let total = run.entries.len();
        let skipped = run.skipped;
        let caption = self.remote_labels().menu[3].clone();
        let Some(form) = self.remote_form() else { return };
        let Some(client) = form.client.clone() else { return };
        form.busy = Some(Busy::Working);
        let session = form.session;
        let plan = run.plan;
        let remaining: HashSet<String> = run.entries.iter().map(|e| e.remote_path.clone()).collect();
        spawn_event(&form.tx, move || {
            let mut remaining = remaining;
            let mut failures = Vec::new();
            let mut succeeded = 0u64;
            match client.lock() {
                Ok(mut client) => {
                    for (entry, mode) in &plan {
                        match client.chmod(&entry.remote_path, *mode & 0o777) {
                            Ok(()) => {
                                succeeded += 1;
                                remaining.remove(&entry.remote_path);
                            }
                            Err(error) => failures.push(format!("{}: {error}", entry.name)),
                        }
                    }
                }
                Err(_) => failures.push("Connection lock failed".into()),
            }
            let status = format!(
                "{}/{}{}{}{}",
                grouped(succeeded),
                grouped(total as u64),
                t(ko, "개 항목 권한 변경, 건너뜀 ", " items changed, skipped "),
                grouped(skipped as u64),
                if ko { format!("개{}", if stopped { " (중단)" } else { "" }) } else { (if stopped { " (stopped)" } else { "" }).to_owned() }
            );
            RemoteEvent::Done { session, op: RemoteOp::Permissions { remaining, status, failures }, result: Ok(()), caption }
        });
    }
}
