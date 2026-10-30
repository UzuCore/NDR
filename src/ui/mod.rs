mod archive_dialog;
mod auto_view;
mod skin_pane;
mod cleanup_dialog;
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

/// 원본 TNXShellChangeNotify 생성자(0x7b3304): Delay(+0x40) = 0x5dc = 1500ms.
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
    /// 탭마다의 (뒤로, 앞으로) 기록(C# 탭마다 PaneState). 지금 탭의 기록은 back/forward에 있다.
    tab_history: Vec<(Vec<PathBuf>, Vec<PathBuf>)>,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// C# FilterPattern: ';'로 나눈 이름 마스크. 목록을 받을 때 적용한다.
    filter: String,
    show_hidden: bool,
    loading: bool,
    generation: u64,
    error: Option<String>,
    archive: Option<ArchiveLocation>,
    /// 원본 패널 FTP 모드(FUN_00481948 == 2): 연결과 원격 폴더. path는 연결 전 로컬 폴더로 남는다.
    remote: Option<ftp_pane::PaneRemote>,
    /// 원본 FTP 기록 칸 높이(처음 60, 위 나눔 막대로 바꾼다).
    ftp_log_height: f32,
    descriptions: HashMap<String, String>,
    /// 화면에 쓰는 열 글·색·묶음 줄(sort_pane에서 다시 만든다).
    view: PaneView,
    /// 정보 표시줄의 목록 요약(C# PaneState.UpdateSummary). 새 목록이 오면 None.
    summary: Option<main_window::PaneSummary>,
    /// 원본 정보 표시줄의 디스크 재료(드라이브 글 "이름 (C:) ", 여유, 전체). 읽지 못했으면 None.
    disk: Option<(String, u64, u64)>,
    /// 내 PC 보기의 드라이브 항목 정보(원본 TNXDriveInfo: 이름, 종류, 고정 디스크면 크기).
    drive_infos: HashMap<PathBuf, (String, u32, Option<(u64, u64)>)>,
    /// 주소 표시줄을 글 칸으로 바꿔 입력 중(C# _addressEditing).
    address_editing: bool,
    /// 폴더 크기 자동 표시(경로 → None = 세는 중, Some(글)). 새 목록이 오면 비운다.
    folder_sizes: HashMap<PathBuf, Option<String>>,
    /// 센 폴더 크기(바이트). 원본 파일 정보 표시줄(FUN_0048a5cc)은 항목 크기를 FloatToStrF(ffNumber)로 보인다.
    folder_bytes: HashMap<PathBuf, u64>,
    /// 지금 목록의 폴더 크기 세기를 멈추는 표시(C# CancelFolderSizeJobs).
    folder_cancel: Arc<AtomicBool>,
    /// 이 위치에서 이미 네트워크 공유 연결 창을 띄웠는지(C# allowNetworkAuthPrompt: 한 번만).
    auth_prompted: bool,
    /// 지난 그림의 쪽 정보(원본 ArrowKey가 쓰는 한 열의 줄 수·보이는 열 수·맨 위 항목).
    page: list_keys::ListPage,
    /// 열로 흐르는 목록(자동·이름)의 처음 보이는 열(원본 가로 위치 0x378 / 열 너비).
    first_column: usize,
    /// 자세히 보기의 가로 위치(원본 0x378, 화소). 칸 너비 합이 목록보다 넓을 때만 0보다 크다.
    detail_x: f32,
    /// 지난 그림의 미리 보기 줄 자리(줄마다 위·높이, 맨 위 위치, 목록 높이, 열 수). 원본 형식 3 PageUp·PageDown이 쓴다.
    thumb_rows: Option<(Vec<f32>, Vec<f32>, f32, f32, usize)>,
    /// 원본 패널 폴더 트리(TNXShellTreeView, miShowFoldersClick): 펼친 폴더와, 펼침을 맞춘 마지막 패널 위치(SetPath).
    tree_open: HashSet<PathBuf>,
    tree_synced: Option<PathBuf>,
    /// 원본 TNXShellChangeNotify(+0x374): 지켜보는 로컬 폴더와 그 감시 스레드를 멈출 표시.
    watch: Option<(PathBuf, Arc<AtomicBool>)>,
    /// 바뀜 알림 뒤 새로 읽을 때와 그동안 늦춘 횟수(원본 Delay 1500ms, 20번까지 다시 늦춘다).
    change_due: Option<(Instant, u32)>,
    /// 다음 그림에서 커서가 보이게 넘긴다(원본 SetItemIndex).
    reveal: bool,
    /// 원본 AdjustPage(0x7daf24): 다음 그림에서 이 항목 번호에 맞춰 위치를 다시 잡는다. 목록 크기가 바뀔 때(WMSize 0x7d5814),
    /// 보기 형식이 바뀔 때(SetListStyle 0x7dc6d4), 같은 위치를 다시 읽었을 때(FileListChanged 0x7d79c8, 열로 흐르는 목록만).
    /// 두 번째 값이 참이면 목록을 다시 읽어서다(자세히·미리 보기는 원본이 그 뒤 SetScrollTop(옛 위치)로 되돌리므로 무시한다).
    adjust: Option<(usize, bool)>,
    /// 지난 그림의 목록 크기와 보기 형식(바뀌면 adjust).
    list_shape: Option<(egui::Vec2, String)>,
    /// Shift를 누른 채 움직이기 시작한 자리(원본 SetShiftDownIndex 0xf4).
    shift_anchor: Option<usize>,
    /// 원본 표시 칸 끌기(+0x3d4 기준 줄, +0x3d8 지난 끝 줄). 왼쪽 단추를 놓으면 없앤다(MouseUp).
    range_drag: Option<(usize, usize)>,
    /// 원본 Edit > 잘라내기(0x433dd8)가 표시한 항목(항목 +100, 아이콘을 흐리게). 새 목록이 오면 없어진다(새 항목).
    cut_marks: HashSet<PathBuf>,
    /// 지난번 받은 목록의 위치(location_key). 같은 위치를 다시 읽으면 커서를 같은 이름에 둔다.
    listed: Option<String>,
    /// 원본 MouseDown이 항목을 눌렀다(두 번째 누름이 아님). 원본은 그때 찾는 글을 지우므로(가상 +0x160("")) 빠른 찾기가 받아 지운다.
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
            tree_open: HashSet::new(),
            tree_synced: None,
            watch: None,
            change_due: None,
            reveal: false,
            adjust: None,
            list_shape: None,
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
    /// 목록 끝 드라이브 항목(원본 종류 4) 수.
    pub(super) fn drive_count(&self) -> usize {
        self.entries.iter().filter(|e| e.drive).count()
    }
    /// 원본 Select(0x7dc834): 드라이브 항목과 ".."은 고르지 않는다(+0x67). 표시한 줄에서 뺀다.
    pub(super) fn drop_drive_marks(&mut self) {
        if self.selected.iter().any(|i| self.entries.get(*i).is_some_and(|e| e.drive || e.is_parent())) {
            let entries = &self.entries;
            self.selected.retain(|i| entries.get(*i).is_none_or(|e| !e.drive && !e.is_parent()));
        }
    }
}
/// 원본 단축키가 받은 숫자판 글쇠 하나의 egui 사건(눌림 Key와 Text 글자)을 없앤다. Delphi IsShortCut이 처리한 글쇠는
/// WM_CHAR가 생기지 않으므로 빠른 찾기에도 가지 않는다.
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
/// egui Copy·Cut·Paste 사건을 원래 글쇠(Ctrl+C·X·V, Ctrl+Insert, Shift+Delete, Shift+Insert)로 되돌리고,
/// egui-winit이 버린 붙여넣기 글쇠(pastes − 바꾼 Paste 수)를 덧붙인다.
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
/// 패널 위치를 하나의 글로(로컬 폴더, 압축 파일 + 안 경로, FTP 호스트 + 경로). 같은 위치를 다시 읽었는지 가린다.
fn location_key(pane: &Pane) -> String {
    match (&pane.archive, &pane.remote) {
        (_, Some(remote)) => format!("ftp|{}|{}", remote.host, remote.path),
        (Some(archive), None) => format!("arc|{}|{}", archive.file.display(), archive.prefix),
        (None, None) => format!("dir|{}", pane.path.display()).to_lowercase(),
    }
}
/// 원본 GetList(0x45a34c): 루트가 아닌 로컬 폴더(내 PC 보기 제외), 압축 안(늘), FTP·SFTP는 "/"가 아닌 폴더에 ".." 항목.
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
/// 원본 TNXCustomFileListView 빠른 찾기의 찾는 글(+0x3e0).
#[derive(Default)]
struct TypedSearch {
    query: String,
    /// 원본 빠른 찾기가 옮긴 커서. 다른 길(화살표·누르기)로 커서가 바뀌면 찾는 글을 지운다(원본 ArrowKey·MouseDown).
    cursor: Option<usize>,
    /// 원본 WMIMEComposition(0x7d584c)의 조합 중 표시(+0x3dd): 조합을 시작하기 전의 찾는 글.
    ime_base: Option<String>,
}
struct Listing {
    pane: usize,
    generation: u64,
    result: Result<Vec<Entry>, String>,
    descriptions: HashMap<String, String>,
    /// 내 PC 보기의 드라이브 항목 정보(루트 → (이름, 종류, 크기)).
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
    /// C# PlayOperationCompletionSound를 부르는 작업(복사·이동·삭제·완전 삭제·압축 만들기).
    chime: bool,
    /// C#의 모달 진행 창(FolderComparisonProgressWindow, SearchProgressWindow, 동기화의 FileOperationProgressWindow) 자리면 Some.
    /// 끝나면 저절로 닫히고 상태 줄과 패널은 건드리지 않는다(결과는 AsyncResult가 처리한다).
    modal: Option<ModalProgress>,
    /// 끝난 뒤 상태 줄은 작업이 보낸 결과 글(AsyncResult::ArchiveStatus 등)이 쓴다.
    custom_status: bool,
    /// 끝났을 때의 C# 결과 알림 방식.
    report: file_operation::JobReport,
    /// Some = C# FileOperationProgressWindow(삭제·완전 삭제는 None, C#도 창이 없다).
    progress: Option<file_operation::ProgressInfo>,
    /// 오류 상자로 이미 보인 오류 수(C# InteractiveErrorCount).
    interactive: usize,
    /// 충돌에서 건너뛴 항목 수(C# SkippedItems).
    skipped: usize,
    /// 작업 전체가 멈춘 오류(C# FatalError).
    fatal: Option<String>,

