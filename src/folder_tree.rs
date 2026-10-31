//! Original NexusFile folder tree data (Nxfoldertreeview: TNXTreeItem, TNXTreeItems, scanning and tree files of TNXFolderTreeView).
//!
//! Item (TNXTreeItem): name (+8), expanded (+0xc, initially 1), parent (+0x10), first child (+0x14), next sibling (+0x18), level (+0x1c, root 0), row (+0x20).
//! Siblings are inserted in name order (lstrcmpiW) and duplicate names are not inserted (FUN_007e48a0, FUN_007e47d4).
//! Row numbers (ItemsChanged 0x7e8fcc): walking visible items in preorder, a new row starts only when the level is not deeper than the previous item, so the first child is drawn
//! on the parent's row in the cell to the right.
use std::{
    fs,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

/// FILE_ATTRIBUTE_HIDDEN (`local_290 & 2` of ScanDir).
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
    /// Deepest visible level counted by ItemsChanged (+0x300).
    pub max_level: usize,
    /// Last visible item (+0x338).
    pub last: Option<usize>,
    /// Per row the leftmost (first in preorder) item of that row. Drawing and clicking follow the first-child chain from this item.
    pub row_starts: Vec<usize>,
    /// Deleted items (their slots remain).
    removed: Vec<bool>,
}

/// One tree file line (ScanDir 0x7e9634, FUN_007e5330 "%d%2d%s"): (expanded, level, name).
pub type Line = (bool, usize, String);

/// FUN_007e3e38: path level = number of '\' excluding the last character (0 for 3 characters or fewer). Paths starting with "\\" (UNC) subtract 3.
pub fn path_level(path: &str) -> usize {
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= 3 {
        return 0;
    }
    let count = chars[..chars.len() - 1].iter().filter(|c| **c == '\\').count() as isize;
    let count = if chars[0] == '\\' { count - 3 } else { count };
    count.max(0) as usize
}

/// Line read by FUN_007e4ba4: first character = expanded (StrToIntExW), next two = level ("%2d"), name from the fourth character. None when not numeric.
pub fn parse_line(text: &str) -> Option<Line> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 4 {
        return None;
    }
    let expanded: i64 = chars[0].to_string().trim().parse().ok()?;
    let level: usize = chars[1..3].iter().collect::<String>().trim().parse().ok()?;
    Some((expanded != 0, level, chars[3..].iter().collect()))
}

