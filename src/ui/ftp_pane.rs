//! 원본 패널 안 FTP 모드(TfMain pmConnect).
//! - 사이트 연결(miFTPConnect 하위 메뉴, FUN_00440734가 "&1. 이름" … 줄을 만들고 FUN_0043e720 → FUN_004408f8)와
//!   빠른 연결(miFTPQuickConnectClick 0x43ee20 → FUN_0043e9f8: TfFTPQuickConnect, 포트 22면 SFTP, "사이트 목록에 추가").
//!   연결은 지금 패널을 FTP 모드(FUN_00481948 == 2)로 바꾼다.
//! - 끊기(miFTPDisconnectClick 0x43e8d4 → FUN_00440d24: 전송 중이면 FTP_CANNOTDISCONNECT, 아니면 끊고 로컬 폴더로).
//! - 내려받기(miFTPGetClick 0x4412f8: FTP 패널이면 FTP_GET 대상 선택 → FUN_0043f800)와
//!   올리기(miFTPPutClick 0x441168: 한쪽 FTP·한쪽 로컬이어야, 아니면 FTP_PUTCRITERIA → FUN_0043feb0).
//!   전송 중에 또 전송하면 FTPENGINE_BUSY.
//! - 다른 패널로 복사(FUN_0042d518·FUN_0042d910): 로컬→FTP면 올리기, FTP→로컬이면 내려받기(대상 묻지 않음).
//! - FTP 패널의 새 폴더(miNewFolderClick 0x432378 → FUN_0045d0b4), 이름 바꾸기(miRenameClick 0x4351cc → TfRename, FUN_0045d214),
//!   삭제(miDeleteClick 0x431c04 → FUN_0042c734: 늘 삭제 확인(휴지통 없음) → FUN_0045de18).
use super::original_dialogs::{DeleteWarningChoice, DeleteWarningMode, DialogKind, DialogResult};
use super::remote_window::{PermissionAction, PermissionForm, SiteProfile, load_sites, parent_path, parse_port, prepare_host, store_sites};
use super::*;
use ndr::msgbox::{MB_ICONEXCLAMATION, MB_OK};
use ndr::remote::{Client, Config, RemoteEntry, TransferProgress};

/// 원본 FUN_0048d7ac: 기록 칸 처음 높이 0x3c, TSplitter MinSize 0x1e(+0x1f8), 나눔 막대 높이(TSplitter 처음 값 3).
pub(super) const FTP_LOG_HEIGHT: f32 = 60.0;
const FTP_LOG_MIN: f32 = 30.0;
const FTP_SPLITTER_HEIGHT: f32 = 3.0;

/// 패널 FTP 모드가 쓰는 원격 동작(시험에서는 가짜로 바꾼다).
pub(super) trait PaneFs: Send {
    fn pwd(&mut self) -> Result<String, String>;
    fn change_directory(&mut self, current: &str, path: &str) -> Result<(String, Option<String>), String>;
    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>, String>;
    fn download(&mut self, remote: &str, local: &Path, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String>;
    fn upload(&mut self, local: &Path, remote: &str, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String>;
    /// 원본 FTP 엔진 FUN_0045d0b4(새 폴더), FUN_0045d214(이름 바꾸기), FUN_0045de18(폴더·파일 목록 지우기).
    fn mkdir(&mut self, path: &str) -> Result<(), String>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String>;
    fn delete(&mut self, path: &str, is_dir: bool, cancel: &AtomicBool) -> Result<(), String>;
    /// 원본 FUN_0045e274(경로, 권한): FTP SITE CHMOD.
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), String>;
    /// 원본 FUN_0082a178(SendCmd, 응답 코드 아무것이나): FTP 제어 명령 하나.
    fn command(&mut self, command: &str) -> Result<String, String>;
    fn close(&mut self);
}

impl PaneFs for Client {
    fn pwd(&mut self) -> Result<String, String> {
        Client::pwd(self)
    }
    fn change_directory(&mut self, current: &str, path: &str) -> Result<(String, Option<String>), String> {
        Client::change_directory(self, current, path)
    }
    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>, String> {
        Client::list(self, path)
    }
    fn download(&mut self, remote: &str, local: &Path, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String> {
        self.download_tree_with_conflicts(remote, local, cancel, resolve, progress)
    }
    fn upload(&mut self, local: &Path, remote: &str, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String> {
        self.upload_tree_with_conflicts(local, remote, cancel, resolve, progress)
    }
    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        Client::mkdir(self, path)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        Client::rename(self, from, to)
    }
    fn delete(&mut self, path: &str, is_dir: bool, cancel: &AtomicBool) -> Result<(), String> {
        if is_dir { self.delete_tree(path, cancel) } else { Client::delete(self, path, false) }
    }
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), String> {
        Client::chmod(self, path, mode)
    }
    fn command(&mut self, command: &str) -> Result<String, String> {
        Client::command(self, command)
    }
    fn close(&mut self) {
        let _ = Client::close(self);
    }
}

/// 실제 연결(RemoteHooks.pane_connect 처음 값).
pub(super) fn connect_client(config: &Config) -> Result<Box<dyn PaneFs>, String> {
    Client::connect(config).map(|client| Box::new(client) as Box<dyn PaneFs>)
}

/// 원본 FTP 엔진 +0x74(기록 칸): 이 연결이 보내고 받는 제어 줄을 기록에 적는다.
/// 보낸 줄 앞 글(원본 엔진 +0x1c)은 PWD·CWD로 알게 된 지금 원격 폴더.
pub(super) struct LoggedFs {
    pub inner: Box<dyn PaneFs>,
    pub log: Arc<Mutex<ndr::remote::ProtocolLog>>,
}

impl LoggedFs {
    fn run<R>(&mut self, f: impl FnOnce(&mut dyn PaneFs) -> R) -> R {
        let log = self.log.clone();
        let inner = self.inner.as_mut();
        ndr::remote::with_protocol_log(Some(&log), || f(inner))
    }
    fn set_prefix(&self, path: &str) {
        if let Ok(mut log) = self.log.lock() {
            log.prefix = path.to_owned();
        }
    }
}

impl PaneFs for LoggedFs {
    fn pwd(&mut self) -> Result<String, String> {
        let result = self.run(|fs| fs.pwd());
        if let Ok(path) = &result {
            self.set_prefix(path);
        }
        result
    }
    fn change_directory(&mut self, current: &str, path: &str) -> Result<(String, Option<String>), String> {
        let result = self.run(|fs| fs.change_directory(current, path));
        if let Ok((path, _)) = &result {
            self.set_prefix(path);
        }
        result
    }
    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>, String> {
        // 원본은 그 폴더로 CWD(엔진 +0x1c)한 뒤 목록을 받는다. NDR은 경로로 바로 받으므로 그 폴더를 앞 글로 쓴다.
        self.set_prefix(path);
        self.run(|fs| fs.list(path))
    }
    fn download(&mut self, remote: &str, local: &Path, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String> {
        self.run(|fs| fs.download(remote, local, cancel, resolve, progress))
    }
    fn upload(&mut self, local: &Path, remote: &str, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String> {
        self.run(|fs| fs.upload(local, remote, cancel, resolve, progress))
    }
    fn mkdir(&mut self, path: &str) -> Result<(), String> {
        self.run(|fs| fs.mkdir(path))
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        self.run(|fs| fs.rename(from, to))
    }
    fn delete(&mut self, path: &str, is_dir: bool, cancel: &AtomicBool) -> Result<(), String> {
        self.run(|fs| fs.delete(path, is_dir, cancel))
    }
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), String> {
        self.run(|fs| fs.chmod(path, mode))
    }
    fn command(&mut self, command: &str) -> Result<String, String> {
        self.run(|fs| fs.command(command))
    }
    fn close(&mut self) {
        self.run(|fs| fs.close());
    }
}

/// FTP 모드 패널의 연결과 지금 원격 폴더.
#[derive(Clone)]
pub(super) struct PaneRemote {
    pub fs: Arc<Mutex<Box<dyn PaneFs>>>,
    pub host: String,
    pub path: String,
    /// 원본 [+0xd6c]+0x14(FTP 엔진 전송 중).
    pub busy: Arc<AtomicBool>,
    /// 마지막 목록의 권한 비트(원격 경로 → 모드). 원본 항목 +0x74(TfFTPAttributes에 넘기는 값).
    pub modes: Arc<Mutex<std::collections::HashMap<String, u32>>>,
    /// 원본 패널 +0x358 FTP 기록 칸(FUN_0048d7ac)의 줄. SFTP 사이트로 처음 연결하면 없다.
    pub log: Option<Arc<Mutex<ndr::remote::ProtocolLog>>>,
}

impl PaneRemote {
    /// SFTP 사이트인지(원본처럼 FTP 기록 칸이 없다).
    pub(super) fn is_sftp(&self) -> bool {
        self.log.is_none()
    }

