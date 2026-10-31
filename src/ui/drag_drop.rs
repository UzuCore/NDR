//! C# drag and drop: BeginFileDragTracking/FileList_PreviewMouseMove (dragging pane items), HandleDropAsync (dropping on panes and folder rows,
//! including files dragged from Explorer), FilterDropPaths, GetModifierDropEffect (Ctrl = copy, Shift = move, both = shortcut),
//! ChooseDropEffect (choose "drag operation" when ConfirmDragDrop), GetDefaultDropEffect (same drive moves, otherwise copies),
//! creating shortcuts (OriginalShortcutService.Create, errors ignored).
//! Out of NDR, when the dragging pointer leaves the window the drag becomes a shell drag (DoDragDrop, copy/move/shortcut). Files dropped from the shell (Explorer, a returning shell drag)
//! get their drop target and effect from the screen mouse position (GetCursorPos) and the Ctrl/Shift state at that moment (GetKeyState).
use super::file_operation::TransferKind;
use super::*;

/// Original pane tree drop timer (+0x2f4, Interval 0x4b0 in FUN_0048da38).
pub(super) const TREE_DROP_EXPAND_DELAY: Duration = Duration::from_millis(1200);
/// Original TNXDropTarget scroll timer (+0x74, Interval 0x64 in constructor 0x7b1bc4).
pub(super) const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(100);

/// Pane items being dragged.
pub(super) struct FileDrag {
    pub paths: Vec<PathBuf>,
    /// Original [+0x2f8]: a right-button drag opens the pmDrop menu on drop.
    pub secondary: bool,
    /// Pane where the drag started.
    pub source: usize,
    /// Allowed effects set by original FUN_004856d8 when the drag starts (1 copy, 2 move, 4 shortcut). Skin theme, left button:
    /// Ctrl+Shift = 4, Ctrl = 1, Shift = 2, none = 7. Right button and the C# theme use 7.
    pub allowed: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropEffect {
    Copy,
    Move,
    Link,
}

/// C# FilterDropPaths: drops items already in the destination folder (parent = destination) and folders that contain the destination.
pub(super) fn filter_drop_paths(candidates: &[PathBuf], target: &Path) -> Vec<PathBuf> {
    let full = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let target = full(target);
    let mut result: Vec<PathBuf> = Vec::new();
    for path in candidates {
        let path = full(path);
        if path.parent().is_some_and(|parent| parent.to_string_lossy().eq_ignore_ascii_case(&target.to_string_lossy())) {
            continue;
        }
        if path.is_dir() && fsops::same_or_inside(&target, &path) {
            continue;
        }
        if !result.iter().any(|r| r.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy())) {
            result.push(path);
        }
    }
    result
}

impl App {
    /// Starts dragging items (left or right button). Archive and remote panes cannot drag. If the dragged item is not selected, only that item.
    /// Skin theme (original FUN_004856d8, FUN_00485bbc): with marks, all marked items are dragged regardless of the pressed item; without marks, the pressed (cursor) item
    /// is dragged. ".." (parent folder) and drive items are never dragged (not put in the original data object).
    pub(super) fn begin_file_drag(&mut self, index: usize, row: usize, secondary: bool, modifiers: egui::Modifiers) {
        if self.file_drag.is_some() {
            return;
        }
        let allowed = match (!secondary, modifiers.ctrl, modifiers.shift) {
            (true, true, true) => 4,
            (true, true, false) => 1,
            (true, false, true) => 2,
            _ => 7,
        };
        let pane = &self.panes[index];
        if pane.archive.is_some() || pane.loading {
            return;
        }
        let rows: Vec<usize> = if !pane.selected.is_empty() {
            pane.selected.iter().copied().collect()
        } else {
            vec![row]
        };
        let mut paths: Vec<PathBuf> = Vec::new();
        // Neither C# BeginFileDragTracking nor the original includes ".." (IsParent).
        for path in rows.iter().filter_map(|r| pane.entries.get(*r)).filter(|e| !e.drive && !e.is_parent()).map(|e| e.path.clone()).filter(|p| p.exists()) {
            let full = std::path::absolute(&path).unwrap_or(path);
            if !paths.iter().any(|p| p.to_string_lossy().eq_ignore_ascii_case(&full.to_string_lossy())) {
                paths.push(full);
            }
        }
        if !paths.is_empty() {
            self.pending_drag_marks = None;
            // Once the original DoDragDrop starts, the list's range drag (press with Shift then move) ends there.
            self.panes[index].range_drag = None;
            self.file_drag = Some(FileDrag { paths, secondary, source: index, allowed });
        }
    }

