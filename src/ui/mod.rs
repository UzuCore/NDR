mod archive_dialog;
mod archive_edit;
mod auto_view;
mod skin_pane;
mod empty_commands;
mod commands;
mod compare_dialog;
mod date_time_picker;
mod destination_dialog;
mod dialogs;
mod disk_copy_dialog;
mod drag_drop;
mod folder_tree_dialog;
mod favorites_dialog;
mod function_key_editor;
mod file_commands;
mod file_operation;
mod ftp_pane;
mod grid;
mod keyboard;
mod list_export_dialog;
mod list_keys;
mod list_view;
mod main_window;
mod menu_data;
mod msgbox;
mod original_dialogs;
mod pane_chrome;
mod pane_commands;
mod panes;
mod quick_search;
mod remote_sites;
mod remote_window;
mod rename_dialog;
mod search_dialog;
mod select_dialog;
mod settings_window;
mod shot;
mod shortcut_window;
mod split_dialogs;
mod system_commands;
mod toolbar;
mod tools_dialog;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod parity_tests;
#[cfg(test)]
mod file_command_tests;
#[cfg(test)]
mod tools_tests;
#[cfg(test)]
mod remote_tests;
#[cfg(test)]
mod shortcut_tests;
#[cfg(test)]
mod settings_tests;
#[cfg(test)]
mod file_operation_tests;
#[cfg(test)]
mod main_window_tests;
mod user_commands;
mod views;

use crate::Options;
use list_view::{DetailColumn, DetailRow, PaneView};
use eframe::egui::{self, Color32, RichText};
use ndr::{
    compat,
    fsops::{self, ConflictChoice, Entry, Event, Operation},
    native,
    settings::Settings,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

/// Original TNXShellChangeNotify constructor (0x7b3304): Delay(+0x40) = 0x5dc = 1500ms.
const FOLDER_CHANGE_DELAY: Duration = Duration::from_millis(1500);

pub(super) fn resource_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("NDR_RESOURCE_DIR") {
        return p.into();
    }
    let executable = std::env::current_exe().unwrap_or_default();
    let folder = executable.parent().unwrap_or(Path::new(".")).to_path_buf();
    if folder.join("Languages").is_dir() {
        folder
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Resources")
    }
}

#[derive(Clone)]
struct ArchiveLocation {
    file: PathBuf,
    prefix: String,
    password: String,
}
struct Pane {
    path: PathBuf,
    address: String,
    entries: Vec<Entry>,
    selected: BTreeSet<usize>,
    cursor: usize,
    anchor: usize,
    tabs: Vec<PathBuf>,
    tab: usize,
    /// (back, forward) history per tab (C# PaneState per tab). The current tab's history lives in back/forward.
    tab_history: Vec<(Vec<PathBuf>, Vec<PathBuf>)>,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// C# FilterPattern: name masks split by ';'. Applied when a listing arrives.
    filter: String,
    show_hidden: bool,
    loading: bool,
    generation: u64,
    error: Option<String>,
    archive: Option<ArchiveLocation>,
    /// Original panel FTP mode (FUN_00481948 == 2): the connection and the remote folder. path stays as the local folder from before connecting.
    remote: Option<ftp_pane::PaneRemote>,
    /// Original FTP log pane height (starts at 60, changed via the upper splitter bar).
    ftp_log_height: f32,
    descriptions: HashMap<String, String>,
    /// Column text/color/grouping rows drawn on screen (rebuilt in sort_pane).
    view: PaneView,
    /// Listing summary for the info bar (C# PaneState.UpdateSummary). None when a new listing arrives.
    summary: Option<main_window::PaneSummary>,
    /// Original info bar disk data (drive letter "Name (C:) ", free, total). None if it could not be read.
    disk: Option<(String, u64, u64)>,
    /// Drive entry info in My PC view (original TNXDriveInfo: name, kind, size if a fixed disk).
    drive_infos: HashMap<PathBuf, (String, u32, Option<(u64, u64)>)>,
    /// Editing the address bar as a text field (C# _addressEditing).
    address_editing: bool,
    /// Automatic folder size display (path -> None = counting, Some(text)). Cleared when a new listing arrives.
    folder_sizes: HashMap<PathBuf, Option<String>>,
    /// Counted folder sizes (bytes). The original file info bar (FUN_0048a5cc) shows the entry size via FloatToStrF(ffNumber).
    folder_bytes: HashMap<PathBuf, u64>,
    /// Flag to stop counting folder sizes for the current listing (C# CancelFolderSizeJobs).
    folder_cancel: Arc<AtomicBool>,
    /// Whether the network share connection dialog has already been shown at this location (C# allowNetworkAuthPrompt: once only).
    auth_prompted: bool,
    /// Page info from the last paint (rows per column, visible column count, top item; used by the original ArrowKey).
    page: list_keys::ListPage,
    /// First visible column of a column-flowing listing (auto/name) (original horizontal position 0x378 / column width).
    first_column: usize,
    /// Horizontal position of the detail view (original 0x378, pixels). Greater than 0 only when the total column width exceeds the list width.
    detail_x: f32,
    /// Thumbnail row positions from the last paint (per-row top/height, top position, list height, column count). Used by the original PageUp/PageDown in style 3.
    thumb_rows: Option<(Vec<f32>, Vec<f32>, f32, f32, usize)>,
    /// Thumbnail row height cache (key, heights): not recomputed when list/column count/size are unchanged.
    thumb_heights: Option<(u64, Vec<f32>)>,
    /// Original panel folder tree (TNXShellTreeView, miShowFoldersClick): expanded folders and the last panel location whose expansion was synced (SetPath).
    tree_open: HashSet<PathBuf>,
    tree_synced: Option<PathBuf>,
    /// Item selected in the tree (key). None means the row at the panel location (selecting a virtual folder leaves the panel unchanged, so it is kept separately).
    tree_selected: Option<PathBuf>,
    /// Tree label rename (original TTreeView ReadOnly = False -> TVS_EDITLABELS, TNXCustomShellTreeView.Edit 0x7edf70):
    /// clicking the selected item's text again edits it in place after the double-click interval. (key, click time) and the (folder, text) being edited.
    tree_label_click: Option<(PathBuf, Instant)>,
    tree_edit: Option<(PathBuf, String)>,
    /// Tree rows from the last frame (item, depth, expandable, expanded) plus position/rows per page, and whether to scroll the selected row into view.
    tree_rows: Vec<(ndr::shell_tree::ShellNode, usize, bool, bool)>,
    tree_rect: Option<egui::Rect>,
    tree_page: usize,
    tree_first: usize,
    tree_scroll_to: bool,
    /// Original TNXShellChangeNotify (+0x374): the local folder being watched and the flag to stop its watcher thread.
    watch: Option<(PathBuf, Arc<AtomicBool>)>,
    /// When to re-read after a change notification and how many times it has been deferred meanwhile (original Delay 1500ms, defers again up to 20 times).
    change_due: Option<(Instant, u32)>,
    /// Scroll the cursor into view on the next paint (original SetItemIndex).
    reveal: bool,
    /// Original AdjustPage (0x7daf24): on the next paint, reposition to this item number. When the list size changes (WMSize 0x7d5814),
    /// when the view style changes (SetListStyle 0x7dc6d4), or when the same location is re-read (FileListChanged 0x7d79c8, column-flowing listings only).
    /// If the second value is true it is because of a re-read (detail/thumbnail views are ignored because the original then reverts with SetScrollTop(old position)).
    adjust: Option<(usize, bool)>,
    /// List size and view style from the last paint (adjust when changed).
    list_shape: Option<(egui::Vec2, String)>,
    /// List area from the last frame (used for auto edge scrolling while dragging).
    list_rect: Option<egui::Rect>,
    /// Lines to scroll the list this frame (vertical, horizontal): where the original TNXDropTarget.DoTargetScroll sent WM_VSCROLL/WM_HSCROLL SB_LINEUP/SB_LINEDOWN.
    drag_scroll: (i8, i8),
    /// Rows to scroll the panel folder tree (top/bottom edge while dragging).
    tree_drag_scroll: i8,
    /// Where a drag started while Shift was held (original SetShiftDownIndex 0xf4).
    shift_anchor: Option<usize>,
    /// Original display column drag (+0x3d4 base row, +0x3d8 last end row). Cleared when the left button is released (MouseUp).
    range_drag: Option<(usize, usize)>,
    /// Items marked by original Edit > Cut (0x433dd8) (item +100, icon dimmed). Cleared when a new listing arrives (new items).
    cut_marks: HashSet<PathBuf>,
    /// Location of the previously received listing (location_key). When the same location is re-read, the cursor stays on the same name.
    listed: Option<String>,
    /// Original MouseDown pressed an item (not a second press). The original clears the search text then (virtual +0x160("")), so quick search picks it up and clears.
    item_pressed: bool,
}
impl Pane {
    fn new(path: PathBuf, show_hidden: bool) -> Self {
        Self {
            address: path.to_string_lossy().into_owned(),
            path: path.clone(),
            entries: vec![],
            selected: BTreeSet::new(),
            cursor: 0,
            anchor: 0,
            tabs: vec![path],
            tab: 0,
            tab_history: Vec::new(),
            back: vec![],
            forward: vec![],
            filter: String::new(),
            show_hidden,
            loading: false,
            generation: 0,
            error: None,
            archive: None,
            remote: None,
            ftp_log_height: ftp_pane::FTP_LOG_HEIGHT,
            descriptions: HashMap::new(),
            view: PaneView::default(),
            summary: None,
            disk: None,
            drive_infos: HashMap::new(),
            address_editing: false,
            folder_sizes: HashMap::new(),
            folder_bytes: HashMap::new(),
            folder_cancel: Arc::new(AtomicBool::new(false)),
            auth_prompted: false,
            page: list_keys::ListPage::default(),
            first_column: 0,
            detail_x: 0.0,
            thumb_rows: None,
            thumb_heights: None,
            tree_open: HashSet::new(),
            tree_synced: None,
            tree_selected: None,
            tree_label_click: None,
            tree_edit: None,
            tree_rows: Vec::new(),
            tree_rect: None,
            tree_page: 10,
            tree_first: 0,
            tree_scroll_to: false,
            watch: None,
            change_due: None,
            reveal: false,
            adjust: None,
            list_shape: None,
            list_rect: None,
            drag_scroll: (0, 0),
            tree_drag_scroll: 0,
            shift_anchor: None,
            range_drag: None,
            cut_marks: HashSet::new(),
            listed: None,
            item_pressed: false,
        }
    }
    fn targets(&self) -> Vec<PathBuf> {
        if self.loading {
            return vec![];
        }
        if self.selected.is_empty() {
            self.entries
                .get(self.cursor)
                .filter(|e| !e.drive && !e.is_parent())
                .map(|e| vec![e.path.clone()])
                .unwrap_or_default()
        } else {
            self.selected
                .iter()
                .filter_map(|i| self.entries.get(*i).filter(|e| !e.drive && !e.is_parent()).map(|e| e.path.clone()))
                .collect()
        }
    }
    /// Number of drive entries at the end of the list (original kind 4).
    pub(super) fn drive_count(&self) -> usize {
        self.entries.iter().filter(|e| e.drive).count()
    }
    /// Original Select (0x7dc834): drive entries and ".." are not selected (+0x67). Remove them from marked rows.
    pub(super) fn drop_drive_marks(&mut self) {
        if self.selected.iter().any(|i| self.entries.get(*i).is_some_and(|e| e.drive || e.is_parent())) {
            let entries = &self.entries;
            self.selected.retain(|i| entries.get(*i).is_none_or(|e| !e.drive && !e.is_parent()));
        }
    }
}
/// Remove the egui events (pressed Key and Text character) of one numpad key received by the original shortcut. A key handled by Delphi IsShortCut
/// produces no WM_CHAR, so it also does not reach quick search.
pub(super) fn strip_numpad_events(raw: &mut egui::RawInput, vk: u16) {
    let (key, text) = match vk {
        0x60..=0x69 => (egui::Key::from_name(&((vk - 0x60).to_string())), char::from_digit((vk - 0x60) as u32, 10).unwrap_or('0')),
        0x6a => (None, '*'),
        0x6b => (Some(egui::Key::Plus), '+'),
        0x6d => (Some(egui::Key::Minus), '-'),
        0x6e => (Some(egui::Key::Period), '.'),
        0x6f => (Some(egui::Key::Slash), '/'),
        _ => return,
    };
    if let Some(key) = key {
        if let Some(i) = raw.events.iter().position(|e| matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key)) {
            raw.events.remove(i);
        }
    }
    if let Some(i) = raw.events.iter().position(|e| matches!(e, egui::Event::Text(t) if t.chars().eq(std::iter::once(text)))) {
        raw.events.remove(i);
    }
}
/// Turn egui Copy/Cut/Paste events back into the original keys (Ctrl+C/X/V, Ctrl+Insert, Shift+Delete, Shift+Insert),
/// and append the paste keys that egui-winit discarded (pastes - number of replaced Pastes).
pub(super) fn restore_clipboard_keys(raw: &mut egui::RawInput, pastes: u32, insert_down: bool) {
    let modifiers = raw.modifiers;
    let shift_only = modifiers.shift && !modifiers.ctrl;
    let key = |key: egui::Key| egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers };
    let mut pasted = 0;
    for event in raw.events.iter_mut() {
        let replacement = match event {
            egui::Event::Copy => Some(if insert_down { egui::Key::Insert } else { egui::Key::C }),
            egui::Event::Cut => Some(if shift_only { egui::Key::Delete } else { egui::Key::X }),
            egui::Event::Paste(_) => {
                pasted += 1;
                Some(if shift_only { egui::Key::Insert } else { egui::Key::V })
            }
            _ => None,
        };
        if let Some(replacement) = replacement {
            *event = key(replacement);
        }
    }
    for _ in pasted..pastes {
        raw.events.push(key(if shift_only { egui::Key::Insert } else { egui::Key::V }));
    }
}
/// A panel location as a single string (local folder, archive file + inner path, FTP host + path). Used to tell whether the same location was re-read.
fn location_key(pane: &Pane) -> String {
    match (&pane.archive, &pane.remote) {
        (_, Some(remote)) => format!("ftp|{}|{}", remote.host, remote.path),
        (Some(archive), None) => format!("arc|{}|{}", archive.file.display(), archive.prefix),
        (None, None) => format!("dir|{}", pane.path.display()).to_lowercase(),
    }
}
/// Original GetList (0x45a34c): a ".." entry for non-root local folders (except My PC view), archive interiors (always), and FTP/SFTP folders other than "/".
fn parent_item_for(pane: &Pane) -> Option<Entry> {
    if let Some(remote) = &pane.remote {
        let path = remote.path.trim_end_matches('/');
        return (!path.is_empty()).then(|| Entry::parent_item(PathBuf::from(".."), None));
    }
    if let Some(location) = &pane.archive {
        let modified = std::fs::metadata(&location.file).and_then(|m| m.modified()).ok();
        return Some(Entry::parent_item(location.file.parent().map(Path::to_path_buf).unwrap_or_default(), modified));
    }
    if panes::is_drives(&pane.path) {
        return None;
    }
    let parent = pane.path.parent()?;
    let modified = std::fs::metadata(&pane.path).and_then(|m| m.modified()).ok();
    Some(Entry::parent_item(parent.to_path_buf(), modified))
}
/// The search text of the original TNXCustomFileListView quick search (+0x3e0).
#[derive(Default)]
struct TypedSearch {
    query: String,
    /// The cursor moved by the original quick search. If the cursor changes by another route (arrow/click), the search text is cleared (original ArrowKey/MouseDown).
    cursor: Option<usize>,
    /// Original WMIMEComposition (0x7d584c) in-composition display (+0x3dd): the search text before composition started.
    ime_base: Option<String>,
}
struct Listing {
    pane: usize,
    generation: u64,
    result: Result<Vec<Entry>, String>,
    descriptions: HashMap<String, String>,
    /// Drive entry info in My PC view (root -> (name, kind, size)).
    drive_infos: HashMap<PathBuf, (String, u32, Option<(u64, u64)>)>,
}
struct Job {
    receiver: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    path: String,
    files: u64,
    bytes: u64,
    errors: Vec<String>,
    title: String,
    finished: bool,
    /// Operation that calls C# PlayOperationCompletionSound (copy/move/delete/wipe/create archive).
    chime: bool,
    /// Some when this is a C# modal progress window (FolderComparisonProgressWindow, SearchProgressWindow, or the sync's FileOperationProgressWindow).
    /// When finished it closes itself and leaves the status bar and panels alone (the result is handled by AsyncResult).
    modal: Option<ModalProgress>,
    /// When finished, the status bar is written by the result text the job sent (AsyncResult::ArchiveStatus etc.).
    custom_status: bool,
    /// The C# result-notification style when finished.
    report: file_operation::JobReport,
    /// Some = C# FileOperationProgressWindow (delete/wipe are None; C# also has no window).
    progress: Option<file_operation::ProgressInfo>,
    /// Number of errors already shown in an error box (C# InteractiveErrorCount).
    interactive: usize,
    /// Number of items skipped in conflicts (C# SkippedItems).
    skipped: usize,
    /// Error that stopped the whole job (C# FatalError).
    fatal: Option<String>,