    /// 목록 항목의 경로("ftp://호스트/폴더/이름"). 로컬 경로로는 열리지 않는다(":"가 있는 첫 조각).
    pub(super) fn item_path(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("ftp://{}{}", self.host, join_remote(&self.path, name)))
    }
    /// item_path의 원격 부분.
    pub(super) fn remote_of(&self, path: &Path) -> Option<String> {
        let text = path.to_string_lossy().replace('\\', "/");
        let rest = text.strip_prefix(&format!("ftp://{}", self.host)).or_else(|| text.strip_prefix(&format!("ftp:/{}", self.host)))?;
        Some(if rest.is_empty() { "/".to_owned() } else { rest.to_owned() })
    }
    /// 주소 줄 글("ftp://호스트/경로").
    pub(super) fn address(&self) -> String {
        format!("ftp://{}{}", self.host, if self.path.starts_with('/') { self.path.clone() } else { format!("/{}", self.path) })
    }
}

/// 원격 경로 붙이기("/" 구분).
pub(super) fn join_remote(directory: &str, name: &str) -> String {
    if directory.ends_with('/') { format!("{directory}{name}") } else { format!("{directory}/{name}") }
}

/// 원본 FUN_00440734: 사이트 메뉴 줄 글(10개까지 "&1. 이름", 35개까지 "&A. 이름", 그 뒤 "36. 이름").
pub(super) fn site_menu_caption(index: usize, name: &str) -> String {
    if index < 10 {
        format!("&{}. {name}", index + 1)
    } else if index < 35 {
        format!("&{}. {name}", char::from(b'A' + (index - 10) as u8))
    } else {
        format!("{}. {name}", index + 1)
    }
}

/// 원본 TfFTPQuickConnect(cbHost·ePort·eUser·ePassword·chkPassive·chkAddToSites).
#[derive(Clone, Debug)]
pub(super) struct FtpQuickForm {
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub passive: bool,
    pub add_to_sites: bool,
    pub recent_hosts: Vec<String>,
    pub pane: usize,
}

/// 원본 TfFTPQuickConnect(ClientWidth 424 × ClientHeight 164): lURL(13,22) cbHost(94,20 220×21), lPort(320,23) ePort(364,20 49×21 가운데),
/// lUser(13,59) eUser(94,57 121×21), lPassword(13,86) ePassword(94,84 121×21 '*'), chkPassive(291,88), chkAddToSites(13,132),
/// btnOk(255,128 75×25 기본) btnCancel(338,128 75×25). 확인이면 Some(true), 취소면 Some(false).
pub(super) fn ftp_quick_ui(ui: &mut egui::Ui, form: &mut FtpQuickForm, language: &compat::Language, enter: bool) -> Option<bool> {
    let text = |key: &str, fallback: &str| super::favorites_dialog::access_key(&language.text(&format!("formFTPQuickConnect.{key}"), fallback)).0;
    let mut result = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(424.0, 164.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    super::original_dialogs::put_left(ui, at(13.0, 22.0, 80.0, 15.0), egui::Label::new(text("Caption.lURL", "A&ddress:")).truncate());
    let host_width = if form.recent_hosts.is_empty() { 220.0 } else { 198.0 };
    ui.put(at(94.0, 20.0, host_width, 21.0), egui::TextEdit::singleline(&mut form.host));
    if !form.recent_hosts.is_empty() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(at(292.0, 20.0, 22.0, 21.0)), |ui| {
            ui.menu_button("▼", |ui| {
                for host in form.recent_hosts.clone() {
                    if ui.button(&host).clicked() {
                        form.host = host;
                        ui.close();
                    }
                }
            });
        });
    }
    super::original_dialogs::put_left(ui, at(320.0, 23.0, 42.0, 15.0), egui::Label::new(text("Caption.lPort", "&Port:")).truncate());
    ui.put(at(364.0, 20.0, 49.0, 21.0), egui::TextEdit::singleline(&mut form.port).horizontal_align(egui::Align::Center));
    super::original_dialogs::put_left(ui, at(13.0, 59.0, 80.0, 15.0), egui::Label::new(text("Caption.lUser", "&User:")).truncate());
    ui.put(at(94.0, 57.0, 121.0, 21.0), egui::TextEdit::singleline(&mut form.user));
    super::original_dialogs::put_left(ui, at(13.0, 86.0, 80.0, 15.0), egui::Label::new(text("Caption.lPassword", "&Password:")).truncate());
    ui.put(at(94.0, 84.0, 121.0, 21.0), egui::TextEdit::singleline(&mut form.password).password(true));
    super::original_dialogs::put_left(ui, at(291.0, 88.0, 122.0, 17.0), egui::Checkbox::new(&mut form.passive, text("Caption.chkPassive", "Passive Mode(&S)")));
    super::original_dialogs::put_left(ui, at(13.0, 132.0, 220.0, 17.0), egui::Checkbox::new(&mut form.add_to_sites, text("Caption.chkAddToSites", "&Add to the FTP site list")));
    if ui.put(at(255.0, 128.0, 75.0, 25.0), egui::Button::new(text("Caption.btnOk", "&Connect"))).clicked() {
        result = Some(true);
    }
    if ui.put(at(338.0, 128.0, 75.0, 25.0), egui::Button::new(text("Caption.btnCancel", "Cancel"))).clicked() {
        result = Some(false);
    }
    if enter && result.is_none() {
        result = Some(true);
    }
    result
}