    /// Drop target: the pane at that position, the folder row under it (except dragged items themselves), otherwise the pane folder.
    pub(super) fn drop_target(&self, position: Option<egui::Pos2>, dragged: &[PathBuf]) -> Option<(usize, PathBuf)> {
        let position = position?;
        let pane = (0..2).find(|i| self.pane_rects[*i].contains(position))?;
        let folder = self.folder_rows.iter().find(|(p, rect, path)| *p == pane && rect.contains(position) && !dragged.contains(path)).map(|(_, _, path)| path.clone());
        Some((pane, folder.unwrap_or_else(|| self.panes[pane].path.clone())))
    }

    /// Edge scrolling of the original list drop target: the drop effect (0x488700) adds the scroll flag (0x80000000) when the pointer is in the top 15 px (y <= 14) or the bottom 15 px (y > height - 15),
    /// and TNXDropTarget.DragOver (0x7b21a0) meanwhile starts a 100 ms timer (+0x74). On every tick
    /// DoTargetScroll (0x7b1e04) sends one WM_VSCROLL (up/down) or WM_HSCROLL (left/right) step when inside the 20 px margin (+0x6c).
    /// The scrolled step goes to `Pane.drag_scroll` and the list moves on the next frame. True when it scrolled.
    pub(super) fn drag_edge_scroll(&mut self, position: Option<egui::Pos2>, now: Instant, ctx: &egui::Context) -> bool {
        // Pane folder tree (the original tree drop effect 0x48e790 adds the scroll flag in the same 15 px band). The tree scrolls vertically only.
        // Slot number: the tree uses pane index + 100.
        let tree_hit = position.and_then(|p| (0..self.panes.len()).find_map(|i| self.panes[i].tree_rect.filter(|r| r.contains(p) && self.pane_rects[i].contains(p)).map(|r| (i, r, p))));
        let hit = tree_hit
            .map(|(i, r, p)| (i + 100, r, p))
            .or_else(|| position.and_then(|p| (0..self.panes.len()).find_map(|i| self.panes[i].list_rect.filter(|r| r.contains(p)).map(|r| (i, r, p)))));
        let Some((pane, rect, p)) = hit else {
            self.drag_scroll_tick = None;
            return false;
        };
        let (x, y, w, h) = (p.x - rect.left(), p.y - rect.top(), rect.width(), rect.height());
        let flagged = !(y > 14.0 && y <= h - 15.0);
        if !flagged {
            self.drag_scroll_tick = None;
            return false;
        }
        let next = match self.drag_scroll_tick {
            Some((p, at)) if p == pane => at,
            _ => {
                self.drag_scroll_tick = Some((pane, now + DRAG_SCROLL_INTERVAL));
                ctx.request_repaint_after(DRAG_SCROLL_INTERVAL);
                return false;
            }
        };
        if now < next {
            ctx.request_repaint_after(next - now);
            return false;
        }
        let dy = if y < 20.0 { -1 } else if y > h - 20.0 { 1 } else { 0 };
        let dx = if x < 20.0 { -1 } else if x > w - 20.0 { 1 } else { 0 };
        self.drag_scroll_tick = Some((pane, now + DRAG_SCROLL_INTERVAL));
        ctx.request_repaint_after(DRAG_SCROLL_INTERVAL);
        if dy == 0 && dx == 0 {
            return false;
        }
        if pane >= 100 {
            if dy == 0 {
                return false;
            }
            self.panes[pane - 100].tree_drag_scroll = dy;
            return true;
        }
        self.panes[pane].drag_scroll = (dy, dx);
        true
    }

