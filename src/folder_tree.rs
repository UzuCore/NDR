//! 원본 NexusFile 폴더 트리 자료(Nxfoldertreeview: TNXTreeItem, TNXTreeItems, TNXFolderTreeView의 훑기·트리 파일).
//!
//! 항목(TNXTreeItem): 이름(+8), 펼침(+0xc, 처음 1), 부모(+0x10), 첫 자식(+0x14), 다음 형제(+0x18), 단계(+0x1c, 뿌리 0), 줄(+0x20).
//! 형제는 이름 순(lstrcmpiW)으로 넣고 같은 이름은 넣지 않는다(FUN_007e48a0·FUN_007e47d4).
//! 줄 번호(ItemsChanged 0x7e8fcc): 보이는 항목을 앞 순서로 걸으며 단계가 앞 항목보다 깊지 않을 때만 다음 줄로 — 첫 자식은 부모와 같은 줄에
//! 오른쪽 칸으로 그려진다.
use std::{
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// FILE_ATTRIBUTE_HIDDEN(ScanDir의 `local_290 & 2`).
fn hidden(meta: &fs::Metadata) -> bool {
    meta.file_attributes() & crate::listing::FILE_ATTRIBUTE_HIDDEN != 0
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub level: usize,
    pub expanded: bool,
    pub parent: Option<usize>,
    pub child: Option<usize>,
    pub next: Option<usize>,
    pub row: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub items: Vec<Item>,
    pub root: Option<usize>,
    /// ItemsChanged가 센 가장 깊은 보이는 단계(+0x300).
    pub max_level: usize,
    /// 마지막 보이는 항목(+0x338).
    pub last: Option<usize>,
    /// 줄마다 그 줄의 맨 왼쪽(앞 순서 첫) 항목. 그리기·누르기는 이 항목부터 첫 자식 사슬을 따른다.
    pub row_starts: Vec<usize>,
    /// 지운 항목(자리는 남긴다).
    removed: Vec<bool>,
}

/// 트리 파일 한 줄(ScanDir 0x7e9634·FUN_007e5330 "%d%2d%s"): (펼침, 단계, 이름).
pub type Line = (bool, usize, String);

/// FUN_007e3e38: 경로 단계 = 끝 글자를 뺀 '\' 수(3글자 이하면 0). "\\"로 시작하면(UNC) 3을 뺀다.
pub fn path_level(path: &str) -> usize {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= 3 {
        return 0;
    }
    let count = chars[..chars.len() - 1].iter().filter(|c| **c == '\\').count() as isize;
    let count = if chars[0] == '\\' { count - 3 } else { count };
    count.max(0) as usize
}

/// FUN_007e4ba4가 읽는 줄: 첫 글자 = 펼침(StrToIntExW), 다음 두 글자 = 단계("%2d"), 넷째 글자부터 이름. 숫자가 아니면 None.
pub fn parse_line(text: &str) -> Option<Line> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 4 {
        return None;
    }
    let expanded: i64 = chars[0].to_string().trim().parse().ok()?;
    let level: usize = chars[1..3].iter().collect::<String>().trim().parse().ok()?;
    Some((expanded != 0, level, chars[3..].iter().collect()))
}