/// 원본 lvFTPProgress 한 줄(열 " ;Local;Progress; ;Remote").
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FtpResultRow {
    pub upload: bool,
    pub local: String,
    pub remote: String,
    pub size: Option<u64>,
    pub done: u64,
    pub state: RowState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RowState {
    Waiting,
    Running,
    Done,
    Cancelled,
    Failed(String),
}

/// 원본 FUN_0043f04c 이벤트 1: 전체 진행(받은 바이트, 전체 바이트, 백분율). 전체를 모르면(0) 100%.
pub(super) fn ftp_totals(rows: &[FtpResultRow]) -> (u64, u64, u64) {
    let total: u64 = rows.iter().map(|r| r.size.unwrap_or(0)).sum();
    let done: u64 = rows
        .iter()
        .map(|r| match r.state {
            RowState::Done => r.size.unwrap_or(r.done),
            RowState::Running | RowState::Cancelled | RowState::Failed(_) => r.size.map_or(r.done, |s| r.done.min(s)),
            RowState::Waiting => 0,
        })
        .sum();
    let percent = if total == 0 { 100 } else { (done.min(total) as u128 * 100 / total as u128) as u64 };
    (done, total, percent)
}

/// 원본 lFTPTransferred: wsprintf("%d %% (%s / %s) - %s", 백분율, 받은 크기, 전체 크기, FormatFloat("0.00 KB/s", 속도)).
/// 크기는 StrFormatByteSizeW(format_size).
pub(super) fn ftp_transferred_text(percent: u64, done: &str, total: &str, kb_per_second: f64) -> String {
    format!("{percent} % ({done} / {total}) - {kb_per_second:.2} KB/s")
}

impl FtpResultRow {
    /// 원본 FUN_0043f04c 이벤트 1의 lvFTPProgress 칸 글: Progress 칸 = 백분율/10개의 '>'(올리기, 로컬 → 원격) 또는 '<'(내려받기),
    /// 넷째 칸(오른쪽 맞춤) = "%d%"(wsprintf "%d%%"). 항목 크기를 모르면 원본처럼 전체 백분율(`overall`)을 쓴다.
    /// 기다리는 줄은 둘 다 빈칸, 취소·실패한 줄은 NDR 상태 글(원본은 끝난 줄을 지운다).
    pub(super) fn progress_cells(&self, overall: u64, ko: bool) -> (String, String) {
        let percent = match self.size.filter(|s| *s > 0) {
            Some(size) => self.done.min(size) * 100 / size,
            None => overall.min(100),
        };
        let bar = |percent: u64| std::iter::repeat_n(if self.upload { '>' } else { '<' }, (percent / 10) as usize).collect::<String>();
        match &self.state {
            RowState::Waiting => (String::new(), String::new()),
            RowState::Running => (bar(percent), format!("{percent}%")),
            RowState::Done => (bar(100), "100%".into()),
            RowState::Cancelled | RowState::Failed(_) => (self.progress_text(ko), String::new()),
        }
    }
    /// 진행 칸 글: 크기를 알면 %, 아니면 받은 바이트.
    pub(super) fn progress_text(&self, ko: bool) -> String {
        match &self.state {
            RowState::Waiting => String::new(),
            RowState::Running => match self.size.filter(|s| *s > 0) {
                Some(size) => format!("{}%", self.done.min(size) * 100 / size),
                None => format!("{} bytes", self.done),
            },
            RowState::Done => "100%".into(),
            RowState::Cancelled => (if ko { "취소" } else { "Cancelled" }).into(),
            RowState::Failed(error) => error.clone(),
        }
    }
}

fn set_row(rows: &Mutex<Vec<FtpResultRow>>, index: usize, update: impl FnOnce(&mut FtpResultRow)) {
    if let Ok(mut rows) = rows.lock() {
        if let Some(row) = rows.get_mut(index) {
            update(row);
        }
    }
}

/// 패널 FTP 모드에서도 쓸 수 있는 명령(목록 이동·고르기·보기·FTP·도움말). 나머지는 로컬 파일을 다루므로 막는다.
pub(super) fn ftp_allows(id: &str) -> bool {
    const EXACT: &[&str] = &[
        "refresh", "goParent", "goRoot", "open", "selectAll", "invertSelection", "select", "copyToOther", "moveToOther",
        "remote", "sites", "remoteCommand", "exit", "options", "language", "theme", "about", "help", "homepage", "checkUpdates",
        "filter", "newTab", "closeTab", "historyBack", "historyForward", "back", "forward", "favoritesMenu", "switchPane",
        // 원본 miNewFolderClick(0x432378)·miRenameClick(0x4351cc)·miDeleteClick(0x431c04 → FUN_0042c734)의 FTP 모드(2) 갈래.
        "newFolder", "rename", "deleteRecycle", "deletePermanent", "deleteToolbar", "attributes",
        // 원본 miCommandLineClick(0x438598): FTP 모드면 입력을 FTP 제어 명령으로 보낸다.
        "command",
        // 원본 miMapNetworkDriveClick(0x435144)·miUnmapNetworkDriveClick(0x4350bc): 모드와 상관없이 연다(새로 읽기는 로컬일 때만).
        "mapDrive", "unmapDrive", "browseNetwork",
    ];
    EXACT.contains(&id) || ["ftp", "toggle", "view", "sort", "column:", "folderSort", "group", "favoriteOpen:", "workOpen:", "newTabFrom:", "closeTab:"].iter().any(|prefix| id.starts_with(prefix))
}

fn to_entry(remote: &PaneRemote, item: &RemoteEntry) -> Entry {
    Entry {
        name: item.name.clone(),
        path: remote.item_path(&item.name),
        is_dir: item.is_dir,
        size: if item.is_dir { 0 } else { item.size },
        modified: item.modified.filter(|s| *s >= 0).map(|s| std::time::UNIX_EPOCH + Duration::from_secs(s as u64)),
        hidden: false,
        attributes: if item.is_dir { 0x10 } else { 0 },
        drive: false,
    }
}

/// 원격 목록 읽기(refresh의 FTP 쪽). "."·".."은 뺀다.
pub(super) fn list_remote(remote: &PaneRemote) -> Result<Vec<Entry>, String> {
    let items = remote.fs.lock().map_err(|_| "FTP connection is unavailable".to_owned())?.list(&remote.path)?;
    let items: Vec<&RemoteEntry> = items.iter().filter(|e| e.name != "." && e.name != "..").collect();
    if let Ok(mut modes) = remote.modes.lock() {
        modes.clear();
        modes.extend(items.iter().filter_map(|e| Some((join_remote(&remote.path, &e.name), e.mode?))));
    }
    Ok(items.into_iter().map(|e| to_entry(remote, e)).collect())
}

impl App {
    fn ftp_title(&self) -> String {
        self.language_pack.text("RES.FTP", "FTP")
    }

    fn ftp_box(&mut self, key: &str, fallback: &str) {
        let title = self.ftp_title();
        let text = self.language_pack.text(&format!("RES.{key}"), fallback);
        self.message_box(&title, "", &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
    }

    /// 원본 FUN_00440734: 사이트 메뉴 항목(번호, 글).
    pub(super) fn ftp_site_items(&self) -> Vec<(usize, String)> {
        load_sites(&self.settings).iter().enumerate().map(|(i, s)| (i, site_menu_caption(i, &s.name))).collect()
    }

    /// 원본 FUN_0043e720: 사이트 목록의 index번 사이트로 지금 패널을 연결한다.
    pub(super) fn ftp_connect_site(&mut self, index: usize) {
        let Some(site) = load_sites(&self.settings).get(index).cloned() else { return };
        let password = native::unprotect_secret(&site.encrypted_password).unwrap_or_default();
        self.ftp_connect(self.active, site, password);
    }

    /// 원본 miFTPQuickConnectClick → FUN_0043e9f8(창, 패널, 주소): TfFTPQuickConnect. 주소는 cbHost 처음 값(명령 줄의 "ftp://…").
    pub(super) fn ftp_quick_connect(&mut self, host: String) {
        let form = FtpQuickForm {
            host,
            port: "21".into(),
            user: String::new(),
            password: String::new(),
            passive: true,
            add_to_sites: false,
            recent_hosts: self.settings.strings("RecentRemoteHosts"),
            pane: self.active,
        };
        let title = self.language_pack.text("formFTPQuickConnect.Form.Caption", "FTP Quick Connection");
        self.push_original_dialog(title, DialogKind::FtpQuickConnect(Box::new(form)), |app, result, _| {
            let DialogResult::FtpQuickConnect(Some(form)) = result else { return };
            let host = prepare_host(form.host.trim());
            if host.is_empty() {
                return;
            }
            // FUN_0043e9f8: 포트(StrToInt, 처음 값 21), 22면 SFTP.
            let port = parse_port(&form.port);
            let site = SiteProfile {
                name: host.clone(),
                protocol: if port == 22 { "SFTP".into() } else { "FTP".into() },
                host: host.clone(),
                port,
                user: form.user.clone(),
                encrypted_password: native::protect_secret(&form.password).unwrap_or_default(),
                remote_directory: String::new(),
                passive: form.passive,
                ..Default::default()
            };
            if form.add_to_sites {
                let mut sites = load_sites(&app.settings);
                sites.push(site.clone());
                store_sites(&mut app.settings, &sites);
                app.save();
            }
            let mut hosts = app.settings.strings("RecentRemoteHosts");
            hosts.retain(|h| !h.eq_ignore_ascii_case(&form.host));
            hosts.insert(0, form.host.clone());
            hosts.truncate(20);
            app.settings.set_strings("RecentRemoteHosts", hosts);
            app.ftp_connect(form.pane, site, form.password);
        });
    }

    /// 원본 FUN_004408f8: 패널을 FTP 모드로 연결(이미 전송 중인 FTP 패널이면 FUN_00440d24가 막는다).
    pub(super) fn ftp_connect(&mut self, index: usize, site: SiteProfile, password: String) {
        if self.panes[index].remote.as_ref().is_some_and(|r| r.busy.load(Ordering::Relaxed)) {
            return self.ftp_box("FTP_CANNOTDISCONNECT", "Cannot be disconnected while any operation is working.");
        }
        let config = Config {
            name: site.name.clone(),
            protocol: site.protocol.clone(),
            host: prepare_host(&site.host),
            port: site.port as u16,
            user: site.user.clone(),
            password,
            private_key: PathBuf::from(&site.private_key_path).is_file().then(|| PathBuf::from(&site.private_key_path)).unwrap_or_default(),
            key_passphrase: native::unprotect_secret(&site.encrypted_key_passphrase).unwrap_or_default(),
            fingerprint: site.known_host_fingerprint.clone(),
            passive: site.passive,
            encoding_name: site.encoding_name.clone(),
        };
        let connect = self.remote_hooks.pane_connect;
        let initial = site.remote_directory.clone();
        let host = config.host.clone();
        let tx = self.async_tx.clone();
        // 원본 FUN_004408f8: 사이트 종류가 "sftp"가 아니면 기록 칸을 만든다(FUN_0048d7ac(패널, 1), 이미 있으면 그대로).
        let existing = self.panes[index].remote.as_ref().and_then(|r| r.log.clone());
        let log = existing.or_else(|| (!site.protocol.eq_ignore_ascii_case("sftp")).then(Arc::default));
        self.status = format!("{} {host}…", self.ftp_title());
        thread::spawn(move || {
            let result = ndr::remote::with_protocol_log(log.as_ref(), || connect(&config)).and_then(|fs| {
                let mut fs: Box<dyn PaneFs> = match &log {
                    Some(log) => Box::new(LoggedFs { inner: fs, log: log.clone() }),
                    None => fs,
                };
                let home = fs.pwd()?;
                let path = if initial.trim().is_empty() { home } else { fs.change_directory(&home, initial.trim())?.0 };
                Ok((fs, path))
            });
            let _ = tx.send(match result {
                Ok((fs, path)) => AsyncResult::FtpConnected { pane: index, remote: PaneRemote { fs: Arc::new(Mutex::new(fs)), host, path, busy: Arc::new(AtomicBool::new(false)), modes: Arc::default(), log } },
                Err(error) => AsyncResult::FtpFailed { error },
            });
        });
    }

    pub(super) fn ftp_connected(&mut self, index: usize, remote: PaneRemote) {
        let same_log = self.panes[index].remote.as_ref().and_then(|r| r.log.as_ref()).zip(remote.log.as_ref()).is_some_and(|(a, b)| Arc::ptr_eq(a, b));
        if let Some(old) = self.panes[index].remote.take() {
            close_later(old);
        }
        self.status = remote.address();
        let pane = &mut self.panes[index];
        if !same_log {
            // 원본 FUN_0048d7ac: 새 기록 칸은 높이 60.
            pane.ftp_log_height = FTP_LOG_HEIGHT;
        }
        pane.archive = None;
        pane.address = remote.address();
        pane.remote = Some(remote);
        pane.cursor = 0;
        self.refresh(index);
    }

    pub(super) fn ftp_failed(&mut self, error: String) {
        self.status.clear();
        let title = self.ftp_title();
        self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
    }

    /// 원본 miFTPDisconnectClick(0x43e8d4): 전송 중이면 FTP_CANNOTDISCONNECT, 아니면 끊고 로컬 폴더로.
    pub(super) fn ftp_disconnect(&mut self) {
        let i = self.active;
        let Some(remote) = self.panes[i].remote.clone() else { return };
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTP_CANNOTDISCONNECT", "Cannot be disconnected while any operation is working.");
        }
        self.panes[i].remote = None;
        close_later(remote);
        let pane = &mut self.panes[i];
        pane.address = pane.path.to_string_lossy().into_owned();
        self.refresh(i);
    }

    /// FTP 패널에서 폴더 열기(목록의 폴더 줄, 원격 경로를 바꾸고 다시 읽는다).
    pub(super) fn ftp_open_dir(&mut self, index: usize, name: &str) {
        let Some(remote) = self.panes[index].remote.as_mut() else { return };
        remote.path = join_remote(&remote.path, name);
        let address = remote.address();
        self.panes[index].address = address;
        self.refresh(index);
    }

    /// FTP 패널의 상위 폴더(루트면 그대로).
    pub(super) fn ftp_parent(&mut self, index: usize) {
        let Some(remote) = self.panes[index].remote.as_mut() else { return };
        let parent = parent_path(&remote.path);
        if parent == remote.path.trim_end_matches('/') || remote.path == "/" {
            return;
        }
        let child = remote.path.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_owned();
        remote.path = parent;
        let address = remote.address();
        self.panes[index].address = address;
        // 원본 FUN_00486694(올라감): 떠나온 폴더에 커서.
        self.pending_focus[index] = (!child.is_empty()).then_some(child);
        self.refresh(index);
    }

    /// FTP 패널의 루트.
    pub(super) fn ftp_root(&mut self, index: usize) {
        let Some(remote) = self.panes[index].remote.as_mut() else { return };
        if remote.path == "/" {
            return;
        }
        remote.path = "/".into();
        let address = remote.address();
        self.panes[index].address = address;
        self.refresh(index);
    }

    /// 원본 miFTPGetClick(0x4412f8): 지금 패널이 FTP면 FTP_GET 대상 선택 뒤 내려받기.
    pub(super) fn ftp_download_command(&mut self) {
        let i = self.active;
        if self.panes[i].remote.is_none() {
            return;
        }
        let other = &self.panes[1 - i];
        let initial = if other.remote.is_none() && other.archive.is_none() { other.path.clone() } else { PathBuf::new() };
        self.select_destination(super::destination_dialog::DestinationPurpose::FtpGet, initial, None, move |app, destination| {
            app.ftp_download(i, destination);
        });
    }

    /// 원본 FUN_0043f800: FTP 패널(index)의 고른 항목을 로컬 폴더로 내려받는다(FTPENGINE_BUSY, 이미 있으면 같은 이름 확인 창).
    pub(super) fn ftp_download(&mut self, index: usize, destination: PathBuf) {
        let Some(remote) = self.panes[index].remote.clone() else { return };
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTPENGINE_BUSY", "Other files are still being transferred.");
        }
        let items: Vec<(String, String, u64)> = self.target_entries(index).into_iter().filter_map(|e| {
            let source = remote.remote_of(&e.path)?;
            let name = source.trim_end_matches('/').rsplit('/').next()?.to_owned();
            Some((source, name, if e.is_dir { 0 } else { e.size }))
        }).collect();
        if items.is_empty() {
            return;
        }
        let rows = self.ftp_results.clone();
        if let Ok(mut list) = rows.lock() {
            list.clear();
            list.extend(items.iter().map(|(source, name, size)| FtpResultRow { upload: false, local: destination.join(name).to_string_lossy().into_owned(), remote: source.clone(), size: (*size > 0).then_some(*size), done: 0, state: RowState::Waiting }));
        }
        // 원본 FUN_0043f800 끝: 아래 칸(pBottom)을 보인다.
        self.show_results = true;
        self.ftp_started = Some(std::time::Instant::now());
        self.ftp_pause.store(false, Ordering::Relaxed);
        let pause = self.ftp_pause.clone();
        let title = self.language_pack.text("RES.FTPGET", "FTP Download");
        let busy = remote.busy.clone();
        busy.store(true, Ordering::Relaxed);
        let started = self.start_task(title, move |events, cancel| {
            let outcome = (|| {
                let mut fs = remote.fs.lock().map_err(|_| "FTP connection is unavailable".to_owned())?;
                for (row, (source, name, _)) in items.iter().enumerate() {
                    super::commands::wait_while_paused(&pause, &cancel);
                    if cancel.load(Ordering::Relaxed) {
                        set_row(&rows, row, |r| r.state = RowState::Cancelled);
                        continue;
                    }
                    set_row(&rows, row, |r| r.state = RowState::Running);
                    let local = destination.join(name);
                    let mut resolve = |src: &str, dst: &str| super::commands::ask_conflict(&events, &cancel, Path::new(src), Path::new(dst));
                    let mut progress = |p: TransferProgress| {
                        set_row(&rows, row, |r| r.done = p.bytes);
                        let _ = events.send(Event::Progress { path: PathBuf::from(p.path), bytes: p.bytes, files: p.files });
                        super::commands::wait_while_paused(&pause, &cancel);
                    };
                    match fs.download(source, &local, &cancel, &mut resolve, &mut progress) {
                        Ok(()) => set_row(&rows, row, |r| r.state = RowState::Done),
                        Err(error) => {
                            let state = if cancel.load(Ordering::Relaxed) { RowState::Cancelled } else { RowState::Failed(error.clone()) };
                            set_row(&rows, row, |r| r.state = state);
                            return Err(error);
                        }
                    }
                }
                Ok(())
            })();
            remote.busy.store(false, Ordering::Relaxed);
            outcome
        });
        if !started {
            busy.store(false, Ordering::Relaxed);
        }
        self.ftp_progress_in_results_only(started);
    }

    /// 원본 FTP 전송(FTP 엔진 스레드)은 진행 창 없이 아래 결과 칸(pBottom)에만 진행을 보이고 메인 창은 그대로 쓸 수 있다.
    /// NDR 파일 작업 진행 창(job.progress)을 감춘다. 충돌·오류 상자는 그대로 뜬다.
    pub(super) fn ftp_progress_in_results_only(&mut self, started: bool) {
        if let Some(job) = self.job.as_mut().filter(|_| started) {
            job.progress = None;
        }
    }

    /// 원본 miFTPPutClick(0x441168): 한쪽 FTP·한쪽 로컬이어야 올린다(아니면 FTP_PUTCRITERIA).
    pub(super) fn ftp_upload_command(&mut self) {
        let (left, right) = (self.panes[0].remote.is_some(), self.panes[1].remote.is_some());
        let local = |app: &App, i: usize| app.panes[i].remote.is_none() && app.panes[i].archive.is_none();
        match (left, right) {
            (true, false) if local(self, 1) => self.ftp_upload(1, 0),
            (false, true) if local(self, 0) => self.ftp_upload(0, 1),
            _ => self.ftp_box("FTP_PUTCRITERIA", "One pane should be in My Computer mode and the other should be in FTP mode."),
        }
    }

    /// 원본 FUN_0043feb0: 로컬 패널(source)의 고른 항목을 FTP 패널(target)의 지금 폴더로 올린다.
    pub(super) fn ftp_upload(&mut self, source: usize, target: usize) {
        let Some(remote) = self.panes[target].remote.clone() else { return };
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTPENGINE_BUSY", "Other files are still being transferred.");
        }
        let items = self.panes[source].targets();
        if items.is_empty() {
            return;
        }
        let rows = self.ftp_results.clone();
        if let Ok(mut list) = rows.lock() {
            list.clear();
            list.extend(items.iter().map(|local| {
                let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let size = std::fs::metadata(local).ok().filter(|m| m.is_file()).map(|m| m.len());
                FtpResultRow { upload: true, local: local.to_string_lossy().into_owned(), remote: join_remote(&remote.path, &name), size, done: 0, state: RowState::Waiting }
            }));
        }
        self.show_results = true;
        self.ftp_started = Some(std::time::Instant::now());
        self.ftp_pause.store(false, Ordering::Relaxed);
        let pause = self.ftp_pause.clone();
        let title = self.tr("FTP 올리기", "FTP Upload").into_owned();
        let busy = remote.busy.clone();
        busy.store(true, Ordering::Relaxed);
        let started = self.start_task(title, move |events, cancel| {
            let outcome = (|| {
                let mut fs = remote.fs.lock().map_err(|_| "FTP connection is unavailable".to_owned())?;
                for (row, local) in items.iter().enumerate() {
                    super::commands::wait_while_paused(&pause, &cancel);
                    if cancel.load(Ordering::Relaxed) {
                        set_row(&rows, row, |r| r.state = RowState::Cancelled);
                        continue;
                    }
                    set_row(&rows, row, |r| r.state = RowState::Running);
                    let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("Upload source has no name")?;
                    let destination = join_remote(&remote.path, &name);
                    let mut resolve = |src: &str, dst: &str| super::commands::ask_conflict(&events, &cancel, Path::new(src), Path::new(dst));
                    let mut progress = |p: TransferProgress| {
                        set_row(&rows, row, |r| r.done = p.bytes);
                        let _ = events.send(Event::Progress { path: PathBuf::from(p.path), bytes: p.bytes, files: p.files });
                        super::commands::wait_while_paused(&pause, &cancel);
                    };
                    match fs.upload(local, &destination, &cancel, &mut resolve, &mut progress) {
                        Ok(()) => set_row(&rows, row, |r| r.state = RowState::Done),
                        Err(error) => {
                            let state = if cancel.load(Ordering::Relaxed) { RowState::Cancelled } else { RowState::Failed(error.clone()) };
                            set_row(&rows, row, |r| r.state = state);
                            return Err(error);
                        }
                    }
                }
                Ok(())
            })();
            remote.busy.store(false, Ordering::Relaxed);
            outcome
        });
        if !started {
            busy.store(false, Ordering::Relaxed);
        }
        self.ftp_progress_in_results_only(started);
    }

    /// 원본 miViewFindResultClick(0x434b20): 아래 칸(pBottom)과 나눔 막대를 보이고/숨긴다.
    pub(super) fn toggle_results(&mut self) {
        self.show_results = !self.show_results;
    }

    /// FTP 전송 중인가(원본 [+0xd6c]+0x14).
    fn ftp_transferring(&self) -> bool {
        self.panes.iter().any(|p| p.remote.as_ref().is_some_and(|r| r.busy.load(Ordering::Relaxed)))
    }

    /// 원본 btnFTPPauseClick(0x440f78): 전송 중이면 잠시 중단 ↔ 계속.
    pub(super) fn ftp_pause_toggle(&mut self) {
        if self.ftp_transferring() {
            let paused = self.ftp_pause.load(Ordering::Relaxed);
            self.ftp_pause.store(!paused, Ordering::Relaxed);
        }
    }

    /// 원본 btnFTPAbortClick(0x440f3c): 전송 중이면 중단 표시(잠시 중단이면 풀어서 끝나게 한다).
    pub(super) fn ftp_abort(&mut self) {
        if !self.ftp_transferring() {
            return;
        }
        if let Some(job) = self.job.as_ref().filter(|j| !j.finished) {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.ftp_pause.store(false, Ordering::Relaxed);
    }

    /// 원본 FUN_0044033c(FTP 대기열 끝, 엔진 이벤트): 끝난 항목을 대기열에서 빼고, 남은 것이 없으면 아래 칸(pBottom)을 숨긴다.
    /// (끝 소리 FUN_0043d9fc와 FTP 패널 새로 읽기 FUN_00487ce8는 report_job이 한다.)
    pub(super) fn ftp_queue_done(&mut self) {
        let Ok(mut rows) = self.ftp_results.lock() else { return };
        if rows.is_empty() {
            return;
        }
        rows.retain(|r| r.state != RowState::Done);
        if rows.is_empty() {
            self.show_results = false;
        }
    }

    /// 원본 FUN_0048d7ac 기록 칸이 차지할 높이(나눔 막대 3 + 기록 칸). 기록이 없으면 0.
    pub(super) fn ftp_log_reserve(&self, index: usize, spacing: f32) -> f32 {
        match self.panes[index].remote.as_ref().and_then(|r| r.log.as_ref()) {
            Some(_) => FTP_SPLITTER_HEIGHT + self.panes[index].ftp_log_height + spacing * 2.0,
            None => 0.0,
        }
    }

    /// 원본 FUN_0048d7ac(패널, 1): FTP 모드 패널 아래의 TListBox(alBottom, 높이 60, 테두리 없음,
    /// 바탕 [FTP] ftplogback·글 ftplogtext)와 그 위 TSplitter(alBottom, MinSize 30, 패널 색).
    /// 줄은 원본 FUN_004404c0처럼 더할 때마다 맨 끝 줄을 고르고(ItemIndex), 고른 줄은 Windows 강조 색이다.
    pub(super) fn ftp_log_ui(&mut self, ui: &mut egui::Ui, index: usize, max_height: f32) {
        let Some(log) = self.panes[index].remote.as_ref().and_then(|r| r.log.clone()) else { return };
        let skin = self.current_skin();
        let (back, text) = match &skin {
            Some(skin) => {
                let color = |key: &str, fallback: Color32| skin.color("FTP", key).map(|c| Color32::from_rgb(c[0], c[1], c[2])).unwrap_or(fallback);
                (color("ftplogback", Color32::BLACK), color("ftplogtext", Color32::from_gray(0xAA)))
            }
            None => (ui.visuals().extreme_bg_color, ui.visuals().text_color()),
        };
        let splitter_color = match &skin {
            Some(skin) => skin.color("List", "lback").map(|c| Color32::from_rgb(c[0], c[1], c[2])).unwrap_or(Color32::BLACK),
            None => ui.visuals().panel_fill,
        };
        let (bar, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), FTP_SPLITTER_HEIGHT), egui::Sense::drag());
        ui.painter().rect_filled(bar, 0.0, splitter_color);
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if response.dragged() {
            // 원본 TSplitter MinSize 30: 기록 칸과 목록 모두 30보다 작아지지 않는다.
            let limit = (max_height - FTP_LOG_MIN).max(FTP_LOG_MIN);
            let height = &mut self.panes[index].ftp_log_height;
            *height = (*height - response.drag_delta().y).clamp(FTP_LOG_MIN, limit);
        }
        let height = self.panes[index].ftp_log_height;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, back);
        let (lines, selected) = log.lock().map(|l| (l.lines.clone(), l.selected)).unwrap_or_default();
        let row = ui.text_style_height(&egui::TextStyle::Body) + 1.0;
        let highlight = {
            let (r, g, b) = native::system_color(13);
            Color32::from_rgb(r, g, b)
        };
        let highlight_text = {
            let (r, g, b) = native::system_color(14);
            Color32::from_rgb(r, g, b)
        };
        let mut clicked = None;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        child.spacing_mut().item_spacing.y = 0.0;
        let count_id = egui::Id::new(("ndr-ftp-log-count", index));
        let mut area = egui::ScrollArea::vertical().id_salt(("ndr-ftp-log", index)).auto_shrink([false, false]);
        if ui.ctx().data(|d| d.get_temp::<usize>(count_id)) != Some(lines.len()) {
            area = area.vertical_scroll_offset(lines.len() as f32 * row);
        }
        area.show_rows(&mut child, row, lines.len(), |ui, range| {
            for i in range {
                let (line_rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), row), egui::Sense::click());
                let chosen = selected == Some(i);
                if chosen {
                    ui.painter().rect_filled(line_rect, 0.0, highlight);
                }
                ui.painter().with_clip_rect(line_rect).text(line_rect.left_center() + egui::vec2(2.0, 0.0), egui::Align2::LEFT_CENTER, &lines[i], egui::TextStyle::Body.resolve(ui.style()), if chosen { highlight_text } else { text });
                if response.clicked() {
                    clicked = Some(i);
                }
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(count_id, lines.len()));
        if let Some(i) = clicked {
            if let Ok(mut log) = log.lock() {
                log.selected = Some(i);
            }
        }
        if !lines.is_empty() && self.panes[index].remote.as_ref().is_some_and(|r| r.busy.load(Ordering::Relaxed)) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    /// 원본 pBottom: lvFTPProgress(" ;Local;Progress; ;Remote")와 btnFTPPause·btnFTPAbort.
    pub(super) fn ftp_results_ui(&mut self, ui: &mut egui::Ui) {
        let ko = self.language == "ko";
        let headers: Vec<String> = self.language_pack.text("formMain.ListViewCols.lvFTPProgress", " ;Local;Progress; ;Remote").split(';').map(str::to_owned).collect();
        let header = |i: usize| headers.get(i).cloned().unwrap_or_default();
        let paused = self.ftp_pause.load(Ordering::Relaxed);
        let mut action = None;
        let rows = self.ftp_results.lock().map(|r| r.clone()).unwrap_or_default();
        let transferring = self.ftp_transferring();
        // 원본 NXPaintPanel1(높이 21): btnFTPPause(1,0 75×21) btnFTPAbort(78,0 75×21) pbFTPTransfer(159,2 150×16)
        // lFTPTransferred(315,4), 오른쪽 끝 btnFTPShowDetail(">>", 오른쪽 − 46, 19×19) btnFTPCloseProgress("X", 오른쪽 − 21).
        let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 21.0), egui::Sense::hover());
        let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(bar.min + egui::vec2(x, y), egui::vec2(w, h));
        // 원본 btnFTPPauseClick(0x440f78): 멈추면 RES.RESUME("다시시작"), 다시 시작하면 RES.PAUSE("잠시중지").
        // FUN_0043f04c 이벤트 4(새 전송 목록)도 RES.PAUSE로 되돌린다. 첫 전송 전에는 창 설계 글(btnFTPPause).
        let pause = if paused {
            self.language_pack.text("RES.RESUME", "Resume")
        } else if self.ftp_started.is_some() {
            self.language_pack.text("RES.PAUSE", "Pause")
        } else {
            self.language_pack.text("formMain.Caption.btnFTPPause", "Pause")
        };
        if super::original_dialogs::put_enabled(ui, at(1.0, 0.0, 75.0, 21.0), transferring, egui::Button::new(pause)).clicked() {
            action = Some(0);
        }
        if super::original_dialogs::put_enabled(ui, at(78.0, 0.0, 75.0, 21.0), transferring, egui::Button::new(self.language_pack.text("formMain.Caption.btnFTPAbort", "Abort"))).clicked() {
            action = Some(1);
        }
        // 원본 FUN_0043f04c 이벤트 1: pbFTPTransfer = 백분율, lFTPTransferred = "%d %% (받음 / 전체) - 속도". 전송이 없으면 0과 빈 글.
        let (done, total, percent) = ftp_totals(&rows);
        let running = transferring && !rows.is_empty();
        ui.put(at(159.0, 2.0, 150.0, 16.0), egui::ProgressBar::new(if running { percent as f32 / 100.0 } else { 0.0 }).desired_height(16.0).corner_radius(0.0));
        if running {
            let seconds = self.ftp_started.map_or(0.0, |t| t.elapsed().as_secs_f64());
            let speed = if seconds > 0.0 { done as f64 / 1024.0 / seconds } else { 0.0 };
            let text = ftp_transferred_text(percent, &native::format_byte_size(done), &native::format_byte_size(total), speed);
            super::original_dialogs::put_left(ui, at(315.0, 3.0, (bar.width() - 315.0 - 52.0).max(0.0), 16.0), egui::Label::new(text).truncate());
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        let right = bar.width();
        if ui.put(at(right - 46.0, 0.0, 19.0, 19.0), egui::Button::new(">>").frame(false)).clicked() {
            action = Some(2);
        }
        if ui.put(at(right - 21.0, 0.0, 19.0, 19.0), egui::Button::new("X").frame(false)).clicked() {
            action = Some(3);
        }
        if self.results_collapsed {
            match action {
                Some(0) => self.ftp_pause_toggle(),
                Some(1) => self.ftp_abort(),
                Some(2) => self.results_collapsed = false,
                Some(3) => self.show_results = false,
                _ => {}
            }
            return;
        }
        // 원본 lvFTPProgress는 색을 정하지 않는다(DFM에 Color 없음, 스킨 적용 0x43daf0도 건드리지 않음, [FTP] ftpprog* 열쇠는
        // 원본 실행 파일이 읽지 않는다). 그래서 스킨 테마에서도 Windows 창 색(clWindow·clWindowText)이다.
        let colors = self.current_skin().map(|_| {
            let rgb = |(r, g, b): (u8, u8, u8)| Color32::from_rgb(r, g, b);
            (rgb(native::system_color(5)), rgb(native::system_color(8)))
        });
        ui.scope(|ui| {
        if let Some((back, text)) = colors {
            ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, back);
            ui.visuals_mut().override_text_color = Some(text);
            ui.visuals_mut().faint_bg_color = back;
        }
        egui::ScrollArea::vertical().id_salt("ftp-results").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("ftp-results-grid").striped(true).num_columns(5).spacing([12.0, 4.0]).show(ui, |ui| {
                for i in 0..5 {
                    ui.label(RichText::new(header(i)).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())));
                }
                ui.end_row();
                // 원본 FUN_0043f04c 이벤트 2(항목 끝): 그 줄을 lvFTPProgress에서 지운다.
                // 원본 이벤트 4(목록 만들기): 첫 칸 글은 비어 있고(lvFTPProgress에 그림 목록 없음), Local·Progress·(백분율)·Remote.
                for row in rows.iter().filter(|r| r.state != RowState::Done) {
                    let (bar, percent_text) = row.progress_cells(percent, ko);
                    ui.label("");
                    ui.label(&row.local);
                    ui.label(RichText::new(bar).monospace());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(percent_text));
                    ui.label(&row.remote);
                    ui.end_row();
                }
            });
        });
        });
        // 원본 btnFTPShowDetailClick(0x441114): 높이 > 30이면 22로(단추 줄만), 아니면 100. btnFTPCloseProgressClick(0x4410f0): pBottom·splBottom 숨김.
        match action {
            Some(0) => self.ftp_pause_toggle(),
            Some(1) => self.ftp_abort(),
            Some(2) => self.results_collapsed = true,
            Some(3) => self.show_results = false,
            _ => {}
        }
    }

    /// 원본 miNewFolderClick(0x432378)의 FTP 갈래: TfGetText(TITLE_NEWFOLDER, MSG_NEWFOLDER, 오늘 날짜 이름) →
    /// FUN_0045d0b4(이름) → 새로 읽고 그 이름에 커서(FUN_00487ce8(목록, 1, 1, 이름)).
    pub(super) fn ftp_new_folder(&mut self, index: usize) {
        let Some(remote) = self.panes[index].remote.clone() else { return };
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTPENGINE_BUSY", "Other files are still being transferred.");
        }
        let title = self.language_pack.text("RES.TITLE_NEWFOLDER", "New Folder");
        let message = self.language_pack.text("RES.MSG_NEWFOLDER", "Type a folder name to be created.");
        self.prompt(&title, &message, &ndr::names::new_folder_default_name(), None, false, move |app, value, _| {
            let Some(name) = value.filter(|v| !v.trim().is_empty()) else { return };
            let path = join_remote(&remote.path, &name);
            app.ftp_edit(index, remote, Some(name), move |fs| fs.mkdir(&path));
        });
    }

    /// 원본 miRenameClick(0x4351cc)의 FTP 갈래: 고른 항목마다 TfRename(이름 칸 = 확장자를 뺀 이름, 그것이 비면 이름 전체;
    /// 확장자가 한 글자보다 길면 확장자 칸). 취소하면 멈춘다. 확인한 것은 FUN_0045d214(옛 이름, 새 이름)로 바꾸고 새로 읽는다.
    pub(super) fn ftp_rename(&mut self, index: usize) {
        let Some(remote) = self.panes[index].remote.clone() else { return };
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTPENGINE_BUSY", "Other files are still being transferred.");
        }
        let items: Vec<(String, String)> = self.target_entries(index).into_iter().filter_map(|e| Some((remote.remote_of(&e.path)?, e.name))).collect();
        if items.is_empty() {
            return;
        }
        let focused = self.panes[index].entries.get(self.panes[index].cursor).map(|e| e.name.clone());
        self.ftp_rename_step(index, remote, items, 0, Vec::new(), focused);
    }

    fn ftp_rename_step(&mut self, index: usize, remote: PaneRemote, items: Vec<(String, String)>, at: usize, mut renames: Vec<(String, String, String)>, focused: Option<String>) {
        let Some((source, name)) = items.get(at).cloned() else {
            return self.ftp_rename_run(index, remote, renames, focused);
        };
        let (stem, extension) = ndr::names::split_for_rename(&name, false);
        let (stem, extension) = if stem.is_empty() { (name.clone(), String::new()) } else { (stem, extension) };
        let count = items.len();
        let original = name.clone();
        self.rename_dialog(&name, &stem, &extension, at + 1, count, move |app, value, _| {
            let Some((new_name, new_extension)) = value else { return app.ftp_rename_run(index, remote, renames, focused) };
            let target = if new_extension.is_empty() { new_name } else { format!("{new_name}.{new_extension}") };
            if target != original {
                renames.push((source, join_remote(&remote.path, &target), target));
            }
            app.ftp_rename_step(index, remote, items, at + 1, renames, focused);
        });
    }

    fn ftp_rename_run(&mut self, index: usize, remote: PaneRemote, renames: Vec<(String, String, String)>, focused: Option<String>) {
        if renames.is_empty() {
            return;
        }
        // 커서 항목의 이름을 바꿨으면 새 이름에 커서를 둔다.
        let focus = focused.as_ref().and_then(|f| renames.iter().find(|(source, _, _)| source.trim_end_matches('/').rsplit('/').next() == Some(f.as_str())).map(|(_, _, target)| target.clone())).or(focused);
        self.ftp_edit(index, remote, focus, move |fs| {
            for (source, target, _) in &renames {
                fs.rename(source, target)?;
            }
            Ok(())
        });
    }

    /// 원본 miDeleteClick(0x431c04) → FUN_0042c734의 FTP 갈래: 휴지통 없이 늘 삭제 확인(FUN_0040c5c4)을 묻고,
    /// 폴더(+0x3c == 1)와 파일 목록을 FUN_0045de18로 지운 뒤 새로 읽는다.
    pub(super) fn ftp_delete(&mut self, index: usize) {
        let Some(remote) = self.panes[index].remote.clone() else { return };
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTPENGINE_BUSY", "Other files are still being transferred.");
        }
        let items: Vec<(String, bool, String)> = self.target_entries(index).into_iter().filter_map(|e| Some((remote.remote_of(&e.path)?, e.is_dir, e.name))).collect();
        let Some(first) = items.first().map(|(_, _, name)| name.clone()) else { return };
        self.delete_warning(DeleteWarningMode::Delete, items.len(), &first, move |app, choice, _| {
            if choice == DeleteWarningChoice::No {
                return;
            }
            let title = app.language_pack.text("formDelWarning.Form.Caption", "Delete");
            let busy = remote.busy.clone();
            busy.store(true, Ordering::Relaxed);
            let started = app.start_task(title, move |events, cancel| {
                let outcome = (|| {
                    let mut fs = remote.fs.lock().map_err(|_| "FTP connection is unavailable".to_owned())?;
                    for (path, is_dir, _) in &items {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let _ = events.send(Event::Progress { path: PathBuf::from(path), bytes: 0, files: 1 });
                        fs.delete(path, *is_dir, &cancel)?;
                    }
                    Ok(())
                })();
                remote.busy.store(false, Ordering::Relaxed);
                outcome
            });
            if !started {
                busy.store(false, Ordering::Relaxed);
            }
        });
    }

    /// 원본 miChangeAttributesClick(0x43657c)의 FTP 갈래 → FUN_00436b98: 고른 항목마다 TfFTPAttributes("이름 (n / 전체)", 목록의 권한).
    /// 확인 = 그 항목, 모두 적용 = 남은 항목 모두 같은 권한, 건너뛰기 = 다음, 취소 = 멈춤. 고른 권한은 FUN_0045e274(경로, 권한)로 바꾸고 새로 읽는다.
    /// 서버가 권한을 알려주지 않은 항목은 644에서 시작한다.
    pub(super) fn ftp_change_attributes(&mut self, index: usize) {
        let Some(remote) = self.panes[index].remote.clone() else { return };
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTPENGINE_BUSY", "Other files are still being transferred.");
        }
        let items: Vec<(String, String, u32)> = self
            .target_entries(index)
            .into_iter()
            .filter_map(|e| {
                let path = remote.remote_of(&e.path)?;
                let mode = remote.modes.lock().ok().and_then(|m| m.get(&path).copied()).unwrap_or(0o644);
                Some((path, e.name, mode))
            })
            .collect();
        if items.is_empty() {
            return;
        }
        self.ftp_attribute_step(index, remote, items, 0, Vec::new(), None);
    }

    fn ftp_attribute_step(&mut self, index: usize, remote: PaneRemote, items: Vec<(String, String, u32)>, at: usize, mut plan: Vec<(String, u32)>, all: Option<u32>) {
        let Some((path, name, mode)) = items.get(at).cloned() else {
            return self.ftp_attribute_run(index, remote, plan);
        };
        if let Some(mode) = all {
            plan.push((path, mode));
            return self.ftp_attribute_step(index, remote, items, at + 1, plan, all);
        }
        let label = format!("{name} ({} / {})", at + 1, items.len());
        let form = PermissionForm::new(mode, &label, &path);
        let title = self.language_pack.text("formFTPAttributes.Form.Caption", "FTP Change Attributes");
        self.push_original_dialog(title, DialogKind::RemotePermissions(Box::new(form)), move |app, result, _| {
            let DialogResult::RemotePermissions(choice) = result else { return };
            match choice {
                None => app.ftp_attribute_run(index, remote, plan),
                Some((PermissionAction::SkipCurrent, _)) => app.ftp_attribute_step(index, remote, items, at + 1, plan, None),
                Some((action, mode)) => {
                    plan.push((path, mode));
                    let all = (action == PermissionAction::ApplyAllRemaining).then_some(mode);
                    app.ftp_attribute_step(index, remote, items, at + 1, plan, all);
                }
            }
        });
    }

    fn ftp_attribute_run(&mut self, index: usize, remote: PaneRemote, plan: Vec<(String, u32)>) {
        if plan.is_empty() {
            return;
        }
        self.ftp_edit(index, remote, None, move |fs| {
            for (path, mode) in &plan {
                fs.chmod(path, *mode & 0o777)?;
            }
            Ok(())
        });
    }

    /// 짧은 FTP 동작(새 폴더·이름 바꾸기)을 따로 된 스레드에서 하고, 끝나면 AsyncResult::FtpEdited로 새로 읽는다.
    fn ftp_edit(&mut self, index: usize, remote: PaneRemote, focus: Option<String>, work: impl FnOnce(&mut dyn PaneFs) -> Result<(), String> + Send + 'static) {
        remote.busy.store(true, Ordering::Relaxed);
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let result = match remote.fs.lock() {
                Ok(mut fs) => work(fs.as_mut()),
                Err(_) => Err("FTP connection is unavailable".to_owned()),
            };
            remote.busy.store(false, Ordering::Relaxed);
            let _ = tx.send(AsyncResult::FtpEdited { pane: index, focus, error: result.err() });
        });
    }

    pub(super) fn ftp_edited(&mut self, index: usize, focus: Option<String>, error: Option<String>) {
        if let Some(error) = error {
            let title = self.ftp_title();
            self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
        if self.panes[index].remote.is_some() {
            self.refresh_and_focus(index, focus);
        }
    }

    /// 원본 miCommandLineClick(0x438598)의 FTP 모드(2) 갈래: 입력한 글을 그대로 FTP 제어 명령으로 보낸다(FUN_0082a178, 응답 코드 -1 = 아무것이나).
    /// 원본은 목록을 다시 읽지 않는다. 응답은 원본처럼 따로 창을 띄우지 않고 상태 줄에 보인다.
    pub(super) fn ftp_raw_command(&mut self, index: usize, command: String) {
        let Some(remote) = self.panes[index].remote.clone() else { return };
        if command.trim().is_empty() {
            return;
        }
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTPENGINE_BUSY", "Other files are still being transferred.");
        }
        remote.busy.store(true, Ordering::Relaxed);
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let result = match remote.fs.lock() {
                Ok(mut fs) => fs.command(&command),
                Err(_) => Err("FTP connection is unavailable".to_owned()),
            };
            remote.busy.store(false, Ordering::Relaxed);
            let _ = tx.send(AsyncResult::FtpReply { result });
        });
    }

    pub(super) fn ftp_reply(&mut self, result: Result<String, String>) {
        match result {
            Ok(reply) => self.status = reply.trim().to_owned(),
            Err(error) => {
                let title = self.ftp_title();
                self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
        }
    }

    /// 원본 FUN_0042d518/FUN_0042d910(다른 패널로 복사): 로컬→FTP면 올리기, FTP→로컬이면 내려받기. 처리했으면 true.
    pub(super) fn ftp_route_copy(&mut self, source: usize) -> bool {
        let target = 1 - source;
        match (self.panes[source].remote.is_some(), self.panes[target].remote.is_some()) {
            (false, false) => false,
            (true, false) if self.panes[target].archive.is_none() => {
                let destination = self.panes[target].path.clone();
                self.ftp_download(source, destination);
                true
            }
            (false, true) if self.panes[source].archive.is_none() => {
                self.ftp_upload(source, target);
                true
            }
            _ => true,
        }
    }
}

