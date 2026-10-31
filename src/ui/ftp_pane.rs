//! The original in-pane FTP mode (TfMain pmConnect).
//! - Site connection (the miFTPConnect submenu, where FUN_00440734 builds the "&1. name" … lines and FUN_0043e720 → FUN_004408f8) and
//!   quick connect (miFTPQuickConnectClick 0x43ee20 → FUN_0043e9f8: TfFTPQuickConnect, SFTP when port 22, "add to site list").
//!   Connecting switches the current pane to FTP mode (FUN_00481948 == 2).
//! - Disconnect (miFTPDisconnectClick 0x43e8d4 → FUN_00440d24: FTP_CANNOTDISCONNECT if transferring, otherwise disconnect and go to a local folder).
//! - Download (miFTPGetClick 0x4412f8: if the pane is FTP, choose an FTP_GET destination → FUN_0043f800) and
//!   upload (miFTPPutClick 0x441168: one pane must be FTP and the other local, otherwise FTP_PUTCRITERIA → FUN_0043feb0).
//!   Transferring again while a transfer is in progress gives FTPENGINE_BUSY.
//! - Copy to the other pane (FUN_0042d518·FUN_0042d910): local→FTP uploads, FTP→local downloads (no destination prompt).
//! - New folder in an FTP pane (miNewFolderClick 0x432378 → FUN_0045d0b4), rename (miRenameClick 0x4351cc → TfRename, FUN_0045d214),
//!   delete (miDeleteClick 0x431c04 → FUN_0042c734: always a delete confirmation (no recycle bin) → FUN_0045de18).
use super::original_dialogs::{DeleteWarningChoice, DeleteWarningMode, DialogKind, DialogResult};
use super::remote_window::{PermissionAction, PermissionForm, SiteProfile, load_sites, parent_path, parse_port, prepare_host, store_sites};
use super::*;
use ndr::msgbox::{MB_ICONEXCLAMATION, MB_OK};
use ndr::remote::{Client, Config, RemoteEntry, TransferProgress};

/// Original FUN_0048d7ac: log pane initial height 0x3c, TSplitter MinSize 0x1e (+0x1f8), splitter bar height (TSplitter default 3).
pub(super) const FTP_LOG_HEIGHT: f32 = 60.0;
const FTP_LOG_MIN: f32 = 30.0;
const FTP_SPLITTER_HEIGHT: f32 = 3.0;

/// Original FTP queue (FUN_00467f0c): folder items are not asked about; if absent they are created, and if present they are reused (folder vs folder = merge).
/// A file with the same name is asked about via TfFileExists (FUN_00467764: shows the remote size and time, and btnResume when the transfer can be resumed).
/// Fix: previously folders vs folders were also asked, the remote size and time were empty so the resume button was unusable, and renaming appended " (1)" without asking.
pub(super) fn ask_remote_conflict(events: &Sender<Event>, cancel: &AtomicBool, source: &str, destination: &str, info: &ndr::remote::ConflictInfo) -> ConflictChoice {
    if info.remote.is_dir && info.local_dir {
        return ConflictChoice::Overwrite;
    }
    let remote = Path::new(if info.remote_is_source { source } else { destination });
    let modified = info.remote.modified.and_then(|s| u64::try_from(s).ok()).map(|s| std::time::UNIX_EPOCH + Duration::from_secs(s));
    super::file_operation::set_remote_entry(remote, Some((info.remote.is_dir, (!info.remote.is_dir).then_some(info.remote.size), modified)));
    let choice = loop {
        let choice = super::commands::ask_conflict_resumable(events, cancel, Path::new(source), Path::new(destination), info.can_resume);
        if choice != ConflictChoice::Rename {
            break choice;
        }
        // Original FUN_00467658 (0x10 rename): ask for the destination name via an input dialog. On OK, retry with that name; on Cancel, ask the same name again.
        let name = destination.rsplit(['/', '\\']).next().unwrap_or(destination).to_owned();
        let (tx, rx) = mpsc::channel();
        if events.send(Event::RenameTarget { name, reply: tx }).is_err() {
            break ConflictChoice::Cancel;
        }
        let answer = loop {
            if cancel.load(Ordering::Relaxed) {
                break Err(());
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => break Ok(answer),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break Err(()),
            }
        };
        match answer {
            Err(()) => break ConflictChoice::Cancel,
            Ok(Some(new)) if !new.trim().is_empty() && !new.contains(['/', '\\']) => {
                *info.new_name.borrow_mut() = Some(new);
                break ConflictChoice::Rename;
            }
            Ok(_) => continue,
        }
    };
    super::file_operation::set_remote_entry(remote, None);
    choice
}