    /// 원본 줄을 처음 알려 온 경로의 폴더로 정했으면 그 값.
    from_seen: Option<String>,
    /// 마지막 Event::Target(원본 파일, 실제 대상 파일). 진행 창 lTo는 지금 원본 파일의 대상 폴더.
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
/// 모달 진행 창의 글과 단추.
struct ModalProgress {
    /// 단추 글(비교: formCompareFolder.Caption.btnStop, 검색·동기화: 취소).
    stop: String,
    /// 누른 뒤의 단추 글(비교: 중단 중…). None이면 그대로 두고 단추만 끈다.
    stopping: Option<String>,
    /// 누른 뒤의 상태 글.
    stopping_text: String,
    kind: ModalKind,
}
/// Job의 path·files·bytes를 읽는 방식.
#[derive(Clone, PartialEq, Eq)]
enum ModalKind {
    /// 비교: 상태 글 = path, 막대 = files / bytes(bytes가 0이면 움직이는 막대).
    Message,
    /// 검색: "검색 항목 N · 결과 N" 다음 줄에 path(files = 확인한 항목, bytes = 결과), 막대는 늘 움직인다.
    Search,
}

struct PasswordRequest {
    /// 원본 상자는 압축 파일 이름을 보이지 않는다. 시험이 어느 압축이 물었는지 확인한다.
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
        path: PathBuf,
        directories: Vec<PathBuf>,
    },
    /// 원본 TNXShellChangeNotify 스레드 알림(ThreadNotification 0x7b3830): 그 패널이 지켜보던 폴더가 바뀌었다.
    FolderChanged { pane: usize, path: PathBuf },
    ProtectedItems(file_commands::DeleteRun),
    AttributesApplied(pane_commands::AttributeRun),
    SplitDone { source: PathBuf, outcome: Result<ndr::split::SplitResult, String>, cancelled: bool },
    JoinDone { count: usize, outcome: Result<(), String>, cancelled: bool },
    SearchInPane { pane: usize, root: PathBuf, outcome: Result<Vec<Entry>, String>, cancelled: bool },
    CompareDone(compare_dialog::CompareRun, Result<(Vec<ndr::folder_compare::Difference>, Vec<String>), String>, bool),
    SyncDone(Result<ndr::folder_compare::SyncResult, String>, bool),
    /// 컴퓨터 청소 사용자 지정 검색이 들어간 폴더(lResult)와 끝(찾은 것).
    CleanupSearchFolder(String),
    CleanupScanned(Vec<ndr::cleanup::Candidate>),
    /// C# RunSafely 모양의 경고 상자(MessageBox(글, "NDR", 경고)).
    Warning(String),
    MergeSources(PathBuf, Result<Vec<PathBuf>, String>),
    MergeDone(usize, Result<(), String>),
    /// 작업이 끝난 뒤 보일 상태 줄 글.
    ArchiveStatus(String),
    /// 드라이브 단추 목록(루트, 도움말).
    Drives(Vec<(PathBuf, String)>),
    /// 원격 명령 결과(사이트 번호, 명령, "호스트:포트", 출력 또는 오류).
    RemoteCommand { index: usize, command: String, host: String, result: Result<String, String> },
    /// 폴더 크기 자동 표시 결과(패널, 목록 세대, 폴더, 글).
    FolderSizeRow { pane: usize, generation: u64, path: PathBuf, text: String, bytes: u64 },
    /// 업데이트 피드를 받음.
    /// 업데이트 피드 결과. bool = 시작할 때의 조용한 확인(원본 [+0x54], 새 버전이면 메뉴 줄 단추만).
    Update(Result<ndr::update::HttpResponse, ndr::update::HttpError>, bool),
    /// 정보 표시줄의 디스크 재료(패널, 목록 세대, (드라이브 글, 여유, 전체)).
    DiskCapacity { pane: usize, generation: u64, disk: Option<(String, u64, u64)> },
    /// 원본 TfChecksum 계산 끝(대화 상자 번호, 결과 글. 실패면 None).
    ChecksumDone { serial: u64, value: Option<String> },
    /// 임시 폴더 비우기 전에 따로 센 파일 수와 크기(원본 miEmptyTempClick의 셈, 화면을 멈추지 않게 따로 돈다).
    TempCounted { folder: PathBuf, files: u64, bytes: u64 },
    /// 셸 오버레이 아이콘을 읽음(C# ProcessShellOverlayIcons). None이면 확장자 아이콘을 쓴다.
    OverlayIcon { path: PathBuf, image: Option<(usize, usize, Vec<u8>)> },
    /// 원본 FUN_004408f8: 패널 FTP 연결이 됨 / 실패.
    FtpConnected { pane: usize, remote: ftp_pane::PaneRemote },
    FtpFailed { error: String },
    /// FTP 패널의 새 폴더·이름 바꾸기가 끝남(패널, 커서를 둘 이름, 오류).
    FtpEdited { pane: usize, focus: Option<String>, error: Option<String> },
    FtpReply { result: Result<String, String> },
    /// 외부 복사 프로그램이 끝남(결과, 끝난 뒤 상태 글, 붙여넣기 이동이면 클립보드 비우기).
    ExternalCopy { result: Result<(), String>, status: Option<String>, clear_clipboard: bool },
    /// 압축 항목을 임시로 풀었음(패널, 압축 파일, 쓴 암호, 풀린 경로, 실패 "이름: 오류", 압축 파일이면 그 안으로 갈지).
    ArchiveEntriesOpened { pane: usize, archive: PathBuf, password: Option<String>, opened: Vec<PathBuf>, failures: Vec<String>, navigate_archives: bool },
    /// 작업 창 없이 도는 압축 풀기가 암호를 묻는다(답은 reply로, 취소면 None).
    ArchivePassword { archive: PathBuf, reply: Sender<Option<String>> },
    /// 텍스트 보기/편집할 파일(압축 항목은 임시로 푼 경로).
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
    /// 원본 TfMessage(FUN_0040e138 → FUN_0044a1d4): 패널 가운데 잠깐 뜨는 검은 알림(글, 패널, 뜬 시각). 3초(Timer1 3000)나 누르면 닫힌다.
    pane_message: Option<(String, usize, Instant)>,
    /// 원본 pBottom("결과", miViewFindResult): FTP 전송 목록(lvFTPProgress)과 잠시 중단·취소.
    show_results: bool,
    /// 지금 "list" 글꼴 묶음에 넣은 ListFont 이름과 굵게·기울임(바뀌면 글꼴을 다시 만든다).
    list_font_name: Option<String>,
    /// 원본 ListFont 모양(TFontStyles 바이트: 1 굵게, 2 기울임, 4 밑줄, 8 취소선). 밑줄·취소선은 목록 글에 직접 입힌다.
    list_font_style: u8,
    /// C# Closing의 WindowLeft·Top·Width·Height: 마지막으로 본 보통(최대화·최소화 아님) 창의 바깥 위치와 바깥 크기(점).
    window_bounds: Option<(egui::Pos2, egui::Vec2)>,
    /// C# WindowMaximized: 마지막으로 본 최대화 여부(최소화 중에는 바꾸지 않음).
    window_maximized: Option<bool>,
    /// ListFontSize(화소, 처음 값 14): 파일 목록 글에만 쓴다. 원본 listfont 크기는 포인트라 가져올 때 바꾼다.
    list_font_size: f32,
    /// 원본 btnFTPShowDetail(">>"): pBottom 높이가 30을 넘으면 22(단추 줄만), 아니면 100.
    results_collapsed: bool,
    /// 지난 그림의 도구 모음 즐겨찾기 단추 왼쪽 아래(원본 miFavoritesClick이 그 단추의 DropDown을 연다). 단추가 없으면 None.
    favorites_button: Option<egui::Pos2>,
    /// 패널 폴더 바뀜 감시(원본 TNXShellChangeNotify). 시험에서는 끈다(시험이 만든 파일 때문에 늦게 새로 읽지 않게).
    watch_folders: bool,
    /// 전송을 시작한 때(lFTPTransferred의 KB/s).
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
    /// C# _instanceMutex/_instancePipeTask: 한 번에 하나만 실행(UseSingleInstance)일 때 잡은 사용자·프로필 뮤텍스와 이동 요청 수신.
    instance_guard: Option<ndr::lifecycle::InstanceGuard>,
    /// 도구 모음 그림(테마|파일 이름 → 그림, 못 읽으면 None).
    toolbar_icons: HashMap<String, Option<egui::TextureHandle>>,
    profile_load_error: bool,
    preferences_dirty: bool,
    close_when_idle: bool,
    /// 원본 FormCloseQuery(0x42a798)의 STILL_HAVE_OPERATION 질문이 떠 있는 동안.
    close_asking: bool,
    toolbar: ndr::commands::ToolbarConfig,
    /// 도구 모음 빠른 찾기 칸의 글(C# _toolbarQuickSearchBox). Enter에서 빠른 필터가 된다.
    toolbar_query: String,
    /// 파일 목록에서 바로 입력하는 빠른 찾기(C# _quickSearchByPane), 패널마다 하나.
    typed_search: [TypedSearch; 2],
    /// 원본 SetItemIndex(0x7dbdec) → ToolTipTimer(600ms) → ShowToolTip: 이번 그림에서 커서 줄 이름 칸(패널, 줄, 칸, 보인 이름 글, 온 이름 글).
    name_tip: panes::NameTip,
    /// 원본 ShowToolTip 형식 3(미리 보기): 이번 그림에서 커서 칸 이름이 줄었으면 그 칸과 온 이름 줄들.
    thumb_tip: panes::ThumbTip,
    /// 이번 그림에 이름 툴팁을 그린 자리(패널마다).
    name_tip_shown: [Option<egui::Rect>; 2],
    /// 스킨 테마 상태 글 겹침: 지금 글과 처음 보인 때(바뀌면 다시 센다).
    status_seen: Option<(String, Instant)>,
    /// 패널마다 커서가 그 줄로 온 때(원본 툴팁 타이머 시작).
    tip_since: [Option<(usize, Instant)>; 2],
    images: HashMap<PathBuf, egui::TextureHandle>,
    /// C# FileVisualConverter.IconCache: 확장자(소문자, 폴더는 "<DIR>")마다 작은 셸 아이콘. 못 읽으면 None을 기억한다.
    file_icons: HashMap<String, Option<egui::TextureHandle>>,
    /// 원본 TNXShellImageList.Draw(0x7b8ebc)의 흐린 아이콘(ILD_BLEND50, 섞는 색 흰색): 아이콘 그림마다 하나.
    ghost_icons: HashMap<egui::TextureId, egui::TextureHandle>,
    /// 지금 테마의 원본 스킨(conf.ini) — (테마 이름, 스킨). 검정·클래식·회색이 아니면 None.
    skin_cache: Option<(String, Option<ndr::styles::Skin>)>,
    /// 스킨 주소 줄에서 연 하위 폴더 목록: (패널, 폴더, 드라이브 목록인가, 위치).
    skin_crumb_menu: Option<(usize, PathBuf, bool, egui::Pos2)>,
    /// 확장자(소문자, 점 없이)마다 셸 아이콘이 파일마다 다른지(system.per_file_icon, 한 번만 묻는다).
    per_file_icon_types: HashMap<String, bool>,
    /// C# ShellOverlayIconCache(경로마다, 512개가 넘으면 비움)와 기다리는 경로, 오버레이 작업 스레드(STA)로 보낼 곳.
    overlay_icons: HashMap<PathBuf, Option<egui::TextureHandle>>,
    overlay_pending: HashSet<PathBuf>,
    overlay_tx: Option<Sender<(PathBuf, bool)>>,
    image_requests: HashSet<PathBuf>,
    tree_cache: HashMap<PathBuf, Vec<PathBuf>>,
    tree_requests: HashSet<PathBuf>,
    msgboxes: Vec<msgbox::MsgBox>,
    msgbox_serial: u64,
    msgbox_icons: HashMap<u16, egui::TextureHandle>,
    original_dialogs: Vec<original_dialogs::OriginalDialog>,
    /// 주소 표시줄 ⌄ 메뉴의 하위 폴더(경로, 드라이브 목록인지) → (경로, 글). 새 목록이 오면 비운다.
    crumb_cache: HashMap<(PathBuf, bool), Vec<(PathBuf, String)>>,
    /// 끌고 있는 패널 항목, 그 아래 폴더 줄(패널, 폴더), 이번 화면의 두 패널 영역.
    file_drag: Option<drag_drop::FileDrag>,
    /// 패널 끌기를 놓은 곳에서 아직 옮기지 않은 것(끈 패널, 끈 항목). 확인 상자·놓기 메뉴로 옮길 때 끈 패널의 표시를 지운다.
    pending_drag_marks: Option<(usize, Vec<PathBuf>)>,
    /// 이번 화면에 그린 폴더 줄(패널, 줄 영역, 폴더). 놓을 곳을 찾는다. 화면마다 비운다.
    folder_rows: Vec<(usize, egui::Rect, PathBuf)>,
    pane_rects: [egui::Rect; 2],
    /// 창 밖으로 끌어 내기(셸 끌기). 테스트에서는 끈다(실제 끌기 고리를 돌리지 않는다).
    allow_drag_out: bool,
    /// 이 창의 HWND(없으면 0). 시작 확인 실행이 앞 창이 됐는지 잴 때 쓴다.
    hwnd: isize,
    /// 시작 확인 중 한 번이라도 앞 창(GetForegroundWindow)이 됐는지. 보고서의 foreground.
    smoke_foreground: bool,
    /// 시작 확인(--smoke-seconds)에서 센 다시 그리기 원인(파일:줄 → 그림 수).
    smoke_repaint_causes: std::collections::BTreeMap<String, u32>,
    /// 시작 확인에서 가장 오래 걸린 그림(ms).
    smoke_slowest_frame_ms: f64,
    /// 시작 확인에서 50ms보다 오래 걸린 그림 수.
    smoke_slow_frames: u32,
    /// 시작 확인에서 두 패널 목록이 처음 다 읽힌 때(시작부터 ms).
    smoke_loaded_ms: Option<u64>,
    /// 진단("tipwindow" 견본): 따로 뜬 툴팁 창의 화면 자리(왼쪽, 위, 너비, 높이). 창이 실제로 만들어졌는지 본다.
    smoke_tip_window: Option<[f32; 4]>,
    /// 마지막으로 창에 적용한 불투명도(0.0~1.0). WindowAlpha가 바뀌면 다시 적용한다.
    applied_alpha: f32,
    /// 폴더 크기 자동 표시 작업 줄과 일꾼(둘)을 띄웠는지.
    folder_size_queue: pane_chrome::FolderSizeQueue,
    folder_size_workers: bool,
    /// 업데이트 피드 받기(C# UpdateHttpClient). 테스트에서 바꿔 끼운다.
    http_get: fn(&str, usize) -> Result<ndr::update::HttpResponse, ndr::update::HttpError>,
    /// 새로 고친 목록이 오면 커서를 둘 이름(C# FocusPaneItemByName).
    pending_focus: [Option<String>; 2],
    /// 압축 작업이 끝나면 커서를 둘 (패널, 만든 압축 파일).
    archive_focus: Option<(usize, PathBuf)>,
    /// 인수와 함께 열기의 ShellExecuteExW(테스트에서 실제 프로그램을 띄우지 않도록 교체 가능).
    shell_runner: fn(&Path, &str, &Path) -> Result<(), String>,
    /// 프로그램을 바로 실행하고 끝 코드를 기다린다(외부 복사 프로그램). 테스트에서 바꿔 끼운다.
    program_runner: fn(&Path, &[String], &Path) -> Result<i32, String>,
    /// 저장 대화 상자(제목, 처음 폴더, 이름, 형식, 기본 확장자). 테스트에서 바꿔 끼운다.
    save_dialog: ndr::file_dialogs::SaveFileFn,
    /// F12 단축키 창(원본 fShortcut), 하나만.
    shortcut_window: Option<shortcut_window::ShortcutWindow>,
    /// Clipboard text writer (replaceable so tests never touch the user's real clipboard; C# `_setClipboardText`).
    clipboard_writer: fn(&str) -> Result<(), String>,
    /// 원본 miEmptyRecycleBinClick: SHEmptyRecycleBinW(창, NULL, 0)(셸 확인 창). 시험은 가짜.
    shell_recycle_emptier: fn() -> Result<(), String>,
    /// 복사·잘라내기의 CF_HDROP 클립보드 쓰기(native::clipboard_files). 시험은 실제 클립보드를 건드리지 않는다.
    file_clipboard: fn(&[PathBuf], bool) -> Result<(), String>,
    /// 기다리는 refresh 안에서 받은 목록의 네트워크 공유 인증 요청(다음 poll이 처리).
    auth_pending: Vec<usize>,
    /// 원본 단축키 표가 받은 숫자판 글쇠(원본 이름, Shift). raw_input_hook이 넣고 keyboard가 실행한다.
    pending_numpad: Vec<(String, bool)>,
    /// Apps 글쇠를 눌렀다(지금 패널의 목록 오른쪽 단추 메뉴, 원본 FUN_00484e8c의 글쇠 경로).
    pending_context_key: bool,
    /// 마지막으로 SetCurrentDirectoryW로 맞춘 폴더(`sync_current_directory`).
    current_directory: Option<PathBuf>,
    /// 폴더 선택 대화 상자(제목, 처음 폴더). 테스트에서 바꿔 끼운다.
    folder_picker: fn(&str, Option<&Path>) -> Option<PathBuf>,
    /// 원본 miNetwork의 네트워크 폴더 고르기(SHBrowseForFolderW). 시험에서는 바꿔 끼운다.
    network_picker: fn(&str, Option<&Path>) -> Option<PathBuf>,
    /// 원본 TColorDialog(ChooseColorW). COLORREF를 받고 돌려준다. 테스트는 가짜로 바꾼다.
    color_picker: fn(u32) -> Option<u32>,
    /// 원본 TFontDialog(ChooseFontW): (이름, 포인트, 모양) → 고른 (이름, 포인트, 모양). 테스트는 가짜로 바꾼다.
    font_picker: fn(&str, f32, u8) -> Option<(String, f32, u8)>,
    /// 즐겨찾기·이동 기록 드롭다운(C# ContextMenu), 하나만.
    popup: Option<favorites_dialog::PopupMenu>,
    /// 처리되지 않은 Alt+글자(원본 TToolBar CM_DIALOGCHAR): 이번 장면에 그 접근 글자의 메뉴 단추 펼침을 연다.
    menu_access: Option<char>,
    /// 메뉴(메뉴 줄 펼침·오른쪽 단추 메뉴)가 열린 동안 친 글자: 그 접근 글자(&X)의 항목을 고른다(원본 Windows 메뉴).
    menu_letter: Option<char>,
    /// 이번 장면에 글자를 받을 수 있던 메뉴 항목(접근 글자, 위젯 id)과 지난 장면의 목록. 같은 글자가 둘 이상이면
    /// Windows 메뉴처럼 글자를 칠 때마다 그 항목들 사이로 선택(초점)만 옮긴다(Enter로 실행).
    menu_targets: std::cell::RefCell<Vec<(char, egui::Id)>>,
    menu_targets_seen: Vec<(char, egui::Id)>,
    /// 다음에 그릴 때 초점을 줄 메뉴 항목.
    menu_focus: std::cell::Cell<Option<egui::Id>>,
    /// 조용한 업데이트 확인에서 찾은 새 버전의 내려받기 주소(원본 tbNewUpdateAvailable이 보이는 동안).
    update_available: Option<String>,
    /// C# _lastCommandWorkingDirectory(명령 실행의 작업 폴더).
    last_command_directory: Option<PathBuf>,
    /// 네트워크 드라이브·탐색기 메뉴·셸 동사(테스트에서 바꿔 끼운다).
    system: tools_dialog::SystemHooks,
    /// 열기 대화 상자(제목, 처음 폴더, 이름, 형식). 테스트에서 바꿔 끼운다.
    open_dialog: ndr::file_dialogs::OpenFileFn,
    /// 디스크 복사 창의 장치 목록·읽기·쓰기·복사(테스트는 실제 장치를 건드리지 않도록 바꿔 끼운다).
    disk: disk_copy_dialog::DiskHooks,
    /// 원격 창의 서버 연결과 여러 파일 열기 대화 상자(테스트에서 바꿔 끼운다).
    remote_hooks: remote_window::RemoteHooks,
    /// 드라이브 단추(읽은 때, (루트, 도움말)).
    drive_cache: Option<(Instant, Vec<(PathBuf, String)>)>,
    /// 패널 목록의 드라이브 목록 캐시(원본 TNXDriveList 자리, 목록 스레드가 함께 씀).
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
                        app.lifecycle = Some(lifecycle);
                        app.lifecycle_events = Some(events);
                    }
                    Err(error) => app.status = error,
                }
            }
        }
        // C# 시작: CheckForUpdatesOnStartup이고 피드가 있으면 업데이트 확인(시작 확인 실행은 빼고).
        // 원본 FUN_00441dd8의 조용한 확인([+0x54] == 1): 상자 없이, 새 버전이면 메뉴 줄의 "새 업데이트" 단추만 보인다.
        // 원본 처음 값 [Operation] checkupdate=1. 피드(UpdateManifestUrl)가 없으면 확인하지 않는다.
        if app.options.smoke_seconds.is_none() && app.settings.flag("CheckForUpdatesOnStartup", true) && !app.settings.text("UpdateManifestUrl", "").trim().is_empty() {
            app.start_update_check(true);
        }
        app
    }
    fn from_context(ctx: &egui::Context, options: Options) -> Self {
        // 두 번 누르기 간격은 Windows 설정(GetDoubleClickTime, 처음 값 500ms)을 따른다(egui 처음 값 0.3초는 더 짧다).
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
        // C# 시작 폴더: 명령줄 → "항상 다음 폴더로 시작"(StartPathEnabled이고 StartPath가 있으면) → 마지막 폴더(LeftPath).
        // 두 번째 값은 명령줄·시작 폴더처럼 정해 준 폴더인지(그러면 저장한 고른 탭보다 그 폴더가 먼저).
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
        // 원본 FUN_004295b0: /pane:single·dual이 설정(dualview)보다 앞서고, /dir2는 두 패널 보기일 때만 오른쪽을 연다.
        let dual = options.dual.unwrap_or_else(|| settings.flag("DualPaneMode", true));
        let right_explicit = options.right.clone().or_else(|| options.dir2.clone().filter(|_| dual));
        let (right, right_fixed) = start_path("Right", &right_explicit);
        // C# ValidStartPath: 있는 폴더면 그대로, 아니면 사용자 폴더, 그것도 없으면 "::drives::"(내 PC).
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
            per_file_icon_types: HashMap::new(),
            overlay_icons: HashMap::new(),
            overlay_pending: HashSet::new(),
            overlay_tx: None,
            image_requests: HashSet::new(),
            tree_cache: HashMap::new(),
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
    /// 원본·C# 판 "투명 메뉴 사용"(UseMenuAlpha, 처음 값 켜짐): 켜져 있으면 이 프레임의 메뉴 층을 MENU_ALPHA로 흐리게.
    fn fade_menus(&self, ctx: &egui::Context) {
        fade_foreground_layers(ctx, if self.settings.flag("UseMenuAlpha", true) { MENU_ALPHA } else { 1.0 });
    }
    /// 창 상태 기록(시험할 수 있게 따로). 최소화 중이면 아무것도 바꾸지 않고, 보통 창일 때만 위치·크기를 바꾼다.
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
            // 원본 TForm.Width·Height, C# Window.Width·Height: 테두리를 포함한 바깥 크기.
            self.window_bounds = Some((outer.min, outer.size()));
        }
    }
    fn save(&mut self) {
        let _ = self.try_save();
    }
    /// save와 같고 실패하면 그 글을 돌려준다(C# SettingsStore.Save의 예외처럼 부르는 쪽이 따로 알릴 때).
    fn try_save(&mut self) -> Result<(), String> {
        if self.profile_load_error {
            self.status=self.tr("기존 설정을 읽지 못해 덮어쓰지 않았습니다. 설정 가져오기를 사용하세요.","Existing settings could not be read and were preserved. Use Import settings to recover.").into();
            return Err(self.status.clone());
        }
        // C# Closing: WindowState가 Normal이면 WindowLeft·Top·Width·Height, 늘 WindowMaximized와 HasWindowBounds = true.
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
        // Theme은 설정 값 그대로(Dark/Light/System/Black/Classic/Gray). self.dark는 그 결과일 뿐이다.
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
    /// 원본 FormCloseQuery(0x42a798): 진행 중인 작업이 있으면 MessageBoxW(STILL_HAVE_OPERATION, TITLE_INFO,
    /// MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_TASKMODAL). 예면 작업을 멈추고 끝나는 대로 닫는다. 아니요면 닫지 않는다.
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
        // C# ErrorResolver: 복사·이동·삭제는 오류마다 묻는다(완전 삭제는 끝에 한꺼번에).
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

    /// 원본 FUN_00486694(위치 바뀜): 로컬 폴더면 TNXShellChangeNotify.SetFolder(그 폴더)·SetActive(1), 압축·FTP·내 PC면 SetActive(0).
    /// 바뀜 알림을 받은 뒤 Delay(1500ms)가 지나면 OnChange = FUN_00487c8c → FUN_00487ce8(패널, 1, 1, "")(같은 폴더 다시 읽기).
    fn update_folder_watches(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        for index in 0..self.panes.len() {
            let pane = &self.panes[index];
            let wanted = (self.watch_folders && pane.archive.is_none() && pane.remote.is_none() && !panes::is_drives(&pane.path)).then(|| pane.path.clone());
            if pane.watch.as_ref().map(|(p, _)| p) != wanted.as_ref() {
                let pane = &mut self.panes[index];
                if let Some((_, stop)) = pane.watch.take() {
                    stop.store(true, Ordering::Relaxed);
                }
                pane.change_due = None;
                if let Some(path) = wanted {
                    let stop = Arc::new(AtomicBool::new(false));
                    let (tx, watched, repaint) = (self.async_tx.clone(), path.clone(), ctx.clone());
                    native::spawn_folder_watch(path.clone(), stop.clone(), move || {
                        let _ = tx.send(AsyncResult::FolderChanged { pane: index, path: watched.clone() });
                        repaint.request_repaint();
                    });
                    pane.watch = Some((path, stop));
                }
            }
            if let Some((due, _)) = self.panes[index].change_due {
                if due <= now {
                    if self.panes[index].loading {
                        self.panes[index].change_due = Some((now + FOLDER_CHANGE_DELAY, 0));
                    } else {
                        self.panes[index].change_due = None;
                        self.refresh(index);
                    }
                } else {
                    ctx.request_repaint_after(due - now);
                }
            }
        }
    }

    /// 원본 ThreadNotification(0x7b3830): 처음 알림이면 Delay(1500ms) 타이머를 켜고, 타이머가 도는 동안 또 오면 20번까지 다시 늦춘다.
    pub(super) fn folder_changed(&mut self, index: usize, path: &Path) {
        let Some(pane) = self.panes.get_mut(index) else { return };
        if pane.watch.as_ref().is_none_or(|(watched, _)| watched != path) {
            return;
        }
        let now = Instant::now();
        pane.change_due = match pane.change_due {
            None => Some((now + FOLDER_CHANGE_DELAY, 0)),
            Some((_, restarts)) if restarts < 20 => Some((now + FOLDER_CHANGE_DELAY, restarts + 1)),
            kept => kept,
        };
    }
    /// 목록 스레드의 결과 하나를 패널에 넣는다(poll과 원본처럼 기다리는 refresh가 함께 쓴다).
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
            // 원본 새로 고침(FUN_00487ce8, 새로 고침·작업 뒤 등 대부분 (1, 1)로 부름)은 같은 폴더를 다시 읽을 때 표시(CopySelection →
            // RestoreSelection, 이름으로), 센 폴더 크기(종류 1이고 +0x68인 항목의 +0x20/+0x24/+0x60을 이름으로 되돌림), 맨 위 위치
            // (SetScrollTop)를 지키고, 커서 이름이 없어졌으면 옛 번호(목록보다 크면 드라이브를 뺀 마지막 항목)에 둔다. 잘라냄 표시(+100)는
            // 새 항목이라 없어진다. C# RefreshPaneAsync는 모두 비우므로 스킨 테마만.
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
                    // 원본 새로 고침(FUN_00487ce8): 같은 폴더를 다시 읽으면 커서는 같은 이름의 항목에 남는다(다른 패널로 복사한 뒤 등).
                    let location = location_key(pane);
                    if pane.listed.as_deref() == Some(location.as_str()) && self.pending_focus[listing.pane].is_none() {
                        self.pending_focus[listing.pane] = pane.entries.get(pane.cursor).map(|e| e.name.clone());
                    }
                    pane.listed = Some(location);
                    if !pane.filter.is_empty() {
                        entries.retain(|e| e.drive || ndr::parity::matches_filter(&pane.filter, &e.name));
                    }
                    // 원본 GetList(0x45a34c)·압축 목록(0x45ecd0)·SFTP 목록(0x45bb0c), C# ListLocal: 루트가 아니면 맨 앞에 "..".
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
            // 새 목록은 정렬한 뒤 맨 위에서 시작한다(sort_pane은 커서 항목을 따라가므로 다시 맞춘다).
            pane.cursor = 0;
            if self.settings.flag("ShowFolderDiskInfoBar", true) {
                self.update_disk_capacity(listing.pane);
            }
            let pane = &mut self.panes[listing.pane];
            let mut focused = false;
            // 같은 위치를 다시 읽으면서 따로 고른 이름이 없으면(원본 FUN_00487ce8 param_2, param_4 빈 글) 원본은 SetItemIndex 뒤
            // SetScrollTop(옛 위치)로 되돌린다: 자세히·미리 보기는 옛 위치를 지키고, 열로 흐르는 목록만 AdjustPage + SetItemIndex.
            let restore_top = kept.as_ref().is_some_and(|k| !k.4);
            if let Some(name) = self.pending_focus[listing.pane].take() {
                if let Some(row) = pane.entries.iter().position(|e| ndr::names::equals_ignore_case(&e.name, &name)) {
                    pane.cursor = row;
                    pane.selected.clear();
                    // 원본 FUN_00486694 → SetItemIndex: 커서 항목이 보이게 넘긴다(쪽 단위면 그 쪽). 고친 것: 전에는 첫 쪽에 머물렀다.
                    pane.reveal = !restore_top;
                    focused = true;
                }
            }
            // 새 위치: 원본은 언제나 SetItemIndex(0 또는 고른 이름)라 그 항목이 보인다. 고친 것: 자세히 보기는 전 폴더의 세로 위치가
            // 남아 첫 항목이 안 보일 수 있었다. (원본은 그 앞에 전 폴더의 커서 번호로 AdjustPage를 하지만, 전 폴더에 따라 화면이
            // 달라지는 부작용이라 옮기지 않았다.)
            if kept.is_none() {
                pane.reveal = true;
            }
            if let Some((marks, sizes, first_column, old_cursor, explicit)) = kept {
                if !focused && !explicit && !pane.entries.is_empty() {
                    let drives = pane.entries.iter().filter(|e| e.drive).count();
                    pane.cursor = if old_cursor < pane.entries.len() { old_cursor } else { (pane.entries.len() - drives).saturating_sub(1) };
                }
                for name in &marks {
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
                // 원본 FileListChanged(0x7d79c8) → AdjustPage(옛 커서 번호, 목록 길이로 자름), 그 뒤 FUN_00487ce8이 SetItemIndex.
                // 자세히·미리 보기는 그 뒤 SetScrollTop(옛 위치)로 되돌리므로 열로 흐르는 목록만 위치가 바뀐다(adjust 두 번째 값).
                if !pane.entries.is_empty() {
                    pane.adjust = Some((old_cursor.min(pane.entries.len() - 1), true));
                }
            }
        }
    }
    /// 목록을 넣은 뒤의 일(네트워크 공유 인증 등). poll이 부른다.
    fn after_listings(&mut self, auth_panes: Vec<usize>, ctx: &egui::Context) {
        // C# RefreshPaneAsync(allowNetworkAuthPrompt): UNC 목록이 자격 증명 실패면 연결 창 → 성공하면 한 번 더 읽기, 실패면 "네트워크 연결" 경고.
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
                    // 원본 0x439593: wsprintf(ARC_CHANGEVOL, 볼륨 이름) → TfMsgBox(TITLE_ERROR, "", 글, 0x31). 확인(1)이면 다시 시도.
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
                AsyncResult::Tree { path, directories } => {
                    self.tree_cache.insert(path, directories);
                }
                AsyncResult::FolderChanged { pane, path } => self.folder_changed(pane, &path),
                AsyncResult::ProtectedItems(run) => self.protected_step(run),
                AsyncResult::AttributesApplied(run) => self.attribute_step(run),
                AsyncResult::SplitDone { source, outcome, cancelled } => self.split_done(source, outcome, cancelled),
                AsyncResult::JoinDone { count, outcome, cancelled } => self.join_done(count, outcome, cancelled),
                AsyncResult::SearchInPane { pane, root, outcome, cancelled } => self.search_done(pane, root, outcome, cancelled),
                AsyncResult::CompareDone(run, outcome, cancelled) => self.compare_done(run, outcome, cancelled),
                AsyncResult::SyncDone(outcome, cancelled) => self.sync_done(outcome, cancelled),
                AsyncResult::CleanupSearchFolder(folder) => self.cleanup_search_folder(folder),
                AsyncResult::CleanupScanned(found) => self.cleanup_scanned(found),
                AsyncResult::Warning(text) => self.system_box("NDR", &text, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {}),
                AsyncResult::MergeSources(output, sources) => self.merge_sources_ready(output, sources),
                AsyncResult::MergeDone(count, outcome) => self.merge_done(count, outcome),
                AsyncResult::ArchiveStatus(text) => self.status = self.archive_error_text(&text),
                AsyncResult::RemoteCommand { index, command, host, result } => self.remote_command_done(index, command, host, result),
                AsyncResult::ExternalCopy { result, status, clear_clipboard } => self.external_copy_done(result, status, clear_clipboard),
                AsyncResult::ArchiveEntriesOpened { pane, archive, password, opened, failures, navigate_archives } => self.archive_entries_opened(pane, archive, password, opened, failures, navigate_archives),
                AsyncResult::ArchivePassword { archive, reply } => self.password_request = Some(PasswordRequest { archive, reply }),
                AsyncResult::TextSource { path, edit } => self.text_source_ready(path, edit),
                AsyncResult::FolderSizeRow { pane, generation, path, text, bytes } => {
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
            }
        }
    }
}
/// C# RestorePaneTabs: 저장한 탭 가운데 있는 폴더만 남기고, 저장한 고른 탭(SelectedLeftTab, 원래 번호)이 남아 있으면 그 탭을 고른다.
/// 남은 탭이 없으면 시작 폴더 하나. 차이: C#은 탭이 있으면 명령줄·시작 폴더를 쓰지 않지만, NDR은 정해 준 폴더(fixed)면
/// 그 폴더의 탭을 고르고 없으면 맨 앞에 더한다(전부터의 NDR 동작). 돌려주는 값 = (탭 목록, 고른 탭 번호).
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