    /// Set when the "from" folder was decided from the folder of the first path reported by the original.
    from_seen: Option<String>,
    /// Last Event::Target (original file, actual destination file). The progress window lTo is the destination folder of the current original file.
    target: Option<(PathBuf, PathBuf)>,
}
impl Job {
    fn new(receiver: Receiver<Event>, cancel: Arc<AtomicBool>, title: String) -> Self {
        Self {
            receiver,
            cancel,
            path: String::new(),
            files: 0,
            bytes: 0,
            errors: vec![],
            title,
            finished: false,
            chime: false,
            modal: None,
            custom_status: false,
            report: file_operation::JobReport::Generic,
            progress: Some(file_operation::ProgressInfo::default()),

            interactive: 0,
            skipped: 0,
            fatal: None,

            from_seen: None,
            target: None,
        }
    }
}
/// Text and buttons of the modal progress window.
struct ModalProgress {
    /// Button text (compare: formCompareFolder.Caption.btnStop; search/sync: Cancel).
    stop: String,
    /// Button text after pressing (compare: Stopping...). None leaves it as-is and only disables the button.
    stopping: Option<String>,
    /// Status text after pressing.
    stopping_text: String,
    kind: ModalKind,
}
/// How to read the Job's path/files/bytes.
#[derive(Clone, PartialEq, Eq)]
enum ModalKind {
    /// Compare: status text = path, bar = files / bytes (a marquee bar if bytes is 0).
    Message,
    /// Search: "Searched N items - N results" then path on the next line (files = items checked, bytes = results); the bar is always a marquee.
    Search,
}

struct PasswordRequest {
    /// The original box does not show the archive file name. Tests check which archive asked.
    #[cfg_attr(not(test), allow(dead_code))]
    archive: PathBuf,
    reply: Sender<Option<String>>,
}
enum AsyncResult {
    Text {
        title: String,
        text: String,
    },

    Image {
        path: PathBuf,
        image: Result<(usize, usize, Vec<u8>), String>,
    },
    Tree {
        /// Shell key (ndr::shell_tree: desktop = empty key).
        path: PathBuf,
        /// Whether hidden folders were read too (hidden view may differ per panel).
        hidden: bool,
        directories: Vec<ndr::shell_tree::ShellNode>,
    },
    /// Original TNXShellChangeNotify thread notification (ThreadNotification 0x7b3830): the folder that panel was watching changed.
    FolderChanged { pane: usize, path: PathBuf, watch: Arc<AtomicBool> },
    ProtectedItems(file_commands::DeleteRun),
    AttributesApplied(pane_commands::AttributeRun),
    SplitDone { source: PathBuf, outcome: Result<ndr::split::SplitResult, String>, cancelled: bool },
    JoinDone { count: usize, outcome: Result<(), String>, cancelled: bool },
    SearchInPane { pane: usize, root: PathBuf, outcome: Result<Vec<Entry>, String>, cancelled: bool },
    CompareDone(compare_dialog::CompareRun, Result<(Vec<ndr::folder_compare::Difference>, Vec<String>), String>, bool),
    SyncDone(Result<ndr::folder_compare::SyncResult, String>, bool),
    /// Warning box in C# RunSafely style (MessageBox(text, "NDR", warning)).
    Warning(String),
    MergeSources(PathBuf, Result<Vec<PathBuf>, String>),
    MergeDone(usize, Result<(), String>),
    /// Status bar text to show after the job finishes.
    ArchiveStatus(String),
    /// Drive button list (root, tooltip).
    Drives(Vec<(PathBuf, String)>),
    /// A drive was added or removed (GetLogicalDrives bits changed; original SHCNE_DRIVEADD/DRIVEREMOVED handler FUN_00442bc8).
    DrivesChanged,
    /// Remote command result (site number, command, "host:port", output or error).
    RemoteCommand { site: String, command: String, host: String, result: Result<String, String> },
    /// Automatic folder size display result (panel, listing generation, folder, text).
    FolderSizeRow { pane: usize, generation: u64, path: PathBuf, text: String, bytes: u64 },
    /// Received an update feed.
    /// Update feed result. bool = the silent check at startup (original [+0x54]; for a new version, only the menu bar button).
    Update(Result<ndr::update::HttpResponse, ndr::update::HttpError>, bool),
    /// Info bar disk data (panel, listing generation, (drive letter, free, total)).
    DiskCapacity { pane: usize, generation: u64, disk: Option<(String, u64, u64)> },
    /// Original TfChecksum computation finished (dialog number, result text; None on failure).
    ChecksumDone { serial: u64, value: Option<String> },
    /// File count and size counted separately before emptying the temp folder (original miEmptyTempClick's count, run separately so it does not block the screen).
    TempCounted { folder: PathBuf, files: u64, bytes: u64 },
    /// Read a shell overlay icon (C# ProcessShellOverlayIcons). None means use the extension icon.
    OverlayIcon { path: PathBuf, image: Option<(usize, usize, Vec<u8>)> },
    /// Original FUN_004408f8: panel FTP connection succeeded / failed.
    FtpConnected { pane: usize, remote: ftp_pane::PaneRemote },
    FtpFailed { error: String },
    /// An FTP panel new-folder/rename finished (panel, name to place the cursor on, error).
    FtpEdited { pane: usize, focus: Option<String>, error: Option<String> },
    FtpReply { result: Result<String, String> },
    /// An external copy program finished (result, status text afterward, clear the clipboard if this was a paste move).
    ExternalCopy { result: Result<(), String>, status: Option<String>, clear_clipboard: bool },
    /// Archive entries were temporarily extracted (panel, archive file, password used, extracted paths, failures "name: error", whether to navigate into it if it is an archive).
    ArchiveEntriesOpened { pane: usize, archive: PathBuf, password: Option<String>, opened: Vec<PathBuf>, failures: Vec<String>, navigate_archives: bool },
    /// An archive extraction running without a job window asks for a password (reply via reply; None if cancelled).
    ArchivePassword { archive: PathBuf, reply: Sender<Option<String>> },
    /// File to view/edit as text (archive entries are temporarily extracted paths).
    TextSource { path: Result<PathBuf, String>, edit: bool },
}
struct TextDocument {
    title: String,
    text: String,
}

