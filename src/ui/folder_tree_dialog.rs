//! 원본 TfFolderTree(폴더 선택 창, ClientWidth 657 × ClientHeight 627, FUN_00423e00 생성·FUN_0040ca90 열기·FUN_0042473c ShowModal)와
//! 그 안의 TNXFolderTreeView(가로 트리: 단계마다 칸, 첫 자식은 부모와 같은 줄 오른쪽).
//! tbMainMenu(0,0 657×22: 폴더·트리·즐겨찾기·바탕 화면·문서·그림·음악·비디오·폴더 찾기), pTop(0,22 657×29: lAddress(3,7),
//! cbAddress(49,4 449×21), btnOk(511,4 70×22, Default), btnCancel(587,4 70×22)), pMain(0,51: pDrives(1,1 43폭), Shape1(44), TreeView(45,1)),
//! bbFunctions(0,608 657×19).
//! 차이(HANDOFF): 폴더·드라이브 그림은 원본 ImageList 대신 그린 모양. 드라이브 단추 그림 없음. 즐겨찾기·폴더 찾기·이름 바꾸기·새 폴더·삭제는 아직.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::folder_tree::{self, ScanOptions, Tree};
use ndr::msgbox::MB_ICONASTERISK;
use std::fs;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

/// 원본 nexusfile.col 폴더 트리 색(FUN_00423e00 → TreeView+0x2e8): 바탕, 줄, 글, 고른 줄, 열린 폴더, 조상 글.
#[derive(Clone, Copy, Debug)]
pub(super) struct TreeColors {
    pub back: Color32,
    pub line: Color32,
    pub fore: Color32,
    pub focused: Color32,
    pub opened: Color32,
    pub hint: Color32,
}

/// 훑기(TfTreeScan 창): 끝나면 트리를 보낸다.
pub(super) struct TreeScan {
    pub cancel: Arc<AtomicBool>,
    pub count: Arc<AtomicUsize>,
    pub current: Arc<Mutex<String>>,
    receiver: mpsc::Receiver<Tree>,
    /// None = 드라이브 전체(Refresh), Some(경로) = 그 항목 아래만(PartRefresh·SetPath).
    part: Option<String>,
    /// 끝나면 고를 경로.
    then_path: Option<String>,
}

pub(super) struct FolderTreeForm {
    pub tree: Tree,
    /// 지금 드라이브 뿌리("C:\", +0x340).
    pub drive: String,
    /// pDrives 단추: (뿌리, 이름).
    pub drives: Vec<(PathBuf, String)>,
    pub focused: Option<usize>,
    pub top_row: usize,
    pub scroll_x: f32,
    pub address: String,
    /// 빠른 찾기 글(+0x2f8).
    pub find: String,
    /// 드라이브마다 열린 폴더(+0x348, 빨간 글).
    pub opened: Vec<String>,
    pub scan: Option<TreeScan>,
    pub options: ScanOptions,
    pub column: f32,
    pub show_icon: bool,
    pub tree_folder: PathBuf,
    pub in_root: bool,
    pub dirty: bool,
    /// 목록(트리)에 글쇠 초점. 경로 칸을 누르면 false.
    pub tree_focus: bool,
    pub first_frame: bool,
    /// 지난 화면의 보이는 줄 수(+0x2d8).
    pub rows_visible: usize,
    /// 경로의 트리 뿌리(드라이브 뿌리, SystemHooks.tree_root). 시험은 가짜 뿌리.
    pub root_of: fn(&str) -> String,
}

/// 창이 App에 맡기는 일.
pub(super) enum FolderTreeAction {
    /// btnOkClick·TreeViewDblClick → FUN_00424a68(경로).
    Accept(String),
    Cancel,
    /// miPropertiesClick 0x4253ac: ShowFileProperties(고른 폴더).
    Properties(String),
}

fn color_of(settings: &ndr::settings::Settings, key: &str) -> Color32 {
    let text = settings.values.get("NexusFileColorMapValues").and_then(|m| m.get(key)).and_then(|v| v.as_str()).map(str::to_owned);
    let fallback = ndr::styles::DEFAULT_ELEMENTS.iter().find(|(k, _)| *k == key).map(|(_, v)| v.to_string()).unwrap_or_default();
    let rgba = text.as_deref().and_then(ndr::styles::parse_color).or_else(|| ndr::styles::parse_color(&fallback)).unwrap_or([0, 0, 0, 255]);
    Color32::from_rgb(rgba[0], rgba[1], rgba[2])
}

impl TreeColors {
    pub(super) fn read(settings: &ndr::settings::Settings) -> Self {
        Self {
            back: color_of(settings, "NXTBACK"),
            line: color_of(settings, "NXTLINE"),
            fore: color_of(settings, "NXTFORE"),
            focused: color_of(settings, "NXTFOCUSED"),
            opened: color_of(settings, "NXTOPENED"),
            hint: color_of(settings, "NXTHINT"),
        }
    }
}

/// "C:\..." → "C:\".
pub(super) fn drive_root(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() >= 2 && chars[1] == ':' { format!("{}:\\", chars[0].to_ascii_uppercase()) } else { path.to_owned() }
}

impl FolderTreeForm {
    /// FUN_00423e00: 설정(숨김 폴더 tshowhidden, 칸 너비 titemwidth, 아이콘 tshowicon, 깊이 tfullrefreshlevel, 제외 경로 +
    /// Windows\winsxs·assembly·boot, 트리 파일 폴더 = 설정 폴더\treedata).
    pub(super) fn new(settings: &ndr::settings::Settings, profile: &Path, drives: Vec<(PathBuf, String)>, opened: Vec<String>) -> Self {
        let mut excluded = settings.strings("FolderTreeExcludedPaths");
        if let Some(windows) = std::env::var_os("SystemRoot").map(PathBuf::from) {
            for name in ["winsxs", "assembly", "boot"] {
                excluded.push(windows.join(name).to_string_lossy().into_owned());
            }
        }
        let depth = settings.values.get("FolderTreeDepth").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        Self {
            tree: Tree::default(),
            drive: String::new(),
            drives,
            focused: None,
            top_row: 0,
            scroll_x: 0.0,
            address: String::new(),
            find: String::new(),
            opened,
            scan: None,
            options: ScanOptions { show_hidden: settings.flag("ShowHiddenFoldersInTree", false), excluded, max_level: Some(depth) },
            column: settings.values.get("FolderTreeItemWidth").and_then(|v| v.as_f64()).unwrap_or(130.0) as f32,
            show_icon: settings.flag("ShowFolderTreeIcons", true),
            tree_folder: profile.join("treedata"),
            in_root: settings.flag("SaveFolderTreeState", false),
            dirty: false,
            tree_focus: true,
            first_frame: true,
            rows_visible: 20,
            root_of: drive_root,
        }
    }

    fn tree_file(&self, root: &str) -> PathBuf {
        let path = Path::new(root);
        let kind = ndr::native::drive_type_name(path);
        let serial = ndr::native::volume_serial(path).map(|s| format!("{s:08X}")).unwrap_or_else(|| root.chars().next().unwrap_or('X').to_string());
        folder_tree::tree_file_name(path, self.in_root, matches!(kind, "Removable" | "CDRom"), &serial, &self.tree_folder)
    }