/// One line of FUN_007e5330: "%d%2d%s" and a line break.
pub fn format_line(expanded: bool, level: usize, name: &str) -> String {
    format!("{}{:>2}{}\r\n", u8::from(expanded), level, name)
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// lstrcmpiW order (case-insensitive).
fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

impl Tree {
    /// A single root (Refresh 0x7e9d94: level 0, expanded, name = drive root path).
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

    /// Number of live items (TNXTreeItems +8).
    pub fn count(&self) -> usize {
        self.removed.iter().filter(|r| !**r).count()
    }

    /// FUN_007e47d4: inserts under `sibling`'s parent in name order (even when the same name exists; the caller filters).
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

    /// FUN_007e48a0: as the first child when there are no children, otherwise in name order only when the name does not exist yet.
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

    /// FUN_007e4904: inserts a new name under `parent` (level = parent + 1). None when the name exists.
    pub fn add_child(&mut self, parent: usize, name: &str) -> Option<usize> {
        let level = self.items[parent].level + 1;
        let item = self.push(name, level, true);
        let added = self.insert_child(parent, item);
        if added.is_none() {
            self.removed[item] = true;
        }
        added
    }

    /// Children (from the first child along the sibling chain).
    pub fn children(&self, parent: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(self.items[parent].child, move |&c| self.items[c].next)
    }

    /// FUN_007e4d30/FUN_007e4ba4: builds the tree from lines. The first line is the root. The same level makes a sibling of the previous item, deeper a child of it,
    /// shallower climbs that many parents and makes a sibling. On a bad line or failed insert it stops there with None (the original shows an error box).
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

    /// FUN_007e5330 and FUN_007e5230: all items in preorder (regardless of visibility) as lines.
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

    /// FUN_007e5230: next item in preorder (first child, next sibling, an ancestor's next sibling).
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

    /// FUN_007e4466: visible when all ancestors are expanded.
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

    /// FUN_007e5264: next visible item.
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

    /// FUN_007e52a0: previous item in preorder (the parent for a first child, otherwise the last descendant of the previous sibling).
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

    /// FUN_007e52e4: previous visible item.
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

    /// GetBefore 0x7e43d8: previous sibling.
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

    /// HasAsParent 0x7e4644: whether `ancestor` is an ancestor of `i`.
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

    /// ItemsChanged 0x7e8fcc: the root on row 0. Walks visible items from the root's first child; a level at or below the previous item's level starts the next row.
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

    /// GetFullPath 0x7e42bc: the root is its name (with a trailing '\' when it has ':' or starts with '/' or '\'), otherwise PathCombine of the ancestor names.
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

    /// FUN_007e5594: the item for a path. The root when the path is not longer than the root. Names are case-insensitive.
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

    /// Expand 0x7e4544 / Collapse 0x7e4498 (all descendants too with recurse). Recounts rows.
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

    /// MakeVisible 0x7e45c8: expands all ancestors.
    pub fn make_visible(&mut self, i: usize) {
        let mut at = self.items[i].parent;
        while let Some(p) = at {
            self.items[p].expanded = true;
            at = self.items[p].parent;
        }
        self.assign_rows();
    }

    /// FUN_007e4a24 (focus handling is the caller's): removes it from the sibling chain and deletes its descendants.
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

    /// FUN_007e5754: attaches the root of `other` (another tree) under `parent` with its name as is and moves its descendants (FUN_007e57fc).
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

    /// QuickSearch 0x7e885c: the item after `from` (or `from` itself with `include_from`) whose name starts with `prefix` (StrCmpNIW). When none to the end,
    /// continues from the root's first item up to before `from`. With `all` collapsed items are checked too (+0x34c).
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

/// TNXFolderTreeView scan settings.
#[derive(Clone, Debug, Default)]
pub struct ScanOptions {
    /// Hidden folders too (+0x355 ShowHiddenFolder).
    pub show_hidden: bool,
    /// Paths to skip (+0x35c, case-insensitive).
    pub excluded: Vec<String>,
    /// Depth limit (+0x360 on, +0x394 level). None = no limit (PartRefresh).
    pub max_level: Option<usize>,
}

/// FindFirstFileW(path\*) of RefreshFirstLevel 0x7e99e0: folder names directly below (without "." and "..", hidden ones only with show_hidden).
/// Same filter as scan (reparse points and excluded paths are left out).
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

/// SetShowHiddenFolder 0x7ea8b0: GetFileAttributesW(path) & FILE_ATTRIBUTE_HIDDEN.
pub fn is_hidden_folder(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| hidden(&m))
}

/// ScanDirectory 0x7e988c + ScanDir 0x7e9634: folders only, from `path`, as lines. First line = `path` (the root path at level 0, otherwise the name, expanded 1).
/// Child line expanded = (level - (start level + 1) < 2). Stops when `cancel` is set (+0x390). `progress` fires for each folder entered (OnScan).
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
        // Fixed: junctions and shortcut folders (reparse points) are not followed (the original enters them and can scan the same folder repeatedly).
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
        // Only folders that SetCurrentDirectoryW can enter.
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

/// Refresh 0x7e9d94: root (`root`, expanded) + RefreshFirstLevel (first-level folders) + Refresh(item) for each first-level folder (scans from that folder and attaches).
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

/// FUN_007e3e7c: the tree file. With tfileinroot and not CD or removable, "root\treeinfo.xcd", otherwise `folder`\<volume serial>.xcd.
pub fn tree_file_name(root: &Path, in_root: bool, removable_or_cd: bool, serial: &str, folder: &Path) -> PathBuf {
    if in_root && !removable_or_cd {
        root.join("treeinfo.xcd")
    } else {
        folder.join(format!("{serial}.xcd"))
    }
}

/// FUN_007e4f64: reads a tree file (UTF-16LE, no BOM). None when a line is invalid.
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

/// FUN_007e5330: writes a tree file (UTF-16LE, no BOM, "%d%2d%s\r\n" per line).
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
        // C:\ a        (row 0: root, first child a)
        //     b  b1    (row 1)
        //        b2    (row 2)
        //     c        (row 3, collapsed: c1 is not visible)
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