pub struct App {
    options: Options,
    settings: Settings,
    profile: PathBuf,
    language: String,
    language_pack: compat::Language,
    resources: PathBuf,
    panes: [Pane; 2],
    active: usize,
    dual: bool,
    /// Original TfMessage (FUN_0040e138 -> FUN_0044a1d4): a brief black notification over the panel center (text, panel, shown time). Closes after 3 seconds (Timer1 3000) or on click.
    pane_message: Option<(String, usize, Instant)>,
    /// Original pBottom ("Results", miViewFindResult): the FTP transfer list (lvFTPProgress) and pause/cancel.
    show_results: bool,
    /// The ListFont name and bold/italic currently in the "list" font family (fonts are rebuilt when changed).
    list_font_name: Option<String>,
    /// Original ListFont style (TFontStyles byte: 1 bold, 2 italic, 4 underline, 8 strikethrough). Underline/strikethrough are applied directly to the list text.
    list_font_style: u8,
    /// C# Closing's WindowLeft/Top/Width/Height: the outer position and outer size (points) of the last normal (not maximized/minimized) window seen.
    window_bounds: Option<(egui::Pos2, egui::Vec2)>,
    /// C# WindowMaximized: the last maximized state seen (not changed while minimized).
    window_maximized: Option<bool>,
    /// ListFontSize (pixels, initial 14): used only for the file list text. The original listfont size is in points, so it is converted when imported.
    list_font_size: f32,
    /// Original btnFTPShowDetail (">>"): if the pBottom height exceeds 30 it becomes 22 (button row only), otherwise 100.
    results_collapsed: bool,
    /// Bottom-left of the toolbar favorites button from the last paint (the original miFavoritesClick opens that button's DropDown). None if there is no button.
    favorites_button: Option<egui::Pos2>,
    /// Panel folder change watch (original TNXShellChangeNotify). Disabled in tests (so files created by tests do not trigger a delayed re-read).
    watch_folders: bool,
    /// The egui context the watcher thread calls to repaint (used when refresh reopens the watch).
    egui_ctx: egui::Context,
    /// Flag to stop the drive-set watcher thread (native::spawn_drive_watch).
    drive_watch_stop: Arc<AtomicBool>,
    /// When a transfer started (for lFTPTransferred's KB/s).
    ftp_started: Option<std::time::Instant>,
    ftp_results: Arc<Mutex<Vec<ftp_pane::FtpResultRow>>>,
    ftp_pause: Arc<AtomicBool>,
    dark: bool,
    listing_tx: Sender<Listing>,
    listing_rx: Receiver<Listing>,
    async_tx: Sender<AsyncResult>,
    async_rx: Receiver<AsyncResult>,
    job: Option<Job>,
    password_request: Option<PasswordRequest>,
    conflict_all: Option<ConflictChoice>,
    text: Option<TextDocument>,
    status: String,
    last_find: String,
    started: Instant,
    frames: u64,
    clipboard: Option<(Vec<PathBuf>, bool)>,
    last_selection: [Vec<PathBuf>; 2],
    saved: bool,
    tool_cancel: Arc<AtomicBool>,
    lifecycle: Option<ndr::lifecycle::NativeLifecycle>,
    lifecycle_events: Option<Receiver<ndr::lifecycle::LifecycleEvent>>,
    navigation: Option<Receiver<ndr::lifecycle::NavigationRequest>>,
    /// C# _instanceMutex/_instancePipeTask: when running single-instance (UseSingleInstance), the per-user/profile mutex held and the navigation request receiver.
    instance_guard: Option<ndr::lifecycle::InstanceGuard>,
    /// Toolbar images (theme|file name -> image; None if unreadable).
    toolbar_icons: HashMap<String, Option<egui::TextureHandle>>,
    profile_load_error: bool,
    preferences_dirty: bool,
    close_when_idle: bool,
    /// While the STILL_HAVE_OPERATION question of the original FormCloseQuery (0x42a798) is showing.
    close_asking: bool,
    toolbar: ndr::commands::ToolbarConfig,
    /// Text of the toolbar quick search box (C# _toolbarQuickSearchBox). Enter turns it into a quick filter.
    toolbar_query: String,
    /// Quick search typed directly in the file list (C# _quickSearchByPane), one per panel.
    typed_search: [TypedSearch; 2],
    /// Original SetItemIndex (0x7dbdec) -> ToolTipTimer (600ms) -> ShowToolTip: this frame's cursor row name cell (panel, row, cell, displayed name text, full name text).
    name_tip: panes::NameTip,
    /// Original ShowToolTip style 3 (thumbnail): if the cursor cell's name was truncated this frame, that cell and the full name lines.
    thumb_tip: panes::ThumbTip,
    /// Where a name tooltip was drawn this frame (per panel).
    name_tip_shown: [Option<egui::Rect>; 2],
    /// Skin theme status text overlay: the current text and when it was first shown (recounted when changed).
    status_seen: Option<(String, Instant)>,
    /// When the cursor arrived on that row, per panel (start of the original tooltip timer).
    tip_since: [Option<(usize, Instant)>; 2],
    images: HashMap<PathBuf, egui::TextureHandle>,
    /// C# FileVisualConverter.IconCache: a small shell icon per extension (lowercase, folder = "<DIR>"). None is remembered if unreadable.
    file_icons: HashMap<String, Option<egui::TextureHandle>>,
    /// The dimmed icon of the original TNXShellImageList.Draw (0x7b8ebc) (ILD_BLEND50, blend color white): one per icon image.
    ghost_icons: HashMap<egui::TextureId, egui::TextureHandle>,
    /// The original skin (conf.ini) of the current theme - (theme name, skin). None if not Black/Classic/Gray.
    skin_cache: Option<(String, Option<ndr::styles::Skin>)>,
    /// Subfolder list opened from the skin address bar: (panel, folder, whether it is a drive list, position).
    skin_crumb_menu: Option<(usize, PathBuf, bool, egui::Pos2)>,
    /// The address bar menu was opened by a press that is not released yet (original MouseDown 0x7c9d94 opens DropDownMenu on press):
    /// its release neither closes the menu nor counts as a click on the address bar.
    skin_crumb_press: bool,
    /// Whether each extension (lowercase, no dot) has a per-file shell icon (system.per_file_icon, asked only once).
    per_file_icon_types: HashMap<String, bool>,
    /// C# ShellOverlayIconCache (per path, cleared when it exceeds 512) plus the pending paths and where to send to the overlay worker thread (STA).
    overlay_icons: HashMap<PathBuf, Option<egui::TextureHandle>>,
    overlay_pending: HashSet<PathBuf>,
    overlay_tx: Option<Sender<(PathBuf, u8)>>,
    image_requests: HashSet<PathBuf>,
    /// Panel folder tree subfolders: (shell key, hidden folders included) -> children. Because the original miShowHiddenClick (0x4348e0) -> FUN_0048e564 changes
    /// the tree's hidden view (+0x3f8 bit 4) and rebuilds it, this is kept separately per panel hidden view.
    tree_cache: HashMap<(PathBuf, bool), Vec<ndr::shell_tree::ShellNode>>,
    /// Desktop root of the panel folder tree (read on first paint).
    shell_desktop: Option<ndr::shell_tree::ShellNode>,
    /// The panel folder tree that has keyboard focus (that panel number). None when the list or another panel is clicked.
    tree_focus: Option<usize>,
    tree_requests: HashSet<(PathBuf, bool)>,
    msgboxes: Vec<msgbox::MsgBox>,
    msgbox_serial: u64,
    msgbox_icons: HashMap<u16, egui::TextureHandle>,
    original_dialogs: Vec<original_dialogs::OriginalDialog>,
    /// Subfolders of the address bar's chevron menu (path, whether it is a drive list) -> (path, text). Cleared when a new listing arrives.
    crumb_cache: HashMap<(PathBuf, bool), Vec<(PathBuf, String)>>,
    /// The panel item being dragged, the folder row under it (panel, folder), and this frame's two panel areas.
    file_drag: Option<drag_drop::FileDrag>,
    /// Items dropped from a panel drag that have not been moved yet (source panel, dragged items). When moving via the confirm box/drop menu, clears the marks in the source panel.
    pending_drag_marks: Option<(usize, Vec<PathBuf>)>,
    /// Folder rows drawn this frame (panel, row area, folder). Used to find a drop target. Cleared each frame.
    folder_rows: Vec<(usize, egui::Rect, PathBuf)>,
    /// Panel folder tree rows drawn this frame (panel, row area, folder, shell key, expandable). The original tree drop target (TNXDropFileTarget +0x36c).
    tree_drop_rows: Vec<(usize, egui::Rect, PathBuf, PathBuf, bool)>,
    /// Tree node hovered while dragging (panel, key, when hovering started). Original FUN_0048eb0c / timer +0x2f4 (1200ms) -> FUN_0048edf0 expand.
    tree_drop_hover: Option<(usize, PathBuf, Instant)>,
    /// Edge scroll timer while dragging (panel, next tick time). Original TNXDropTarget +0x74 (100ms).
    drag_scroll_tick: Option<(usize, Instant)>,
    pane_rects: [egui::Rect; 2],
    /// Drag out of the window (shell drag). Disabled in tests (does not run the real drag loop).
    allow_drag_out: bool,
    /// This window's HWND (0 if none). Used in the startup check to measure whether the launched instance became the foreground window.
    hwnd: isize,
    /// Whether it ever became the foreground window (GetForegroundWindow) during the startup check. Report's foreground.
    smoke_foreground: bool,
    /// Repaint causes counted in the startup check (--smoke-seconds) (file:line -> paint count).
    smoke_repaint_causes: std::collections::BTreeMap<String, u32>,
    /// Slowest frame in the startup check (ms).
    smoke_slowest_frame_ms: f64,
    /// Number of frames in the startup check that took longer than 50ms.
    smoke_slow_frames: u32,
    /// When both panel listings were first fully read in the startup check (ms from start).
    smoke_loaded_ms: Option<u64>,
    /// Diagnostics ("tipwindow" sample): screen position of a separately shown tooltip window (left, top, width, height). Checks whether the window was actually created.
    smoke_tip_window: Option<[f32; 4]>,
    /// The opacity last applied to the window (0.0~1.0). Reapplied when WindowAlpha changes.
    applied_alpha: f32,
    /// Whether the folder size auto-display work queue and workers (two) have been started.
    folder_size_queue: pane_chrome::FolderSizeQueue,
    folder_size_workers: bool,
    /// Fetch the update feed (C# UpdateHttpClient). Swapped out in tests.
    http_get: fn(&str, usize) -> Result<ndr::update::HttpResponse, ndr::update::HttpError>,
    /// Name to place the cursor on when the refreshed listing arrives (C# FocusPaneItemByName).
    pending_focus: [Option<String>; 2],
    /// (panel, created archive file) to place the cursor on when an archive job finishes.
    archive_focus: Option<(usize, PathBuf)>,
    /// ShellExecuteExW of open-with-arguments (replaceable so tests do not launch a real program).
    shell_runner: fn(&Path, &str, &Path) -> Result<(), String>,
    /// Run a program directly and wait for the exit code (external copy program). Swapped out in tests.
    program_runner: fn(&Path, &[String], &Path) -> Result<i32, String>,
    /// Save dialog (title, initial folder, name, format, default extension). Swapped out in tests.
    save_dialog: ndr::file_dialogs::SaveFileFn,
    /// F12 shortcut window (original fShortcut), only one.
    shortcut_window: Option<shortcut_window::ShortcutWindow>,
    /// Clipboard text writer (replaceable so tests never touch the user's real clipboard; C# `_setClipboardText`).
    clipboard_writer: fn(&str) -> Result<(), String>,
    /// Original miEmptyRecycleBinClick: SHEmptyRecycleBinW(window, NULL, 0) (shell confirmation dialog). Tests use a fake.
    shell_recycle_emptier: fn() -> Result<(), String>,
    /// CF_HDROP clipboard write for copy/cut (native::clipboard_files). Tests do not touch the real clipboard.
    file_clipboard: fn(&[PathBuf], bool) -> Result<(), String>,
    /// Network share auth requests from listings received inside a pending refresh (handled by the next poll).
    auth_pending: Vec<usize>,
    /// Numpad keys received by the original shortcut table (original name, Shift). raw_input_hook pushes them and keyboard executes them.
    pending_numpad: Vec<(String, bool)>,
    /// The Apps key was pressed (right-click menu of the current panel's list, the key path of the original FUN_00484e8c).
    pending_context_key: bool,
    /// The folder last set via SetCurrentDirectoryW (`sync_current_directory`).
    current_directory: Option<PathBuf>,
    /// Folder select dialog (title, initial folder). Swapped out in tests.
    folder_picker: fn(&str, Option<&Path>) -> Option<PathBuf>,
    /// Original miNetwork network folder picker (SHBrowseForFolderW). Swapped out in tests.
    network_picker: fn(&str, Option<&Path>) -> Option<PathBuf>,
    /// Original TColorDialog (ChooseColorW). Takes and returns a COLORREF. Tests swap in a fake.
    color_picker: fn(u32) -> Option<u32>,
    /// Original TFontDialog (ChooseFontW): (name, points, style) -> chosen (name, points, style). Tests swap in a fake.
    font_picker: fn(&str, f32, u8) -> Option<(String, f32, u8)>,
    /// Favorites / navigation history dropdown (C# ContextMenu), only one.
    popup: Option<favorites_dialog::PopupMenu>,
    /// Unhandled Alt+letter (original TToolBar CM_DIALOGCHAR): this frame, open the menu button dropdown for that access letter.
    menu_access: Option<char>,
    /// Letters typed while a menu (menu bar dropdown / right-click menu) is open: select the item for that access letter (&X) (original Windows menu).
    menu_letter: Option<char>,
    /// Menu items that could receive a letter this frame (access letter, widget id) and the previous frame's list. If the same letter appears more than once,
    /// like a Windows menu, typing the letter repeatedly only moves the selection (focus) among those items (Enter to execute).
    menu_targets: std::cell::RefCell<Vec<(char, egui::Id)>>,
    menu_targets_seen: Vec<(char, egui::Id)>,
    /// Menu item to focus on the next paint.
    menu_focus: std::cell::Cell<Option<egui::Id>>,
    /// Download URL of the new version found by the silent update check (while the original tbNewUpdateAvailable is shown).
    update_available: Option<String>,
    /// C# _lastCommandWorkingDirectory (working directory of command execution).
    last_command_directory: Option<PathBuf>,
    /// Network drives, Explorer menu, and shell verbs (swapped out in tests).
    system: tools_dialog::SystemHooks,
    /// Open dialog (title, initial folder, name, format). Swapped out in tests.
    open_dialog: ndr::file_dialogs::OpenFileFn,
    /// Device list / read / write / copy of the disk copy window (tests swap in so real devices are not touched).
    disk: disk_copy_dialog::DiskHooks,
    /// Server connection and multi-file open dialog of the remote window (swapped out in tests).
    remote_hooks: remote_window::RemoteHooks,
    /// Drive buttons (read time, (root, tooltip)).
    drive_cache: Option<(Instant, Vec<(PathBuf, String)>)>,
    /// Drive list cache of the panel list (original TNXDriveList slot, shared with the listing thread).
    drive_list: Arc<Mutex<panes::DriveListCache>>,
    drive_cache_pending: bool,
}
impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        options: Options,
        navigation: Option<Receiver<ndr::lifecycle::NavigationRequest>>,
        instance_guard: Option<ndr::lifecycle::InstanceGuard>,
    ) -> Self {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let mut app = Self::from_context(&cc.egui_ctx, options);
        app.navigation = navigation;
        app.instance_guard = instance_guard;
        if let Ok(handle) = cc.window_handle() {
            if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                app.hwnd = handle.hwnd.get();
                if let Err(error) = ndr::shell_menu::install_paste_key_hook(handle.hwnd.get()) {
                    app.status = error;
                }
                let key = app.settings.text("ExplorerShortcutKey", "Q");
                let tray = app.settings.flag("HideWhenMinimized", false)
                    && app.options.smoke_seconds.is_none();
                match ndr::lifecycle::NativeLifecycle::attach(
                    handle.hwnd.get(),
                    if app.options.smoke_seconds.is_some() {
                        None
                    } else {
                        Some(&key)
                    },
                    tray,
                ) {
                    Ok((lifecycle, events)) => {
                        if let Some(error) = lifecycle.hotkey_error() {
                            app.status = error;
                        }
                        // When a lifecycle event like a device change (WM_DEVICECHANGE) arrives, wake an idle UI too.
                        let repaint = cc.egui_ctx.clone();
                        let _ = lifecycle.set_wake(Box::new(move || repaint.request_repaint()));
                        app.lifecycle = Some(lifecycle);
                        app.lifecycle_events = Some(events);
                    }
                    Err(error) => app.status = error,
                }
            }
        }
        // C# startup: if CheckForUpdatesOnStartup and a feed exists, check for updates (except in the startup check run).
        // The silent check of the original FUN_00441dd8 ([+0x54] == 1): no box; for a new version, only the "New update" button on the menu bar.
        // Original initial value [Operation] checkupdate=1. If there is no feed (UpdateManifestUrl), no check is done.
        if app.options.smoke_seconds.is_none() && app.settings.flag("CheckForUpdatesOnStartup", true) && !app.settings.text("UpdateManifestUrl", "").trim().is_empty() {
            app.start_update_check(true);
        }
        app
    }
    fn from_context(ctx: &egui::Context, options: Options) -> Self {
        // The double-click interval follows the Windows setting (GetDoubleClickTime, default 500ms) (egui's default 0.3s is shorter).
        let (double_ms, _, _) = native::double_click_metrics();
        ctx.options_mut(|o| o.input_options.max_double_click_delay = double_ms as f64 / 1000.0);
        let profile = options.profile_directory();
        let loaded = Settings::load(&profile.join("settings.json"));
        let error = loaded.as_ref().err().cloned();
        let settings = loaded.unwrap_or_default();
        let language = options
            .language
            .clone()
            .unwrap_or_else(|| settings.text("Language", "ko"));
        let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_else(|| "C:\\".into()));
        // C# start folder: command line -> "Always start in the following folder" (if StartPathEnabled and StartPath exist) -> last folder (LeftPath).
        // The second value is whether the folder was given explicitly like a command line / start folder (in which case that folder takes priority over the saved selected tab).
        let start_path = |side: &str, explicit: &Option<PathBuf>| -> (PathBuf, bool) {
            if let Some(path) = explicit {
                return (path.clone(), true);
            }
            let start = settings.text(&format!("{side}StartPath"), "");
            if settings.flag(&format!("{side}StartPathEnabled"), false) && !start.trim().is_empty() {
                return (PathBuf::from(start.trim()), true);
            }
            (PathBuf::from(settings.text(&format!("{side}Path"), &home.to_string_lossy())), false)
        };
        let (left, left_fixed) = start_path("Left", &options.left);
        // Original FUN_004295b0: /pane:single/dual takes precedence over the setting (dualview), and /dir2 opens the right side only in dual-pane view.
        let dual = options.dual.unwrap_or_else(|| settings.flag("DualPaneMode", true));
        let right_explicit = options.right.clone().or_else(|| options.dir2.clone().filter(|_| dual));
        let (right, right_fixed) = start_path("Right", &right_explicit);
        // C# ValidStartPath: an existing folder as-is, otherwise the user folder, otherwise "::drives::" (My PC).
        let valid_start = |candidate: PathBuf| {
            if panes::is_drives(&candidate) || candidate.is_dir() {
                candidate
            } else if home.is_dir() {
                home.clone()
            } else {
                PathBuf::from(remote_window::DRIVES)
            }
        };
        let (left, right) = (valid_start(left), valid_start(right));
        let mut panes_init = Vec::new();
        for (start, fixed, side) in [(left, left_fixed, "Left"), (right, right_fixed, "Right")] {
            let selected = settings.values.get(&format!("Selected{side}Tab")).and_then(|v| v.as_u64()).map(|v| v as usize);
            let (tabs, tab) = restore_tabs(&settings.strings(&format!("{side}Tabs")), selected, &start, fixed, |p| panes::is_drives(p) || p.is_dir());
            let mut pane = Pane::new(tabs[tab].clone(), settings.flag(&format!("{side}ShowHiddenFiles"), false));
            pane.tabs = tabs;
            pane.tab = tab;
            panes_init.push(pane);
        }
        let right = panes_init.pop().expect("right pane");
        let left = panes_init.pop().expect("left pane");
        let dark = system_commands::theme_palette(&settings.text("Theme", "Black")).dark();
        ctx.set_visuals(if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        set_app_fonts(&ctx, None, false);
        let (listing_tx, listing_rx) = mpsc::channel();
        let (async_tx, async_rx) = mpsc::channel();
        let active = usize::from(settings.flag("StartWithRightPaneActive", false));
        let resources = resource_dir();
        let language_pack = compat::Language::load(&resources.join("Languages"), &language);
        let toolbar =
            ndr::commands::load_toolbar(&profile, include_bytes!("../../Resources/toolbar.xml"))
                .unwrap_or(ndr::commands::ToolbarConfig {
                    items: vec![],
                    warning: None,
                });
        let mut app = Self {
            options,
            settings,
            profile,
            language,
            language_pack,
            resources,
            panes: [left, right],
            active,
            dual,
            pane_message: None,
            show_results: false,
            list_font_name: None,
            list_font_style: 0,
            window_bounds: None,
            window_maximized: None,
            list_font_size: ndr::settings::DEFAULT_LIST_FONT_PIXELS as f32,
            results_collapsed: false,
            favorites_button: None,
            watch_folders: !cfg!(test),
            egui_ctx: ctx.clone(),
            drive_watch_stop: Arc::new(AtomicBool::new(false)),
            ftp_started: None,
            ftp_results: Arc::new(Mutex::new(Vec::new())),
            ftp_pause: Arc::new(AtomicBool::new(false)),
            dark,
            listing_tx,
            listing_rx,
            async_tx,
            async_rx,
            job: None,
            password_request: None,
            conflict_all: None,
            text: None,
            status: error.clone().unwrap_or_default(),
            last_find: String::new(),
            started: Instant::now(),
            frames: 0,
            clipboard: None,
            last_selection: [vec![], vec![]],
            saved: false,
            tool_cancel: Arc::new(AtomicBool::new(false)),
            lifecycle: None,
            lifecycle_events: None,
            navigation: None,
            instance_guard: None,
            toolbar_icons: HashMap::new(),
            profile_load_error: error.is_some(),
            preferences_dirty: true,
            close_when_idle: false,
            close_asking: false,
            toolbar,
            toolbar_query: String::new(),
            typed_search: Default::default(),
            name_tip: Default::default(),
            thumb_tip: Default::default(),
            name_tip_shown: [None, None],
            status_seen: None,
            tip_since: [None, None],
            images: HashMap::new(),
            file_icons: HashMap::new(),
            ghost_icons: HashMap::new(),
            skin_cache: None,
            skin_crumb_menu: None,
            skin_crumb_press: false,
            per_file_icon_types: HashMap::new(),
            overlay_icons: HashMap::new(),
            overlay_pending: HashSet::new(),
            overlay_tx: None,
            image_requests: HashSet::new(),
            tree_cache: HashMap::new(),
            shell_desktop: None,
            tree_focus: None,
            tree_requests: HashSet::new(),
            msgboxes: Vec::new(),
            msgbox_serial: 0,
            msgbox_icons: HashMap::new(),
            original_dialogs: Vec::new(),
            crumb_cache: HashMap::new(),
            folder_size_queue: Arc::new((Mutex::new(std::collections::VecDeque::new()), std::sync::Condvar::new())),
            folder_size_workers: false,
            file_drag: None,
            pending_drag_marks: None,
            folder_rows: Vec::new(),
            tree_drop_rows: Vec::new(),
            tree_drop_hover: None,
            drag_scroll_tick: None,
            pane_rects: [egui::Rect::NOTHING; 2],
            allow_drag_out: !cfg!(test),
            hwnd: 0,
            smoke_foreground: false,
            smoke_repaint_causes: Default::default(),
            smoke_slowest_frame_ms: 0.0,
            smoke_slow_frames: 0,
            smoke_loaded_ms: None,
            smoke_tip_window: None,
            applied_alpha: -1.0,
            http_get: native::https_get,
            pending_focus: [None, None],
            archive_focus: None,
            shell_runner: native::shell_execute,
            program_runner: file_operation::run_program,
            save_dialog: ndr::file_dialogs::save_file,
            shortcut_window: None,
            clipboard_writer: native::clipboard_text,
            shell_recycle_emptier: if cfg!(test) { || Err("not in tests".into()) } else { native::empty_recycle_bin_with_shell_ui },
            file_clipboard: if cfg!(test) { |_, _| Ok(()) } else { native::clipboard_files },
            auth_pending: Vec::new(),
            pending_numpad: Vec::new(),
            pending_context_key: false,
            current_directory: None,
            folder_picker: ndr::file_dialogs::browse_folder,
            network_picker: if cfg!(test) { |_, _| None } else { ndr::file_dialogs::browse_network },
            color_picker: if cfg!(test) { |_| None } else { native::choose_color },
            font_picker: if cfg!(test) { |_, _, _| None } else { native::choose_font },
            popup: None,
            menu_access: None,
            menu_letter: None,
            menu_targets: Default::default(),
            menu_targets_seen: Vec::new(),
            menu_focus: Default::default(),
            update_available: None,
            last_command_directory: None,
            system: tools_dialog::SystemHooks::default(),
            open_dialog: ndr::file_dialogs::open_file,
            disk: disk_copy_dialog::DiskHooks::default(),
            remote_hooks: remote_window::RemoteHooks::default(),
            drive_cache: None,
            drive_list: Default::default(),
            drive_cache_pending: false,
        };
        app.refresh(0);
        app.refresh(1);
        if !cfg!(test) {
            let (tx, repaint) = (app.async_tx.clone(), ctx.clone());
            native::spawn_drive_watch(app.drive_watch_stop.clone(), move || {
                let _ = tx.send(AsyncResult::DrivesChanged);
                repaint.request_repaint();
            });
        }
        app
    }
    fn tr<'a>(&self, ko: &'a str, en: &'a str) -> std::borrow::Cow<'a, str> {
        if self.language == "ko" {
            std::borrow::Cow::Borrowed(ko)
        } else if self.language == "en" {
            std::borrow::Cow::Borrowed(en)
        } else {
            std::borrow::Cow::Owned(self.language_pack.translate_pair(ko, en))
        }
    }
    /// The original/C# "Use transparent menus" (UseMenuAlpha, default on): when on, dims this frame's menu layers to MENU_ALPHA.
    fn fade_menus(&self, ctx: &egui::Context) {
        fade_foreground_layers(ctx, if self.settings.flag("UseMenuAlpha", true) { MENU_ALPHA } else { 1.0 });
    }
    /// Track window state (kept separate so it is testable). Changes nothing while minimized; updates position/size only for a normal window.
    fn track_window(&mut self, outer: Option<egui::Rect>, inner: Option<egui::Rect>, maximized: Option<bool>, minimized: Option<bool>) {
        if minimized == Some(true) {
            return;
        }
        if let Some(maximized) = maximized {
            self.window_maximized = Some(maximized);
        }
        if maximized == Some(false)
            && let (Some(outer), Some(_)) = (outer, inner)
        {
            // Original TForm.Width/Height, C# Window.Width/Height: the outer size including the border.
            self.window_bounds = Some((outer.min, outer.size()));
        }
    }
    fn save(&mut self) {
        let _ = self.try_save();
    }
    /// Same as save but returns the error text on failure (like a C# SettingsStore.Save exception, when the caller notifies separately).
    fn try_save(&mut self) -> Result<(), String> {
        if self.profile_load_error {
            self.status=self.tr("기존 설정을 읽지 못해 덮어쓰지 않았습니다. 설정 가져오기를 사용하세요.","Existing settings could not be read and were preserved. Use Import settings to recover.").into();
            return Err(self.status.clone());
        }
        // C# Closing: if WindowState is Normal, WindowLeft/Top/Width/Height, and always WindowMaximized and HasWindowBounds = true.
        if let Some((position, size)) = self.window_bounds {
            for (key, value) in [("WindowLeft", position.x), ("WindowTop", position.y), ("WindowWidth", size.x), ("WindowHeight", size.y)] {
                self.settings.values.insert(key.into(), serde_json::json!(value.round()));
            }
        }
        if let Some(maximized) = self.window_maximized {
            self.settings.set_flag("WindowMaximized", maximized);
            self.settings.set_flag("HasWindowBounds", true);
        }
        for (i, name) in ["Left", "Right"].into_iter().enumerate() {
            if self.panes[i].archive.is_none() {
                self.settings.set_text(
                    &format!("{name}Path"),
                    self.panes[i].path.to_string_lossy().into_owned(),
                );
                self.settings.set_strings(
                    &format!("{name}Tabs"),
                    self.panes[i]
                        .tabs
                        .iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect(),
                );
                // C# _settings.SelectedLeftTab = Math.Max(0, LeftTabs.SelectedIndex).
                self.settings.values.insert(format!("Selected{name}Tab"), serde_json::json!(self.panes[i].tab));
            }
            self.settings
                .set_flag(&format!("{name}ShowHiddenFiles"), self.panes[i].show_hidden);
        }
        self.settings.set_text("Language", &self.language);
        // Theme stays the raw setting value (Dark/Light/System/Black/Classic/Gray). self.dark is only its result.
        self.settings.set_flag("DualPaneMode", self.dual);
        let any_tree = self.settings.flag("ShowLeftFolderTree", false) || self.settings.flag("ShowRightFolderTree", false);
        self.settings.set_flag("ShowFolderTree", any_tree);
        match self.settings.save(&self.profile.join("settings.json")) {
            Ok(()) => {
                self.saved = true;
                Ok(())
            }
            Err(e) => {
                self.status = e.clone();
                Err(e)
            }
        }
    }
    /// Original FormCloseQuery (0x42a798): if an operation is in progress, MessageBoxW(STILL_HAVE_OPERATION, TITLE_INFO,
    /// MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_TASKMODAL). Yes stops the operation and closes once finished. No does not close.
    pub(super) fn confirm_close_while_working(&mut self) {
        if self.close_when_idle || self.close_asking {
            return;
        }
        self.close_asking = true;
        let title = self.language_pack.text("RES.TITLE_INFO", "nexusfile Information");
        let text = self.language_pack.text("RES.STILL_HAVE_OPERATION", "There is an working operation.\nDo you really want to quit this operation and close the program?");
        self.system_box(&title, &text, 0x2124, |app, answer, _| {
            app.close_asking = false;
            if answer == ndr::msgbox::IDYES {
                app.close_when_idle = true;
                if let Some(job) = &app.job {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            }
        });
    }
    fn start_job(&mut self, title: String, operation: Operation, report: file_operation::JobReport, progress: Option<file_operation::ProgressInfo>) {
        if self.job.as_ref().is_some_and(|j| !j.finished) {
            self.status = self
                .tr(
                    "진행 중인 작업을 먼저 마치세요.",
                    "Finish the current operation first.",
                )
                .into();
            return;
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let mut job = Job::new(rx, cancel, title);
        // C# ErrorResolver: copy/move/delete ask on each error (wipe asks all at once at the end).
        let options = fsops::OperationOptions {
            remove_read_only: self.settings.flag("RemoveReadOnlyWhenCopying", true),
            use_descriptions: self.settings.flag("UseDescriptionFile", true),

            interactive_errors: matches!(operation, Operation::Copy { .. } | Operation::Delete { .. } | Operation::DeleteSkipping { .. }),
            prompt_rename: matches!(operation, Operation::Copy { .. }),
        };
        thread::spawn(move || fsops::run_with_options(operation, tx, flag, options));
        self.conflict_all = None;
        job.report = report;
        job.progress = progress;
        self.job = Some(job);
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let mut auth_panes = std::mem::take(&mut self.auth_pending);
        while let Ok(listing) = self.listing_rx.try_recv() {
            self.apply_listing(listing, &mut auth_panes);
        }
        self.after_listings(auth_panes, ctx);
        self.update_folder_watches(ctx);
    }

    /// Original FUN_00486694 (location changed): for a local folder, TNXShellChangeNotify.SetFolder(that folder)/SetActive(1); for archive/FTP/My PC, SetActive(0).
    /// After a change notification, once Delay (1500ms) passes, OnChange = FUN_00487c8c -> FUN_00487ce8(panel, 1, 1, "") (re-read the same folder).
    fn update_folder_watches(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        for index in 0..self.panes.len() {
            let wanted = self.watch_wanted(index);
            if self.panes[index].watch.as_ref().map(|(p, _)| p) != wanted.as_ref() {
                self.arm_folder_watch(index, wanted, ctx);
            }
            if let Some((due, _)) = self.panes[index].change_due {
                if due <= now {
                    if self.panes[index].loading {
                        self.panes[index].change_due = Some((now + FOLDER_CHANGE_DELAY, 0));
                    } else {
                        self.panes[index].change_due = None;
                        // When re-reading due to a change notification, preserve the typed quick-search text (the original FUN_00487ce8 does not touch the search text).
                        let typed = std::mem::take(&mut self.typed_search[index]);
                        self.refresh(index);
                        self.typed_search[index] = typed;
                    }
                } else {
                    ctx.request_repaint_after(due - now);
                }
            }
        }
    }

    /// Folder to watch: if watching is on and it is a local folder (not archive/FTP/My PC), that folder.
    fn watch_wanted(&self, index: usize) -> Option<PathBuf> {
        let pane = &self.panes[index];
        (self.watch_folders && pane.archive.is_none() && pane.remote.is_none() && !panes::is_drives(&pane.path)).then(|| pane.path.clone())
    }

    /// Stop the current watch (original FreeThread) and, if `wanted` exists, open a new watcher thread (original ResetThread -> FindFirstChangeNotificationW).
    /// A new watch only reports changes after it is opened. Pending changes (change_due) are also discarded (original Clear).
    pub(super) fn arm_folder_watch(&mut self, index: usize, wanted: Option<PathBuf>, ctx: &egui::Context) {
        let pane = &mut self.panes[index];
        if let Some((_, stop)) = pane.watch.take() {
            stop.store(true, Ordering::Relaxed);
        }
        pane.change_due = None;
        if let Some(path) = wanted {
            let stop = Arc::new(AtomicBool::new(false));
            let (tx, watched, repaint, token) = (self.async_tx.clone(), path.clone(), ctx.clone(), stop.clone());
            native::spawn_folder_watch(path.clone(), stop.clone(), move || {
                let _ = tx.send(AsyncResult::FolderChanged { pane: index, path: watched.clone(), watch: token.clone() });
                repaint.request_repaint();
            });
            pane.watch = Some((path, stop));
        }
    }

    /// Original ThreadNotification (0x7b3830): on the first notification, start the Delay (1500ms) timer; if more arrive while the timer runs, defer again up to 20 times.
    /// Notifications from a stopped old watch (the one before the new watch opened on re-read) are discarded.
    pub(super) fn folder_changed(&mut self, index: usize, path: &Path, watch: &Arc<AtomicBool>) {
        let Some(pane) = self.panes.get_mut(index) else { return };
        if pane.watch.as_ref().is_none_or(|(watched, current)| watched != path || !Arc::ptr_eq(current, watch)) {
            return;
        }
        let now = Instant::now();
        pane.change_due = match pane.change_due {
            None => Some((now + FOLDER_CHANGE_DELAY, 0)),
            Some((_, restarts)) if restarts < 20 => Some((now + FOLDER_CHANGE_DELAY, restarts + 1)),
            kept => kept,
        };
    }
    /// Puts one result of the listing thread into the pane (shared by poll and the waiting refresh, like the original).
    fn apply_listing(&mut self, listing: Listing, auth_panes: &mut Vec<usize>) {
        let skin = true;
        {
            let pane = &mut self.panes[listing.pane];
            if pane.generation != listing.generation {
                return;
            }
            pane.loading = false;
            self.crumb_cache.clear();
            let pane = &mut self.panes[listing.pane];
            // The original refresh (FUN_00487ce8, mostly called with (1, 1) after refreshes and operations) keeps, when rereading the same folder, the marks (CopySelection ->
            // RestoreSelection, by name), counted folder sizes (+0x20/+0x24/+0x60 of kind-1 items with +0x68 restored by name), and the top position
            // (SetScrollTop); when the cursor name is gone it uses the old index (the last item before the drives when beyond the list). Cut marks (+100)
            // disappear because the items are new. C# RefreshPaneAsync clears everything, so only the skin theme does this.
            let kept = (skin && listing.result.is_ok() && pane.listed.as_deref() == Some(location_key(pane).as_str())).then(|| {
                let marks: Vec<String> = pane.selected.iter().filter_map(|&row| pane.entries.get(row)).filter(|e| !e.drive && !e.is_parent()).map(|e| e.name.clone()).collect();
                let sizes: Vec<(PathBuf, String, Option<u64>)> = pane.folder_sizes.iter().filter_map(|(path, size)| size.clone().map(|s| (path.clone(), s, pane.folder_bytes.get(path).copied()))).collect();
                (marks, sizes, pane.first_column, pane.cursor, self.pending_focus[listing.pane].is_some())
            });
            pane.summary = None;
            pane.folder_cancel.store(true, Ordering::Relaxed);
            pane.folder_cancel = Arc::new(AtomicBool::new(false));
            pane.folder_sizes.clear();
            pane.folder_bytes.clear();
            pane.cut_marks.clear();
            pane.disk = None;
            match listing.result {
                Ok(mut entries) => {
                    // Original refresh (FUN_00487ce8): rereading the same folder keeps the cursor on the item with the same name (after copying to the other pane and so on).
                    let location = location_key(pane);
                    if pane.listed.as_deref() == Some(location.as_str()) && self.pending_focus[listing.pane].is_none() {
                        self.pending_focus[listing.pane] = pane.entries.get(pane.cursor).map(|e| e.name.clone());
                    }
                    pane.listed = Some(location);
                    if !pane.filter.is_empty() {
                        entries.retain(|e| e.drive || ndr::parity::matches_filter(&pane.filter, &e.name));
                    }
                    // Original GetList (0x45a34c), archive listing (0x45ecd0), SFTP listing (0x45bb0c), C# ListLocal: ".." first unless at a root.
                    if let Some(parent) = parent_item_for(pane) {
                        entries.retain(|e| !e.is_parent());
                        entries.insert(0, parent);
                    }
                    pane.entries = entries;
                    pane.first_column = 0;
                    pane.shift_anchor = None;
                    pane.descriptions = listing.descriptions;
                    pane.drive_infos = listing.drive_infos;
                    pane.cursor = 0;
                    pane.selected.clear();
                    pane.error = None;
                    pane.auth_prompted = false;
                }
                Err(e) => {
                    pane.error = Some(e);
                    if pane.archive.is_none() && pane.path.to_string_lossy().starts_with("\\\\") && !pane.auth_prompted {
                        pane.auth_prompted = true;
                        auth_panes.push(listing.pane);
                    }
                }
            }
            self.sort_pane(listing.pane);
            let pane = &mut self.panes[listing.pane];
            // A new list starts at the top after sorting (sort_pane follows the cursor item, so set it again).
            pane.cursor = 0;
            if self.settings.flag("ShowFolderDiskInfoBar", true) {
                self.update_disk_capacity(listing.pane);
            }
            let pane = &mut self.panes[listing.pane];
            let mut focused = false;
            // When rereading the same location with no chosen name (original FUN_00487ce8 param_2, empty param_4), the original does SetItemIndex then
            // SetScrollTop (old position): Details and Thumbnails keep the old position, only column-flowing lists do AdjustPage + SetItemIndex.
            let restore_top = kept.as_ref().is_some_and(|k| !k.4);
            if let Some(name) = self.pending_focus[listing.pane].take() {
                if let Some(row) = pane.entries.iter().position(|e| ndr::names::equals_ignore_case(&e.name, &name)) {
                    pane.cursor = row;
                    pane.selected.clear();
                    // Original FUN_00486694 -> SetItemIndex: scrolls the cursor item into view (that page when paging). Fixed: previously it stayed on the first page.
                    pane.reveal = !restore_top;
                    focused = true;
                }
            }
            // New location: the original always does SetItemIndex (0 or the chosen name), so that item is visible. Fixed: in Details the previous folder's vertical position
            // remained and the first item could be hidden. (The original first does AdjustPage with the previous folder's cursor index, but that side effect makes the screen
            // depend on the previous folder, so it was not ported.)
            if kept.is_none() {
                pane.reveal = true;
            }
            if let Some((marks, sizes, first_column, old_cursor, explicit)) = kept {
                if !focused && !explicit && !pane.entries.is_empty() {
                    let drives = pane.entries.iter().filter(|e| e.drive).count();
                    pane.cursor = if old_cursor < pane.entries.len() { old_cursor } else { (pane.entries.len() - drives).saturating_sub(1) };
                }
                for name in &marks {
                    // Original FileListChanged (0x7d79c8) -> FixSelection (0x7dcc78): every item is unmarked, the TNXSelectedFiles names are collected and
                    // cleared, and each name still found in the new list (FUN_007d2ee4) is marked and added again.
                    if let Some(row) = pane.entries.iter().position(|e| !e.drive && !e.is_parent() && ndr::names::equals_ignore_case(&e.name, name)) {
                        pane.selected.insert(row);
                    }
                }
                for (path, size, bytes) in sizes {
                    if pane.entries.iter().any(|e| e.is_dir && !e.is_parent() && e.path == path) {
                        if let Some(bytes) = bytes {
                            pane.folder_bytes.insert(path.clone(), bytes);
                        }
                        pane.folder_sizes.insert(path, Some(size));
                    }
                }
                pane.first_column = first_column;
                // Original FileListChanged (0x7d79c8) -> AdjustPage (old cursor index, clamped to the list length), then FUN_00487ce8 does SetItemIndex.
                // Details and Thumbnails then restore SetScrollTop (old position), so only column-flowing lists change position (second value of adjust).
                if !pane.entries.is_empty() {
                    pane.adjust = Some((old_cursor.min(pane.entries.len() - 1), true));
                }
            }
        }
    }
    /// Work after the list is put in (network share authentication etc.). Called by poll.
    fn after_listings(&mut self, auth_panes: Vec<usize>, ctx: &egui::Context) {
        // C# RefreshPaneAsync(allowNetworkAuthPrompt): when a UNC listing fails on credentials, the connect window -> one more read on success, a "network connection" warning on failure.
        for index in auth_panes {
            let path = self.panes[index].path.clone();
            let code = (self.system.network_path_error)(&path);
            if !matches!(code, 5 | 65 | 86 | 1326) {
                continue;
            }
            match (self.system.connect_share)(&path.to_string_lossy()) {
                0 => self.refresh(index),
                1223 => {}
                connection => {
                    let ko = self.language == "ko";
                    let message = native::win32_message(connection);
                    let text = if ko { format!("네트워크 공유 인증에 실패했습니다. Windows 오류 {connection}: {message}") } else { format!("Network share authentication failed. Windows error {connection}: {message}") };
                    self.system_box(if ko { "네트워크 연결" } else { "Network connection" }, &text, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
                }
            }
        }
        let mut finished = None;
        let mut close_modal = false;
        let events: Vec<Event> = self.job.as_ref().map(|job| job.receiver.try_iter().collect()).unwrap_or_default();
        for event in events {
            let Some(job) = self.job.as_mut() else { break };
            match event {
                Event::Progress { path, bytes, files } => {
                    job.path = path.to_string_lossy().into_owned();
                    job.bytes = bytes;
                    job.files = files;
                }
                Event::Conflict { source, destination, allow_resume, reply } => self.ask_file_conflict(source, destination, allow_resume, reply),
                Event::Target { source, destination } => job.target = Some((source, destination)),
                Event::Error { path, message } => job.errors.push(if path.as_os_str().is_empty() { message } else { format!("{}: {message}", path.display()) }),
                Event::Failure { path, message, can_continue, reply } => {
                    job.errors.push(format!("{}: {message}", path.display()));
                    job.interactive += 1;
                    self.ask_file_failure(path, message, can_continue, reply);
                }
                Event::Skipped { .. } => job.skipped += 1,
                Event::Totals { files, bytes } => {
                    if let Some(progress) = job.progress.as_mut() {
                        if files.is_some() {
                            progress.total_files = files;
                        }
                        if bytes.is_some() {
                            progress.total_bytes = bytes;
                        }
                    }
                }
                Event::RenameTarget { name, reply } => self.ask_rename_target(name, reply),
                Event::Fatal(message) => job.fatal = Some(message),
                Event::Password { archive, reply } => {
                    self.password_request = Some(PasswordRequest { archive, reply });
                }
                Event::ChangeVolume { volume, reply } => {
                    // Original 0x439593: wsprintf(ARC_CHANGEVOL, volume name) -> TfMsgBox(TITLE_ERROR, "", text, 0x31). OK (1) retries.
                    let title = self.language_pack.text("RES.TITLE_ERROR", "Error");
                    let text = self.language_pack.text("RES.ARC_CHANGEVOL", "%s\nPlease, insert a disk containing this file and click 'OK'").replacen("%s", &volume, 1);
                    self.message_box(&title, "", &text, ndr::msgbox::MB_OKCANCEL | ndr::msgbox::MB_ICONEXCLAMATION, move |_, answer, _| {
                        let _ = reply.send(answer == ndr::msgbox::IDOK);
                    });
                }
                Event::Finished { cancelled, files, bytes } => {
                    job.finished = true;
                    if job.modal.is_some() {
                        close_modal = true;
                        continue;
                    }
                    if files > 0 {
                        job.files = files;
                    }
                    if bytes > 0 {
                        job.bytes = bytes;
                    }
                    finished = Some(cancelled);
                }
            }
        }
        if close_modal {
            self.job = None;
        }
        if let Some(cancelled) = finished {
            self.report_job(cancelled);
        }
        self.poll_disk_copy(ctx);
        self.poll_remote_window(ctx);
        while let Ok(result) = self.async_rx.try_recv() {
            match result {
                AsyncResult::Text { title, text } => {
                    self.text = Some(TextDocument { title, text })
                }
                AsyncResult::Image { path, image } => {
                    if let Ok((width, height, rgba)) = image {
                        if self.images.len() >= 512 {
                            self.images.clear();
                            self.image_requests.clear();
                        }
                        let texture = ctx.load_texture(
                            path.to_string_lossy(),
                            egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba),
                            egui::TextureOptions::LINEAR,
                        );
                        self.images.insert(path, texture);
                    }
                }
                AsyncResult::Tree { path, hidden, directories } => {
                    self.tree_cache.insert((path, hidden), directories);
                }
                AsyncResult::FolderChanged { pane, path, watch } => self.folder_changed(pane, &path, &watch),
                AsyncResult::ProtectedItems(run) => self.protected_step(run),
                AsyncResult::AttributesApplied(run) => self.attribute_step(run),
                AsyncResult::SplitDone { source, outcome, cancelled } => self.split_done(source, outcome, cancelled),
                AsyncResult::JoinDone { count, outcome, cancelled } => self.join_done(count, outcome, cancelled),
                AsyncResult::SearchInPane { pane, root, outcome, cancelled } => self.search_done(pane, root, outcome, cancelled),
                AsyncResult::CompareDone(run, outcome, cancelled) => self.compare_done(run, outcome, cancelled),
                AsyncResult::SyncDone(outcome, cancelled) => self.sync_done(outcome, cancelled),
                AsyncResult::Warning(text) => self.system_box("NDR", &text, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {}),
                AsyncResult::MergeSources(output, sources) => self.merge_sources_ready(output, sources),
                AsyncResult::MergeDone(count, outcome) => self.merge_done(count, outcome),
                AsyncResult::ArchiveStatus(text) => self.status = self.archive_error_text(&text),
                AsyncResult::RemoteCommand { site, command, host, result } => self.remote_command_done(site, command, host, result),
                AsyncResult::ExternalCopy { result, status, clear_clipboard } => self.external_copy_done(result, status, clear_clipboard),
                AsyncResult::ArchiveEntriesOpened { pane, archive, password, opened, failures, navigate_archives } => self.archive_entries_opened(pane, archive, password, opened, failures, navigate_archives),
                AsyncResult::ArchivePassword { archive, reply } => self.password_request = Some(PasswordRequest { archive, reply }),
                AsyncResult::TextSource { path, edit } => self.text_source_ready(path, edit),
                AsyncResult::FolderSizeRow { pane, generation, path, text, bytes } => {
                    // Original FUN_0048ee0c: finds the row whose folder + name equals the counted path (lstrcmpiW), stores the 64-bit size
                    // (+0x20/+0x24) and the count (+0x60), marks it counted (+0x68 = 1) and repaints. Rows are keyed by path here.
                    if self.panes[pane].generation == generation {
                        self.panes[pane].folder_bytes.insert(path.clone(), bytes);
                        self.panes[pane].folder_sizes.insert(path, Some(text));
                    }
                }
                AsyncResult::Update(result, silent) => self.update_done(result, silent),
                AsyncResult::DiskCapacity { pane, generation, disk } => {
                    if self.panes[pane].generation == generation {
                        self.panes[pane].disk = disk;
                    }
                }
                AsyncResult::ChecksumDone { serial, value } => self.checksum_done(serial, value),
                AsyncResult::TempCounted { folder, files, bytes } => self.empty_temp_counted(folder, files, bytes),
                AsyncResult::FtpConnected { pane, remote } => self.ftp_connected(pane, remote),
                AsyncResult::FtpFailed { error } => self.ftp_failed(error),
                AsyncResult::FtpEdited { pane, focus, error } => self.ftp_edited(pane, focus, error),
                AsyncResult::FtpReply { result } => self.ftp_reply(result),
                AsyncResult::OverlayIcon { path, image } => {
                    self.overlay_pending.remove(&path);
                    if self.overlay_icons.len() >= 512 {
                        self.overlay_icons.clear();
                        let kept: HashSet<egui::TextureId> = self.file_icons.values().flatten().map(|t| t.id()).collect();
                        self.ghost_icons.retain(|id, _| kept.contains(id));
                    }
                    let texture = image.map(|(width, height, rgba)| {
                        self.icon_texture(ctx, format!("ndr-overlay|{}", path.display()), width, height, &rgba)
                    });
                    self.overlay_icons.insert(path, texture);
                    ctx.request_repaint();
                }
                AsyncResult::Drives(drives) => {
                    self.drive_cache = Some((Instant::now(), drives));
                    self.drive_cache_pending = false;
                }
                AsyncResult::DrivesChanged => self.drives_changed(),
            }
        }
    }

    /// Original FUN_00442bc8 -> FUN_00442d80: when a drive appears or disappears, each pane rebuilds its drive rows (FUN_00459628: re-appends the drive items at the end of
    /// local and drive list panes), drive buttons (FUN_00483108) and folder/disk info bar (FUN_0048b500).
    /// NDR clears the drive cache, rereads the drive buttons, then rereads local panes that show drive rows (keeping cursor and marks).
    /// Fixed: previously plugging or removing a USB drive did not show up until the 5-second cache expired and that pane was reread.
    pub(super) fn drives_changed(&mut self) {
        panes::DriveListCache::invalidate(&self.drive_list);
        self.drive_cache = None;
        let show_drive_list = self.settings.flag("ShowDriveList", true);
        for index in 0..self.panes.len() {
            let pane = &self.panes[index];
            let local = pane.archive.is_none() && pane.remote.is_none();
            if local && (panes::is_drives(&pane.path) || show_drive_list) && !pane.loading {
                let typed = std::mem::take(&mut self.typed_search[index]);
                self.refresh(index);
                self.typed_search[index] = typed;
            }
        }
    }
}
/// C# RestorePaneTabs: keeps only saved tabs whose folders exist, and selects the saved selected tab (SelectedLeftTab, original index) if it remains.
/// With no tab left, one start folder. Difference: C# ignores the command line and start folder when tabs exist, but NDR, for a given folder (fixed),
/// selects that folder's tab or adds it at the front (earlier NDR behavior). Returns (tab list, selected tab index).
pub(super) fn restore_tabs(saved: &[String], selected: Option<usize>, start: &Path, fixed: bool, exists: impl Fn(&Path) -> bool) -> (Vec<PathBuf>, usize) {
    let available: Vec<(usize, PathBuf)> = saved.iter().enumerate().map(|(index, path)| (index, PathBuf::from(path))).filter(|(_, path)| exists(path)).collect();
    if available.is_empty() {
        return (vec![start.to_path_buf()], 0);
    }
    let restored = selected.and_then(|wanted| available.iter().position(|(index, _)| *index == wanted));
    let mut tabs: Vec<PathBuf> = available.into_iter().map(|(_, path)| path).collect();
    if !fixed && let Some(index) = restored {
        return (tabs, index);
    }
    match tabs.iter().position(|tab| tab == start) {
        Some(index) => (tabs, index),
        None => {
            tabs.insert(0, start.to_path_buf());
            (tabs, 0)
        }
    }
}

/// Original usemenualpha (setting +0xb6): when drawing menus (FUN_0042e518) the menu window gets WS_EX_LAYERED + alpha 0xF0 (240/255).
/// C# version MainMenu/ContextMenu.Opacity = 0.94.
pub(super) const MENU_ALPHA: f32 = 240.0 / 255.0;
/// Foreground layer ids not to fade this frame (ctx temp data).
const OPAQUE_LAYERS: &str = "ndr-opaque-foreground-layers";

/// Excludes things drawn on the foreground layer that are not menus (egui::Modal windows etc.) from this frame's menu transparency.
pub(super) fn keep_opaque(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<egui::Id>>(egui::Id::new(OPAQUE_LAYERS)).push(id));
}

