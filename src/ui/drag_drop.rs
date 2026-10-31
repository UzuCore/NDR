//! C# 끌어서 놓기: BeginFileDragTracking/FileList_PreviewMouseMove(패널 항목 끌기), HandleDropAsync(패널·폴더 줄에 놓기,
//! 탐색기에서 끌어 온 파일 포함), FilterDropPaths, GetModifierDropEffect(Ctrl = 복사, Shift = 이동, 둘 다 = 바로 가기),
//! ChooseDropEffect(ConfirmDragDrop이면 "드래그 작업" 고르기), GetDefaultDropEffect(같은 드라이브면 이동, 아니면 복사),
//! 바로 가기 만들기(OriginalShortcutService.Create, 오류는 무시).
//! NDR 밖으로는 끌던 포인터가 창을 벗어나면 셸 끌기(DoDragDrop, 복사·이동·바로 가기)로 넘긴다. 셸에서 놓인 파일(탐색기, 되돌아온 셸 끌기)은
//! 화면 마우스 위치(GetCursorPos)와 그때의 Ctrl/Shift(GetKeyState)로 놓을 곳과 효과를 정한다.
use super::file_operation::TransferKind;
use super::*;

/// 원본 패널 트리 놓기 타이머(+0x2f4, FUN_0048da38의 Interval 0x4b0).
pub(super) const TREE_DROP_EXPAND_DELAY: Duration = Duration::from_millis(1200);