    /// SaveTree 0x7ea380(FUN_007e5330). 실패는 조용히 넘긴다(원본 CreateFileW 실패).
    pub(super) fn save(&mut self) {
        if self.tree.is_empty() || !self.dirty {
            return;
        }
        let file = self.tree_file(&self.drive);
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if folder_tree::save_file(&self.tree, &file).is_ok() {
            self.dirty = false;
        }
    }

    fn opened_path(&self) -> Option<String> {
        self.opened.iter().find(|p| (self.root_of)(p).eq_ignore_ascii_case(&self.drive)).cloned()
    }

    /// SetDrive 0x7ea418: 다른 드라이브면 트리 파일이 있으면 읽고(뿌리 이름이 다르면 고쳐 저장), 없으면 전체 훑기(Refresh).
    /// 읽었으면 그 드라이브의 열린 폴더(없으면 뿌리)를 고른다.
    pub(super) fn set_drive(&mut self, root: &str) {
        let root = (self.root_of)(root);
        if root.eq_ignore_ascii_case(&self.drive) || !Path::new(&root).exists() {
            return;
        }
        self.save();
        self.drive = root.clone();
        self.tree = Tree::default();
        self.focused = None;
        self.top_row = 0;
        self.scroll_x = 0.0;
        self.find.clear();
        match folder_tree::load_file(&self.tree_file(&root)) {
            Some(mut tree) => {
                if let Some(r) = tree.root.filter(|r| !tree.items[*r].name.eq_ignore_ascii_case(&root)) {
                    tree.items[r].name = root.clone();
                    self.dirty = true;
                }
                self.tree = tree;
                match self.opened_path() {
                    Some(path) => self.set_path(&path),
                    None => self.focus(self.tree.root, 0),
                }
            }
            None => self.start_scan(None, self.opened_path()),
        }
    }