/// 원본 usemenualpha(설정 +0xb6): 메뉴를 그릴 때(FUN_0042e518) 메뉴 창에 WS_EX_LAYERED + 알파 0xF0(240/255).
/// C# 판 MainMenu·ContextMenu.Opacity = 0.94.
pub(super) const MENU_ALPHA: f32 = 240.0 / 255.0;
/// 이 프레임에 흐리게 하지 않을 앞쪽 층 id 목록(ctx 임시 자료).
const OPAQUE_LAYERS: &str = "ndr-opaque-foreground-layers";

/// 앞쪽(Foreground) 층에 그리지만 메뉴가 아닌 것(egui::Modal 창 등)을 이 프레임의 투명 메뉴 처리에서 뺀다.
pub(super) fn keep_opaque(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<egui::Id>>(egui::Id::new(OPAQUE_LAYERS)).push(id));
}

/// 원본 창(VCL TForm)은 제목 줄을 끌어 옮길 수 있다. egui::Modal은 가운데에 고정되므로, 가운데에서 끈 만큼(ctx 임시 자료) 옮긴 자리에 둔다.
/// 새 창(새 id)은 원본 poOwnerFormCenter·poMainFormCenter처럼 가운데에서 시작한다.
pub(super) fn movable_modal(ctx: &egui::Context, id: egui::Id) -> egui::Modal {
    let offset = ctx.data(|d| d.get_temp::<egui::Vec2>(id.with("ndr-drag-offset"))).unwrap_or_default();
    egui::Modal::new(id).area(egui::Modal::default_area(id).anchor(egui::Align2::CENTER_CENTER, offset))
}