/// 연결을 따로 된 스레드에서 닫는다(서버가 늦게 답해도 화면을 멈추지 않게).
fn close_later(remote: PaneRemote) {
    thread::spawn(move || {
        if let Ok(mut fs) = remote.fs.lock() {
            fs.close();
        }
    });
}


#[cfg(test)]
mod tests {
    use super::*;

    fn row(size: Option<u64>, done: u64, state: RowState) -> FtpResultRow {
        FtpResultRow { upload: false, local: String::new(), remote: String::new(), size, done, state }
    }

    #[test]
    fn total_progress_follows_the_original_ftp_event() {
        // 원본 FUN_0043f04c 이벤트 1: 받은 바이트 × 100 / 전체 바이트. 끝난 줄은 크기 전체, 기다리는 줄은 0.
        let rows = [row(Some(1000), 1000, RowState::Done), row(Some(3000), 1000, RowState::Running), row(Some(4000), 0, RowState::Waiting)];
        assert_eq!(ftp_totals(&rows), (2000, 8000, 25));
        // 전체를 모르면 100%(원본 iVar6 = 100에서 시작).
        assert_eq!(ftp_totals(&[row(None, 10, RowState::Running)]).2, 100);
        // lFTPTransferred: "%d %% (%s / %s) - %s"(wsprintf %% = %), 속도는 FormatFloat("0.00 KB/s").
        assert_eq!(ftp_transferred_text(25, "1.95 KB", "7.81 KB", 12.345), "25 % (1.95 KB / 7.81 KB) - 12.35 KB/s");
    }

    #[test]
    fn row_cells_follow_the_original_lvftpprogress_text() {
        // 원본 FUN_0043f04c 이벤트 1: Progress 칸 = 백분율/10개의 '<'(내려받기) 또는 '>'(올리기), 넷째 칸 = "%d%".
        assert_eq!(row(Some(1000), 370, RowState::Running).progress_cells(0, true), ("<<<".to_owned(), "37%".to_owned()));
        let upload = FtpResultRow { upload: true, ..row(Some(1000), 1000, RowState::Running) };
        assert_eq!(upload.progress_cells(0, true), (">>>>>>>>>>".to_owned(), "100%".to_owned()));
        // 항목 크기를 모르면 전체 백분율.
        assert_eq!(row(None, 5, RowState::Running).progress_cells(42, true), ("<<<<".to_owned(), "42%".to_owned()));
        // 기다리는 줄은 빈칸, 실패한 줄은 상태 글.
        assert_eq!(row(Some(10), 0, RowState::Waiting).progress_cells(0, true), (String::new(), String::new()));
        assert_eq!(row(Some(10), 0, RowState::Failed("550".into())).progress_cells(0, true), ("550".to_owned(), String::new()));
    }
}