    /// Refresh 0x7e9d94(part None) / PartRefresh 0x7ea044·SetPath의 훑기(part Some): TfTreeScan 창을 띄우고 따로 훑는다.
    fn start_scan(&mut self, part: Option<String>, then_path: Option<String>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));
        let current = Arc::new(Mutex::new(String::new()));
        let (tx, rx) = mpsc::channel();
        let (flag, counter, shown) = (cancel.clone(), count.clone(), current.clone());
        let mut options = self.options.clone();
        let root = PathBuf::from(&self.drive);
        let target = part.clone();
        if part.is_some() {
            // PartRefresh: +0x360 = 0(깊이 제한 없음).
            options.max_level = None;
        }
        thread::spawn(move || {
            let mut progress = |path: &Path| {
                counter.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut text) = shown.lock() {
                    *text = path.to_string_lossy().into_owned();
                }
            };
            let tree = match target {
                None => folder_tree::build(&root, &options, &flag, &mut progress),
                Some(path) => Tree::from_lines(&folder_tree::scan(Path::new(&path), &options, &flag, &mut progress)).unwrap_or_default(),
            };
            let _ = tx.send(tree);
        });
        self.scan = Some(TreeScan { cancel, count, current, receiver: rx, part, then_path });
    }

    /// 훑기가 끝났으면 트리에 넣는다. 끝났으면 true.
    pub(super) fn poll_scan(&mut self) -> bool {
        let Some(scan) = &self.scan else { return false };
        let Ok(result) = scan.receiver.try_recv() else { return false };
        let scan = self.scan.take().expect("scan");
        match scan.part {
            None => {
                self.tree = result;
                if self.tree.is_empty() {
                    self.tree = Tree::with_root(&self.drive);
                }
            }
            Some(path) => {
                // Refresh(항목) 0x7ea140: 항목을 지우고 훑은 트리를 그 부모 아래에 붙인다.
                if let Some(item) = self.tree.find_path(&path).filter(|i| Some(*i) != self.tree.root) {
                    let parent = self.tree.items[item].parent;
                    self.tree.remove(item);
                    if let (Some(parent), false) = (parent, result.is_empty()) {
                        if let Some(added) = self.tree.graft(parent, &result) {
                            self.tree.items[added].expanded = true;
                            self.tree.assign_rows();
                        }
                    }
                } else if let (Some(root), false) = (self.tree.root, result.is_empty()) {
                    self.tree.graft(root, &result);
                }
            }
        }
        self.dirty = true;
        match scan.then_path {
            Some(path) if self.tree.find_path(&path).is_some() => {
                let item = self.tree.find_path(&path);
                self.focus(item, 1);
            }
            _ => {
                let root = self.tree.root;
                self.focus(root, 0);
            }
        }
        true
    }

    /// SetPath 0x7e91dc: 폴더가 있으면 드라이브를 맞추고, 트리에 있으면 고른다. 없으면 트리에 있는 가장 가까운 조상부터 훑어 붙인 뒤 고른다.
    pub(super) fn set_path(&mut self, path: &str) {
        let path = path.trim_end_matches('\\');
        let path = if path.len() == 2 && path.ends_with(':') { format!("{path}\\") } else { path.to_owned() };
        if !Path::new(&path).is_dir() {
            return;
        }
        let root = (self.root_of)(&path);
        if !root.eq_ignore_ascii_case(&self.drive) {
            self.set_drive(&root);
            if let Some(scan) = self.scan.as_mut() {
                scan.then_path = Some(path);
                return;
            }
        }
        if let Some(item) = self.tree.find_path(&path) {
            if self.tree.full_path(item).trim_end_matches('\\').eq_ignore_ascii_case(path.trim_end_matches('\\')) {
                return self.focus(Some(item), 1);
            }
        }
        // 트리에 없는 폴더: 단계를 하나씩 올라 찾은 조상(없으면 뿌리) 경로부터 훑는다.
        let mut at = PathBuf::from(&path);
        let mut found = None;
        while let Some(parent) = at.parent().map(Path::to_path_buf) {
            at = parent;
            if let Some(item) = self.tree.find_path(&at.to_string_lossy()) {
                if self.tree.full_path(item).trim_end_matches('\\').eq_ignore_ascii_case(at.to_string_lossy().trim_end_matches('\\')) {
                    found = Some(item);
                    break;
                }
            }
        }
        match found {
            Some(item) if Some(item) != self.tree.root => {
                let from = self.tree.full_path(item);
                self.start_scan(Some(from), Some(path));
            }
            _ => {
                // 뿌리 아래 첫 단계 폴더부터 훑는다.
                let first = Path::new(&path).ancestors().find(|a| a.parent().is_some_and(|p| p.parent().is_none())).map(|a| a.to_string_lossy().into_owned());
                match first {
                    Some(first) => self.start_scan(Some(first), Some(path)),
                    None => {
                        let root = self.tree.root;
                        self.focus(root, 0);
                    }
                }
            }
        }
    }

    /// SetFocused 0x7e77a8 + AlignFocused(mode): 보이게 펼치고, 줄을 맞추고(0 위, 1 가운데, 2 아래), 경로 칸(TreeViewChange) = 그 경로.
    pub(super) fn focus(&mut self, item: Option<usize>, mode: u8) {
        let Some(item) = item else { return };
        if !self.tree.is_visible(item) {
            self.tree.make_visible(item);
        }
        self.focused = Some(item);
        self.align(mode);
        self.address = self.tree.full_path(item);
    }

    /// AlignFocused 0x7e6390.
    fn align(&mut self, mode: u8) {
        let Some(item) = self.focused else { return };
        let row = self.tree.items[item].row;
        let visible = self.rows_visible.max(1);
        self.top_row = match mode {
            0 => row,
            1 if row > visible - 1 => row - visible / 2,
            1 => 0,
            _ if row >= visible => row + 1 - visible,
            _ => 0,
        };
    }

    fn after_move(&mut self, before: Option<usize>) {
        if let Some(item) = self.focused {
            if self.focused != before {
                self.address = self.tree.full_path(item);
                self.find.clear();
            }
        }
    }

    /// ArrowKey 0x7e7c3c(+ KeyDown 0x7e8284: Alt가 없을 때 0x21~0x28, '\').
    pub(super) fn arrow_key(&mut self, key: egui::Key, shift: bool, ctrl: bool) {
        let (Some(focused), Some(root)) = (self.focused, self.tree.root) else { return };
        let visible = self.rows_visible.max(1);
        let tree = &self.tree;
        let row_of = |i: usize| tree.items[i].row;
        let last = tree.last.unwrap_or(root);
        // 고른 줄이 화면 밖이면 가운데로 맞추기만 한다.
        if row_of(focused) < self.top_row || row_of(focused) > self.top_row + visible {
            self.align(1);
            return;
        }
        let item = &tree.items[focused];
        let mut target = focused;
        let mut mode = None;
        match key {
            egui::Key::ArrowLeft => {
                if shift && item.child.is_some() && item.expanded && focused != root {
                    self.tree.set_expanded(focused, false, false);
                    return;
                }
                if focused != root {
                    target = if ctrl { tree.prev_visible(focused).unwrap_or(root) } else { item.parent.unwrap_or(root) };
                    if row_of(target) < self.top_row {
                        mode = Some(0);
                    }
                }
            }
            egui::Key::ArrowRight => {
                if shift && !item.expanded && item.child.is_some() {
                    self.tree.set_expanded(focused, true, false);
                    return;
                }
                if focused != last {
                    if let Some(next) = tree.next_visible(focused) {
                        target = next;
                        if row_of(next) >= self.top_row + visible {
                            mode = Some(2);
                        }
                    }
                }
            }
            egui::Key::ArrowUp => {
                if focused != root {
                    if shift {
                        if let Some(before) = tree.before(focused) {
                            target = before;
                        }
                    } else {
                        let level = item.level;
                        let mut at = tree.prev_visible(focused);
                        while let Some(i) = at {
                            if tree.items[i].level == level {
                                break;
                            }
                            at = tree.prev_visible(i);
                        }
                        target = at.or(item.parent).unwrap_or(root);
                    }
                    if row_of(target) < self.top_row {
                        mode = Some(0);
                    }
                }
            }
            egui::Key::ArrowDown => {
                if focused != last {
                    if shift {
                        if let Some(next) = item.next {
                            target = next;
                        }
                    } else {
                        let level = item.level;
                        let mut at = tree.next_visible(focused);
                        while let Some(i) = at {
                            if tree.items[i].level == level {
                                break;
                            }
                            at = tree.next_visible(i);
                        }
                        target = at.or_else(|| tree.next_visible(focused)).unwrap_or(focused);
                    }
                    if row_of(target) >= self.top_row + visible {
                        mode = Some(2);
                    }
                }
            }
            egui::Key::PageUp => {
                if focused != root {
                    let top = tree.row_starts.get(self.top_row).copied().unwrap_or(root);
                    if focused == top {
                        let mut at = focused;
                        while at != root && row_of(top).saturating_sub(row_of(at)) != visible {
                            match tree.prev_visible(at) {
                                Some(p) => at = p,
                                None => break,
                            }
                        }
                        target = at;
                    } else {
                        target = top;
                    }
                    mode = Some(0);
                }
            }
            egui::Key::PageDown => {
                if focused != last {
                    let goal = if self.top_row + visible - 1 == row_of(focused) { self.top_row + visible * 2 - 1 } else { self.top_row + visible - 1 };
                    let mut at = focused;
                    while at != last && row_of(at) != goal {
                        match tree.next_visible(at) {
                            Some(n) => at = n,
                            None => break,
                        }
                    }
                    target = at;
                    mode = Some(2);
                }
            }
            egui::Key::End => {
                if focused != last {
                    target = last;
                    mode = Some(2);
                }
            }
            egui::Key::Home | egui::Key::Backslash => {
                if focused != root {
                    target = root;
                    mode = Some(0);
                }
            }
            _ => return,
        }
        let before = self.focused;
        self.focused = Some(target);
        if let Some(mode) = mode {
            self.align(mode);
        }
        self.after_move(before);
    }

    /// QuickSearch(글) 0x7e8b3c/FindNext 0x7e8ab4: 접힌 항목도(+0x34c = 1) 찾아 고른다.
    pub(super) fn quick_search(&mut self, include_current: bool) {
        let (Some(focused), false) = (self.focused, self.find.is_empty()) else { return };
        if let Some(found) = self.tree.quick_search(focused, &self.find, include_current, true) {
            let find = self.find.clone();
            self.focus(Some(found), 1);
            self.find = find;
        }
    }

    /// Expand/Collapse(+ 재귀) 0x7e4544/0x7e4498 — OneLevelExpandClick·RecurseExpandClick·숫자판 +/−. 접어서 고른 항목이 숨으면
    /// 그 항목을 고른다(MouseDown의 HasAsParent).
    pub(super) fn expand(&mut self, item: usize, expanded: bool, recurse: bool) {
        self.tree.set_expanded(item, expanded, recurse);
        if let Some(focused) = self.focused {
            if !expanded && self.tree.has_as_parent(focused, item) {
                self.focus(Some(item), 1);
            }
        }
        self.dirty = true;
    }

    /// TreeExpandClick 0x4254f0: 항목이 셋 이상이면 뿌리의 자식마다 재귀로 펼치거나 접는다. 접으면 뿌리를 고르고, 펼치면 가운데로.
    pub(super) fn expand_all(&mut self, expanded: bool) {
        let Some(root) = self.tree.root else { return };
        if self.tree.count() < 3 {
            return;
        }
        let children: Vec<usize> = self.tree.children(root).collect();
        for c in children {
            self.tree.set_expanded(c, expanded, true);
        }
        if expanded { self.align(1) } else { self.focus(Some(root), 0) }
        self.dirty = true;
    }

    /// LevelExpandClick 0x425604: 고른 항목과 그 자손을, 단계가 (고른 단계 + n)보다 얕으면 펼치고 아니면 접는다.
    pub(super) fn expand_levels(&mut self, levels: usize) {
        let Some(focused) = self.focused else { return };
        let base = self.tree.items[focused].level;
        let mut at = Some(focused);
        while let Some(i) = at {
            self.tree.items[i].expanded = self.tree.items[i].level < base + levels;
            at = self.tree.next_any(i).filter(|n| self.tree.items[*n].level > base);
        }
        self.tree.assign_rows();
        self.dirty = true;
    }
}