/// Original windows (VCL TForm) move by dragging the title bar. egui::Modal is fixed at the center, so it is placed offset by the dragged amount (ctx temp data).
/// A new window (new id) starts centered like the original poOwnerFormCenter / poMainFormCenter.
pub(super) fn movable_modal(ctx: &egui::Context, id: egui::Id) -> egui::Modal {
    let offset = ctx.data(|d| d.get_temp::<egui::Vec2>(id.with("ndr-drag-offset"))).unwrap_or_default();
    egui::Modal::new(id).area(egui::Modal::default_area(id).anchor(egui::Align2::CENTER_CENTER, offset))
}

/// Dragging the title bar area (`rect`, passed without the close button) moves that window (id of `movable_modal`).
pub(super) fn drag_title(ui: &egui::Ui, id: egui::Id, rect: egui::Rect) {
    let response = ui.interact(rect, id.with("ndr-title-drag"), egui::Sense::drag());
    if response.dragged() {
        let delta = response.drag_delta();
        let ctx = ui.ctx();
        // Keep the window from leaving the screen entirely: move only so far that the title bar center stays on screen.
        let screen = ctx.content_rect();
        let center = rect.center() + delta;
        let delta = egui::vec2(
            if (screen.left()..=screen.right()).contains(&center.x) { delta.x } else { 0.0 },
            if (screen.top()..=screen.bottom()).contains(&center.y) { delta.y } else { 0.0 },
        );
        ctx.data_mut(|d| *d.get_temp_mut_or_default::<egui::Vec2>(id.with("ndr-drag-offset")) += delta);
        ctx.request_repaint();
    }
}