/// FUN_007e5330의 한 줄: "%d%2d%s"와 줄바꿈.
pub fn format_line(expanded: bool, level: usize, name: &str) -> String {
    format!("{}{:>2}{}\r\n", u8::from(expanded), level, name)
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// lstrcmpiW 순서(대소문자 무시).
fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

impl Tree {
    /// 뿌리 하나(Refresh 0x7e9d94: 단계 0, 펼침, 이름 = 드라이브 뿌리 경로).
    pub fn with_root(name: &str) -> Self {
        let mut tree = Tree::default();
        let root = tree.push(name, 0, true);
        tree.root = Some(root);
        tree
    }

    fn push(&mut self, name: &str, level: usize, expanded: bool) -> usize {
        self.items.push(Item { name: name.to_owned(), level, expanded, parent: None, child: None, next: None, row: 0 });
        self.removed.push(false);
        self.items.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    /// 살아 있는 항목 수(TNXTreeItems +8).
    pub fn count(&self) -> usize {
        self.removed.iter().filter(|r| !**r).count()
    }

    /// FUN_007e47d4: `sibling`의 부모 아래에 이름 순으로 넣는다(같은 이름이 있어도 넣는다 — 부르는 쪽이 거른다).
    fn insert_beside(&mut self, sibling: usize, item: usize) -> Option<usize> {
        let parent = self.items[sibling].parent?;
        let first = self.items[parent].child?;
        self.items[item].level = self.items[sibling].level;
        self.items[item].parent = Some(parent);
        let name = self.items[item].name.clone();
        let mut before = None;
        let mut cursor = Some(first);
        while let Some(at) = cursor {
            if name_order(&name, &self.items[at].name) != std::cmp::Ordering::Greater {
                break;
            }
            before = Some(at);
            cursor = self.items[at].next;
        }
        match before {
            None => {
                self.items[item].next = Some(first);
                self.items[parent].child = Some(item);
            }
            Some(at) => {
                self.items[item].next = self.items[at].next;
                self.items[at].next = Some(item);
            }
        }
        Some(item)
    }

    /// FUN_007e48a0: 자식이 없으면 첫 자식으로, 있으면 같은 이름이 없을 때만 이름 순으로.
    fn insert_child(&mut self, parent: usize, item: usize) -> Option<usize> {
        match self.items[parent].child {
            None => {
                self.items[item].parent = Some(parent);
                self.items[item].level = self.items[parent].level + 1;
                self.items[parent].child = Some(item);
                Some(item)
            }
            Some(first) => {
                let name = self.items[item].name.clone();
                if self.children(parent).any(|c| same_name(&self.items[c].name, &name)) {
                    return None;
                }
                self.insert_beside(first, item)
            }
        }
    }

    /// FUN_007e4904: 새 이름을 `parent` 아래에 넣는다(단계 = 부모 + 1). 같은 이름이 있으면 None.
    pub fn add_child(&mut self, parent: usize, name: &str) -> Option<usize> {
        let level = self.items[parent].level + 1;
        let item = self.push(name, level, true);
        let added = self.insert_child(parent, item);
        if added.is_none() {
            self.removed[item] = true;
        }
        added
    }

    /// 자식들(첫 자식부터 형제 사슬).
    pub fn children(&self, parent: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.items[parent].child, move |&c| self.items[c].next)
    }

    /// FUN_007e4d30/FUN_007e4ba4: 줄들로 트리를 만든다. 첫 줄이 뿌리. 같은 단계면 앞 항목의 형제, 깊으면 앞 항목의 자식,
    /// 얕으면 그만큼 부모로 올라가 형제로. 줄이 틀리거나 넣을 수 없으면 거기서 멈추고 None(원본은 오류 상자).
    pub fn from_lines(lines: &[Line]) -> Option<Tree> {
        let mut tree = Tree::default();
        let mut previous: Option<usize> = None;
        for (expanded, level, name) in lines {
            let item = tree.push(name, *level, *expanded);
            let placed = match previous {
                None => {
                    tree.root = Some(item);
                    Some(item)
                }
                Some(prev) => {
                    let prev_level = tree.items[prev].level;
                    if *level == prev_level {
                        tree.insert_beside(prev, item)
                    } else if *level > prev_level {
                        let added = tree.insert_child(prev, item);
                        if added.is_some() {
                            tree.items[item].level = *level;
                        }
                        added
                    } else {
                        let mut at = Some(prev);
                        for _ in 0..prev_level - level {
                            at = at.and_then(|a| tree.items[a].parent);
                        }
                        at.and_then(|a| tree.insert_beside(a, item))
                    }
                }
            };
            previous = Some(placed?);
        }
        tree.root?;
        tree.assign_rows();
        Some(tree)
    }

    /// FUN_007e5330·FUN_007e5230: 모든 항목을 앞 순서로(보임과 관계없이) 줄로.
    pub fn to_lines(&self) -> Vec<Line> {
        let mut out = vec![];
        let mut at = self.root;
        while let Some(i) = at {
            let item = &self.items[i];
            out.push((item.expanded, item.level, item.name.clone()));
            at = self.next_any(i);
        }
        out
    }

    /// FUN_007e5230: 앞 순서 다음 항목(첫 자식, 다음 형제, 조상의 다음 형제).
    pub fn next_any(&self, i: usize) -> Option<usize> {
        if let Some(c) = self.items[i].child {
            return Some(c);
        }
        if let Some(n) = self.items[i].next {
            return Some(n);
        }
        let mut at = self.items[i].parent;
        while let Some(p) = at {
            if let Some(n) = self.items[p].next {
                return Some(n);
            }
            at = self.items[p].parent;
        }
        None
    }

    /// FUN_007e4466: 조상이 모두 펼쳐져 있으면 보인다.
    pub fn is_visible(&self, i: usize) -> bool {
        let mut at = self.items[i].parent;
        while let Some(p) = at {
            if !self.items[p].expanded {
                return false;
            }
            at = self.items[p].parent;
        }
        true
    }

    /// FUN_007e5264: 다음 보이는 항목.
    pub fn next_visible(&self, i: usize) -> Option<usize> {
        let mut at = self.next_any(i);
        while let Some(c) = at {
            if self.is_visible(c) {
                return Some(c);
            }
            at = match self.items[c].next {
                Some(n) => Some(n),
                None => self.next_any(c),
            };
        }
        None
    }

    /// FUN_007e52a0: 앞 순서 앞 항목(첫 자식이면 부모, 아니면 앞 형제의 마지막 자손).
    fn prev_any(&self, i: usize) -> Option<usize> {
        let parent = self.items[i].parent?;
        if self.items[parent].child == Some(i) {
            return Some(parent);
        }
        let mut at = self.before(i)?;
        while let Some(mut c) = self.items[at].child {
            while let Some(n) = self.items[c].next {
                c = n;
            }
            at = c;
        }
        Some(at)
    }

    /// FUN_007e52e4: 앞 보이는 항목.
    pub fn prev_visible(&self, i: usize) -> Option<usize> {
        let mut at = self.prev_any(i);
        while let Some(c) = at {
            if self.is_visible(c) {
                return Some(c);
            }
            at = match self.items[c].parent {
                Some(p) => Some(p),
                None => self.prev_any(c),
            };
        }
        None
    }

    /// GetBefore 0x7e43d8: 앞 형제.
    pub fn before(&self, i: usize) -> Option<usize> {
        let parent = self.items[i].parent?;
        let mut at = self.items[parent].child?;
        if at == i {
            return None;
        }
        while let Some(n) = self.items[at].next {
            if n == i {
                return Some(at);
            }
            at = n;
        }
        None
    }

    /// IsFirstChild 0x7e4410.
    pub fn is_first_child(&self, i: usize) -> bool {
        self.items[i].parent.is_some_and(|p| self.items[p].child == Some(i))
    }

    /// HasAsParent 0x7e4644: `ancestor`가 `i`의 조상인가.
    pub fn has_as_parent(&self, i: usize, ancestor: usize) -> bool {
        let mut at = self.items[i].parent;
        while let Some(p) = at {
            if p == ancestor {
                return true;
            }
            at = self.items[p].parent;
        }
        false
    }

    /// ItemsChanged 0x7e8fcc: 뿌리 줄 0. 뿌리의 첫 자식부터 보이는 항목을 걸으며, 단계가 앞 항목 단계 이하면 다음 줄.
    pub fn assign_rows(&mut self) {
        let Some(root) = self.root else { return };
        self.items[root].row = 0;
        self.max_level = 0;
        self.row_starts = vec![root];
        let (mut row, mut previous_level, mut last) = (0, 0, root);
        let mut at = self.items[root].child;
        while let Some(i) = at {
            if self.items[i].level <= previous_level {
                row += 1;
                self.row_starts.push(i);
            }
            self.items[i].row = row;
            previous_level = self.items[i].level;
            self.max_level = self.max_level.max(previous_level);
            last = i;
            at = self.next_visible(i);
        }
        self.last = Some(last);
    }

    /// GetFullPath 0x7e42bc: 뿌리는 이름(':'가 있거나 '/'·'\'로 시작하면 끝에 '\'), 아니면 조상 이름을 PathCombine.
    pub fn full_path(&self, i: usize) -> String {
        let item = &self.items[i];
        if item.level == 0 && item.parent.is_none() {
            let mut name = item.name.clone();
            if !name.is_empty() && (name.contains(':') || name.starts_with(['/', '\\'])) && !name.ends_with('\\') {
                name.push('\\');
            }
            return name;
        }
        let mut path = PathBuf::from(&item.name);
        let mut at = item.parent;
        while let Some(p) = at {
            let parent = &self.items[p];
            let base = if parent.parent.is_none() { self.full_path(p) } else { parent.name.clone() };
            path = Path::new(&base).join(path);
            at = parent.parent;
        }
        path.to_string_lossy().into_owned()
    }

    /// FUN_007e5594: 경로의 항목. 뿌리보다 길지 않으면 뿌리. 이름은 대소문자 무시.
    pub fn find_path(&self, path: &str) -> Option<usize> {
        let root = self.root?;
        let trimmed = path.trim_end_matches('\\');
        let root_text = self.full_path(root);
        let root_trim = root_text.trim_end_matches('\\');
        if trimmed.chars().count() <= root_trim.chars().count() {
            return Some(root);
        }
        if !trimmed.to_lowercase().starts_with(&root_trim.to_lowercase()) {
            return None;
        }
        let rest = &trimmed[root_trim.len()..];
        let mut at = root;
        for part in rest.split('\\').filter(|p| !p.is_empty()) {
            at = self.children(at).find(|c| same_name(&self.items[*c].name, part))?;
        }
        Some(at)
    }

    /// Expand 0x7e4544 / Collapse 0x7e4498 (recurse면 모든 자손도). 줄을 다시 센다.
    pub fn set_expanded(&mut self, i: usize, expanded: bool, recurse: bool) {
        self.items[i].expanded = expanded;
        if recurse {
            let mut stack: Vec<usize> = self.children(i).collect();
            while let Some(c) = stack.pop() {
                self.items[c].expanded = expanded;
                stack.extend(self.children(c));
            }
        }
        self.assign_rows();
    }

    /// MakeVisible 0x7e45c8: 조상을 모두 펼친다.
    pub fn make_visible(&mut self, i: usize) {
        let mut at = self.items[i].parent;
        while let Some(p) = at {
            self.items[p].expanded = true;
            at = self.items[p].parent;
        }
        self.assign_rows();
    }

    /// FUN_007e4a24(초점 처리는 부르는 쪽): 형제 사슬에서 빼고 자손까지 지운다.
    pub fn remove(&mut self, i: usize) {
        if let Some(parent) = self.items[i].parent {
            if self.items[parent].child == Some(i) {
                self.items[parent].child = self.items[i].next;
            } else if let Some(before) = self.before(i) {
                self.items[before].next = self.items[i].next;
            }
        } else if self.root == Some(i) {
            self.root = None;
        }
        let mut stack = vec![i];
        while let Some(c) = stack.pop() {
            self.removed[c] = true;
            stack.extend(self.children(c));
        }
        self.items[i].next = None;
        self.items[i].child = None;
        self.assign_rows();
    }

    /// FUN_007e5754: `other`(다른 트리)의 뿌리를 `parent` 아래에 이름 그대로 붙이고 그 자손을 옮긴다(FUN_007e57fc).
    pub fn graft(&mut self, parent: usize, other: &Tree) -> Option<usize> {
        let other_root = other.root?;
        let top = self.add_child(parent, &other.items[other_root].name)?;
        self.items[top].expanded = other.items[other_root].expanded;
        let mut stack = vec![(other_root, top)];
        while let Some((from, to)) = stack.pop() {
            for c in other.children(from) {
                if let Some(added) = self.add_child(to, &other.items[c].name) {
                    self.items[added].expanded = other.items[c].expanded;
                    stack.push((c, added));
                }
            }
        }
        self.assign_rows();
        Some(top)
    }

    /// QuickSearch 0x7e885c: `from` 다음부터(`include_from`이면 `from`부터) 이름이 `prefix`로 시작하는(StrCmpNIW) 항목. 끝까지 없으면
    /// 뿌리 첫 항목부터 `from` 앞까지 돈다. `all`이면 접힌 항목도 본다(+0x34c).
    pub fn quick_search(&self, from: usize, prefix: &str, include_from: bool, all: bool) -> Option<usize> {
        let root = self.root?;
        let step = |i: usize| if all { self.next_any(i) } else { self.next_visible(i) };
        let matches = |i: usize| self.items[i].name.to_lowercase().starts_with(&prefix.to_lowercase());
        let mut at = if include_from { Some(from) } else if self.last == Some(from) && !all { Some(root) } else { step(from) };
        while let Some(i) = at {
            if i != root && matches(i) {
                return Some(i);
            }
            at = step(i);
        }
        let mut at = Some(root);
        while let Some(i) = at {
            if i == from {
                break;
            }
            if i != root && matches(i) {
                return Some(i);
            }
            at = step(i);
        }
        None
    }
}

/// TNXFolderTreeView 훑기 설정.
#[derive(Clone, Debug, Default)]
pub struct ScanOptions {
    /// 숨김 폴더도(+0x355 ShowHiddenFolder).
    pub show_hidden: bool,
    /// 건너뛸 경로(+0x35c, 대소문자 무시).
    pub excluded: Vec<String>,
    /// 깊이 제한(+0x360 켬, +0x394 단계). None = 제한 없음(PartRefresh).
    pub max_level: Option<usize>,
}

/// RefreshFirstLevel 0x7e99e0의 FindFirstFileW(경로\*): 바로 아래 폴더 이름("."·".." 빼고, 숨김은 show_hidden일 때만).
/// 거르개는 scan과 같다(재분석 지점·제외 경로는 뺀다).
pub fn child_folder_names(folder: &Path, options: &ScanOptions) -> Vec<String> {
    let Ok(read) = fs::read_dir(folder) else { return vec![] };
    read.flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 || (!options.show_hidden && hidden(&meta)) {
                return None;
            }
            let text = entry.path().to_string_lossy().to_lowercase();
            if options.excluded.iter().any(|e| e.to_lowercase().trim_end_matches('\\') == text) {
                return None;
            }
            Some(entry.file_name().to_string_lossy().into_owned())
        })
        .collect()
}