const ROW_HEIGHT: f32 = 17.0;

/// 그리기 치수(CalculateParams 0x7e61e4): 줄 높이(+0x2c4), 칸 너비(+0x2d0), 이름 칸(+0x2c8 = 칸 − 12), 뿌리 칸(+0x2cc = 뿌리 이름 + 0x24, 빈 이름 0x32).
struct Metrics {
    row: f32,
    column: f32,
    name: f32,
    root: f32,
}

fn text_width(ui: &egui::Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size().x
}

/// FUN_007e6544: `max` 안에 드는 글자까지 자르고 마지막 글자를 '~'로.
fn fit_with_tilde(ui: &egui::Ui, text: &str, max: f32, font: &egui::FontId) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut fit = 0;
    for n in 1..=chars.len() {
        if text_width(ui, &chars[..n].iter().collect::<String>(), font) > max {
            break;
        }
        fit = n;
    }
    if fit == 0 {
        return "~".into();
    }
    let mut out: String = chars[..fit - 1].iter().collect();
    out.push('~');
    out
}

fn item_x(form: &FolderTreeForm, metrics: &Metrics, item: usize) -> f32 {
    let level = form.tree.items[item].level;
    if Some(item) == form.tree.root { -form.scroll_x } else { (level as f32 - 1.0) * metrics.column + metrics.root - form.scroll_x }
}

/// 원본 그림 대신 그리는 16×16 폴더(ImageList 0번).
fn draw_folder_icon(painter: &egui::Painter, at: egui::Pos2, selected: bool) {
    let body = egui::Rect::from_min_size(at + egui::vec2(1.0, 4.0), egui::vec2(14.0, 10.0));
    let tab = egui::Rect::from_min_size(at + egui::vec2(1.0, 2.0), egui::vec2(6.0, 3.0));
    let fill = if selected { Color32::from_rgb(0xF0, 0xC8, 0x50) } else { Color32::from_rgb(0xE8, 0xBE, 0x48) };
    painter.rect_filled(tab, 1.0, fill);
    painter.rect_filled(body, 1.0, fill);
    painter.rect_stroke(body, 1.0, egui::Stroke::new(1.0, Color32::from_rgb(0xB0, 0x88, 0x20)), egui::StrokeKind::Inside);
}

fn draw_drive_icon(painter: &egui::Painter, at: egui::Pos2) {
    let body = egui::Rect::from_min_size(at + egui::vec2(1.0, 5.0), egui::vec2(14.0, 7.0));
    painter.rect_filled(body, 1.0, Color32::from_rgb(0xC8, 0xC8, 0xC8));
    painter.rect_stroke(body, 1.0, egui::Stroke::new(1.0, Color32::from_rgb(0x70, 0x70, 0x70)), egui::StrokeKind::Inside);
    painter.rect_filled(egui::Rect::from_min_size(at + egui::vec2(11.0, 9.0), egui::vec2(2.0, 1.0)), 0.0, Color32::from_rgb(0x30, 0xC0, 0x30));
}