/// 제목 줄 칸(`rect`, 닫기 단추는 빼고 넘긴다)을 끌면 그 창(`movable_modal`의 id)을 옮긴다.
pub(super) fn drag_title(ui: &egui::Ui, id: egui::Id, rect: egui::Rect) {
    let response = ui.interact(rect, id.with("ndr-title-drag"), egui::Sense::drag());
    if response.dragged() {
        let delta = response.drag_delta();
        let ctx = ui.ctx();
        // 창이 화면 밖으로 다 나가지 않게: 제목 줄 가운데가 화면 안에 남을 만큼만 옮긴다.
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

/// 제목 줄 한 줄(굵은 제목 + 오른쪽 닫기 단추). 줄을 끌면 창이 움직인다. 닫기 단추를 누르면 true.
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

/// 펼침 목록(egui ComboBox)의 목록도 앞쪽 층의 Popup이지만 메뉴가 아니다. 원본·C# 판은 메뉴만 투명하므로
/// 목록 층(egui: 단추 id.with("popup"))을 이 프레임의 투명 메뉴 처리에서 뺀다. ComboBox는 show_ui 대신 이것으로 띄운다.
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

/// 이 프레임에 그린 앞쪽(Foreground) 층 = 메뉴·오른쪽 단추 메뉴·펼침 목록의 모든 모양에 alpha를 곱한다
/// (egui 메뉴는 따로 된 창이 아니어서 그린 뒤 층째로 흐리게 한다). 앞쪽 층에 그리는 메뉴가 아닌 것(egui::Modal로 띄운 원본 창·
/// 메시지 상자·진행 창, 패널 알림 글)은 `keep_opaque`로 알린 층이라 그대로 둔다. alpha가 1이면 알린 목록만 비운다.
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

/// 파일 목록 글꼴 묶음 이름(원본·C# 판 ListFont). 목록을 그리는 동안만 Body·Button 글꼴로 쓴다.
pub(super) const LIST_FONT_FAMILY: &str = "list";
/// 목록 밖 글 크기(화소). C# 판은 ListFontSize를 목록에만 쓰므로 나머지는 처음 값 12 그대로.
pub(super) const BASE_TEXT_SIZE: f32 = 12.0;

/// 원본 ListFont 모양의 밑줄(4)·취소선(8)을 목록 글에 입힌다(굵게·기울임은 "list" 묶음의 글꼴 파일로 정한다).
pub(super) fn list_text(text: impl Into<String>, style: u8) -> RichText {
    let text = RichText::new(text);
    let text = if style & 4 != 0 { text.underline() } else { text };
    if style & 8 != 0 { text.strikethrough() } else { text }
}

/// 앱 글꼴: egui 기본에 맑은 고딕(한글)을 앞에 넣고, 원본 WinMain(0x403568)처럼 Vista 이상의 기본 글꼴 Segoe UI를 맨 앞에 둔다
/// (원본 창은 모두 ParentFont라 Application DefaultFont = Segoe UI 9pt = 12화소, 한글은 Windows 글꼴 연결로 맑은 고딕).
/// list_font(글꼴 파일, TTC 번호)가 있으면 "list" 묶음 = 그 글꼴 → 맑은 고딕 → 기본. 없으면 "list" 묶음은 보통 글꼴과 같다.
/// list_bold(원본 ListFont 모양의 굵게)면 "list" 묶음의 한글은 맑은 고딕 Bold(malgunbd.ttf, 있으면)로 그린다.
pub(super) fn set_app_fonts(ctx: &egui::Context, list_font: Option<(Vec<u8>, u32)>, list_bold: bool) {
    let mut fonts = egui::FontDefinitions::default();
    if let Ok(data) = std::fs::read("C:/Windows/Fonts/malgun.ttf") {
        fonts.font_data.insert("Korean".into(), Arc::new(egui::FontData::from_owned(data)));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(family).or_default().insert(0, "Korean".into());
        }
    }
    let mut korean_first = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    if list_bold {
        if let Ok(data) = std::fs::read("C:/Windows/Fonts/malgunbd.ttf") {
            fonts.font_data.insert("KoreanBold".into(), Arc::new(egui::FontData::from_owned(data)));
            korean_first.insert(0, "KoreanBold".into());
        }
    }
    if let Ok(data) = std::fs::read("C:/Windows/Fonts/segoeui.ttf") {
        fonts.font_data.insert("UI".into(), Arc::new(egui::FontData::from_owned(data)));
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "UI".into());
    }
    let list = match list_font {
        Some((data, index)) => {
            let mut data = egui::FontData::from_owned(data);
            data.index = index;
            fonts.font_data.insert("ListFont".into(), Arc::new(data));
            let mut list = korean_first;
            list.insert(0, "ListFont".into());
            list
        }
        None => fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default(),
    };
    fonts.families.insert(egui::FontFamily::Name(LIST_FONT_FAMILY.into()), list);
    // NXLabel 마크업 <b>(원본 TNXLabel: 그 글꼴의 굵은 꼴): Segoe UI Bold → 맑은 고딕 Bold → 보통 글꼴. 파일이 없으면 보통 글꼴.
    let mut bold = fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, file) in [("KoreanBoldUI", "C:/Windows/Fonts/malgunbd.ttf"), ("UIBold", "C:/Windows/Fonts/segoeuib.ttf")] {
        if let Ok(data) = std::fs::read(file) {
            fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_owned(data)));
            bold.insert(0, name.into());
        }
    }
    fonts.families.insert(egui::FontFamily::Name(BOLD_FONT_FAMILY.into()), bold);
    ctx.set_fonts(fonts);
}