    /// Pane folder tree row at that position (pane, folder, shell key, expandable). GetNodeAt of the original tree OnDragOver FUN_0048eb0c.
    pub(super) fn tree_drop_at(&self, position: Option<egui::Pos2>) -> Option<(usize, PathBuf, PathBuf, bool)> {
        let position = position?;
        self.tree_drop_rows
            .iter()
            .find(|(pane, rect, ..)| self.pane_rects[*pane].contains(position) && rect.contains(position))
            .map(|(pane, _, folder, key, expandable)| (*pane, folder.clone(), key.clone(), *expandable))
    }

    /// Original tree OnDragOver (FUN_0048eb0c): marks the node under the pointer as the drop target (DropTarget) and, for a node with subfolders, starts the timer
    /// (+0x2f4, 1200 ms). When it fires FUN_0048edf0 expands that node. Leaving the node clears the mark and stops the timer (OnLeave
    /// FUN_0048ec90, no node). True when it expanded.
    pub(super) fn tree_drag_hover(&mut self, position: Option<egui::Pos2>, now: Instant, ctx: &egui::Context) -> bool {
        let Some((pane, _, key, expandable)) = self.tree_drop_at(position) else {
            self.tree_drop_hover = None;
            return false;
        };
        let since = match &self.tree_drop_hover {
            Some((p, k, since)) if *p == pane && ndr::shell_tree::same_key(k, &key) => *since,
            _ => now,
        };
        self.tree_drop_hover = Some((pane, key.clone(), since));
        if !expandable {
            return false;
        }
        let wait = TREE_DROP_EXPAND_DELAY.saturating_sub(now.saturating_duration_since(since));
        if wait.is_zero() {
            self.panes[pane].tree_open.insert(key.clone());
            // The original timer keeps running and fires on that node every 1200 ms (an already expanded node stays as is).
            self.tree_drop_hover = Some((pane, key, now));
            return true;
        }
        ctx.request_repaint_after(wait);
        false
    }

    /// Original tree OnDrop (FUN_0048ecd0): a right-button drag opens pmDrop, otherwise FUN_00488b64(effect, node folder, 1 = skip confirmation, 0).
    /// So dropping on the tree never asks "confirm drag and drop" (only dropping on the list asks).
    pub(super) fn tree_drop(&mut self, pane: usize, candidates: Vec<PathBuf>, destination: PathBuf, modifiers: egui::Modifiers, allowed: u8) {
        self.tree_drop_hover = None;
        if !destination.is_dir() || candidates.is_empty() {
            return;
        }
        let Some(effect) = self.original_drop_effect(pane, &candidates, &destination, modifiers, allowed) else { return };
        self.active = pane;
        if effect == DropEffect::Link {
            return self.apply_drop(effect, candidates, destination);
        }
        let paths = filter_drop_paths(&candidates, &destination);
        if !paths.is_empty() {
            self.apply_drop(effect, paths, destination);
        }
    }