/// TNXFolderTreeView Paint 0x7e7224 + DrawItem 0x7e6590.
fn paint_tree(ui: &egui::Ui, rect: egui::Rect, form: &FolderTreeForm, colors: &TreeColors, root_label: &str, metrics: &Metrics, font: &egui::FontId) {
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 0.0, colors.back);
    let Some(root) = form.tree.root else { return };
    // 바탕 글: 드라이브 이름(100) 오른쪽 위 + 볼륨 이름(30), 바탕과 줄 색 사이 색.
    let mark = colors.back.lerp_to_gamma(colors.line, 0.25);
    let drive_text = form.tree.items[root].name.trim_end_matches('\\').to_owned();
    let big = egui::FontId::proportional(110.0);
    let galley = painter.layout_no_wrap(drive_text, big, mark);
    let drive_height = galley.size().y;
    painter.galley(egui::pos2(rect.right() - galley.size().x - 20.0, rect.top() + 6.0), galley, mark);
    if !root_label.is_empty() {
        let label = painter.layout_no_wrap(root_label.to_owned(), egui::FontId::proportional(36.0), mark);
        painter.galley(egui::pos2(rect.right() - label.size().x - 20.0, rect.top() + drive_height + 6.0), label, mark);
    }
    let line = egui::Stroke::new(1.0, colors.line);
    let at = |x: f32, y: f32| rect.min + egui::vec2(x, y);
    let y_of = |i: usize| (form.tree.items[i].row as f32 - form.top_row as f32) * metrics.row;
    let half = (metrics.row / 2.0).floor();
    let Some(&top) = form.tree.row_starts.get(form.top_row) else { return };
    // 위 줄보다 위에서 내려오는 조상 형제 줄.
    let mut ancestor = top;
    while form.tree.items[ancestor].level > 1 {
        let Some(parent) = form.tree.items[ancestor].parent else { break };
        ancestor = parent;
        if let Some(next) = form.tree.items[ancestor].next {
            let next_row = form.tree.items[next].row;
            if form.tree.items[top].row < next_row {
                let x = item_x(form, metrics, ancestor) - 8.0;
                let end = ((next_row - form.tree.items[top].row) as f32 * metrics.row + half).min(rect.height());
                painter.line_segment([at(x, 0.0), at(x, end)], line);
            }
        }
    }
    let mut cursor = Some(top);
    while let Some(i) = cursor {
        if form.tree.items[i].row > form.top_row + form.rows_visible + 1 {
            break;
        }
        draw_item(ui, &painter, rect, form, colors, metrics, font, i, item_x(form, metrics, i), y_of(i));
        cursor = form.tree.next_visible(i);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_item(ui: &egui::Ui, painter: &egui::Painter, rect: egui::Rect, form: &FolderTreeForm, colors: &TreeColors, metrics: &Metrics, font: &egui::FontId, i: usize, x: f32, y: f32) {
    let tree = &form.tree;
    let item = &tree.items[i];
    let is_root = Some(i) == tree.root;
    let at = |dx: f32, dy: f32| rect.min + egui::vec2(dx, dy);
    let half = (metrics.row / 2.0).floor();
    let line = egui::Stroke::new(1.0, colors.line);
    if !is_root {
        if !tree.is_first_child(i) {
            painter.line_segment([at(x - 8.0, y + half), at(x - 8.0, y)], line);
        }
        let reach = if item.child.is_none() && tree.is_first_child(i) { 5.0 } else { 0.0 };
        painter.line_segment([at(x - 1.0, y + half), at(x - 8.0 - reach, y + half)], line);
        if let Some(next) = item.next {
            let end = (tree.items[next].row as f32 - item.row as f32) * metrics.row + y + 1.0;
            painter.line_segment([at(x - 8.0, y + half), at(x - 8.0, end)], line);
        }
        if item.child.is_some() {
            // 9×9 단추(+0x308 접힘 "+", +0x310 펼침 "−").
            let top = ((metrics.row - 9.0) / 2.0).floor();
            let button = egui::Rect::from_min_size(at(x - 12.0, y + top), egui::vec2(9.0, 9.0));
            painter.rect_filled(button, 0.0, colors.back);
            painter.rect_stroke(button, 0.0, egui::Stroke::new(1.0, colors.line), egui::StrokeKind::Inside);
            let c = button.center();
            painter.line_segment([egui::pos2(c.x - 2.0, c.y), egui::pos2(c.x + 2.5, c.y)], egui::Stroke::new(1.0, colors.fore));
            if !item.expanded {
                painter.line_segment([egui::pos2(c.x, c.y - 2.0), egui::pos2(c.x, c.y + 2.5)], egui::Stroke::new(1.0, colors.fore));
            }
        }
    }
    let mut text = if is_root { item.name.clone() } else { item.name.clone() };
    let mut width = text_width(ui, &text, font);
    if item.child.is_some() && item.expanded {
        if !is_root {
            painter.line_segment([at(x + 24.0, y + half), at(x + metrics.name, y + half)], line);
        }
        let max = metrics.name - (if form.show_icon { 16.0 } else { 0.0 } + 8.0);
        if max < width {
            text = fit_with_tilde(ui, &text, max, font);
            width = max;
        }
    }
    let icon_shift = if form.show_icon { 18.0 } else { 2.0 };
    let mut text_rect = egui::Rect::from_min_max(at(x + icon_shift, y + 1.0), at(x + icon_shift + width + 1.0, y + metrics.row - 1.0));
    let icon_top = ((metrics.row - 16.0) / 2.0).floor();
    let full = tree.full_path(i);
    let opened = form.opened.iter().any(|p| p.trim_end_matches('\\').eq_ignore_ascii_case(full.trim_end_matches('\\')));
    if form.show_icon {
        if is_root {
            draw_drive_icon(painter, at(x + 2.0, y + icon_top));
        } else {
            draw_folder_icon(painter, at(x, y + icon_top), form.focused == Some(i));
        }
    }
    if is_root {
        text_rect = text_rect.translate(egui::vec2(2.0, 0.0));
    }
    if form.focused == Some(i) {
        // 고른 줄: 바탕(열린 폴더면 NXTOPENED, 아니면 NXTFOCUSED) + 바탕 글(NXTBACK) + 어두운 테두리.
        let fill = if opened { colors.opened } else { colors.focused };
        painter.rect_filled(text_rect, 0.0, fill);
        painter.text(text_rect.left_center(), egui::Align2::LEFT_CENTER, &text, font.clone(), colors.back);
        let frame = Color32::from_rgb(fill.r().saturating_sub(0x46), fill.g().saturating_sub(0x46), fill.b().saturating_sub(0x46));
        painter.rect_stroke(text_rect.expand2(egui::vec2(1.0, 1.0)), 0.0, egui::Stroke::new(1.0, frame), egui::StrokeKind::Inside);
        // 이 줄에 없는 조상 이름을 제 칸에(NXTHINT) — 고른 줄이 경로처럼 읽힌다.
        let mut ancestor = i;
        while tree.items[ancestor].level > 0 {
            let Some(parent) = tree.items[ancestor].parent else { break };
            ancestor = parent;
            if tree.items[ancestor].row == item.row {
                continue;
            }
            let ax = item_x(form, metrics, ancestor);
            let is_top = Some(ancestor) == tree.root;
            if !is_top && form.show_icon {
                draw_folder_icon(painter, at(ax, y + icon_top), false);
            }
            let left = ax + if form.show_icon { 16.0 } else { 0.0 } + if is_top { 2.0 } else { 0.0 };
            let right = if is_top { ax + metrics.root - 16.0 } else { ax + metrics.name - 6.0 };
            let clip = egui::Rect::from_min_max(at(left, y + 1.0), at(right.max(left), y + metrics.row - 1.0));
            painter.with_clip_rect(clip.intersect(rect)).text(clip.left_center(), egui::Align2::LEFT_CENTER, &tree.items[ancestor].name, font.clone(), colors.hint);
        }
    } else {
        let color = if opened { colors.opened } else { colors.fore };
        painter.text(text_rect.left_center(), egui::Align2::LEFT_CENTER, &text, font.clone(), color);
    }
}

/// GetItemAt 0x7e8e84 / GetButtonAt 0x7e8de4: 누른 줄의 맨 왼쪽 항목부터 첫 자식 사슬을 본다.
fn hit(ui: &egui::Ui, form: &FolderTreeForm, metrics: &Metrics, font: &egui::FontId, local: egui::Vec2) -> (Option<usize>, Option<usize>) {
    let row = (local.y / metrics.row).floor() as isize + form.top_row as isize;
    if row < 0 {
        return (None, None);
    }
    let Some(&start) = form.tree.row_starts.get(row as usize) else { return (None, None) };
    let (px, mut item_hit, mut button_hit) = (local.x, None, None);
    let mut at = Some(start);
    while let Some(i) = at {
        if form.tree.items[i].row != row as usize {
            break;
        }
        let x = item_x(form, metrics, i);
        let item = &form.tree.items[i];
        if Some(i) == form.tree.root && form.scroll_x == 0.0 && row == 0 && px > 5.0 && px < 32.0 {
            item_hit = Some(i);
            break;
        }
        let width = text_width(ui, &item.name, font);
        let end = if item.child.is_some() && item.expanded && width > metrics.name { x + metrics.name } else { x + width + 19.0 };
        if px >= x && px <= end {
            item_hit = Some(i);
            break;
        }
        if Some(i) != form.tree.root && px >= x - 12.0 && px <= x - 3.0 {
            button_hit = Some(i);
        }
        at = item.child.filter(|_| item.expanded);
    }
    (item_hit, button_hit)
}

/// TNXButtonBar bbFunctions: "F2=이름 바꾸기;…" 칸(HeadColor 파랑 = 글쇠, 글 = 기능).
fn function_bar(ui: &mut egui::Ui, rect: egui::Rect, items: &[(String, String)]) -> Option<usize> {
    let mut clicked = None;
    if items.is_empty() {
        return None;
    }
    let width = rect.width() / items.len() as f32;
    let font = egui::FontId::proportional(12.0);
    for (n, (key, caption)) in items.iter().enumerate() {
        let cell = egui::Rect::from_min_size(rect.min + egui::vec2(width * n as f32, 0.0), egui::vec2(width, rect.height()));
        let response = ui.interact(cell, egui::Id::new(("ndr-tree-funcbar", n)), egui::Sense::click());
        let caption_color = if response.hovered() { Color32::RED } else { ui.visuals().text_color() };
        let key_galley = ui.painter().layout_no_wrap(key.clone(), font.clone(), Color32::from_rgb(0x40, 0x60, 0xFF));
        let key_width = key_galley.size().x;
        ui.painter().galley(cell.left_center() + egui::vec2(4.0, -key_galley.size().y / 2.0), key_galley, Color32::BLUE);
        ui.painter().with_clip_rect(cell).text(cell.left_center() + egui::vec2(8.0 + key_width, 0.0), egui::Align2::LEFT_CENTER, caption, font.clone(), caption_color);
        if response.clicked() {
            clicked = Some(n);
        }
    }
    clicked
}

/// 원본 TfFolderTree 창. 돌려준 일은 App이 처리한다. 창 안의 트리 움직임은 여기서 끝낸다.
pub(super) fn folder_tree_ui(ui: &mut egui::Ui, form: &mut FolderTreeForm, language: &compat::Language, colors: &TreeColors, size: egui::Vec2, input: bool) -> Option<FolderTreeAction> {
    let text = |key: &str, fallback: &str| language.raw(&format!("formFolderTree.{key}")).unwrap_or_else(|| fallback.to_owned());
    let plain = |key: &str, fallback: &str| super::favorites_dialog::access_key(&text(key, fallback)).0;
    let mut action = None;
    if form.poll_scan() {
        ui.ctx().request_repaint();
    }
    if form.scan.is_some() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    let scanning = form.scan.is_some();
    let (area, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let (w, h) = (size.x, size.y);
    let visuals = ui.visuals().clone();
    // tbMainMenu: 메뉴 단추(글).
    let menu_font = egui::FontId::proportional(12.0);
    let mut x = 2.0;
    let mut menu_action: Option<&'static str> = None;
    for (key, fallback, id) in [("ToolButton.tbFolder", "Fold&er", "folder"), ("ToolButton.tbTree", "&Tree", "tree")] {
        let caption = plain(key, fallback);
        let width = text_width(ui, &caption, &menu_font) + 14.0;
        ui.scope_builder(egui::UiBuilder::new().max_rect(at(x, 0.0, width, 22.0)), |ui| {
            ui.add_enabled_ui(!scanning, |ui| {
                ui.menu_button(caption.as_str(), |ui| {
                    let items: &[(&str, &str, &str)] = if id == "folder" {
                        &[("Menu.miProperties", "&E. Properties", "properties"), ("-", "", ""), ("Menu.miClose", "&C. Close", "close")]
                    } else {
                        &[
                            ("Menu.miExpand", "&E. Expand", "expand"),
                            ("Menu.miCollapse", "&C. Collapse", "collapse"),
                            ("-", "", ""),
                            ("Menu.miExpandRecursive", "&X. Expand All Subfolders", "expandRecursive"),
                            ("Menu.miCollapseRecursive", "&A. Collapse All Subfolders", "collapseRecursive"),
                            ("-", "", ""),
                            ("Menu.miExpandTree", "&T. Expand All", "expandTree"),
                            ("Menu.miCollapseTree", "&P. Collapse All", "collapseTree"),
                            ("-", "", ""),
                            ("Menu.miLevelExpand1", "&1 Level", "level1"),
                            ("Menu.miLevelExpand2", "&2 Level", "level2"),
                            ("Menu.miLevelExpand3", "&3 Level", "level3"),
                            ("Menu.miLevelExpand4", "&4 Level", "level4"),
                            ("Menu.miLevelExpand5", "&5 Level", "level5"),
                            ("-", "", ""),
                            ("Menu.miSubRefresh", "&S. Refresh Subfolders", "subRefresh"),
                            ("Menu.miTreeRefresh", "&F. Refresh All", "treeRefresh"),
                            ("-", "", ""),
                            ("Menu.miFindNext", "&N. Find Next", "findNext"),
                        ]
                    };
                    for (key, fallback, command) in items {
                        if *key == "-" {
                            ui.separator();
                            continue;
                        }
                        if ui.button(plain(key, fallback)).clicked() {
                            menu_action = Some(command);
                            ui.close();
                        }
                    }
                });
            });
        });
        x += width + 2.0;
    }
    ui.painter().hline(area.min.x..=area.max.x, area.min.y + 22.0, visuals.widgets.noninteractive.bg_stroke);
    // pTop: 경로 칸과 단추.
    ui.painter().text(at(3.0, 29.0, 40.0, 13.0).left_top(), egui::Align2::LEFT_TOP, plain("Caption.lAddress", "Path:"), egui::FontId::proportional(12.0), visuals.text_color());
    let address_id = egui::Id::new("ndr-folder-tree-address");
    let address = ui.put(at(49.0, 26.0, (w - 208.0).max(80.0), 21.0), egui::TextEdit::singleline(&mut form.address).id(address_id));
    if address.clicked() || address.has_focus() {
        form.tree_focus = false;
    }
    let address_enter = address.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let ok = ui.add_enabled_ui(!scanning, |ui| ui.put(at(w - 146.0, 26.0, 70.0, 22.0), egui::Button::new(plain("Caption.btnOk", "OK")))).inner.clicked();
    let cancel = ui.add_enabled_ui(!scanning, |ui| ui.put(at(w - 70.0, 26.0, 70.0, 22.0), egui::Button::new(plain("Caption.btnCancel", "Cancel")))).inner.clicked();
    // pMain.
    let main = at(0.0, 51.0, w, (h - 51.0 - 19.0).max(40.0));
    ui.painter().rect_stroke(main, 0.0, visuals.widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    let drives_rect = egui::Rect::from_min_size(main.min + egui::vec2(1.0, 1.0), egui::vec2(43.0, main.height() - 2.0));
    // pDrivesPaint 0x426800: 흰색 → F0F0F0 바탕.
    ui.painter().rect_filled(drives_rect, 0.0, Color32::from_rgb(0xF6, 0xF6, 0xF6));
    ui.painter().vline(drives_rect.right() + 0.5, drives_rect.y_range(), egui::Stroke::new(1.0, Color32::from_rgb(0xE4, 0xE4, 0xE4)));
    let mut picked_drive = None;
    for (n, (root, _)) in form.drives.iter().enumerate() {
        let caption = root.to_string_lossy().trim_end_matches('\\').to_owned();
        let button = egui::Rect::from_min_size(drives_rect.min + egui::vec2(1.0, 26.0 * n as f32), egui::vec2(41.0, 24.0));
        let down = (form.root_of)(&root.to_string_lossy()).eq_ignore_ascii_case(&form.drive);
        let response = ui.interact(button, egui::Id::new(("ndr-tree-drive", n)), egui::Sense::click());
        if down || response.hovered() {
            ui.painter().rect(button, 2.0, if down { Color32::from_rgb(0xD8, 0xE6, 0xF8) } else { Color32::from_rgb(0xEC, 0xF2, 0xFB) }, egui::Stroke::new(1.0, Color32::from_rgb(0x9C, 0xB4, 0xD8)), egui::StrokeKind::Inside);
        }
        ui.painter().text(button.center(), egui::Align2::CENTER_CENTER, caption, egui::FontId::proportional(12.0), Color32::BLACK);
        if response.clicked() && !scanning {
            picked_drive = Some(root.to_string_lossy().into_owned());
        }
    }
    let tree_rect = egui::Rect::from_min_max(egui::pos2(drives_rect.right() + 1.0, main.top() + 1.0), main.max - egui::vec2(1.0, 1.0));
    let font = egui::FontId::proportional(12.0);
    let root_name = form.tree.root.map(|r| form.tree.items[r].name.clone()).unwrap_or_default();
    let root_width = text_width(ui, &root_name, &font);
    let metrics = Metrics { row: ROW_HEIGHT, column: form.column.max(40.0), name: form.column.max(40.0) - 12.0, root: if root_name.is_empty() { 50.0 } else { root_width + 36.0 } };
    form.rows_visible = ((tree_rect.height() / metrics.row).floor() as usize).max(1);
    let root_label = form.drives.iter().find(|(r, _)| (form.root_of)(&r.to_string_lossy()).eq_ignore_ascii_case(&form.drive)).map(|(_, l)| l.clone()).unwrap_or_default();
    paint_tree(ui, tree_rect, form, colors, &root_label, &metrics, &font);
    let tree_response = ui.interact(tree_rect, egui::Id::new("ndr-folder-tree-view"), egui::Sense::click());
    if input && !scanning {
        if tree_response.clicked() || tree_response.double_clicked() || tree_response.secondary_clicked() {
            form.tree_focus = true;
            ui.memory_mut(|m| m.surrender_focus(address_id));
            if let Some(pos) = tree_response.interact_pointer_pos() {
                let (item, button) = hit(ui, form, &metrics, &font, pos - tree_rect.min);
                if let Some(item) = item {
                    let before = form.focused;
                    form.focused = Some(item);
                    if form.tree.items[item].row > form.top_row + form.rows_visible - 1 {
                        form.align(2);
                    }
                    form.after_move(before);
                    if tree_response.double_clicked() {
                        action = Some(FolderTreeAction::Accept(form.tree.full_path(item)));
                    }
                } else if let Some(button) = button {
                    let expanded = form.tree.items[button].expanded;
                    form.expand(button, !expanded, false);
                }
            }
        }
        let scroll = ui.input(|i| i.raw_scroll_delta);
        if tree_response.hovered() && scroll != egui::Vec2::ZERO {
            if scroll.y.abs() >= scroll.x.abs() {
                let rows = (form.tree.row_starts.len()).saturating_sub(1);
                let step = if scroll.y > 0.0 { -3isize } else { 3 };
                form.top_row = (form.top_row as isize + step).clamp(0, rows as isize) as usize;
            } else {
                form.scroll_x = (form.scroll_x - scroll.x).max(0.0);
            }
        }
    }
    // bbFunctions.
    let entries: Vec<(String, String)> = text("ButtonBar.bbFunctions", "F2=Rename;F3=Find Next;F5=Refresh;F6=Refresh Subfolders;F7=New Folder;F9=Properties")
        .split(';')
        .filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
        .collect();
    let bar = at(0.0, h - 19.0, w, 19.0);
    let bar_click = function_bar(ui, bar, &entries).filter(|_| !scanning && input);
    // TfTreeScan(547×102): lPath(8,19 531, 경로 줄임), lCount(8,43), btnCancel(244,70 75×25).
    let mut scan_cancel = false;
    if let Some(scan) = &form.scan {
        let window = egui::Rect::from_center_size(tree_rect.center(), egui::vec2(547.0, 128.0));
        ui.painter().rect(window, 2.0, visuals.window_fill(), visuals.window_stroke(), egui::StrokeKind::Inside);
        let caption = language.raw("formTreeScan.Form.Caption").unwrap_or_else(|| "Scanning...".into());
        ui.painter().text(window.left_top() + egui::vec2(8.0, 5.0), egui::Align2::LEFT_TOP, caption, egui::FontId::proportional(13.0), visuals.strong_text_color());
        let body = egui::Rect::from_min_size(window.min + egui::vec2(0.0, 26.0), egui::vec2(547.0, 102.0));
        let current = scan.current.lock().map(|c| c.clone()).unwrap_or_default();
        let shown = super::original_dialogs::path_ellipsis(ui, &current, 531.0, &font);
        ui.painter().text(body.min + egui::vec2(8.0, 19.0), egui::Align2::LEFT_TOP, shown, font.clone(), visuals.text_color());
        ui.painter().text(body.min + egui::vec2(8.0, 43.0), egui::Align2::LEFT_TOP, scan.count.load(Ordering::Relaxed).to_string(), font.clone(), visuals.text_color());
        let cancel_caption = super::favorites_dialog::access_key(&language.raw("formTreeScan.Caption.btnCancel").unwrap_or_else(|| "Cancel".into())).0;
        if ui.put(egui::Rect::from_min_size(body.min + egui::vec2(244.0, 70.0), egui::vec2(75.0, 25.0)), egui::Button::new(cancel_caption)).clicked() {
            scan_cancel = true;
        }
    }
    // 글쇠(창 KeyPreview → TntFormShortCut 0x425704 → TreeViewKeyDown 0x424c54 → ArrowKey).
    let mut keys: Vec<(egui::Key, egui::Modifiers)> = vec![];
    let mut typed = String::new();
    if input {
        ui.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => keys.push((*key, *modifiers)),
                    egui::Event::Text(t) => typed.push_str(t),
                    _ => {}
                }
            }
        });
    }
    if scanning {
        if scan_cancel || keys.iter().any(|(k, _)| *k == egui::Key::Escape) {
            if let Some(scan) = &form.scan {
                scan.cancel.store(true, Ordering::Relaxed);
            }
        }
        return None;
    }
    if let Some(drive) = picked_drive {
        form.set_drive(&drive);
    }
    let tree_keys = form.tree_focus && !address.has_focus();
    for (key, m) in keys {
        let plain_key = !m.alt && !m.ctrl;
        match key {
            egui::Key::Escape => action = Some(FolderTreeAction::Cancel),
            egui::Key::Enter if !m.alt => action = Some(FolderTreeAction::Accept(form.address.clone())),
            egui::Key::Enter if m.alt => menu_action = Some("properties"),
            egui::Key::F9 => menu_action = Some("properties"),
            egui::Key::F3 if plain_key => menu_action = Some("findNext"),
            egui::Key::L if m.ctrl && !m.alt => menu_action = Some("findNext"),
            egui::Key::F5 if plain_key => menu_action = Some("treeRefresh"),
            egui::Key::F6 if plain_key => menu_action = Some("subRefresh"),
            egui::Key::F if m.alt && !m.ctrl => menu_action = Some("treeRefresh"),
            egui::Key::S if m.alt && !m.ctrl => menu_action = Some("subRefresh"),
            egui::Key::Num1 | egui::Key::Num2 | egui::Key::Num3 | egui::Key::Num4 | egui::Key::Num5 if m.alt && !m.ctrl => {
                let n = match key { egui::Key::Num1 => "level1", egui::Key::Num2 => "level2", egui::Key::Num3 => "level3", egui::Key::Num4 => "level4", _ => "level5" };
                menu_action = Some(n);
            }
            // Alt+`: 경로 칸에 초점·모두 고르기.
            egui::Key::Backtick if m.alt => {
                ui.memory_mut(|mem| mem.request_focus(address_id));
                form.tree_focus = false;
            }
            _ if !tree_keys => {}
            egui::Key::ArrowRight | egui::Key::ArrowLeft if m.ctrl && m.alt => menu_action = Some(if key == egui::Key::ArrowRight { "expandTree" } else { "collapseTree" }),
            egui::Key::ArrowRight | egui::Key::ArrowLeft if m.ctrl && !m.alt && !m.shift => {
                menu_action = Some(if key == egui::Key::ArrowRight { "expandRecursive" } else { "collapseRecursive" })
            }
            egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::ArrowUp | egui::Key::ArrowDown | egui::Key::PageUp | egui::Key::PageDown | egui::Key::Home | egui::Key::End | egui::Key::Backslash if !m.alt => {
                form.arrow_key(key, m.shift, m.ctrl)
            }
            egui::Key::Plus if plain_key => menu_action = Some("expand"),
            egui::Key::Minus if plain_key => menu_action = Some("collapse"),
            // Shift+글자: 그 드라이브(SetDrive("%C:\")).
            k if m.shift && !m.ctrl && !m.alt && k.name().len() == 1 && k.name().chars().all(|c| c.is_ascii_alphabetic()) => {
                form.set_drive(&format!("{}:\\", k.name()));
                typed.clear();
            }
            _ => {}
        }
    }
    // WMChar 0x7e86d0: 빠른 찾기(대문자는 Shift+글자 = 드라이브라 받지 않는다).
    if tree_keys && !typed.is_empty() {
        for c in typed.chars() {
            if c.is_ascii_uppercase() {
                continue;
            }
            if c == '\u{8}' {
                form.find.pop();
            } else if (c as u32) >= 0x20 && !"\\/:*?\"<>|".contains(c) {
                form.find.push(c);
            } else {
                continue;
            }
            form.quick_search(true);
        }
    }
    if ui.input(|i| i.key_pressed(egui::Key::Backspace)) && tree_keys && !form.find.is_empty() {
        form.find.pop();
    }
    if let Some(n) = bar_click {
        // bbFunctionsClick 0x4258a0: 0 이름 바꾸기, 1 다음 찾기, 2 전체 최신, 3 하위만, 4 새 폴더, 5 등록정보.
        menu_action = match n { 1 => Some("findNext"), 2 => Some("treeRefresh"), 3 => Some("subRefresh"), 5 => Some("properties"), _ => None };
    }
    if let Some(command) = menu_action {
        let focused = form.focused;
        match command {
            "properties" => {
                if let Some(item) = focused {
                    action = Some(FolderTreeAction::Properties(form.tree.full_path(item)));
                }
            }
            "close" => action = Some(FolderTreeAction::Cancel),
            "expand" | "collapse" | "expandRecursive" | "collapseRecursive" => {
                if let Some(item) = focused {
                    form.expand(item, command.starts_with("expand"), command.ends_with("Recursive"));
                }
            }
            "expandTree" => form.expand_all(true),
            "collapseTree" => form.expand_all(false),
            "level1" | "level2" | "level3" | "level4" | "level5" => form.expand_levels(command[5..].parse().unwrap_or(1)),
            "findNext" => form.quick_search(false),
            "treeRefresh" => {
                let then = focused.map(|i| form.tree.full_path(i));
                form.start_scan(None, then);
            }
            "subRefresh" => {
                if let Some(item) = focused.filter(|i| Some(*i) != form.tree.root) {
                    let path = form.tree.full_path(item);
                    form.start_scan(Some(path.clone()), Some(path));
                } else if focused.is_some() {
                    let then = focused.map(|i| form.tree.full_path(i));
                    form.start_scan(None, then);
                }
            }
            _ => {}
        }
    }
    if ok || address_enter {
        action = Some(FolderTreeAction::Accept(form.address.clone()));
    }
    if cancel {
        action = Some(FolderTreeAction::Cancel);
    }
    form.first_frame = false;
    action
}