/// One title bar row (bold title + close button on the right). Dragging the row moves the window. True when the close button is pressed.
pub(super) fn modal_title_bar(ui: &mut egui::Ui, id: egui::Id, title: &str, width: Option<f32>) -> bool {
    let mut close = false;
    let mut button_left = None;
    let row = ui.horizontal(|ui| {
        if let Some(width) = width {
            ui.set_width(width);
        }
        ui.label(RichText::new(title).strong().family(egui::FontFamily::Name(BOLD_FONT_FAMILY.into())));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = ui.small_button("×");
            button_left = Some(response.rect.left());
            close = response.clicked();
        });
    });
    let mut rect = row.response.rect;
    if let Some(left) = button_left {
        rect.max.x = (left - 2.0).max(rect.min.x);
    }
    drag_title(ui, id, rect);
    close
}

/// The list of a drop-down (egui ComboBox) is also a foreground Popup but not a menu. The original and C# version make only menus transparent, so
/// the list layer (egui: button id.with("popup")) is excluded from this frame's menu transparency. ComboBoxes are opened with this instead of show_ui.
pub(super) trait ComboOpaque {
    fn show_opaque<R>(self, ui: &mut egui::Ui, menu_contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<Option<R>>;
}

impl ComboOpaque for egui::ComboBox {
    fn show_opaque<R>(self, ui: &mut egui::Ui, menu_contents: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<Option<R>> {
        let shown = self.show_ui(ui, menu_contents);
        keep_opaque(ui.ctx(), shown.response.id.with("popup"));
        shown
    }
}

/// Multiplies all shapes of the foreground layers drawn this frame (menus, right-click menus, drop-down lists) by alpha
/// (egui menus are not separate windows, so whole layers are faded after drawing). Non-menu things on the foreground layer (original windows opened with egui::Modal,
/// message boxes, progress windows, pane notices) are layers announced with `keep_opaque` and stay as is. With alpha 1 only the announced list is cleared.
pub(super) fn fade_foreground_layers(ctx: &egui::Context, alpha: f32) {
    let opaque: Vec<egui::Id> = ctx.data_mut(|d| d.remove_temp::<Vec<egui::Id>>(egui::Id::new(OPAQUE_LAYERS))).unwrap_or_default();
    if alpha >= 1.0 {
        return;
    }
    let skip = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("ndr-pane-message"));
    let layers: Vec<egui::LayerId> = ctx
        .memory(|m| m.areas().visible_layer_ids())
        .into_iter()
        .filter(|layer| layer.order == egui::Order::Foreground && *layer != skip && !opaque.contains(&layer.id))
        .collect();
    if layers.is_empty() {
        return;
    }
    ctx.graphics_mut(|graphics| {
        for layer in layers {
            if let Some(list) = graphics.get_mut(layer) {
                for index in 0..list.next_idx().0 {
                    list.mutate_shape(egui::layers::ShapeIdx(index), |clipped| {
                        egui::epaint::shape_transform::adjust_colors(&mut clipped.shape, move |color| {
                            if *color != Color32::PLACEHOLDER {
                                *color = color.gamma_multiply(alpha);
                            }
                        });
                    });
                }
            }
        }
    });
}

/// Font family name of the file list (original and C# ListFont). Used as the Body and Button font only while drawing lists.
pub(super) const LIST_FONT_FAMILY: &str = "list";
/// Text size outside lists (pixels). The C# version applies ListFontSize only to lists, so the rest stays at the default 12.
pub(super) const BASE_TEXT_SIZE: f32 = 12.0;

/// Applies underline (4) and strikeout (8) of the original ListFont style to list text (bold and italic come from the font files of the "list" family).
pub(super) fn list_text(text: impl Into<String>, style: u8) -> RichText {
    let text = RichText::new(text);
    let text = if style & 4 != 0 { text.underline() } else { text };
    if style & 8 != 0 { text.strikethrough() } else { text }
}

/// App fonts: Malgun Gothic (Korean) before the egui defaults, and Segoe UI, the default font of Vista and later, at the very front like original WinMain (0x403568)
/// (all original windows are ParentFont, so Application DefaultFont = Segoe UI 9pt = 12 px, and Korean uses Malgun Gothic through Windows font linking).
/// With list_font (font file, TTC index) the "list" family = that font -> Malgun Gothic -> defaults. Without it the "list" family equals the normal font.
/// With list_bold (bold of the original ListFont style), Korean in the "list" family is drawn with Malgun Gothic Bold (malgunbd.ttf, when present).
pub(super) fn set_app_fonts(ctx: &egui::Context, list_font: Option<(&'static [u8], u32)>, list_bold: bool) {
    // Font files are memory-mapped (native::mapped_file) and passed as 'static: egui does not copy borrowed fonts (a Vec would be copied once more).
    let font = |path: &str| native::mapped_file(Path::new(path)).map(|bytes| Arc::new(egui::FontData::from_static(bytes)));
    let mut fonts = egui::FontDefinitions::default();
    if let Some(data) = font("C:/Windows/Fonts/malgun.ttf") {
        fonts.font_data.insert("Korean".into(), data);
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().insert(0, "Korean".into());
        }
    }
    let mut korean_first = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    if list_bold {
        if let Some(data) = font("C:/Windows/Fonts/malgunbd.ttf") {
            fonts.font_data.insert("KoreanBold".into(), data);
            korean_first.insert(0, "KoreanBold".into());
        }
    }
    if let Some(data) = font("C:/Windows/Fonts/segoeui.ttf") {
        fonts.font_data.insert("UI".into(), data);
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "UI".into());
    }
    let list = match list_font {
        Some((bytes, index)) => {
            let mut data = egui::FontData::from_static(bytes);
            data.index = index;
            fonts.font_data.insert("ListFont".into(), Arc::new(data));
            let mut list = korean_first;
            list.insert(0, "ListFont".into());
            list
        }
        None => fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default(),
    };
    fonts.families.insert(egui::FontFamily::Name(LIST_FONT_FAMILY.into()), list);
    // NXLabel markup <b> (original TNXLabel: the bold face of that font): Segoe UI Bold -> Malgun Gothic Bold -> normal font. The normal font when the files are missing.
    let mut bold = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, file) in [("KoreanBoldUI", "C:/Windows/Fonts/malgunbd.ttf"), ("UIBold", "C:/Windows/Fonts/segoeuib.ttf")] {
        if let Some(data) = font(file) {
            fonts.font_data.insert(name.into(), data);
            bold.insert(0, name.into());
        }
    }
    fonts.families.insert(egui::FontFamily::Name(BOLD_FONT_FAMILY.into()), bold);
    ctx.set_fonts(fonts);
}