    /// At the end of every frame: while dragging, a count next to the pointer; on drop, drop on that pane (on a folder row, that folder). Also files dropped from Explorer or a shell drag.
    pub(super) fn drag_drop_frame(&mut self, ctx: &egui::Context) {
        let (pointer, released, modifiers, dropped, inner) = ctx.input(|i| {
            (i.pointer.latest_pos(), i.pointer.any_released(), i.modifiers, i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect::<Vec<_>>(), i.viewport().inner_rect)
        });
        if !dropped.is_empty() {
            // During a shell drag the egui pointer does not move: convert the screen mouse position to window coordinates and take Ctrl/Shift as they are now (C# e.KeyStates).
            let (position, modifiers) = if cfg!(test) {
                (pointer, modifiers)
            } else {
                let ppp = ctx.pixels_per_point();
                let position = native::cursor_position().zip(inner).map(|((x, y), inner)| egui::pos2(x as f32 / ppp - inner.min.x, y as f32 / ppp - inner.min.y)).or(pointer);
                let (ctrl, shift) = native::drop_modifiers();
                (position, egui::Modifiers { ctrl, shift, ..Default::default() })
            };
            if let Some((tree_pane, folder, ..)) = self.tree_drop_at(position) {
                return self.tree_drop(tree_pane, dropped, folder, modifiers, 7);
            }
            let (pane, destination) = self.drop_target(position, &[]).unwrap_or_else(|| (self.active, self.panes[self.active].path.clone()));
            self.handle_drop(pane, dropped, destination, modifiers);
            return;
        }
        if self.file_drag.is_none() {
            self.tree_drop_hover = None;
            return;
        }
        // Over the pane folder tree, the original tree OnDragOver: highlight that node (pane_tree_ui) and expand it after hovering 1200 ms.
        if !released && self.tree_drag_hover(pointer, Instant::now(), ctx) {
            ctx.request_repaint();
        }
        // At the top or bottom edge of a list, scroll one step every 100 ms like the original DoTargetScroll.
        if !released && self.drag_edge_scroll(pointer, Instant::now(), ctx) {
            ctx.request_repaint();
        }
        if released {
            self.drag_scroll_tick = None;
        }
        let Some(drag) = &self.file_drag else { return };
        // Dragging out of the window hands over to a shell drag (C# DoDragDrop). The drop target (Explorer etc.) does the copy, move or shortcut.
        // Dropping back on this window, the dropped-file handling above finds the target from the mouse position.
        let outside = pointer.is_some_and(|p| !ctx.content_rect().contains(p));
        if !released && outside && self.allow_drag_out {
            let (paths, source) = (drag.paths.clone(), drag.source);
            self.file_drag = None;
            match native::drag_files_out(&paths) {
                // Original FUN_004856d8: when Execute ends with copy, move or shortcut (< 3), the dragging pane's marks are saved as the last selection and cleared.
                Ok(effect) if effect != 0 => self.take_drag_marks(source),
                Ok(_) => {}
                Err(error) => self.status = error,
            }
            self.refresh(0);
            self.refresh(1);
            return;
        }
        if !released {
            // Original DoDragDrop pointer: the target's effect (copy, move, shortcut), or the no-drop shape. A right-button drag opens the drop menu, so it uses the normal drag shape.
            let effect = self.drop_target(pointer, &drag.paths).and_then(|(pane, folder)| self.original_drop_effect(pane, &drag.paths, &folder, modifiers, drag.allowed));
            ctx.set_cursor_icon(match (drag.secondary, effect) {
                (true, _) => egui::CursorIcon::Default,
                (false, Some(DropEffect::Copy)) => egui::CursorIcon::Copy,
                (false, Some(DropEffect::Link)) => egui::CursorIcon::Alias,
                (false, Some(_)) => egui::CursorIcon::Default,
                (false, None) => egui::CursorIcon::NoDrop,
            });
            // Original OnDragOver (FUN_004888fc): if the item under the pointer is a folder (kind 1) and not being dragged, SetDropTargetIndex(that item), otherwise -1.
            // DrawListItem (0x7d90bc) and DrawThumbnailItem (0x7da2ae) draw Rectangle(left, top, right, bottom - 1) in that item's cell with a text-colored pen.
            // Fixed: previously the target folder had no mark.
            if let Some(position) = pointer {
                let on_tree = self.tree_drop_at(pointer).is_some();
                let target = if on_tree { None } else { self.folder_rows.iter().find(|(pane, rect, path)| self.pane_rects[*pane].contains(position) && rect.contains(position) && !drag.paths.contains(path)).cloned() };
                if let Some((pane, rect, path)) = target {
                    let row = self.panes[pane].entries.iter().position(|e| e.path == path);
                    let cursor_bar = row == Some(self.panes[pane].cursor) && self.active == pane;
                    let color = if cursor_bar {
                        self.basic_list_color("NXBACK")
                    } else {
                        row.and_then(|r| self.panes[pane].view.colors.get(r).copied().flatten()).map_or(self.basic_list_color("NXFOLDER"), |(r, g, b)| Color32::from_rgb(r, g, b))
                    };
                    let frame = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - 1.0));
                    ctx.layer_painter(egui::LayerId::background()).with_clip_rect(self.pane_rects[pane]).rect_stroke(frame, 0.0, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
                }
            }
            if let Some(position) = pointer {
                // Original FUN_0048514c (drag image): 0x3C3C3C background inside a black border, 0xF0F0F0 text. Bold header DRAGFILES_INFO (count),
                // a 0xC0C0C0 line, then up to 15 dragged item names (PathFindFileNameW) with small shell icons. Row height = text height + 2 (at least 18), width = longest text + 22.
                let count = drag.paths.len();
                let shown: Vec<PathBuf> = drag.paths.iter().take(15).cloned().collect();
                let icons: Vec<Option<egui::TextureHandle>> = shown.iter().map(|p| self.path_icon(ctx, p, p.is_dir())).collect();
                let header = ndr::markup::substitute(&self.language_pack.text("RES.DRAGFILES_INFO", "Dragging %d object(s)"), &[&count.to_string()]);
                let names: Vec<String> = shown.iter().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())).collect();
                let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("ndr-drag")));
                let text = Color32::from_rgb(0xF0, 0xF0, 0xF0);
                let header = painter.layout_no_wrap(header, egui::FontId::proportional(12.5), Color32::WHITE);
                let names: Vec<_> = names.into_iter().map(|name| painter.layout_no_wrap(name, egui::FontId::proportional(12.0), text)).collect();
                let row = (header.size().y + 2.0).max(18.0);
                let widest = names.iter().map(|g| g.size().x).fold(header.size().x, f32::max);
                let size = egui::vec2(widest + 22.0, (names.len() + 1) as f32 * row + 4.0);
                let rect = egui::Rect::from_min_size(position + egui::vec2(14.0, 14.0), size);
                painter.rect_filled(rect, 0.0, Color32::BLACK);
                painter.rect_filled(rect.shrink(1.0), 0.0, Color32::from_rgb(0x3C, 0x3C, 0x3C));
                painter.galley(rect.min + egui::vec2(4.0, 2.0), header, Color32::WHITE);
                painter.line_segment([rect.min + egui::vec2(4.0, row), rect.min + egui::vec2(size.x - 4.0, row)], egui::Stroke::new(1.0, Color32::from_rgb(0xC0, 0xC0, 0xC0)));
                for (index, name) in names.into_iter().enumerate() {
                    let top = rect.min + egui::vec2(2.0, (index + 1) as f32 * row + 3.0);
                    if let Some(Some(icon)) = icons.get(index) {
                        painter.image(icon.id(), egui::Rect::from_min_size(top, egui::vec2(16.0, 16.0)), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    painter.galley(rect.min + egui::vec2(20.0, (index + 1) as f32 * row + 3.0), name, text);
                }
            }
            return;
        }
        let drag = self.file_drag.take().expect("checked above");
        let Some((pane, destination)) = self.drop_target(pointer, &drag.paths) else { return };
        // The original saves the dragging pane's marks as the last selection and clears them after the drop (when DoDragDrop ends with copy, move or shortcut).
        // NDR does it when items actually move from the confirmation box or drop menu (apply_drop) (fixed: the original also clears on "No" and cancel).
        self.pending_drag_marks = Some((drag.source, drag.paths.clone()));
        if drag.secondary {
            self.tree_drop_hover = None;
            return self.show_drop_menu(pane, drag.paths, destination, pointer.unwrap_or_default());
        }
        if let Some((tree_pane, folder, ..)) = self.tree_drop_at(pointer).filter(|(_, folder, ..)| !drag.paths.contains(folder)) {
            return self.tree_drop(tree_pane, drag.paths, folder, modifiers, drag.allowed);
        }
        // On empty space of the same pane only a shortcut can be chosen, like C# (items whose parent is the destination are not moved).
        self.handle_drop_with(pane, drag.paths, destination, modifiers, drag.allowed);
    }

    /// Effect of the original OnDragOver (0x488700). None = 0 (cannot drop, same folder).
    /// The values it reads are recorded at drag start by FUN_00488520: +0x2f8 = right-button drag (effect left to the shell), +0x2fc =
    /// the first item's folder, +0x2f9 = some item lies in another folder (`mixed` here). The pane tree uses the same rules (drag start
    /// 0x48e93c, drag over 0x48e790).
    pub(super) fn original_drop_effect(&self, pane: usize, candidates: &[PathBuf], destination: &Path, modifiers: egui::Modifiers, allowed: u8) -> Option<DropEffect> {
        if self.panes[pane].archive.is_some() {
            // User request (the original cannot drop on non-local panes): a modifiable archive adds local items by copy (Shift = move). No shortcuts.
            if self.editable_archive(pane).is_none() || candidates.is_empty() || candidates.iter().any(|p| !p.is_absolute()) {
                return None;
            }
            let (copy, moving) = (allowed & 1 != 0, allowed & 2 != 0);
            return if modifiers.shift && !modifiers.ctrl && moving {
                Some(DropEffect::Move)
            } else if copy {
                Some(DropEffect::Copy)
            } else if moving {
                Some(DropEffect::Move)
            } else {
                None
            };
        }
        let key = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
        let parents: Vec<Option<String>> = candidates.iter().map(|p| p.parent().map(key)).collect();
        let first = parents.first()?;
        let mixed = parents.iter().any(|p| p != first);
        if !mixed && first.as_deref() == Some(key(destination).as_str()) {
            return None;
        }
        if self.panes[pane].archive.is_some() || self.panes[pane].remote.is_some() {
            return None;
        }
        let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase());
        let (copy, moving, link) = (allowed & 1 != 0, allowed & 2 != 0, allowed & 4 != 0);
        let same_root = !mixed && candidates.first().and_then(|p| root(p)) == root(&self.panes[pane].path);
        if modifiers.shift && modifiers.ctrl && link {
            Some(DropEffect::Link)
        } else if modifiers.shift && moving {
            Some(DropEffect::Move)
        } else if modifiers.ctrl && copy {
            Some(DropEffect::Copy)
        } else if same_root && moving {
            Some(DropEffect::Move)
        } else if copy {
            Some(DropEffect::Copy)
        } else if moving {
            Some(DropEffect::Move)
        } else if link {
            Some(DropEffect::Link)
        } else {
            None
        }
    }

    /// C# HandleDropAsync.
    pub(super) fn handle_drop(&mut self, pane: usize, candidates: Vec<PathBuf>, destination: PathBuf, modifiers: egui::Modifiers) {
        self.handle_drop_with(pane, candidates, destination, modifiers, 7);
    }

    /// Original FUN_00488700 (drop effect, TNXDropTarget +0x98) + FUN_00488b64 (drop). Skin theme.
    /// - If all dragged items come from one folder and are dropped on that folder, effect 0 (nothing).
    /// - Ctrl+Shift = shortcut, Shift = move, Ctrl = copy (within the effects allowed when the drag started).
    /// - Without keys: move when they come from one folder whose root equals the root of the target pane folder, otherwise copy (in the order copy, move, shortcut among the allowed).
    /// - Copy and move ask DROPCOPY_CONFIRM / DROPMOVE_CONFIRM whenever "confirm drag and drop" is on, regardless of keys. Shortcuts never ask.
    fn original_drop(&mut self, pane: usize, candidates: Vec<PathBuf>, destination: PathBuf, modifiers: egui::Modifiers, allowed: u8) {
        let Some(effect) = self.original_drop_effect(pane, &candidates, &destination, modifiers, allowed) else { return };
        self.active = pane;
        if effect == DropEffect::Link {
            return self.apply_drop(effect, candidates, destination);
        }
        // If the destination is inside a dragged folder, that folder is left out (the original reports it as a copy job error).
        let paths = filter_drop_paths(&candidates, &destination);
        if paths.is_empty() {
            return;
        }
        if !self.settings.flag("ConfirmDragDrop", true) {
            return self.apply_drop(effect, paths, destination);
        }
        let (key, fallback) = if effect == DropEffect::Move { ("RES.DROPMOVE_CONFIRM", "You want to move %d obejct(s)?") } else { ("RES.DROPCOPY_CONFIRM", "You want to copy %d object(s)?") };
        let text = self.language_pack.text(key, fallback).replace("%d", &paths.len().to_string());
        let title = self.language_pack.text("RES.DRAGDROP", "Drag and Drop");
        self.message_box(&title, "", &text, ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONQUESTION, move |app, answer, _| {
            if answer == ndr::msgbox::IDYES {
                app.apply_drop(effect, paths, destination);
            }
        });
    }

    /// handle_drop plus the effects allowed when the drag started (allowed). The skin theme uses original_drop, the C# theme C# HandleDropAsync.
    pub(super) fn handle_drop_with(&mut self, pane: usize, candidates: Vec<PathBuf>, destination: PathBuf, modifiers: egui::Modifiers, allowed: u8) {
        if self.panes[pane].archive.is_some() {
            // User request (not in the original): dropping local files or folders on an archive pane adds them to its current folder (or the archive folder row under the pointer).
            let local: Vec<PathBuf> = candidates.into_iter().filter(|p| p.is_absolute() && p.exists()).collect();
            let Some(effect) = self.original_drop_effect(pane, &local, &destination, modifiers, allowed) else { return };
            self.active = pane;
            let moving = effect == DropEffect::Move;
            let prefix = self.archive_drop_prefix(pane, &destination);
            if !self.settings.flag("ConfirmDragDrop", true) {
                return self.archive_add_into_at(pane, prefix, local, moving);
            }
            let (key, fallback) = if moving { ("RES.DROPMOVE_CONFIRM", "You want to move %d obejct(s)?") } else { ("RES.DROPCOPY_CONFIRM", "You want to copy %d object(s)?") };
            let text = self.language_pack.text(key, fallback).replace("%d", &local.len().to_string());
            let title = self.language_pack.text("RES.DRAGDROP", "Drag and Drop");
            return self.message_box(&title, "", &text, ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONQUESTION, move |app, answer, _| {
                if answer == ndr::msgbox::IDYES {
                    app.archive_add_into_at(pane, prefix, local, moving);
                }
            });
        }
        if !destination.is_dir() || candidates.is_empty() {
            return;
        }
        return self.original_drop(pane, candidates, destination, modifiers, allowed);
    }

    /// Original right-button drag (FUN_00488a.. `[+0x2f8] == 1`): opens pmDrop (miDropCopy, miDropMove, miDropShortcut, N2 "-", miDropCancel) at the drop point.
    pub(super) fn show_drop_menu(&mut self, pane: usize, paths: Vec<PathBuf>, destination: PathBuf, position: egui::Pos2) {
        use super::favorites_dialog::{access_key, PopupAction, PopupItem, PopupMenu};
        // User request: on a modifiable archive pane only copy and move are enabled and shortcut is gray (shortcuts cannot be created inside an archive).
        let in_archive = self.panes[pane].archive.is_some();
        if in_archive && (self.editable_archive(pane).is_none() || paths.is_empty() || paths.iter().any(|p| !p.is_absolute())) {
            return;
        }
        if !in_archive && (!destination.is_dir() || paths.is_empty()) {
            return;
        }
        let ko = self.language == "ko";
        let mut items = Vec::new();
        for (key, ko_text, en_text, effect) in [
            ("miDropCopy", "여기에 복사(&C)", "&Copy Here", Some(DropEffect::Copy)),
            ("miDropMove", "여기에 이동(&M)", "&Move Here", Some(DropEffect::Move)),
            ("miDropShortcut", "여기에 단축아이콘 만들기(&S)", "Create &Shortcut Here", Some(DropEffect::Link)),
            ("-", "", "", None),
            ("miDropCancel", "취소(&E)", "Canc&el", None),
        ] {
            if key == "-" {
                items.push(PopupItem::Separator);
                continue;
            }
            let caption = self.language_pack.raw(&format!("Menu.{key}")).unwrap_or_else(|| (if ko { ko_text } else { en_text }).to_owned());
            let (label, access) = access_key(&caption);
            let enabled = !(in_archive && effect == Some(DropEffect::Link));
            let action = match effect {
                Some(effect) if enabled => PopupAction::Drop { pane, effect, paths: paths.clone(), destination: destination.clone() },
                _ => PopupAction::Close,
            };
            items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled, checked: false, action });
        }
        self.popup = Some(PopupMenu { items, position: Some(position), highlight: None, opened: true });
    }

    /// Original DropMenuClick (0x43025c): Tag 1..3 -> effect 1/2/4 via FUN_00488b64(..., skip confirmation = 1). So ConfirmDragDrop is not asked.
    /// When nothing would move (same folder), only shortcuts are created (C# FilterDropPaths).
    pub(super) fn drop_from_menu(&mut self, pane: usize, effect: DropEffect, candidates: Vec<PathBuf>, destination: PathBuf) {
        if self.panes[pane].archive.is_some() {
            if effect == DropEffect::Link || candidates.is_empty() {
                return;
            }
            self.active = pane;
            if let Some((source, dragged)) = self.pending_drag_marks.take() {
                if candidates.iter().all(|p| dragged.contains(p)) {
                    self.take_drag_marks(source);
                }
            }
            let prefix = self.archive_drop_prefix(pane, &destination);
            return self.archive_add_into_at(pane, prefix, candidates, effect == DropEffect::Move);
        }
        if !destination.is_dir() || candidates.is_empty() {
            return;
        }
        self.active = pane;
        let transfer = filter_drop_paths(&candidates, &destination);
        if transfer.is_empty() {
            if effect == DropEffect::Link {
                self.apply_drop(effect, candidates, destination);
            }
            return;
        }
        self.apply_drop(effect, transfer, destination);
    }

    /// Original FUN_004856d8: after the drop, if the dragging list has marks (GetSelCount > 0), CopySelection (last selection) + ClearSelection. Skin theme.
    fn take_drag_marks(&mut self, source: usize) {
        if self.panes.get(source).is_some_and(|p| !p.selected.is_empty()) {
            self.original_take_pane_marks(source);
        }
    }

    fn apply_drop(&mut self, effect: DropEffect, paths: Vec<PathBuf>, destination: PathBuf) {
        if let Some((source, dragged)) = self.pending_drag_marks.take() {
            let from_drag = paths.iter().all(|p| dragged.iter().any(|d| d.to_string_lossy().eq_ignore_ascii_case(&p.to_string_lossy())));
            if from_drag {
                self.take_drag_marks(source);
            }
        }
        match effect {
            DropEffect::Link => {
                // Original FUN_00488b64 (effect 4): quietly creates "name.ext.lnk" (replacing an existing one) and rereads both panes.
                for path in &paths {
                    let mut name = path.file_name().unwrap_or_default().to_os_string();
                    name.push(".lnk");
                    let _ = native::shortcut(path, &destination.join(name));
                }
                self.refresh(0);
                self.refresh(1);
            }
            DropEffect::Copy | DropEffect::Move => self.local_transfer(paths, destination, effect == DropEffect::Move, TransferKind::Drag),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_paths_and_default_effect_follow_csharp() {
        let root = std::env::temp_dir().join(format!("ndr-drop-{}", std::process::id()));
        std::fs::create_dir_all(root.join("a").join("inner")).unwrap();
        std::fs::create_dir_all(root.join("b")).unwrap();
        let candidates = vec![root.join("a"), root.join("b").join("x.txt"), root.join("a")];
        // When the destination is inside a, a is left out, and items whose parent is the destination are left out too.
        assert_eq!(filter_drop_paths(&candidates, &root.join("a").join("inner")), vec![root.join("b").join("x.txt")]);
        assert!(filter_drop_paths(&[root.join("b").join("x.txt")], &root.join("b")).is_empty());
        assert_eq!(filter_drop_paths(&candidates, &root.join("b")), vec![root.join("a")]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