/// SetShowHiddenFolder 0x7ea8b0: GetFileAttributesW(경로) & FILE_ATTRIBUTE_HIDDEN.
pub fn is_hidden_folder(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| hidden(&m))
}

/// ScanDirectory 0x7e988c + ScanDir 0x7e9634: `path`부터 폴더만 줄로. 첫 줄 = `path`(단계 0이면 뿌리 경로, 아니면 이름, 펼침 1).
/// 자식 줄의 펼침 = (단계 − (시작 단계 + 1) < 2). `cancel`이 켜지면 멈춘다(+0x390). `progress`는 들어간 폴더마다(OnScan).
pub fn scan(path: &Path, options: &ScanOptions, cancel: &AtomicBool, progress: &mut dyn FnMut(&Path)) -> Vec<Line> {
    let text = path.to_string_lossy().into_owned();
    let level = path_level(&text);
    let mut lines = vec![];
    if level == 0 {
        let root = root_of(&text);
        lines.push((true, 0, root));
    } else {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        lines.push((true, level, name));
    }
    let base = level + 1;
    scan_dir(path, base, base, options, cancel, progress, &mut lines);
    lines
}

fn root_of(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() >= 2 && chars[1] == ':' {
        return format!("{}:\\", chars[0]);
    }
    path.to_owned()
}