/// Bold text family name (NXLabel markup <b>).
pub(super) const BOLD_FONT_FAMILY: &str = "bold";

impl eframe::App for App {
    /// egui-winit turns Ctrl+C, Ctrl+X, Ctrl+V (Ctrl+Insert, Shift+Delete, Shift+Insert) into Copy, Cut and Paste instead of key events,
    /// and drops paste when the clipboard has no text. When no text field has focus they are turned back into key events so the original shortcuts
    /// (miEditCopy, miEditCut, miEditPaste) receive them.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Memory: the egui glyph atlas width is the GPU maximum texture size (NVIDIA 32768 -> 16384), so a few more rows cost tens of MB.
        // At 2048 it grows downward only as needed (the real limit on the rendering side stays the same).
        raw_input.max_texture_side = Some(raw_input.max_texture_side.unwrap_or(2048).min(2048));
        let pastes = ndr::shell_menu::take_paste_keys();
        let numpad = ndr::shell_menu::take_numpad_keys();
        let context_keys = ndr::shell_menu::take_context_keys();
        let text_focus = ctx.memory(|m| m.focused()).is_some_and(|id| egui::text_edit::TextEditState::load(ctx, id).is_some());
        if !text_focus {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_INSERT};
            let insert_down = unsafe { GetKeyState(VK_INSERT as i32) } < 0;
            restore_clipboard_keys(raw_input, pastes, insert_down);
            if !self.keyboard_blocked(ctx) {
                for (vk, ctrl, shift, alt) in numpad {
                    if let Some(original) = self.numpad_shortcut(vk, ctrl, shift, alt) {
                        strip_numpad_events(raw_input, vk);
                        self.pending_numpad.push((original, shift));
                    } else if matches!(vk, 0x6e | 0x6f) {
                        // Fixed: egui reports numpad '.' (VK_DECIMAL) and '/' (VK_DIVIDE) as the same Key as the main '.' and '/'. In the original they are different keys,
                        // so they do not act as '.' (parent folder) or '/' (miCommandLine). Only the press events are removed; the characters (quick search) stay.
                        let key = if vk == 0x6e { egui::Key::Period } else { egui::Key::Slash };
                        if let Some(i) = raw_input.events.iter().position(|e| matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key)) {
                            raw_input.events.remove(i);
                        }
                    }
                }
                if context_keys > 0 {
                    self.pending_context_key = true;
                }
            }
        }
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.frames += 1;
        let frame_started = Instant::now();
        // Count left button presses by the Windows double-click rule (so presses in frames that do not draw the list are not missed).
        let _ = panes::windows_double_press(ctx);
        // Fixed (user report: flicker when starting maximized): winit with_maximized showed an empty window with ShowWindow(SW_MAXIMIZE) before the first frame.
        // main.rs creates a hidden window of work area size, and after the first frame is shown (second frame) the saved normal size becomes the restore size and the window is maximized.
        match self.options.maximize_restore {
            Some((restore, true)) => {
                native::maximize_with_restore(self.hwnd, restore);
                self.options.maximize_restore = None;
                self.window_bounds = Some((egui::pos2(restore.0, restore.1), egui::vec2(restore.2, restore.3)));
                self.window_maximized = Some(true);
            }
            Some((restore, false)) => {
                self.options.maximize_restore = Some((restore, true));
                ctx.request_repaint();
            }
            None => {}
        }
        // C# Closing: a normal window saves its position and size, and the maximized state separately. The startup check (off-screen window) is not recorded.
        if self.options.smoke_seconds.is_none() && self.options.maximize_restore.is_none() {
            let (outer, inner, maximized, minimized) = ctx.input(|i| {
                let viewport = i.viewport();
                (viewport.outer_rect, viewport.inner_rect, viewport.maximized, viewport.minimized)
            });
            self.track_window(outer, inner, maximized, minimized);
        }
        if self.preferences_dirty {
            self.apply_preferences(ctx);
            self.preferences_dirty = false;
        }
        if ctx.input(|i| i.viewport().close_requested())
            && self.job.as_ref().is_some_and(|j| !j.finished)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close_while_working();
        }
        if self.close_when_idle && self.job.as_ref().is_none_or(|j| j.finished) {
            self.close_when_idle = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // C# DiskCopyWindow OnClosing: no closing during disk work (image creation gets a cancel request).
        if ctx.input(|i| i.viewport().close_requested()) && self.disk_copy_blocks_close() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        let requests = self
            .navigation
            .as_ref()
            .map(|rx| rx.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for request in requests {
            if let Some(path) = request.left {
                self.navigate(0, path, true);
            }
            if let Some(path) = request.right {
                self.navigate(1, path, true);
            }
            if let Some(lifecycle) = &self.lifecycle {
                let _ = lifecycle.show();
            }
        }
        let events = self
            .lifecycle_events
            .as_ref()
            .map(|rx| rx.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in events {
            match event {
                ndr::lifecycle::LifecycleEvent::Restore => {
                    if let Some(lifecycle) = &self.lifecycle {
                        let _ = lifecycle.show();
                    }
                }
                ndr::lifecycle::LifecycleEvent::Exit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close)
                }
                // Original FUN_00442bc8 (SHCNE_MEDIAINSERTED/MEDIAREMOVED, DRIVEADD/REMOVED): like inserting or removing a CD, even when the drive letter stays and
                // only the media changes, the drive rows, buttons and info bars are rebuilt.
                ndr::lifecycle::LifecycleEvent::DevicesChanged => self.drives_changed(),
            }
        }
        if let Some(lifecycle) = &self.lifecycle {
            if self.settings.flag("HideWhenMinimized", false) {
                let _ = lifecycle.hide_if_minimized();
            }
        }
        if self.language_pack.code != self.language {
            self.language_pack =
                compat::Language::load(&self.resources.join("Languages"), &self.language);
        }
        self.poll(ctx);
        self.keyboard(ctx);
        self.sync_current_directory();
        for pane in &mut self.panes {
            pane.drop_drive_marks();
        }
        // Original menu bar background (tbMainMenuAdvancedCustomDraw): [Menu] mmainback.
        let skin_now = self.current_skin();
        skin_pane::set_menu_style(ctx, skin_now.as_ref().map(skin_pane::skin_menu_style));
        let menu_frame = match &skin_now {
            Some(skin) => egui::Frame::side_top_panel(&ctx.style()).fill(skin_pane::skin_menu_bar_style(skin).0),
            None => egui::Frame::side_top_panel(&ctx.style()),
        };
        egui::TopBottomPanel::top("menu").frame(menu_frame).show(ctx, |ui| {
            self.menu(ui);
            self.paint_original_search_text(ui);
        });
        // Original TfMain: menu bar (tbMainMenu), toolbar (pToolbar), two panes, function key row (bbFunctions). There is no status bar.
        if let Some(skin) = skin_now {
            // Original pToolbar (alTop): height tbheight (black 51), background image and image buttons (toolbar.rs skin_toolbar).
            if self.settings.flag("ShowToolBar", true) {
                let height = skin.positive_integer("Toolbar", "tbheight", 51) as f32;
                egui::TopBottomPanel::top("toolbar").exact_height(height).frame(egui::Frame::NONE).show_separator_line(false).show(ctx, |ui| self.skin_toolbar(ui, ctx, &skin));
            }
            // Original bbFunctions (DFM Height 20): F2-F8 cells (user_commands.rs skin_function_key_bar). Turned off, no row at all.
            // NDR status text does not exist in the original (the window has no status bar), so it is overlaid briefly above the function key row without shrinking the list.
            let function_bar = self.settings.flag("ShowFunctionBar", true);
            if function_bar {
                egui::TopBottomPanel::bottom("functionbar").exact_height(20.0).frame(egui::Frame::NONE).show_separator_line(false).show(ctx, |ui| self.skin_function_key_bar(ui, ctx, &skin));
            }
            let _ = self.skin_status_overlay(ctx, if function_bar { 20.0 } else { 0.0 });
        }
        if self.show_results {
            // Original pBottom Height 100 (22 <-> 100 with btnFTPShowDetail), resized with splBottom.
            let panel = egui::TopBottomPanel::bottom(if self.results_collapsed { "ftp-results-collapsed" } else { "ftp-results" });
            let panel = if self.results_collapsed { panel.resizable(false).exact_height(24.0) } else { panel.resizable(true).default_height(100.0) };
            panel.show(ctx, |ui| self.ftp_results_ui(ui));
        }
        // The original folder view (Alt+F3) is drawn per pane left of the list (pane_ui -> pane_tree_ui).
        // Drop target lookup: only the panes and folder rows drawn this frame (in single-pane view the hidden pane has none).
        self.pane_rects = [egui::Rect::NOTHING; 2];
        self.folder_rows.clear();
        self.tree_drop_rows.clear();
        // Original TfMain: panes (frames) fill the space between pToolbar (alTop) and bbFunctions (alBottom) without gaps (FUN_0043d7c0: pane width = window width -
        // visible VertSplitter and pVertBar widths). Fixed: previously the egui default panel margin (8) and gaps left 13 px above, 9 below and 16 in the middle.
        egui::CentralPanel::default().frame(egui::Frame::central_panel(&ctx.style()).inner_margin(0.0)).show(ctx, |ui| {
            if self.dual {
                // Between the panes: the original pVertBar (DFM Width 18, alRight), or VertSplitter (Width 4, clBlack) when the bar is hidden. Dragging sets the width (LeftPaneWidth %, 20-80),
                // a double click makes equal widths (original pmiFrameWidthSame).
                let full = ui.available_rect_before_wrap();
                let show_bar = self.settings.flag("ShowVerticalBar", true);
                let (bar, gap) = (if show_bar { 18.0 } else { 4.0 }, 0.0);
                let usable = (full.width() - bar - 2.0 * gap).max(0.0);
                let left_width = usable * self.left_pane_ratio();
                let left = egui::Rect::from_min_size(full.min, egui::vec2(left_width, full.height()));
                let middle = egui::Rect::from_min_size(egui::pos2(left.max.x + gap, full.min.y), egui::vec2(bar, full.height()));
                let right = egui::Rect::from_min_size(egui::pos2(middle.max.x + gap, full.min.y), egui::vec2((usable - left_width).max(0.0), full.height()));
                let divider = ui.interact(middle, egui::Id::new("ndr-pane-divider"), egui::Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                if panes::windows_double_clicked(&divider) {
                    self.set_left_pane_width(50.0);
                    self.save();
                } else if divider.dragged() {
                    if let (Some(pointer), true) = (divider.interact_pointer_pos(), usable > 0.0) {
                        self.set_left_pane_width(((pointer.x - full.min.x - gap - bar / 2.0) / usable * 100.0) as f64);
                    }
                } else if divider.drag_stopped() {
                    self.save();
                }
                // Original pmVertBar (pVertBarContextPopup, right-click outside the buttons): equal widths / left width => right width /
                // left width <= right width. The last two (pmiFrameWidth 0x43d960) set 50:50 and change the window width to twice that side's pane width.
                let mut frame_action = None;
                skin_pane::context_menu(&divider, |ui| {
                    let label = |key: &str, fallback: &str| self.language_pack.text(&format!("Menu.{key}"), fallback).replace('&', "");
                    for (index, key, fallback) in [(0, "pmiFrameWidthSame", "Equalize Width"), (1, "pmiFrameWidthLeftToRight", "Left Width => Right Width"), (2, "pmiFrameWidthRightToLeft", "Left Width <= Right Width")] {
                        if ui.button(label(key, fallback)).clicked() {
                            frame_action = Some(index);
                            ui.close();
                        }
                    }
                });
                if let Some(index) = frame_action {
                    let right_width = usable - left_width;
                    self.set_left_pane_width(50.0);
                    self.save();
                    let delta = match index {
                        1 => left_width - right_width,
                        2 => right_width - left_width,
                        _ => 0.0,
                    };
                    if delta != 0.0 {
                        if let Some(inner) = ctx.input(|i| i.viewport().inner_rect) {
                            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2((inner.width() + delta).max(200.0), inner.height())));
                        }
                    }
                }
                if !show_bar {
                    ui.painter().rect_filled(middle, 0.0, Color32::BLACK);
                }
                // Fixed: each pane is clipped to its own area (when dragged narrow, the address bar, info bar and list spilled over the next pane and the vertical bar).
                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                    ui.set_clip_rect(left.intersect(ui.clip_rect()));
                    self.pane_ui(ui, 0, ctx)
                });
                if show_bar {
                    ui.scope_builder(egui::UiBuilder::new().max_rect(middle), |ui| {
                        ui.set_clip_rect(middle.intersect(ui.clip_rect()));
                        self.vertical_bar(ui, ctx)
                    });
                }
                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                    ui.set_clip_rect(right.intersect(ui.clip_rect()));
                    self.pane_ui(ui, 1, ctx)
                });
            } else {
                self.pane_ui(ui, self.active, ctx);
            }
        });
        self.drag_drop_frame(ctx);
        self.dialogs(ctx);
        self.popup_menu_ui(ctx);
        self.shortcut_window_ui(ctx);
        self.original_dialog_ui(ctx);
        // The modal progress window is drawn above the window under it (for example the custom search of the cleanup options window).
        self.modal_progress_ui(ctx);
        self.msgbox_ui(ctx);
        self.pane_message_ui(ctx);
        self.fade_menus(ctx);
        if let Some(seconds) = self.options.smoke_seconds {
            self.shot_step(ctx);
            // The startup check must run behind other windows (if it becomes the foreground window, the report records it and the release script stops).
            if self.hwnd != 0 && native::is_foreground(self.hwnd) {
                self.smoke_foreground = true;
            }
            // Diagnostics: what requested this frame's repaint (egui RepaintCause, file:line). Used to find continuous repaints while idle.
            let causes: Vec<String> = ctx.repaint_causes().iter().map(|c| c.to_string()).collect();
            for cause in causes {
                *self.smoke_repaint_causes.entry(cause).or_insert(0) += 1;
            }
            // Diagnostics: the slowest frame (ms; the first 3 frames are excluded because they prepare fonts and icons).
            if self.frames > 3 {
                let ms = frame_started.elapsed().as_secs_f64() * 1000.0;
                self.smoke_slowest_frame_ms = self.smoke_slowest_frame_ms.max(ms);
                if ms > 50.0 {
                    self.smoke_slow_frames += 1;
                }
            }
            // Diagnostics: from startup until both pane lists are first fully read (ms).
            if self.smoke_loaded_ms.is_none() && self.panes.iter().all(|p| !p.loading) {
                self.smoke_loaded_ms = Some(self.started.elapsed().as_millis() as u64);
            }
            if self.started.elapsed() > Duration::from_secs(seconds) {
                if let Some(path) = &self.options.report {
                    // leftCount/rightCount = list items (drive items appended at the end are counted separately in leftDrives/rightDrives).
                    let items = |p: &Pane| p.entries.len() - p.drive_count();
                    let report = serde_json::json!({"language":self.language,"frames":self.frames,"leftCount":items(&self.panes[0]),"rightCount":items(&self.panes[1]),"leftDrives":self.panes[0].drive_count(),"rightDrives":self.panes[1].drive_count(),"leftError":self.panes[0].error,"rightError":self.panes[1].error,"leftLoading":self.panes[0].loading,"rightLoading":self.panes[1].loading,"profile":self.profile,"runtime":"Rust","renderer":"OpenGL","foreground":self.smoke_foreground,"repaintCauses":self.smoke_repaint_causes,"slowestFrameMs":(self.smoke_slowest_frame_ms * 10.0).round() / 10.0,"slowFrames":self.smoke_slow_frames,"loadedMs":self.smoke_loaded_ms,"tipWindow":self.smoke_tip_window});
                    let _ = std::fs::write(
                        path,
                        serde_json::to_vec_pretty(&report).unwrap_or_default(),
                    );
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(
            if self.panes.iter().any(|p| p.loading)
                || self.job.as_ref().is_some_and(|j| !j.finished)
            {
                60
            } else {
                500
            },
        ));
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.tool_cancel.store(true, Ordering::Relaxed);
        self.drive_watch_stop.store(true, Ordering::Relaxed);
        if let Some(password) = self.password_request.take() {
            let _ = password.reply.send(None);
        }
        // C# Closing: PruneMissing(ExecutableHistory, _lastCommandWorkingDirectory).
        let mut rows = self.settings.strings("ExecutableHistory");
        ndr::command_line::prune_missing(&mut rows, self.last_command_directory.as_deref());
        self.settings.set_strings("ExecutableHistory", rows);
        self.save();
    }
}
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < 4 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