/// 끌고 있는 패널 항목.
pub(super) struct FileDrag {
    pub paths: Vec<PathBuf>,
    /// 원본 [+0x2f8]: 오른쪽 단추로 끌면 놓을 때 pmDrop 메뉴를 연다.
    pub secondary: bool,
    /// 끌기를 시작한 패널.
    pub source: usize,
    /// 원본 FUN_004856d8이 끌기를 시작할 때 정하는 허용 효과(1 복사, 2 이동, 4 바로 가기). 스킨 테마 왼쪽 단추:
    /// Ctrl+Shift = 4, Ctrl = 1, Shift = 2, 없으면 7. 오른쪽 단추·C# 테마는 7.
    pub allowed: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropEffect {
    Copy,
    Move,
    Link,
}

/// C# FilterDropPaths: 대상 폴더에 이미 있는 항목(부모 = 대상)과, 대상이 그 안에 있는 폴더는 뺀다.
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
    /// 항목 끌기 시작(왼쪽 또는 오른쪽 단추). 압축·원격 패널은 끌 수 없다. 끈 항목이 선택에 없으면 그 항목만.
    /// 스킨 테마(원본 FUN_004856d8·FUN_00485bbc): 표시가 있으면 누른 항목과 상관없이 표시한 항목을 모두 끌고, 없으면 누른(커서) 항목을
    /// 끈다. ".."(상위 폴더)와 드라이브 항목은 끌지 않는다(원본 데이터 개체에 넣지 않음).
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
        // C# BeginFileDragTracking도 원본도 ".."(IsParent)는 넣지 않는다.
        for path in rows.iter().filter_map(|r| pane.entries.get(*r)).filter(|e| !e.drive && !e.is_parent()).map(|e| e.path.clone()).filter(|p| p.exists()) {
            let full = std::path::absolute(&path).unwrap_or(path);
            if !paths.iter().any(|p| p.to_string_lossy().eq_ignore_ascii_case(&full.to_string_lossy())) {
                paths.push(full);
            }
        }
        if !paths.is_empty() {
            self.pending_drag_marks = None;
            // 원본 DoDragDrop가 시작되면 목록의 범위 끌기(Shift로 누른 뒤 움직이기)는 거기서 끝난다.
            self.panes[index].range_drag = None;
            self.file_drag = Some(FileDrag { paths, secondary, source: index, allowed });
        }
    }

    /// 놓을 곳: 그 위치의 패널, 그 위의 폴더 줄이면 그 폴더(끌고 있는 항목 자신은 빼고), 아니면 패널 폴더.
    pub(super) fn drop_target(&self, position: Option<egui::Pos2>, dragged: &[PathBuf]) -> Option<(usize, PathBuf)> {
        let position = position?;
        let pane = (0..2).find(|i| self.pane_rects[*i].contains(position))?;
        let folder = self.folder_rows.iter().find(|(p, rect, path)| *p == pane && rect.contains(position) && !dragged.contains(path)).map(|(_, _, path)| path.clone());
        Some((pane, folder.unwrap_or_else(|| self.panes[pane].path.clone())))
    }

    /// 그 위치의 패널 폴더 트리 줄(패널, 폴더, 셸 열쇠, 펼칠 수 있음). 원본 트리 OnDragOver FUN_0048eb0c의 GetNodeAt.
    pub(super) fn tree_drop_at(&self, position: Option<egui::Pos2>) -> Option<(usize, PathBuf, PathBuf, bool)> {
        let position = position?;
        self.tree_drop_rows
            .iter()
            .find(|(pane, rect, ..)| self.pane_rects[*pane].contains(position) && rect.contains(position))
            .map(|(pane, _, folder, key, expandable)| (*pane, folder.clone(), key.clone(), *expandable))
    }

    /// 원본 트리 OnDragOver(FUN_0048eb0c): 포인터 아래 노드를 놓을 곳 표시(DropTarget)로 두고, 하위 폴더가 있는 노드면 타이머
    /// (+0x2f4, 1200ms)를 켠다. 타이머가 울리면 FUN_0048edf0이 그 노드를 펼친다. 노드에서 벗어나면 표시를 지우고 타이머를 끈다(OnLeave
    /// FUN_0048ec90, 노드 없음). 펼쳤으면 true.
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
            // 원본 타이머는 꺼지지 않고 1200ms마다 그 노드에 다시 울린다(이미 펼친 노드는 그대로).
            self.tree_drop_hover = Some((pane, key, now));
            return true;
        }
        ctx.request_repaint_after(wait);
        false
    }

    /// 원본 트리 OnDrop(FUN_0048ecd0): 오른쪽 끌기면 pmDrop, 아니면 FUN_00488b64(효과, 노드 폴더, 1 = 확인 건너뜀, 0).
    /// 그래서 트리에 놓으면 "드래그 앤 드롭 확인"을 묻지 않는다(목록에 놓을 때만 묻는다).
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

    /// 매 화면 끝: 끌고 있으면 포인터 옆에 개수, 놓으면 그 패널(폴더 줄 위면 그 폴더)에 놓기. 탐색기·셸 끌기에서 놓은 파일도.
    pub(super) fn drag_drop_frame(&mut self, ctx: &egui::Context) {
        let (pointer, released, modifiers, dropped, inner) = ctx.input(|i| {
            (i.pointer.latest_pos(), i.pointer.any_released(), i.modifiers, i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect::<Vec<_>>(), i.viewport().inner_rect)
        });
        if !dropped.is_empty() {
            // 셸 끌기 동안 egui 포인터는 움직이지 않는다: 화면 마우스 위치를 창 안 좌표로 바꾸고, Ctrl/Shift도 지금 상태로(C# e.KeyStates).
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
        // 패널 폴더 트리 위면 원본 트리 OnDragOver: 그 노드를 강조하고(pane_tree_ui), 머물면 1200ms 뒤 펼친다.
        if !released && self.tree_drag_hover(pointer, Instant::now(), ctx) {
            ctx.request_repaint();
        }
        let Some(drag) = &self.file_drag else { return };
        // 창 밖으로 끌고 나가면 셸 끌기(C# DoDragDrop)로 넘긴다. 놓을 곳(탐색기 등)이 복사·이동·바로 가기를 한다.
        // 다시 이 창에 놓으면 위의 놓은 파일 처리가 마우스 위치로 놓을 곳을 찾는다.
        let outside = pointer.is_some_and(|p| !ctx.content_rect().contains(p));
        if !released && outside && self.allow_drag_out {
            let (paths, source) = (drag.paths.clone(), drag.source);
            self.file_drag = None;
            match native::drag_files_out(&paths) {
                // 원본 FUN_004856d8: Execute가 복사·이동·바로 가기(< 3)로 끝나면 끈 패널의 표시를 마지막 선택에 남기고 지운다.
                Ok(effect) if effect != 0 => self.take_drag_marks(source),
                Ok(_) => {}
                Err(error) => self.status = error,
            }
            self.refresh(0);
            self.refresh(1);
            return;
        }
        if !released {
            // 원본 DoDragDrop 포인터: 놓을 곳의 효과(복사·이동·바로 가기), 놓을 수 없으면 금지 모양. 오른쪽 끌기는 놓기 메뉴라 보통 끌기 모양.
            let effect = self.drop_target(pointer, &drag.paths).and_then(|(pane, folder)| self.original_drop_effect(pane, &drag.paths, &folder, modifiers, drag.allowed));
            ctx.set_cursor_icon(match (drag.secondary, effect) {
                (true, _) => egui::CursorIcon::Default,
                (false, Some(DropEffect::Copy)) => egui::CursorIcon::Copy,
                (false, Some(DropEffect::Link)) => egui::CursorIcon::Alias,
                (false, Some(_)) => egui::CursorIcon::Default,
                (false, None) => egui::CursorIcon::NoDrop,
            });
            // 원본 OnDragOver(FUN_004888fc): 포인터 아래 항목이 폴더(종류 1)이고 끄는 항목이 아니면 SetDropTargetIndex(그 항목), 그 밖은 −1.
            // DrawListItem(0x7d90bc)·DrawThumbnailItem(0x7da2ae)은 그 항목 칸에 글 색 펜으로 Rectangle(왼쪽, 위, 오른쪽, 아래 − 1)을 그린다.
            // 고친 것: 전에는 놓을 폴더에 아무 표시가 없었다.
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
                // 원본 FUN_0048514c(끌기 그림): 검은 테두리 안 0x3C3C3C 바탕, 0xF0F0F0 글. 굵은 머리글 DRAGFILES_INFO(개수),
                // 0xC0C0C0 줄, 그 아래 끄는 항목 이름(PathFindFileNameW) 최대 15개와 그 앞의 셸 작은 아이콘. 줄 높이 = 글 높이 + 2(최소 18), 너비 = 가장 긴 글 + 22.
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
        // 원본은 놓기가 끝난 뒤(DoDragDrop가 복사·이동·바로 가기로 끝나면) 끈 패널의 표시를 마지막 선택에 남기고 지운다.
        // NDR은 확인 상자·놓기 메뉴에서 실제로 옮길 때(apply_drop) 한다(고친 것: 원본은 "아니요"·취소에도 지운다).
        self.pending_drag_marks = Some((drag.source, drag.paths.clone()));
        if drag.secondary {
            self.tree_drop_hover = None;
            return self.show_drop_menu(pane, drag.paths, destination, pointer.unwrap_or_default());
        }
        if let Some((tree_pane, folder, ..)) = self.tree_drop_at(pointer).filter(|(_, folder, ..)| !drag.paths.contains(folder)) {
            return self.tree_drop(tree_pane, drag.paths, folder, modifiers, drag.allowed);
        }
        // 같은 패널의 빈 곳이면 C#처럼 바로 가기만 고를 수 있다(부모 = 대상인 항목은 옮기지 않는다).
        self.handle_drop_with(pane, drag.paths, destination, modifiers, drag.allowed);
    }

    /// 원본 OnDragOver의 효과(0x488700). None = 0(놓을 수 없음, 같은 폴더).
    pub(super) fn original_drop_effect(&self, pane: usize, candidates: &[PathBuf], destination: &Path, modifiers: egui::Modifiers, allowed: u8) -> Option<DropEffect> {
        if self.panes[pane].archive.is_some() {
            // 사용자 요청(원본은 로컬이 아닌 패널에 놓을 수 없다): 고칠 수 있는 압축이면 로컬 항목을 복사(Shift = 이동)로 더한다. 바로 가기는 없다.
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

    /// 원본 FUN_00488700(놓기 효과, TNXDropTarget +0x98) + FUN_00488b64(놓기). 스킨 테마.
    /// - 끈 항목이 모두 한 폴더에서 왔고 그 폴더에 놓으면 효과 0(아무것도 안 함).
    /// - Ctrl+Shift = 바로 가기, Shift = 이동, Ctrl = 복사(끌기를 시작할 때 허용한 효과 안에서).
    /// - 글쇠가 없으면 한 폴더에서 왔고 그 루트가 놓는 패널 폴더의 루트와 같으면 이동, 아니면 복사(허용한 것 중 복사·이동·바로 가기 차례).
    /// - 복사·이동은 "드래그 앤 드롭 확인"이 켜져 있으면 글쇠와 상관없이 DROPCOPY_CONFIRM·DROPMOVE_CONFIRM으로 묻는다. 바로 가기는 묻지 않는다.
    fn original_drop(&mut self, pane: usize, candidates: Vec<PathBuf>, destination: PathBuf, modifiers: egui::Modifiers, allowed: u8) {
        let Some(effect) = self.original_drop_effect(pane, &candidates, &destination, modifiers, allowed) else { return };
        self.active = pane;
        if effect == DropEffect::Link {
            return self.apply_drop(effect, candidates, destination);
        }
        // 대상이 끈 폴더 안이면 그 폴더는 빼고 옮긴다(원본은 복사 작업이 오류로 알린다).
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

    /// handle_drop에 끌기를 시작할 때 허용한 효과(allowed)를 더한 것. 스킨 테마는 original_drop, C# 테마는 C# HandleDropAsync.
    pub(super) fn handle_drop_with(&mut self, pane: usize, candidates: Vec<PathBuf>, destination: PathBuf, modifiers: egui::Modifiers, allowed: u8) {
        if self.panes[pane].archive.is_some() {
            // 사용자 요청(원본에는 없음): 압축 패널에 로컬 파일·폴더를 놓으면 그 안 지금 폴더(압축 안 폴더 줄 위면 그 폴더)에 더한다.
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

    /// 원본 오른쪽 단추 끌기(FUN_00488a.. `[+0x2f8] == 1`): 놓은 곳에 pmDrop(miDropCopy, miDropMove, miDropShortcut, N2 "-", miDropCancel)을 연다.
    pub(super) fn show_drop_menu(&mut self, pane: usize, paths: Vec<PathBuf>, destination: PathBuf, position: egui::Pos2) {
        use super::favorites_dialog::{access_key, PopupAction, PopupItem, PopupMenu};
        // 사용자 요청: 고칠 수 있는 압축 패널이면 복사·이동만 켜고 바로 가기는 회색(압축 안에는 바로 가기를 만들 수 없다).
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

    /// 원본 DropMenuClick(0x43025c): Tag 1..3 → 효과 1/2/4로 FUN_00488b64(…, 확인 건너뜀 = 1). 그래서 ConfirmDragDrop을 묻지 않는다.
    /// 옮길 것이 없으면(같은 폴더) 바로 가기만 만든다(C# FilterDropPaths).
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

    /// 원본 FUN_004856d8: 놓기가 끝나고 끈 목록에 표시가 있으면(GetSelCount > 0) CopySelection(마지막 선택) + ClearSelection. 스킨 테마.
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
                // 원본 FUN_00488b64(효과 4): "이름.확장자.lnk"를 조용히 만들고(있으면 바꿈) 두 패널을 다시 읽는다.
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
        // 대상이 a 안이면 a는 빠지고, 부모가 대상인 항목도 빠진다.
        assert_eq!(filter_drop_paths(&candidates, &root.join("a").join("inner")), vec![root.join("b").join("x.txt")]);
        assert!(filter_drop_paths(&[root.join("b").join("x.txt")], &root.join("b")).is_empty());
        assert_eq!(filter_drop_paths(&candidates, &root.join("b")), vec![root.join("a")]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