impl App {
    /// 원본 FUN_0040ca90(결과, 처음 경로, 제목): TfFolderTree를 만들어 SetPath(처음 경로) 뒤 ShowModal. 고른 경로(취소면 None).
    pub(super) fn folder_tree_dialog(&mut self, initial: &Path, caption: Option<String>, on_result: impl FnOnce(&mut App, Option<PathBuf>, &egui::Context) + 'static) {
        let drives: Vec<(PathBuf, String)> = (self.system.ready_drives)().into_iter().map(|root| {
            let label = ndr::native::drive_name(&root);
            (root, label)
        }).collect();
        let mut opened = vec![initial.to_string_lossy().into_owned()];
        for pane in &self.panes {
            let path = pane.path.to_string_lossy().into_owned();
            if !opened.iter().any(|p| (self.system.tree_root)(p).eq_ignore_ascii_case(&(self.system.tree_root)(&path))) {
                opened.push(path);
            }
        }
        let mut form = FolderTreeForm::new(&self.settings, &self.profile, drives, opened);
        form.root_of = self.system.tree_root;
        form.set_path(&initial.to_string_lossy());
        let title = caption.unwrap_or_else(|| super::favorites_dialog::access_key(&self.language_pack.text("formFolderTree.Form.Caption", "Folder Tree")).0);
        self.push_original_dialog(title, DialogKind::FolderTree(Box::new(form)), move |app, result, ctx| {
            let DialogResult::FolderTree(path) = result else { return };
            on_result(app, path, ctx)
        });
    }