fn scan_dir(folder: &Path, level: usize, base: usize, options: &ScanOptions, cancel: &AtomicBool, progress: &mut dyn FnMut(&Path), out: &mut Vec<Line>) {
    let Ok(read) = fs::read_dir(folder) else { return };
    let mut names: Vec<(String, PathBuf)> = vec![];
    for entry in read.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(meta) = entry.metadata() else { continue };
        // 고친 것: 연결 지점·바로 가기 폴더(재분석 지점)는 따라가지 않는다(원본은 들어가 같은 폴더를 거듭 훑을 수 있다).
        if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
            continue;
        }
        if !options.show_hidden && hidden(&meta) {
            continue;
        }
        let path = entry.path();
        let text = path.to_string_lossy().to_lowercase();
        if options.excluded.iter().any(|e| e.to_lowercase().trim_end_matches('\\') == text) {
            continue;
        }
        names.push((entry.file_name().to_string_lossy().into_owned(), path));
    }
    for (name, path) in names {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        // SetCurrentDirectoryW로 들어갈 수 있는 폴더만.
        if fs::read_dir(&path).is_err() {
            continue;
        }
        progress(&path);
        out.push((level - base < 2, level, name));
        if options.max_level.is_none_or(|max| max == 0 || level < max) {
            scan_dir(&path, level + 1, base, options, cancel, progress, out);
        }
    }
}