/// Remote operations used by a pane's FTP mode (replaced with a fake in tests).
pub(super) trait PaneFs: Send {
    fn pwd(&mut self) -> Result<String, String>;
    fn change_directory(&mut self, current: &str, path: &str) -> Result<(String, Option<String>), String>;
    fn list(&mut self, path: &str) -> Result<Vec<RemoteEntry>, String>;
    fn download(&mut self, remote: &str, local: &Path, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String>;
    fn upload(&mut self, local: &Path, remote: &str, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String>;
    /// Original FTP engine FUN_0045d0b4 (new folder), FUN_0045d214 (rename), FUN_0045de18 (delete folder/file list).
    fn mkdir(&mut self, path: &str) -> Result<(), String>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String>;
    fn delete(&mut self, path: &str, is_dir: bool, cancel: &AtomicBool) -> Result<(), String>;
    /// Original FUN_0045e274 (path, permissions): FTP SITE CHMOD.
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), String>;
    /// Original FUN_0082a178 (SendCmd, any response code): a single FTP control command.
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
    fn download(&mut self, remote: &str, local: &Path, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String> {
        self.download_tree_with_conflicts(remote, local, cancel, resolve, progress)
    }
    fn upload(&mut self, local: &Path, remote: &str, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String> {
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

/// The real connection (RemoteHooks.pane_connect default).
pub(super) fn connect_client(config: &Config) -> Result<Box<dyn PaneFs>, String> {
    Client::connect(config).map(|client| Box::new(client) as Box<dyn PaneFs>)
}

/// Original FTP engine +0x74 (log pane): record the control lines this connection sends and receives into the log.
/// The prefix on sent lines (original engine +0x1c) is the current remote folder learned from PWD·CWD.
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
        // The original does CWD (engine +0x1c) into that folder before listing. NDR lists by path directly, so it uses that folder as the prefix.
        self.set_prefix(path);
        self.run(|fs| fs.list(path))
    }
    fn download(&mut self, remote: &str, local: &Path, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String> {
        self.run(|fs| fs.download(remote, local, cancel, resolve, progress))
    }
    fn upload(&mut self, local: &Path, remote: &str, cancel: &AtomicBool, resolve: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, progress: &mut dyn FnMut(TransferProgress)) -> Result<(), String> {
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

/// An FTP-mode pane's connection and its current remote folder.
#[derive(Clone)]
pub(super) struct PaneRemote {
    pub fs: Arc<Mutex<Box<dyn PaneFs>>>,
    pub host: String,
    pub path: String,
    /// Original [+0xd6c]+0x14 (FTP engine transferring).
    pub busy: Arc<AtomicBool>,
    /// Permission bits from the last listing (remote path → mode). Original item +0x74 (the value passed to TfFTPAttributes).
    pub modes: Arc<Mutex<std::collections::HashMap<String, u32>>>,
    /// The lines of the original pane +0x358 FTP log pane (FUN_0048d7ac). Absent on the first connection to an SFTP site.
    /// The adapter's client setup FUN_0045adac attaches an IO handler whose sent (FUN_0045afd0, direction 1) and received
    /// (FUN_0045b0b4, direction 0) callbacks forward each control line to the adapter's OnLog event (+0x48), which fills this pane.
    pub log: Option<Arc<Mutex<ndr::remote::ProtocolLog>>>,
}

impl PaneRemote {
    /// Whether this is an SFTP site (no FTP log pane, like the original).
    pub(super) fn is_sftp(&self) -> bool {
        self.log.is_none()
    }

    /// Path of a list item ("ftp://host/folder/name"). It cannot be opened as a local path (the first segment contains ":").
    pub(super) fn item_path(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("ftp://{}{}", self.host, join_remote(&self.path, name)))
    }
    /// The remote part of item_path.
    pub(super) fn remote_of(&self, path: &Path) -> Option<String> {
        let text = path.to_string_lossy().replace('\\', "/");
        let rest = text.strip_prefix(&format!("ftp://{}", self.host)).or_else(|| text.strip_prefix(&format!("ftp:/{}", self.host)))?;
        Some(if rest.is_empty() { "/".to_owned() } else { rest.to_owned() })
    }
    /// The address bar text ("ftp://host/path").
    pub(super) fn address(&self) -> String {
        format!("ftp://{}{}", self.host, if self.path.starts_with('/') { self.path.clone() } else { format!("/{}", self.path) })
    }
}

/// Join a remote path ("/"-separated).
pub(super) fn join_remote(directory: &str, name: &str) -> String {
    if directory.ends_with('/') { format!("{directory}{name}") } else { format!("{directory}/{name}") }
}

/// Original FUN_00440734: site menu line text (up to 10 as "&1. name", up to 35 as "&A. name", then "36. name").
pub(super) fn site_menu_caption(index: usize, name: &str) -> String {
    if index < 10 {
        format!("&{}. {name}", index + 1)
    } else if index < 35 {
        format!("&{}. {name}", char::from(b'A' + (index - 10) as u8))
    } else {
        format!("{}. {name}", index + 1)
    }
}

/// Original TfFTPQuickConnect (cbHost·ePort·eUser·ePassword·chkPassive·chkAddToSites).
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

/// Original TfFTPQuickConnect (ClientWidth 424 × ClientHeight 164): lURL(13,22) cbHost(94,20 220×21), lPort(320,23) ePort(364,20 49×21 centered),
/// lUser(13,59) eUser(94,57 121×21), lPassword(13,86) ePassword(94,84 121×21 '*'), chkPassive(291,88), chkAddToSites(13,132),
/// btnOk(255,128 75×25 default) btnCancel(338,128 75×25). Some(true) on OK, Some(false) on Cancel.
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

/// A single lvFTPProgress row (columns " ;Local;Progress; ;Remote").
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FtpResultRow {
    pub upload: bool,
    pub local: String,
    pub remote: String,
    pub size: Option<u64>,
    pub done: u64,
    pub state: RowState,
    /// When this row's transfer began (original OnWorkBegin FUN_00468d08 stores Now and resets the speed), for lFTPTransferred's KB/s.
    pub started: Option<std::time::Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RowState {
    Waiting,
    Running,
    Done,
    Cancelled,
    Failed(String),
}

/// Original FUN_0043f04c event 1: overall progress (bytes received, total bytes, percent). 100% when the total is unknown (0).
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

/// Original OnWork FUN_00468f0c: speed = bytes of the item in transfer / 1024 / whole seconds (DecodeTime h·m·s) since its OnWorkBegin
/// (FUN_00468d08); it stays 0 during the first second. A folder row counts the bytes of all its files. Previously all bytes of the queue were
/// divided by the time since the queue began.
pub(super) fn ftp_speed(rows: &[FtpResultRow], now: std::time::Instant) -> f64 {
    rows.iter()
        .find(|r| r.state == RowState::Running)
        .and_then(|r| r.started.map(|t| (r.done, now.saturating_duration_since(t).as_secs())))
        .filter(|&(_, seconds)| seconds > 0)
        .map_or(0.0, |(bytes, seconds)| bytes as f64 / 1024.0 / seconds as f64)
}

/// Original lFTPTransferred: wsprintf("%d %% (%s / %s) - %s", percent, bytes received, total size, FormatFloat("0.00 KB/s", speed)).
/// Sizes come from StrFormatByteSizeW (format_size).
pub(super) fn ftp_transferred_text(percent: u64, done: &str, total: &str, kb_per_second: f64) -> String {
    format!("{percent} % ({done} / {total}) - {kb_per_second:.2} KB/s")
}

impl FtpResultRow {
    /// Original FUN_0043f04c event 1 lvFTPProgress column text: Progress column = percent/10 '>' chars (upload, local → remote) or '<' (download),
    /// fourth column (right-aligned) = "%d%" (wsprintf "%d%%"). When the item size is unknown, use the overall percent (`overall`) like the original.
    /// Waiting rows leave both blank; cancelled/failed rows show the NDR status text (the original removes finished rows).
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
    /// Progress column text: % if the size is known, otherwise bytes received.
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

/// Original queue preparation FUN_00466e66: before the first transfer every folder item is expanded (FUN_00466168 remote, FUN_00465e6c local)
/// and each file size is added to the queue total (+0x20/+0x24, reported as event 4). NDR keeps one row per chosen item, so a folder row
/// gets the sum of the file sizes below it. Symbolic links and reparse points are not followed, like the transfer itself.
pub(super) fn remote_tree_size(list: &mut dyn FnMut(&str) -> Result<Vec<RemoteEntry>, String>, folder: &str, cancel: &AtomicBool) -> Option<u64> {
    let (mut total, mut pending) = (0u64, vec![folder.to_owned()]);
    while let Some(directory) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        for entry in list(&directory).ok()? {
            if entry.is_symlink || entry.name == "." || entry.name == ".." {
                continue;
            }
            if entry.is_dir {
                pending.push(join_remote(&directory, &entry.name));
            } else {
                total = total.saturating_add(entry.size);
            }
        }
    }
    Some(total)
}

/// Local side of `remote_tree_size` (upload of a folder). Original FUN_00465e6c walks FindFirstFileW("*") and adds files with nFileSizeLow only.
/// Fixed: files of 4 GB or more counted wrong in the original; the full 64-bit size is used here.
pub(super) fn local_tree_size(folder: &Path, cancel: &AtomicBool) -> Option<u64> {
    use std::os::windows::fs::MetadataExt;
    let (mut total, mut pending) = (0u64, vec![folder.to_path_buf()]);
    while let Some(directory) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        for entry in std::fs::read_dir(&directory).ok()?.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.file_attributes() & 0x400 != 0 {
                continue;
            }
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Some(total)
}

/// Commands usable even in a pane's FTP mode (list navigation/selection/view/FTP/help). The rest handle local files, so they are blocked.
pub(super) fn ftp_allows(id: &str) -> bool {
    const EXACT: &[&str] = &[
        "refresh", "goParent", "goRoot", "open", "selectAll", "invertSelection", "select", "copyToOther", "moveToOther",
        "remote", "sites", "remoteCommand", "exit", "options", "language", "theme", "about", "help", "homepage", "checkUpdates",
        "filter", "newTab", "closeTab", "historyBack", "historyForward", "back", "forward", "favoritesMenu", "switchPane",
        // The FTP mode (2) branch of the original miNewFolderClick(0x432378)·miRenameClick(0x4351cc)·miDeleteClick(0x431c04 → FUN_0042c734).
        "newFolder", "rename", "deleteRecycle", "deletePermanent", "deleteToolbar", "shred", "attributes",
        // Original miCommandLineClick(0x438598): in FTP mode, the input is sent as an FTP control command.
        "command",
        // Original miMapNetworkDriveClick(0x435144)·miUnmapNetworkDriveClick(0x4350bc): open regardless of mode (re-reading happens only when local).
        "mapDrive", "unmapDrive", "browseNetwork",
    ];
    EXACT.contains(&id) || ["ftp", "toggle", "view", "sort", "column:", "folderSort", "group", "favoriteOpen:", "workOpen:", "newTabFrom:", "closeTab:"].iter().any(|prefix| id.starts_with(prefix))
}

fn to_entry(remote: &PaneRemote, item: &RemoteEntry) -> Entry {
    // Original FUN_0045bb0c: if the name starts with '.', add it with the hidden attribute (folder 0x12, file 0x02).
    let dot = item.name.starts_with('.');
    Entry {
        name: item.name.clone(),
        path: remote.item_path(&item.name),
        is_dir: item.is_dir,
        size: if item.is_dir { 0 } else { item.size },
        modified: item.modified.filter(|s| *s >= 0).map(|s| std::time::UNIX_EPOCH + Duration::from_secs(s as u64)),
        hidden: dot,
        attributes: if item.is_dir { 0x10 } else { 0 } | if dot { 0x2 } else { 0 },
        drive: false,
    }
}

/// Read the remote listing (the FTP side of refresh). "." and ".." are dropped. Like the original FUN_0045bb0c, when hidden view is off, names starting with '.' are also dropped.
/// Fix: previously dot files were always shown regardless of Alt+Z.
pub(super) fn list_remote(remote: &PaneRemote, show_hidden: bool) -> Result<Vec<Entry>, String> {
    let items = remote.fs.lock().map_err(|_| "FTP connection is unavailable".to_owned())?.list(&remote.path)?;
    let items: Vec<&RemoteEntry> = items.iter().filter(|e| e.name != "." && e.name != ".." && (show_hidden || !e.name.starts_with('.'))).collect();
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

    /// Original FUN_00440734: site menu items (number, text).
    pub(super) fn ftp_site_items(&self) -> Vec<(usize, String)> {
        load_sites(&self.settings).iter().enumerate().map(|(i, s)| (i, site_menu_caption(i, &s.name))).collect()
    }

    /// Original FUN_0043e720: connect the current pane to the site at index in the site list.
    pub(super) fn ftp_connect_site(&mut self, index: usize) {
        let Some(site) = load_sites(&self.settings).get(index).cloned() else { return };
        let password = native::unprotect_secret(&site.encrypted_password).unwrap_or_default();
        self.ftp_connect(self.active, site, password);
    }

    /// Original miFTPQuickConnectClick → FUN_0043e9f8(window, pane, address): TfFTPQuickConnect. The address is the first cbHost value (the command line's "ftp://…").
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
            // FUN_0043e9f8: port (StrToInt, default 21); SFTP when 22.
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

    /// Original FUN_004408f8: connect the pane in FTP mode (if it is an FTP pane already transferring, FUN_00440d24 blocks it).
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
        // Original FUN_004408f8: if the site type is not "sftp", create a log pane (FUN_0048d7ac(pane, 1); keep the existing one if present).
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
            // Original FUN_0048d7ac: a new log pane has height 60.
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

    /// Original miFTPDisconnectClick(0x43e8d4): FTP_CANNOTDISCONNECT if transferring, otherwise disconnect and go to a local folder.
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

    /// Open a folder in an FTP pane (a folder row in the list: change the remote path and re-read).
    /// Original TNXFileListAdapter FUN_0045b198 (hourglass cursor): if the server's PWD differs from the adapter's folder (+0x1c) it first CWDs
    /// back there, then sends CWD name (CDUP for ".."), and the new folder is the server's PWD (SFTP: SetCurrentDir / GetCurrentDir).
    /// NDR lists absolute paths and never depends on the server's current folder, so the re-sync step is not needed; the folder shown
    /// is the joined path rather than the server's resolved one. Connect FUN_0045cd40 also starts a TTimer whose OnTimer (0x45e6b0) is
    /// an empty function, so there is no keep-alive to port.
    pub(super) fn ftp_open_dir(&mut self, index: usize, name: &str) {
        let Some(remote) = self.panes[index].remote.as_mut() else { return };
        remote.path = join_remote(&remote.path, name);
        let address = remote.address();
        self.panes[index].address = address;
        self.refresh(index);
    }

    /// The parent folder of an FTP pane (unchanged at the root).
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
        // Original FUN_00486694 (moving up): put the cursor on the folder we left.
        self.pending_focus[index] = (!child.is_empty()).then_some(child);
        self.refresh(index);
    }

    /// The root of an FTP pane.
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

    /// Original miFTPGetClick(0x4412f8): if the current pane is FTP, choose an FTP_GET destination and then download.
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

    /// Original FUN_0043f800: download the selected items of the FTP pane (index) to a local folder (FTPENGINE_BUSY; a same-name confirmation dialog if it already exists).
    pub(super) fn ftp_download(&mut self, index: usize, destination: PathBuf) {
        let Some(remote) = self.panes[index].remote.clone() else { return };
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTPENGINE_BUSY", "Other files are still being transferred.");
        }
        let items: Vec<(String, String, u64, bool)> = self.target_entries(index).into_iter().filter_map(|e| {
            let source = remote.remote_of(&e.path)?;
            let name = source.trim_end_matches('/').rsplit('/').next()?.to_owned();
            Some((source, name, if e.is_dir { 0 } else { e.size }, e.is_dir))
        }).collect();
        if items.is_empty() {
            return;
        }
        let rows = self.ftp_results.clone();
        if let Ok(mut list) = rows.lock() {
            list.clear();
            list.extend(items.iter().map(|(source, name, size, _)| FtpResultRow { upload: false, local: destination.join(name).to_string_lossy().into_owned(), remote: source.clone(), size: (*size > 0).then_some(*size), done: 0, state: RowState::Waiting, started: None }));
        }
        // End of original FUN_0043f800: show the bottom pane (pBottom).
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
                for (row, (source, _, _, is_dir)) in items.iter().enumerate() {
                    if *is_dir && let Some(size) = remote_tree_size(&mut |path| fs.list(path), source, &cancel) {
                        set_row(&rows, row, |r| r.size = Some(size));
                    }
                }
                for (row, (source, name, _, _)) in items.iter().enumerate() {
                    super::commands::wait_while_paused(&pause, &cancel);
                    if cancel.load(Ordering::Relaxed) {
                        set_row(&rows, row, |r| r.state = RowState::Cancelled);
                        continue;
                    }
                    set_row(&rows, row, |r| {
                        r.state = RowState::Running;
                        r.started = Some(std::time::Instant::now());
                    });
                    let local = destination.join(name);
                    let mut resolve = |src: &str, dst: &str, info: &ndr::remote::ConflictInfo| ask_remote_conflict(&events, &cancel, src, dst, info);
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

    /// The original FTP transfer (the FTP engine thread) shows progress only in the bottom results pane (pBottom), with no progress window, so the main window stays usable.
    /// Hide the NDR file-operation progress window (job.progress). Conflict and error boxes still appear.
    pub(super) fn ftp_progress_in_results_only(&mut self, started: bool) {
        if let Some(job) = self.job.as_mut().filter(|_| started) {
            job.progress = None;
        }
    }

    /// Original miFTPPutClick(0x441168): upload only when one pane is FTP and the other is local (otherwise FTP_PUTCRITERIA).
    pub(super) fn ftp_upload_command(&mut self) {
        let (left, right) = (self.panes[0].remote.is_some(), self.panes[1].remote.is_some());
        let local = |app: &App, i: usize| app.panes[i].remote.is_none() && app.panes[i].archive.is_none();
        match (left, right) {
            (true, false) if local(self, 1) => self.ftp_upload(1, 0),
            (false, true) if local(self, 0) => self.ftp_upload(0, 1),
            _ => self.ftp_box("FTP_PUTCRITERIA", "One pane should be in My Computer mode and the other should be in FTP mode."),
        }
    }

    /// Original FUN_0043feb0: upload the selected items of the local pane (source) to the current folder of the FTP pane (target).
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
                FtpResultRow { upload: true, local: local.to_string_lossy().into_owned(), remote: join_remote(&remote.path, &name), size, done: 0, state: RowState::Waiting, started: None }
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
                    if local.is_dir() && let Some(size) = local_tree_size(local, &cancel) {
                        set_row(&rows, row, |r| r.size = Some(size));
                    }
                }
                for (row, local) in items.iter().enumerate() {
                    super::commands::wait_while_paused(&pause, &cancel);
                    if cancel.load(Ordering::Relaxed) {
                        set_row(&rows, row, |r| r.state = RowState::Cancelled);
                        continue;
                    }
                    set_row(&rows, row, |r| {
                        r.state = RowState::Running;
                        r.started = Some(std::time::Instant::now());
                    });
                    let name = local.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("Upload source has no name")?;
                    let destination = join_remote(&remote.path, &name);
                    let mut resolve = |src: &str, dst: &str, info: &ndr::remote::ConflictInfo| ask_remote_conflict(&events, &cancel, src, dst, info);
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

    /// Original miViewFindResultClick(0x434b20): show/hide the bottom pane (pBottom) and the splitter bar.
    pub(super) fn toggle_results(&mut self) {
        self.show_results = !self.show_results;
    }

    /// Whether an FTP transfer is in progress (original [+0xd6c]+0x14).
    fn ftp_transferring(&self) -> bool {
        self.panes.iter().any(|p| p.remote.as_ref().is_some_and(|r| r.busy.load(Ordering::Relaxed)))
    }

    /// Original btnFTPPauseClick(0x440f78): while transferring, toggle pause ↔ resume.
    pub(super) fn ftp_pause_toggle(&mut self) {
        if self.ftp_transferring() {
            let paused = self.ftp_pause.load(Ordering::Relaxed);
            self.ftp_pause.store(!paused, Ordering::Relaxed);
        }
    }

    /// Original btnFTPAbortClick(0x440f3c): while transferring, flag an abort (if paused, release it so it can finish).
    pub(super) fn ftp_abort(&mut self) {
        if !self.ftp_transferring() {
            return;
        }
        if let Some(job) = self.job.as_ref().filter(|j| !j.finished) {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.ftp_pause.store(false, Ordering::Relaxed);
    }

    /// Original FUN_0044033c (FTP queue end, engine event): removes finished items from the queue and hides the bottom panel (pBottom) when none remain.
    /// (The end sound FUN_0043d9fc and rereading the FTP pane FUN_00487ce8 are done by report_job.)
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

    /// Height taken by the log panel of original FUN_0048d7ac (splitter 3 + log panel). 0 without a log.
    pub(super) fn ftp_log_reserve(&self, index: usize, spacing: f32) -> f32 {
        match self.panes[index].remote.as_ref().and_then(|r| r.log.as_ref()) {
            Some(_) => FTP_SPLITTER_HEIGHT + self.panes[index].ftp_log_height + spacing * 2.0,
            None => 0.0,
        }
    }

    /// Original FUN_0048d7ac(pane, 1): TListBox below an FTP-mode pane (alBottom, height 60, no border,
    /// background [FTP] ftplogback, text ftplogtext) and the TSplitter above it (alBottom, MinSize 30, pane color).
    /// Like original FUN_004404c0, each added row selects the last row (ItemIndex); the selected row uses the Windows highlight color.
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
            // Original TSplitter MinSize 30: neither the log panel nor the list gets smaller than 30.
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

    /// Original pBottom: lvFTPProgress (" ;Local;Progress; ;Remote") and btnFTPPause, btnFTPAbort.
    pub(super) fn ftp_results_ui(&mut self, ui: &mut egui::Ui) {
        let ko = self.language == "ko";
        let headers: Vec<String> = self.language_pack.text("formMain.ListViewCols.lvFTPProgress", " ;Local;Progress; ;Remote").split(';').map(str::to_owned).collect();
        let header = |i: usize| headers.get(i).cloned().unwrap_or_default();
        let paused = self.ftp_pause.load(Ordering::Relaxed);
        let mut action = None;
        let rows = self.ftp_results.lock().map(|r| r.clone()).unwrap_or_default();
        let transferring = self.ftp_transferring();
        // Original NXPaintPanel1 (height 21): btnFTPPause (1,0 75x21) btnFTPAbort (78,0 75x21) pbFTPTransfer (159,2 150x16)
        // lFTPTransferred (315,4), at the right end btnFTPShowDetail (">>", right - 46, 19x19) btnFTPCloseProgress ("X", right - 21).
        let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 21.0), egui::Sense::hover());
        let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(bar.min + egui::vec2(x, y), egui::vec2(w, h));
        // Original btnFTPPauseClick (0x440f78): pausing shows RES.RESUME ("resume"), resuming shows RES.PAUSE ("pause").
        // Event 4 of FUN_0043f04c (new transfer list) also resets it to RES.PAUSE. Before the first transfer, the designed caption (btnFTPPause).
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
        // Original FUN_0043f04c event 1: pbFTPTransfer = percentage, lFTPTransferred = "%d %% (received / total) - speed". Without a transfer, 0 and empty text.
        let (done, total, percent) = ftp_totals(&rows);
        let running = transferring && !rows.is_empty();
        ui.put(at(159.0, 2.0, 150.0, 16.0), egui::ProgressBar::new(if running { percent as f32 / 100.0 } else { 0.0 }).desired_height(16.0).corner_radius(0.0));
        if running {
            let speed = ftp_speed(&rows, std::time::Instant::now());
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
        // The original lvFTPProgress sets no colors (no Color in the DFM, the skin apply 0x43daf0 leaves it alone, and the [FTP] ftpprog* keys
        // are never read by the original executable). So even the skin theme uses the Windows window colors (clWindow, clWindowText).
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
                // Original FUN_0043f04c event 2 (item end): removes that row from lvFTPProgress.
                // Original event 4 (build the list): the first cell is empty (lvFTPProgress has no image list), then Local, Progress, (percentage), Remote.
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
        // Original btnFTPShowDetailClick (0x441114): height > 30 becomes 22 (button row only), otherwise 100. btnFTPCloseProgressClick (0x4410f0): hides pBottom and splBottom.
        match action {
            Some(0) => self.ftp_pause_toggle(),
            Some(1) => self.ftp_abort(),
            Some(2) => self.results_collapsed = true,
            Some(3) => self.show_results = false,
            _ => {}
        }
    }

    /// FTP branch of original miNewFolderClick (0x432378): TfGetText(TITLE_NEWFOLDER, MSG_NEWFOLDER, today's date name) ->
    /// FUN_0045d0b4(name) -> reread and put the cursor on that name (FUN_00487ce8(list, 1, 1, name)).
    pub(super) fn ftp_new_folder(&mut self, index: usize) {
        let Some(remote) = self.panes[index].remote.clone() else { return };
        if remote.busy.load(Ordering::Relaxed) {
            return self.ftp_box("FTPENGINE_BUSY", "Other files are still being transferred.");
        }
        let title = self.language_pack.text("RES.TITLE_NEWFOLDER", "New Folder");
        let message = self.language_pack.text("RES.MSG_NEWFOLDER", "Type a folder name to be created.");
        // Original FUN_00432378 (pane mode 2 branch) uses the same TfGetText (flags 1).
        self.prompt_file_name(&title, &message, &ndr::names::new_folder_default_name(), move |app, value, _| {
            let Some(name) = value.filter(|v| !v.trim().is_empty()) else { return };
            let path = join_remote(&remote.path, &name);
            app.ftp_edit(index, remote, Some(name), move |fs| fs.mkdir(&path));
        });
    }

    /// FTP branch of original miRenameClick (0x4351cc): TfRename for each chosen item (name field = name without extension, the whole name when that is empty;
    /// extension field when the extension is longer than one character). Cancel stops. Confirmed ones are renamed with FUN_0045d214(old name, new name), then reread.
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
        // If the cursor item was renamed, put the cursor on the new name.
        let focus = focused.as_ref().and_then(|f| renames.iter().find(|(source, _, _)| source.trim_end_matches('/').rsplit('/').next() == Some(f.as_str())).map(|(_, _, target)| target.clone())).or(focused);
        self.ftp_edit(index, remote, focus, move |fs| {
            for (source, target, _) in &renames {
                fs.rename(source, target)?;
            }
            Ok(())
        });
    }

    /// Original miDeleteClick (0x431c04) -> FTP branch of FUN_0042c734: always asks the delete confirmation (FUN_0040c5c4) without the Recycle Bin,
    /// deletes the folder (+0x3c == 1) and file lists with FUN_0045de18, then rereads.
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

    /// FTP branch of original miChangeAttributesClick (0x43657c) -> FUN_00436b98: TfFTPAttributes ("name (n / total)", the listed permissions) for each chosen item.
    /// OK = that item, apply to all = same permissions for all remaining items, skip = next, cancel = stop. Chosen permissions are applied with FUN_0045e274(path, permissions), then reread.
    /// Items whose permissions the server did not report start from 644.
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

    /// Runs short FTP actions (new folder, rename) on a separate thread and rereads with AsyncResult::FtpEdited when done.
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

    /// FTP mode (2) branch of original miCommandLineClick (0x438598): sends the typed text as an FTP control command (FUN_0082a178, reply code -1 = any).
    /// The original does not reread the list. The reply is shown in the status line instead of a separate window, like the original.
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

    /// Original FUN_0042d518/FUN_0042d910 (copy to the other pane): local -> FTP uploads, FTP -> local downloads. True when handled.
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

/// Closes the connection on a separate thread (so a slow server reply does not freeze the screen).
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
        FtpResultRow { upload: false, local: String::new(), remote: String::new(), size, done, state, started: None }
    }

    /// Original FUN_00468f0c: the running item's bytes / 1024 / whole seconds since it began; 0 in its first second.
    #[test]
    fn ftp_speed_follows_the_original_on_work() {
        let now = std::time::Instant::now();
        let begun = |secs: u64| now.checked_sub(std::time::Duration::from_millis(secs)).unwrap();
        let rows = vec![
            FtpResultRow { started: Some(begun(9_000)), ..row(Some(10), 10, RowState::Done) },
            FtpResultRow { started: Some(begun(2_900)), ..row(Some(8192), 4096, RowState::Running) },
        ];
        assert_eq!(ftp_speed(&rows, now), 2.0, "4 KB over 2 whole seconds");
        let fresh = vec![FtpResultRow { started: Some(begun(900)), ..row(Some(8192), 4096, RowState::Running) }];
        assert_eq!(ftp_speed(&fresh, now), 0.0, "under one second");
        assert_eq!(ftp_speed(&[row(Some(1), 0, RowState::Waiting)], now), 0.0);
    }

    #[test]
    fn folder_rows_get_the_total_size_before_the_transfer() {
        // Original FUN_00466e66: folder items are expanded and their file sizes added to the queue total before transferring.
        let tree: std::collections::BTreeMap<&str, Vec<RemoteEntry>> = [
            ("/pub", vec![RemoteEntry { name: "a.bin".into(), size: 100, ..Default::default() }, RemoteEntry { name: "sub".into(), is_dir: true, ..Default::default() }, RemoteEntry { name: "link".into(), is_symlink: true, size: 999, ..Default::default() }]),
            ("/pub/sub", vec![RemoteEntry { name: "b.bin".into(), size: 23, ..Default::default() }]),
        ]
        .into_iter()
        .collect();
        let cancel = AtomicBool::new(false);
        let mut list = |path: &str| tree.get(path).cloned().ok_or_else(|| "550".to_owned());
        assert_eq!(remote_tree_size(&mut list, "/pub", &cancel), Some(123), "files below the folder, symbolic links not followed");
        assert_eq!(remote_tree_size(&mut list, "/missing", &cancel), None, "a listing error leaves the size unknown");
        let root = std::env::temp_dir().join(format!("ndr-ftp-tree-size-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("x.txt"), [0u8; 7]).unwrap();
        std::fs::write(root.join("sub").join("y.txt"), [0u8; 5]).unwrap();
        assert_eq!(local_tree_size(&root, &cancel), Some(12));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(local_tree_size(&root, &cancel), None, "cancelled");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn total_progress_follows_the_original_ftp_event() {
        // Original FUN_0043f04c event 1: received bytes x 100 / total bytes. Finished rows count their whole size, waiting rows 0.
        let rows = [row(Some(1000), 1000, RowState::Done), row(Some(3000), 1000, RowState::Running), row(Some(4000), 0, RowState::Waiting)];
        assert_eq!(ftp_totals(&rows), (2000, 8000, 25));
        // When the total is unknown, 100% (original iVar6 starts at 100).
        assert_eq!(ftp_totals(&[row(None, 10, RowState::Running)]).2, 100);
        // lFTPTransferred: "%d %% (%s / %s) - %s" (wsprintf %% = %), speed via FormatFloat("0.00 KB/s").
        assert_eq!(ftp_transferred_text(25, "1.95 KB", "7.81 KB", 12.345), "25 % (1.95 KB / 7.81 KB) - 12.35 KB/s");
    }

    #[test]
    fn row_cells_follow_the_original_lvftpprogress_text() {
        // Original FUN_0043f04c event 1: the Progress cell = percentage/10 '<' (download) or '>' (upload) characters, the fourth cell = "%d%".
        assert_eq!(row(Some(1000), 370, RowState::Running).progress_cells(0, true), ("<<<".to_owned(), "37%".to_owned()));
        let upload = FtpResultRow { upload: true, ..row(Some(1000), 1000, RowState::Running) };
        assert_eq!(upload.progress_cells(0, true), (">>>>>>>>>>".to_owned(), "100%".to_owned()));
        // When the item size is unknown, the overall percentage.
        assert_eq!(row(None, 5, RowState::Running).progress_cells(42, true), ("<<<<".to_owned(), "42%".to_owned()));
        // Waiting rows are blank, failed rows show the state text.
        assert_eq!(row(Some(10), 0, RowState::Waiting).progress_cells(0, true), (String::new(), String::new()));
        assert_eq!(row(Some(10), 0, RowState::Failed("550".into())).progress_cells(0, true), ("550".to_owned(), String::new()));
    }
}