/// 굵은 글 묶음 이름(NXLabel 마크업 <b>).
pub(super) const BOLD_FONT_FAMILY: &str = "bold";

impl eframe::App for App {
    /// egui-winit은 Ctrl+C·Ctrl+X·Ctrl+V(Ctrl+Insert·Shift+Delete·Shift+Insert)를 글쇠 사건 대신 Copy·Cut·Paste로 바꾸고,
    /// 클립보드에 글이 없으면 붙여넣기는 버린다. 글 입력 칸에 초점이 없으면 원본 단축키(miEditCopy·miEditCut·miEditPaste)가
    /// 받도록 글쇠 사건으로 되돌린다.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
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
        // 왼쪽 단추 누름을 Windows 두 번 누르기 기준으로 센다(목록이 그려지지 않는 그림의 누름도 빠지지 않게).
        let _ = panes::windows_double_press(ctx);
        // 고친 것(사용자 지적: 최대화로 시작할 때 깜박임): winit의 with_maximized는 첫 그림 전에 ShowWindow(SW_MAXIMIZE)로 빈 창을
        // 보였다. main.rs가 작업 영역 크기의 숨은 창으로 만들고, 첫 그림이 보인 뒤(두 번째 그림) 저장한 보통 크기를 되돌릴 크기로 두고 최대화한다.
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
        // C# Closing: 보통 창이면 위치·크기를 저장하고 최대화 여부를 따로 둔다. 시작 확인(화면 밖 창)은 기록하지 않는다.
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
        // C# DiskCopyWindow OnClosing: 디스크 작업 중에는 닫지 않는다(이미지 만들기는 취소 요청).
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
        // 원본 메뉴 줄 바탕(tbMainMenuAdvancedCustomDraw): [Menu] mmainback.
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
        // 원본 TfMain: 메뉴 줄(tbMainMenu), 도구 모음(pToolbar), 두 패널, 기능키 줄(bbFunctions). 상태 줄은 없다.
        if let Some(skin) = skin_now {
            // 원본 pToolbar(alTop): 높이 tbheight(검정 51), 배경 그림과 그림 단추(toolbar.rs skin_toolbar).
            if self.settings.flag("ShowToolBar", true) {
                let height = skin.positive_integer("Toolbar", "tbheight", 51) as f32;
                egui::TopBottomPanel::top("toolbar").exact_height(height).frame(egui::Frame::NONE).show_separator_line(false).show(ctx, |ui| self.skin_toolbar(ui, ctx, &skin));
            }
            // 원본 bbFunctions(DFM Height 20): F2~F8 칸(user_commands.rs skin_function_key_bar). 끄면 아무 줄도 없다.
            // NDR 상태 글은 원본에 없으므로(원본 창에는 상태 줄이 없음) 목록 자리를 줄이지 않고 기능키 줄 위에 잠깐 겹쳐 보인다.
            let function_bar = self.settings.flag("ShowFunctionBar", true);
            if function_bar {
                egui::TopBottomPanel::bottom("functionbar").exact_height(20.0).frame(egui::Frame::NONE).show_separator_line(false).show(ctx, |ui| self.skin_function_key_bar(ui, ctx, &skin));
            }
            let _ = self.skin_status_overlay(ctx, if function_bar { 20.0 } else { 0.0 });
        }
        if self.show_results {
            // 원본 pBottom Height 100(btnFTPShowDetail로 22 ↔ 100), splBottom으로 높이를 바꾼다.
            let panel = egui::TopBottomPanel::bottom(if self.results_collapsed { "ftp-results-collapsed" } else { "ftp-results" });
            let panel = if self.results_collapsed { panel.resizable(false).exact_height(24.0) } else { panel.resizable(true).default_height(100.0) };
            panel.show(ctx, |ui| self.ftp_results_ui(ui));
        }
        // 원본 폴더 보기(Alt+F3)는 패널마다 목록 왼쪽에 그린다(pane_ui → pane_tree_ui).
        // 놓을 곳 찾기: 이번 화면에 그리는 패널과 폴더 줄만(한 패널 보기면 숨은 패널은 없다).
        self.pane_rects = [egui::Rect::NOTHING; 2];
        self.folder_rows.clear();
        // 원본 TfMain: 패널(틀)은 pToolbar(alTop)와 bbFunctions(alBottom) 사이를 틈 없이 채운다(FUN_0043d7c0: 패널 너비 = 창 너비 −
        // 보이는 VertSplitter·pVertBar 너비). 고친 것: 전에는 egui 기본 패널 여백(8)과 틈이 들어가 위 13·아래 9·가운데 16화소가 더 비었다.
        egui::CentralPanel::default().frame(egui::Frame::central_panel(&ctx.style()).inner_margin(0.0)).show(ctx, |ui| {
            if self.dual {
                // 두 패널 사이: 원본 pVertBar(DFM Width 18, alRight) 또는 막대를 숨기면 VertSplitter(Width 4, clBlack). 끌면 너비(LeftPaneWidth %, 20~80),
                // 두 번 누르면 같은 너비(원본 pmiFrameWidthSame).
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
                // 원본 pmVertBar(pVertBarContextPopup, 단추 밖 오른쪽 단추): 양쪽 너비를 같게 / 왼쪽 너비 => 오른쪽 너비 /
                // 왼쪽 너비 <= 오른쪽 너비. 뒤의 둘(pmiFrameWidth 0x43d960)은 50:50으로 두고 창 너비를 그쪽 패널 너비의 두 배로 바꾼다.
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
                // 고친 것: 패널마다 제 자리로 잘라 그린다(좁게 끌면 주소 줄·정보 줄·목록이 옆 패널·세로 막대 위로 넘쳐 겹쳤다).
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
        // 모달 진행 창은 그 아래 창(예: 청소 옵션 창의 사용자 지정 검색) 위에 그린다.
        self.modal_progress_ui(ctx);
        self.msgbox_ui(ctx);
        self.pane_message_ui(ctx);
        self.fade_menus(ctx);
        if let Some(seconds) = self.options.smoke_seconds {
            self.shot_step(ctx);
            // 시작 확인은 다른 창 뒤에서 돌아야 한다(앞 창이 되면 보고서에 남기고 배포 스크립트가 멈춘다).
            if self.hwnd != 0 && native::is_foreground(self.hwnd) {
                self.smoke_foreground = true;
            }
            // 진단: 이번 그림을 다시 그리게 한 곳(egui RepaintCause, 파일:줄). 가만히 있을 때 계속 다시 그리는지 찾는 데 쓴다.
            let causes: Vec<String> = ctx.repaint_causes().iter().map(|c| c.to_string()).collect();
            for cause in causes {
                *self.smoke_repaint_causes.entry(cause).or_insert(0) += 1;
            }
            // 진단: 가장 오래 걸린 그림(ms, 첫 그림 3개는 글꼴·아이콘 준비라 뺀다).
            if self.frames > 3 {
                let ms = frame_started.elapsed().as_secs_f64() * 1000.0;
                self.smoke_slowest_frame_ms = self.smoke_slowest_frame_ms.max(ms);
                if ms > 50.0 {
                    self.smoke_slow_frames += 1;
                }
            }
            // 진단: 시작부터 두 패널 목록이 처음 다 읽힐 때까지(ms).
            if self.smoke_loaded_ms.is_none() && self.panes.iter().all(|p| !p.loading) {
                self.smoke_loaded_ms = Some(self.started.elapsed().as_millis() as u64);
            }
            if self.started.elapsed() > Duration::from_secs(seconds) {
                if let Some(path) = &self.options.report {
                    // leftCount/rightCount = 목록 항목(끝에 덧붙인 드라이브 항목은 leftDrives/rightDrives로 따로).
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