/// Refresh 0x7e9d94: 뿌리(`root`, 펼침) + RefreshFirstLevel(첫 단계 폴더) + 첫 단계 폴더마다 Refresh(항목)(그 폴더부터 훑어 붙인다).
pub fn build(root: &Path, options: &ScanOptions, cancel: &AtomicBool, progress: &mut dyn FnMut(&Path)) -> Tree {
    let root_text = root.to_string_lossy().into_owned();
    let mut tree = Tree::with_root(&root_text);
    let top = tree.root.unwrap_or(0);
    let mut first: Vec<(String, PathBuf)> = vec![];
    if let Ok(read) = fs::read_dir(root) {
        for entry in read.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() && (options.show_hidden || !hidden(&meta)) {
                first.push((entry.file_name().to_string_lossy().into_owned(), entry.path()));
            }
        }
    }
    for (name, _) in &first {
        tree.add_child(top, name);
    }
    for (name, path) in first {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Some(item) = tree.children(top).find(|c| tree.items[*c].name == name) else { continue };
        if fs::read_dir(&path).is_err() {
            continue;
        }
        let lines = scan(&path, options, cancel, progress);
        if let Some(sub) = Tree::from_lines(&lines) {
            tree.remove(item);
            if let Some(added) = tree.graft(top, &sub) {
                tree.items[added].expanded = true;
            }
        }
    }
    tree.assign_rows();
    tree
}