    /// 창이 돌려준 일.
    pub(super) fn folder_tree_action(&mut self, action: FolderTreeAction, ctx: &egui::Context) {
        match action {
            FolderTreeAction::Cancel => {
                // TntFormClose 0x42643c: 트리를 저장하고 닫는다.
                if let Some(DialogKind::FolderTree(form)) = self.original_dialogs.last_mut().map(|d| &mut d.kind) {
                    form.save();
                }
                self.answer_dialog(DialogResult::FolderTree(None), ctx);
            }
            FolderTreeAction::Properties(path) => {
                let _ = (self.system.run_verb)(Path::new(&path), "properties");
            }
            FolderTreeAction::Accept(path) => {
                // FUN_00424a68: PathFileExistsW면 그 경로로 닫고, 아니면 NXSTR_TREE_INVALIDPATH를 알리고 고른 항목을 트리에서 지운다.
                if !path.is_empty() && Path::new(&path).exists() {
                    if let Some(DialogKind::FolderTree(form)) = self.original_dialogs.last_mut().map(|d| &mut d.kind) {
                        form.save();
                    }
                    self.answer_dialog(DialogResult::FolderTree(Some(PathBuf::from(path))), ctx);
                } else {
                    if let Some(DialogKind::FolderTree(form)) = self.original_dialogs.last_mut().map(|d| &mut d.kind) {
                        if let Some(item) = form.focused.filter(|i| Some(*i) != form.tree.root) {
                            let parent = form.tree.items[item].parent;
                            form.tree.remove(item);
                            form.dirty = true;
                            form.focus(parent.or(form.tree.root), 1);
                        }
                    }
                    let text = self.language_pack.text("RES.TREE_INVALIDPATH", "Invalid Path");
                    self.message_box("", "", &text, MB_ICONASTERISK, |_, _, _| {});
                }
            }
        }
    }
}