/// FUN_007e3e7c: 트리 파일. 뿌리에 두기(tfileinroot)이고 CD·이동식이 아니면 "뿌리\treeinfo.xcd", 아니면 `folder`\볼륨 일련번호.xcd.
pub fn tree_file_name(root: &Path, in_root: bool, removable_or_cd: bool, serial: &str, folder: &Path) -> PathBuf {
    if in_root && !removable_or_cd {
        root.join("treeinfo.xcd")
    } else {
        folder.join(format!("{serial}.xcd"))
    }
}

/// FUN_007e4f64: 트리 파일(UTF-16LE, BOM 없음)을 읽는다. 틀린 줄이 있으면 None.
pub fn load_file(path: &Path) -> Option<Tree> {
    let bytes = fs::read(path).ok()?;
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let text = String::from_utf16_lossy(&units);
    let mut lines = vec![];
    for line in text.split(['\r', '\n']).filter(|l| !l.is_empty()) {
        lines.push(parse_line(line)?);
    }
    Tree::from_lines(&lines)
}

/// FUN_007e5330: 트리 파일로 쓴다(UTF-16LE, BOM 없음, 줄마다 "%d%2d%s\r\n").
pub fn save_file(tree: &Tree, path: &Path) -> Result<(), String> {
    let mut bytes = vec![];
    for (expanded, level, name) in tree.to_lines() {
        for unit in format_line(expanded, level, &name).encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    fs::write(path, bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let lines: Vec<Line> = [(true, 0, "C:\\"), (true, 1, "b"), (true, 2, "b1"), (true, 2, "b2"), (true, 1, "a"), (false, 1, "c"), (true, 2, "c1")]
            .iter()
            .map(|(e, l, n)| (*e, *l, n.to_string()))
            .collect();
        Tree::from_lines(&lines).unwrap()
    }

    fn names(tree: &Tree, ids: impl Iterator<Item = usize>) -> Vec<String> {
        ids.map(|i| tree.items[i].name.clone()).collect()
    }

    #[test]
    fn lines_round_trip_and_siblings_are_sorted() {
        assert_eq!(parse_line("1 2Program Files"), Some((true, 2, "Program Files".into())));
        assert_eq!(parse_line("012x"), Some((false, 12, "x".into())));
        assert_eq!(parse_line("x 1a"), None);
        assert_eq!(format_line(true, 3, "a b"), "1 3a b\r\n");
        let tree = sample();
        let root = tree.root.unwrap();
        assert_eq!(names(&tree, tree.children(root)), ["a", "b", "c"], "lstrcmpiW order");
        let lines = tree.to_lines();
        assert_eq!(lines.iter().map(|l| l.2.as_str()).collect::<Vec<_>>(), ["C:\\", "a", "b", "b1", "b2", "c", "c1"]);
        let again = Tree::from_lines(&lines).unwrap();
        assert_eq!(again.to_lines(), lines);
    }

    #[test]
    fn rows_put_the_first_child_on_its_parent_row() {
        let tree = sample();
        let id = |name: &str| tree.items.iter().position(|i| i.name == name).unwrap();
        // C:\ a        (row 0: 뿌리, 첫 자식 a)
        //     b  b1    (row 1)
        //        b2    (row 2)
        //     c        (row 3, 접힘: c1은 보이지 않는다)
        assert_eq!([0, 0, 1, 1, 2, 3].to_vec(), ["C:\\", "a", "b", "b1", "b2", "c"].iter().map(|n| tree.items[id(n)].row).collect::<Vec<_>>());
        assert!(!tree.is_visible(id("c1")));
        assert_eq!(tree.last, Some(id("c")));
        assert_eq!(tree.row_starts, [id("C:\\"), id("b"), id("b2"), id("c")]);
        assert_eq!(tree.next_visible(id("b2")), Some(id("c")));
        assert_eq!(tree.prev_visible(id("c")), Some(id("b2")));
        assert_eq!(tree.prev_visible(id("b")), Some(id("a")));
        assert_eq!(tree.full_path(id("b2")), "C:\\b\\b2");
        assert_eq!(tree.find_path("c:\\B\\b1"), Some(id("b1")));
        assert_eq!(tree.find_path("C:\\"), tree.root);
        assert_eq!(tree.quick_search(id("a"), "C", false, false), Some(id("c")));
        assert_eq!(tree.quick_search(id("c"), "c1", false, true), Some(id("c1")), "collapsed items with +0x34c");
        assert_eq!(path_level("C:\\"), 0);
        assert_eq!(path_level("C:\\Windows"), 1);
        assert_eq!(path_level("C:\\Windows\\System32\\"), 2);
    }

    #[test]
    fn scanning_builds_the_tree_and_the_file_round_trips() {
        let root = std::env::temp_dir().join(format!("ndr-tree-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        for dir in ["a/a1/a2/a3", "b", "B2"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("a/file.txt"), b"x").unwrap();
        let mut seen = 0;
        let lines = scan(&root, &ScanOptions::default(), &AtomicBool::new(false), &mut |_| seen += 1);
        assert_eq!(seen, 6, "every folder entered");
        let level = path_level(&root.to_string_lossy());
        let base = level + 1;
        let find = |name: &str| lines.iter().find(|l| l.2 == name).map(|l| (l.0, l.1)).unwrap();
        assert_eq!([find("a"), find("a1"), find("a2"), find("a3")], [(true, base), (true, base + 1), (false, base + 2), (false, base + 3)], "expanded only within two levels of the scan");
        let tree = Tree::from_lines(&lines).unwrap();
        let file = root.join("tree.xcd");
        save_file(&tree, &file).unwrap();
        let loaded = load_file(&file).unwrap();
        assert_eq!(loaded.to_lines(), tree.to_lines());
        let limited = scan(&root, &ScanOptions { max_level: Some(base + 1), ..Default::default() }, &AtomicBool::new(false), &mut |_| {});
        assert!(limited.iter().all(|l| l.1 <= base + 1), "{limited:?}");
        fs::remove_dir_all(root).unwrap();
    }
}
