use super::*;

/// 셸 아이콘이 파일마다 다른 형식(DefaultIcon이 "%1"인 실행 파일·아이콘 파일, 대상 아이콘을 쓰는 바로 가기 등).
pub(super) fn has_own_icon(path: &Path) -> bool {
    const OWN: [&str; 10] = ["exe", "lnk", "ico", "cur", "ani", "url", "scr", "cpl", "msc", "appref-ms"];
    path.extension().is_some_and(|e| OWN.iter().any(|o| e.eq_ignore_ascii_case(o)))
}

/// 원본 DrawThumbnailText(0x7e1998): 글자 단위로 폭에 들어가는 만큼(GetTextExtentExPointW nFit)씩 줄을 나눈다(최대 max_lines줄).
/// 마지막 줄에 남은 글이 다 들어가지 않으면 들어가는 글자 수 − 1부터 줄여 가며 "..."를 붙인 글이 들어가는 첫 길이로 그린다.
pub(super) fn thumbnail_lines(text: &str, width: f32, max_lines: usize, glyph: &mut dyn FnMut(char) -> f32) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let extent = |slice: &[char], glyph: &mut dyn FnMut(char) -> f32| slice.iter().map(|&c| glyph(c)).sum::<f32>();
    let fit = |slice: &[char], glyph: &mut dyn FnMut(char) -> f32| {
        let mut total = 0.0;
        slice.iter().take_while(|&&c| {
            total += glyph(c);
            total <= width
        })
        .count()
    };
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in 1..=max_lines.max(1) {
        if offset >= chars.len() {
            break;
        }
        let rest = &chars[offset..];
        // 고친 것: 칸이 첫 글자보다 좁으면 빈 줄만 쌓였다. 줄마다 적어도 한 글자는 넣는다.
        let n = fit(rest, glyph).max(1);
        if (line < max_lines && offset + n < chars.len()) || extent(rest, glyph) <= width {
            lines.push(rest[..n].iter().collect());
            offset += n;
        } else {
            let shortened = (1..n).rev().find_map(|keep| {
                let candidate: String = rest[..keep].iter().chain("...".chars().collect::<Vec<_>>().iter()).collect();
                (extent(&candidate.chars().collect::<Vec<_>>(), glyph) <= width).then_some(candidate)
            });
            // 고친 것: "..."를 붙일 자리가 없으면 마지막 줄이 통째로 빠졌다. 그때는 들어가는 글자만 보인다.
            lines.push(shortened.unwrap_or_else(|| rest[..n].iter().collect()));
            break;
        }
    }
    lines
}

/// 원본 GetThumbnailTextHeight(0x7e1858): 필요한 줄 수(최대 max_lines) × (줄 높이 + spacing), 드라이브 항목(종류 4)이면 + 8.
pub(super) fn thumbnail_text_height(text: &str, width: f32, max_lines: usize, spacing: f32, line_height: f32, drive: bool, glyph: &mut dyn FnMut(char) -> f32) -> f32 {
    let chars: Vec<char> = text.chars().collect();
    let (mut lines, mut offset) = (0usize, 0usize);
    loop {
        lines += 1;
        let mut total = 0.0;
        let n = chars[offset.min(chars.len())..]
            .iter()
            .take_while(|&&c| {
                total += glyph(c);
                total <= width
            })
            .count()
            .max(1);
        if chars.len() <= offset + n || lines >= max_lines.max(1) {
            break;
        }
        offset += n;
    }
    lines as f32 * (line_height + spacing) + if drive { 8.0 } else { 0.0 }
}

impl App {
    /// C# FileVisualConverter.Convert(아이콘 부분): ShowIcons가 꺼져 있으면 없음. 확장자(폴더는 "<DIR>")마다 한 번 읽어 둔다.
    /// 셸 아이콘이 파일마다 다른 형식인가: has_own_icon 고정 목록이거나, 확장자마다 한 번 HKCR로 물은 결과(system.per_file_icon).
    pub(super) fn per_file_icon_type(&mut self, path: &Path) -> bool {
        if has_own_icon(path) {
            return true;
        }
        let Some(extension) = path.extension().map(|e| e.to_string_lossy().to_lowercase()) else { return false };
        let hook = self.system.per_file_icon;
        *self.per_file_icon_types.entry(extension).or_insert_with_key(|e| hook(e))
    }

    pub(super) fn file_icon(&mut self, ctx: &egui::Context, index: usize, row: usize) -> Option<egui::TextureHandle> {
        let entry = self.panes[index].entries.get(row)?;
        // 원본 ".." 항목: GetIconIndex("A", 폴더 속성) = 보통 폴더 아이콘(상위가 드라이브 루트여도 드라이브 아이콘이 아니다).
        if entry.is_parent() {
            return self.path_icon(ctx, Path::new("A"), true);
        }
        let (path, directory, drive, attributes) = (entry.path.clone(), entry.is_dir, entry.drive, entry.attributes);
        // C# Convert: UseOverlayIcons면 압축 안이 아닌 전체 경로 항목은 오버레이 아이콘(경로마다, 따로 된 STA 스레드). 오기 전에는 확장자 아이콘.
        // 원본 드라이브 항목(종류 4)은 늘 그 드라이브의 셸 아이콘(TNXShellImageList.GetIconIndex(루트))이므로 같은 경로 아이콘을 쓴다.
        // 원본 처음 값 [Operation] overlay=1.
        let overlays = self.settings.flag("UseOverlayIcons", true);
        let local = self.panes[index].archive.is_none() && self.panes[index].remote.is_none() && path.is_absolute();
        // 원본 GetIconIndex(0x7b8d54)는 파일마다 실제 경로로 셸 아이콘을 묻는다. 확장자마다 같은 아이콘인 형식은 확장자 캐시로 같고,
        // 파일마다 아이콘이 다른 형식(고정 목록 + HKCR의 DefaultIcon "%1"·shellex\IconHandler 형식)만 실제 경로로 읽는다(오버레이 없이).
        let own_icon = !directory && local && !overlays && self.per_file_icon_type(&path);
        // 폴더는 셸이 desktop.ini를 읽는 경우(읽기 전용 또는 시스템 속성, 사용자 지정 폴더 아이콘·특수 폴더)만 실제 경로로 읽는다.
        const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
        const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
        let own_folder_icon = directory && local && !overlays && !drive && attributes & (FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_SYSTEM) != 0;
        let real_path = drive || (overlays && local) || own_icon || own_folder_icon;
        if self.settings.flag("ShowIcons", true) && real_path {
            match self.overlay_icons.get(&path) {
                Some(Some(texture)) => return Some(texture.clone()),
                Some(None) => {}
                None => self.queue_overlay_icon(path.clone(), overlays || drive),
            }
        }
        self.path_icon(ctx, &path, directory)
    }

    /// 드라이브 루트의 셸 아이콘(원본 FUN_00483108: SHGetFileInfo(루트, SHGFI_ICON | SHGFI_SMALLICON)). 오기 전에는 폴더 아이콘.
    pub(super) fn drive_icon(&mut self, ctx: &egui::Context, root: &Path) -> Option<egui::TextureHandle> {
        if !self.settings.flag("ShowIcons", true) {
            return None;
        }
        match self.overlay_icons.get(root) {
            Some(Some(texture)) => return Some(texture.clone()),
            Some(None) => {}
            None => self.queue_overlay_icon(root.to_path_buf(), false),
        }
        self.path_icon(ctx, root, true)
    }

    /// C# QueueShellOverlayIcon / EnsureShellOverlayWorker: 처음 부를 때 STA 작업 스레드를 띄우고, 같은 경로는 한 번만 보낸다.
    /// overlay가 false면 오버레이 없이 그 파일의 셸 아이콘(native::shell_path_icon).
    fn queue_overlay_icon(&mut self, path: PathBuf, overlay: bool) {
        self.queue_icon_job(path, if overlay { 1 } else { 0 });
    }

    /// 아이콘 작업: 0 = 그 파일의 셸 아이콘, 1 = 오버레이 아이콘, 2 = 셸 열쇠(패널 폴더 트리의 바탕 화면·가상 폴더).
    fn queue_icon_job(&mut self, path: PathBuf, mode: u8) {
        if !self.overlay_pending.insert(path.clone()) {
            return;
        }
        if self.overlay_tx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel::<(PathBuf, u8)>();
            let (with_overlay, plain, results) = (self.system.overlay_icon, self.system.path_icon, self.async_tx.clone());
            thread::spawn(move || {
                native::com_sta_init();
                for (path, mode) in rx {
                    let image = match mode {
                        1 => with_overlay(&path),
                        2 => native::shell_key_icon(&path),
                        _ => plain(&path),
                    };
                    if results.send(AsyncResult::OverlayIcon { path, image }).is_err() {
                        break;
                    }
                }
            });
            self.overlay_tx = Some(tx);
        }
        if let Some(tx) = &self.overlay_tx {
            let _ = tx.send((path, mode));
        }
    }

    /// 패널 폴더 트리 항목의 아이콘(TNXShellTreeView: 셸 이미지 목록의 그 항목 아이콘). 읽기 전에는 폴더 아이콘.
    pub(super) fn shell_node_icon(&mut self, ctx: &egui::Context, node: &ndr::shell_tree::ShellNode) -> Option<egui::TextureHandle> {
        if !self.settings.flag("ShowIcons", true) {
            return None;
        }
        match self.overlay_icons.get(&node.key) {
            Some(Some(texture)) => Some(texture.clone()),
            Some(None) => self.path_icon(ctx, &node.key, true),
            None => {
                self.queue_icon_job(node.key.clone(), 2);
                self.path_icon(ctx, &node.key, true)
            }
        }
    }

    /// 경로 하나의 작은 셸 아이콘(확장자 캐시). 끌기 그림도 쓴다.
    pub(super) fn path_icon(&mut self, ctx: &egui::Context, path: &Path, directory: bool) -> Option<egui::TextureHandle> {
        if !self.settings.flag("ShowIcons", true) {
            return None;
        }
        let key = if directory {
            "<DIR>".to_owned()
        } else {
            path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default()
        };
        if let Some(cached) = self.file_icons.get(&key) {
            return cached.clone();
        }
        let texture = (self.system.file_icon)(path, directory)
            .map(|(width, height, rgba)| self.icon_texture(ctx, format!("ndr-file-icon{key}"), width, height, &rgba));
        self.file_icons.insert(key, texture.clone());
        texture
    }

    /// 셸 아이콘 그림을 올리고, 원본 흐린 아이콘(ImageList_DrawEx ILD_BLEND50, rgbFg 흰색 = 색을 흰색과 반씩 섞고 알파는 그대로)도 함께 만든다.
    pub(super) fn icon_texture(&mut self, ctx: &egui::Context, name: String, width: usize, height: usize, rgba: &[u8]) -> egui::TextureHandle {
        let texture = ctx.load_texture(name.clone(), egui::ColorImage::from_rgba_unmultiplied([width, height], rgba), egui::TextureOptions::LINEAR);
        let ghost: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [((p[0] as u16 + 255) / 2) as u8, ((p[1] as u16 + 255) / 2) as u8, ((p[2] as u16 + 255) / 2) as u8, p[3]]).collect();
        let ghost = ctx.load_texture(format!("{name}|ghost"), egui::ColorImage::from_rgba_unmultiplied([width, height], &ghost), egui::TextureOptions::LINEAR);
        self.ghost_icons.insert(texture.id(), ghost);
        texture
    }

    /// 원본 DrawListItem(0x7d8f58)·DrawThumbnailItem: 잘라낸 항목(+100), 숨김 속성(2), SFTP 목록(+0x78)의 "."로 시작하는 이름은
    /// 아이콘을 흐리게 그린다(스킨 테마). 흐린 그림이 있으면 그것을 돌려준다.
    pub(super) fn row_icon_for_skin(&self, index: usize, row: usize, icon: Option<egui::TextureHandle>) -> Option<egui::TextureHandle> {
        let icon = icon?;
        if self.row_ghosted(index, row) {
            if let Some(ghost) = self.ghost_icons.get(&icon.id()) {
                return Some(ghost.clone());
            }
        }
        Some(icon)
    }

    pub(super) fn row_ghosted(&self, index: usize, row: usize) -> bool {
        let pane = &self.panes[index];
        let Some(entry) = pane.entries.get(row) else { return false };
        let sftp = pane.remote.as_ref().is_some_and(|r| r.is_sftp());
        pane.cut_marks.contains(&entry.path) || entry.attributes & 2 != 0 || (sftp && entry.name.starts_with('.'))
    }

    /// 원본 GetListHeaderWidth: 이름 칸 앞 머리 너비(표시 칸 12 + 아이콘 16 + 2, 아이콘을 보이지 않으면 14).
    pub(super) fn list_header_width(icons: bool) -> f32 {
        if icons { 30.0 } else { 14.0 }
    }

    /// 원본 DrawListItem(0x7d8f58) 이름 칸: 아이콘은 칸 왼쪽 + 12(왼쪽 12화소는 표시 ▶ 칸), 세로 가운데((항목 높이 − 16) / 2).
    /// 이름은 칸 왼쪽 + 머리 너비(`list_header_width`) + 5부터 오른쪽 − 5까지 한 줄(넘치면 …, DT_END_ELLIPSIS).
    /// 고친 것(사용자 요청: 가이드선에 아이콘·글이 물리지 않게): 전에는 egui 단추 안쪽 여백(4화소)부터 그려 열 선·패널 테두리에 붙었다.
    /// style = 원본 ListFont 모양(밑줄·취소선은 list_text로 입힌다).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn name_cell(ui: &mut egui::Ui, size: egui::Vec2, icon: Option<&egui::TextureHandle>, icons: bool, text: &str, color: Color32, style: u8) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            if let Some(texture) = icon {
                let top = rect.top() + ((rect.height() - 16.0) / 2.0).floor();
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(texture.id(), egui::Rect::from_min_size(egui::pos2(rect.left() + 12.0, top), egui::vec2(16.0, 16.0)), uv, Color32::WHITE);
            }
            let left = rect.left() + Self::list_header_width(icons) + 5.0;
            let mut job = egui::text::LayoutJob::default();
            super::list_text(text, style).color(color).append_to(&mut job, ui.style(), egui::FontSelection::Default, egui::Align::Center);
            job.wrap = egui::text::TextWrapping { max_width: (rect.right() - 5.0 - left).max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(egui::pos2(left, rect.top() + ((rect.height() - galley.size().y) / 2.0).floor()), galley, color);
        }
        response
    }

    pub(super) fn tile_rows(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        visible: &[usize],
        height: f32,
        thumbnails: bool,
    ) -> (bool, bool, Option<&'static str>) {
        let mut activate = false;
        let mut open = false;
        let command = None;
        let list_style = self.list_font_style;
        let original_click = true;
        // 스킨 테마의 미리 보기: 원본 DrawThumbnailItem(0x7da1c4)·DrawFocus 형식 3. 이름 글색은 늘 NXFORE(Font.Color = GetBasic(1)).
        let skin = self.current_skin();
        let original = Some(self.original_list_colors());
        let original_fore = original.map(|_| self.basic_list_color("NXFORE"));
        let list_focused = self.active == index;
        // 원본 스킨 세로 막대(TNXScrollBar, 미리 보기도 같은 막대): 넘칠 때만 보이고 그 너비만큼 칸 자리가 좁아진다(자세히 보기와 같은 방식).
        let ctx = ui.ctx().clone();
        let skin_bar = match &skin {
            Some(s) => self.skin_scrollbar_width(&ctx, s),
            None => None,
        };
        let bar_flag = egui::Id::new(("ndr-skin-tile-vbar", index));
        let bar_reserve = match skin_bar {
            Some(w) if ui.data(|d| d.get_temp::<bool>(bar_flag)).unwrap_or(false) => w,
            _ => 0.0,
        };
        let avail = ui.available_width() - bar_reserve;
        // 원본 UpdateDisplayThumbnails(0x7e1ba0): 칸 너비 = 썸네일 크기(+0x3b0, ini thumbsize) + 12로 열 수를 정하고,
        // 남는 폭은 열마다 나눠 칸을 넓힌다. 칸 높이 = 크기 + 12 + 글 높이. C# 판도 그림을 ThumbnailSize로 그린다.
        let thumb = self.settings.values.get("ThumbnailSize").and_then(|v| v.as_i64()).unwrap_or(120).clamp(32, 512) as f32;
        let tile_width = if thumbnails { thumb + 12.0 } else { 230.0 };
        // C# ResolveColumnCount: 이름 목록은 Left/RightColumnCount(1~19)면 그 수, 0(자동)이면 가장 긴 이름 + 14(+ 아이콘 16) 너비로 나눈다.
        let configured = self.settings.values.get(if index == 0 { "LeftColumnCount" } else { "RightColumnCount" }).and_then(|v| v.as_i64()).unwrap_or(0);
        let columns = if thumbnails {
            (avail / tile_width).floor().max(1.0) as usize
        } else if (1..=19).contains(&configured) {
            configured as usize
        } else {
            let font = egui::TextStyle::Body.resolve(ui.style());
            let widest = ui.fonts_mut(|fonts| {
                visible.iter().filter_map(|&row| self.panes[index].entries.get(row)).map(|e| fonts.layout_no_wrap(e.name.clone(), font.clone(), Color32::WHITE).size().x).fold(0.0f32, f32::max)
            });
            let item = widest + 14.0 + if self.settings.flag("ShowIcons", true) { 16.0 } else { 0.0 };
            ((ui.available_width().max(1.0) / item.max(1.0)) as usize).clamp(1, 19)
        };
        let gap = if thumbnails { 0.0 } else { ui.spacing().item_spacing.x };
        let tile_width = ((avail - gap * columns.saturating_sub(1) as f32) / columns as f32).floor().max(if thumbnails { tile_width.min(avail) } else { 40.0 });
        let rows = visible.len().div_ceil(columns);
        // 원본 UpdateDisplayThumbnails: 줄 높이 = 그 줄에서 가장 큰 GetThumbnailTextHeight(폭 = 썸네일 크기, 줄 간격 2, 최대 4줄) + 크기 + 12.
        let spacing_y = if thumbnails { 0.0 } else { ui.spacing().item_spacing.y };
        let heights: Vec<f32> = if thumbnails {
            let font = egui::TextStyle::Body.resolve(ui.style());
            let line_height = ui.fonts_mut(|fonts| fonts.row_height(&font));
            // 고친 것(빠르기): 전에는 그림마다 모든 항목의 이름 폭을 재서(2만 개면 한 장면에 15ms) 미리 보기 보기가 느렸다.
            // 원본처럼 목록·칸 수·크기가 바뀔 때만 다시 잰다(항목 차례·이름·드라이브 이름을 열쇠로).
            let key = {
                use std::hash::{Hash, Hasher};
                let pane = &self.panes[index];
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                (columns, rows, thumb.to_bits(), line_height.to_bits(), font.size.to_bits(), format!("{:?}", font.family), pane.drive_infos.len()).hash(&mut hasher);
                for &row in visible.iter() {
                    row.hash(&mut hasher);
                    if let Some(entry) = pane.entries.get(row) {
                        (entry.name.as_str(), entry.drive).hash(&mut hasher);
                    }
                }
                hasher.finish()
            };
            match &self.panes[index].thumb_heights {
                Some((cached_key, cached)) if *cached_key == key => cached.clone(),
                _ => {
                    let pane = &self.panes[index];
                    let heights: Vec<f32> = ui.fonts_mut(|fonts| {
                        (0..rows)
                            .map(|r| {
                                let tallest = visible[r * columns..((r + 1) * columns).min(visible.len())]
                                    .iter()
                                    .filter_map(|&row| pane.entries.get(row))
                                    .map(|entry| {
                                        let caption = match pane.drive_infos.get(&entry.path).filter(|_| entry.drive) {
                                            Some((label, _, _)) => format!("{label} ({}:)", entry.name.trim_end_matches(['\\', ':'])),
                                            None => entry.name.clone(),
                                        };
                                        thumbnail_text_height(&caption, thumb, 4, 2.0, line_height, entry.drive, &mut |c| fonts.glyph_width(&font, c))
                                    })
                                    .fold(0.0f32, f32::max);
                                thumb + 12.0 + tallest
                            })
                            .collect()
                    });
                    self.panes[index].thumb_heights = Some((key, heights.clone()));
                    heights
                }
            }
        } else {
            vec![26.0; rows]
        };
        let mut tops = Vec::with_capacity(rows);
        let mut bottom_edge = 0.0;
        for h in &heights {
            tops.push(bottom_edge);
            bottom_edge += h + spacing_y;
        }
        let content_height = (bottom_edge - spacing_y).max(0.0);
        let line_at = |offset: f32| tops.partition_point(|&t| t <= offset + 0.5).saturating_sub(1);
        // 원본 UpdateScrollBar: 미리 보기(스타일 3)는 쪽 설정과 상관없이 한 칸에 한 줄.
        let (by_page, wheel_lines) = if thumbnails { (false, 1) } else { (self.scroll_by_page(index), (self.system.wheel_lines)()) };
        let current = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(egui::Id::new(("tiles", index)))).map_or(0.0, |s| s.offset.y);
        let here = line_at(current);
        let stride = heights.get(here).map_or(26.0, |h| *h) + spacing_y;
        let wheel = Self::wheel_scroll(ui, egui::Id::new(("tiles", index)), height, stride, by_page, wheel_lines);
        let mut area = egui::ScrollArea::vertical()
            .id_salt(("tiles", index))
            .scroll_source(super::panes::WHEEL_OFF)
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height);
        if skin_bar.is_some() {
            area = area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        }
        // 줄 높이가 줄마다 다르므로 바퀴는 줄 단위로 옮긴다.
        let mut offset = match wheel {
            Some(target) if rows > 0 => {
                let steps = ((target - current) / stride).round() as isize;
                tops[(here as isize + steps).clamp(0, rows as isize - 1) as usize]
            }
            Some(target) => target,
            None => current,
        };
        // 원본 SetItemIndex(형식 3)·ArrowKey: 커서 줄이 보이게 넘기고, 한 줄의 칸 수와 다 보이는 줄 수를 남긴다.
        // 원본 AdjustPage(형식 3): 그 항목의 줄이 위로 벗어나면 맨 위, 아래로 벗어나면 맨 아래로. 다시 읽은 목록은 원본이 그 뒤
        // SetScrollTop(옛 위치)로 되돌리므로 하지 않는다.
        if let Some((item, false)) = self.panes[index].adjust.take() {
            let line = item / columns.max(1);
            if let (Some(&top), Some(&h)) = (tops.get(line), heights.get(line)) {
                if top + h > offset + height {
                    offset = top + h - height;
                } else if top < offset {
                    offset = top;
                }
            }
        }
        if std::mem::take(&mut self.panes[index].reveal) && wheel.is_none() {
            let line = self.panes[index].cursor / columns.max(1);
            if let (Some(&top), Some(&h)) = (tops.get(line), heights.get(line)) {
                if top < offset {
                    offset = top;
                } else if top + h > offset + height {
                    offset = top + h - height;
                }
            }
        }
        offset = offset.min((content_height - height).max(0.0)).max(0.0);
        let first = tops.partition_point(|&t| t < offset - 0.01);
        let fully = (first..rows).take_while(|&r| tops[r] + heights[r] <= offset + height + 0.01).count();
        self.panes[index].page = super::list_keys::ListPage { style: super::list_keys::ListStyle::Thumbnails, rows: fully.max(1), columns, top: first * columns, last: 0 };
        self.panes[index].thumb_rows = thumbnails.then(|| (tops.clone(), heights.clone(), offset, height, columns));
        if (offset - current).abs() > 0.5 {
            area = area.vertical_scroll_offset(offset);
        }
        let output = area
            .show_viewport(ui, |ui, viewport| {
                let origin = ui.max_rect().min;
                let width = ui.max_rect().width() - bar_reserve;
                ui.set_height(content_height);
                for tile_row in line_at(viewport.min.y)..rows {
                    if tops[tile_row] > viewport.max.y {
                        break;
                    }
                    let row_height = heights[tile_row];
                    let rect = egui::Rect::from_min_size(origin + egui::vec2(0.0, tops[tile_row]), egui::vec2(width, row_height));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Min)), |ui| {
                        // 원본 UpdateDisplayThumbnails(0x7e1ba0): 칸 너비 = 목록 너비 / 열 수, 칸 사이 틈 없음(줄 사이도).
                        if thumbnails {
                            ui.spacing_mut().item_spacing.x = 0.0;
                        }
                        for column in 0..columns {
                            let Some(&row) = visible.get(tile_row * columns + column) else {
                                break;
                            };
                            let entry = self.panes[index].entries[row].clone();
                            if thumbnails
                                && self.panes[index].archive.is_none()
                                && self.panes[index].remote.is_none()
                                && !self.image_requests.contains(&entry.path)
                            {
                                self.image_requests.insert(entry.path.clone());
                                let path = entry.path.clone();
                                let tx = self.async_tx.clone();
                                thread::spawn(move || {
                                    let image = native::thumbnail(&path, thumb as u32);
                                    let _ = tx.send(AsyncResult::Image { path, image });
                                });
                            }
                            let selected = self.panes[index].selected.contains(&row)
                                || (self.active == index && self.panes[index].cursor == row);
                            let icon = self.file_icon(ui.ctx(), index, row);
                            let icon = self.row_icon_for_skin(index, row, icon);
                            // 원본 DrawThumbnailItem: 흐리게 그릴 항목(잘라냄·숨김·SFTP "." 이름)이면 미리 보기 그림은 알파 0x55.
                            let ghosted = self.row_ghosted(index, row);
                            // 원본 DrawThumbnailItem(0x7da1c4) 종류 4: 이름 대신 "%s (%s:)"(볼륨 이름, 드라이브 글자), 글 밑에 남은 공간 막대.
                            let drive_info = if thumbnails && entry.drive { self.panes[index].drive_infos.get(&entry.path).cloned() } else { None };
                            let caption = match &drive_info {
                                Some((label, _, _)) => format!("{label} ({}:)", entry.name.trim_end_matches(['\\', ':'])),
                                None => entry.name.clone(),
                            };
                            // 스킨 테마: 커서·표시 칸은 원본 막대(DrawFocus 형식 3)를 그림·글 밑에 깐다(자리를 먼저 잡고, 칸 크기를 안 뒤 채운다).
                            let cursor_row = self.panes[index].cursor == row;
                            let marked = self.panes[index].selected.contains(&row);
                            let focus_slot = original.filter(|_| cursor_row || marked).map(|_| ui.painter().add(egui::Shape::Noop));
                            let response = egui::Frame::new()
                                .fill(if selected && original.is_none() {
                                    ui.visuals().selection.bg_fill
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .inner_margin(5.0)
                                .show(ui, |ui| {
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(tile_width - 10.0, row_height - 10.0),
                                        egui::Layout::top_down(egui::Align::Center),
                                        |ui| {
                                            if thumbnails {
                                                // 그림, 글 줄(원본 줄마다 높이 + 2), 드라이브 막대를 틈 없이 쌓는다(줄 높이는 위에서 셈).
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                // 칸은 줄 높이 전체를 쓴다(고른 칸 바탕·누르는 자리).
                                                ui.set_min_size(egui::vec2(tile_width - 10.0, row_height - 10.0));
                                            }
                                            if thumbnails {
                                                // 원본: 흐리게 그릴 항목은 그림을 알파 0x55로(AlphaBlend SourceConstantAlpha), 아이콘은 ILD_BLEND50(흐린 아이콘 그림). 스킨 테마만.
                                                let tint = if ghosted { Color32::from_white_alpha(0x55) } else { Color32::WHITE };
                                                if let Some(texture) = self.images.get(&entry.path)
                                                {
                                                    ui.add(egui::Image::new((
                                                        texture.id(),
                                                        egui::vec2(thumb, thumb),
                                                    )).tint(tint));
                                                } else if let Some(icon) = &icon {
                                                    // C# FileVisualConverter: 미리 보기가 없으면 아이콘.
                                                    ui.add_sized([thumb, thumb], egui::Image::new((icon.id(), egui::vec2(32.0, 32.0))));
                                                } else {
                                                    ui.add_sized(
                                                        [thumb, thumb],
                                                        egui::Label::new(if entry.is_dir {
                                                            "[DIR]"
                                                        } else {
                                                            "[FILE]"
                                                        }),
                                                    );
                                                }
                                            }
                                            // 스킨 테마는 원본처럼 NXFORE(종류와 상관없음), 아니면 C# 판 색.
                                            let name_color = original_fore.unwrap_or_else(|| if entry.is_dir { Color32::from_rgb(228, 188, 85) } else { ui.visuals().text_color() });
                                            let name = super::list_text(&caption, list_style).color(name_color);
                                            match (&icon, thumbnails) {
                                                (Some(icon), false) => {
                                                    ui.horizontal(|ui| {
                                                        ui.image((icon.id(), egui::vec2(16.0, 16.0)));
                                                        ui.add(egui::Label::new(name).truncate());
                                                    });
                                                }
                                                (_, true) => {
                                                    // 원본 DrawThumbnailText: 폭 = 썸네일 크기, 최대 4줄, 줄마다 가운데, 줄 간격 2.
                                                    let font = egui::TextStyle::Body.resolve(ui.style());
                                                    let (line_height, lines) = ui.fonts_mut(|fonts| (fonts.row_height(&font), thumbnail_lines(&caption, thumb, 4, &mut |c| fonts.glyph_width(&font, c))));
                                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(thumb, lines.len() as f32 * (line_height + 2.0)), egui::Sense::hover());
                                                    for (i, line) in lines.iter().enumerate() {
                                                        ui.painter().text(egui::pos2(rect.center().x, rect.top() + 2.0 + i as f32 * (line_height + 2.0)), egui::Align2::CENTER_TOP, line, font.clone(), name_color);
                                                    }
                                                }
                                                _ => {
                                                    ui.add(egui::Label::new(name).truncate());
                                                }
                                            }
                                            // 원본: 고정 디스크(종류 3)이고 전체 크기를 알면 썸네일 크기 − 16 너비, 높이 4 막대.
                                            // 전체는 FUN_007d7e6c(NXFORE, NXBACK, 0.3), 쓴 비율만큼은 (NXFORE, NXBACK, 0.5).
                                            if let Some((_, 3, Some((free, total)))) = drive_info.as_ref().map(|(l, k, s)| (l, *k, *s)) {
                                                if total > 0 {
                                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(thumb - 16.0, 8.0), egui::Sense::hover());
                                                    let bar = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + 2.0), egui::vec2(thumb - 16.0, 4.0));
                                                    let (fore, back) = match (original, original_fore) {
                                                        (Some(o), Some(fore)) => (fore, o.back),
                                                        _ => (ui.visuals().text_color(), ui.visuals().extreme_bg_color),
                                                    };
                                                    ui.painter().rect_filled(bar, 0.0, super::list_view::blend(fore, back, 0.3));
                                                    let used = (total.saturating_sub(free) as u128 * 100 / total as u128) as f32;
                                                    ui.painter().rect_filled(egui::Rect::from_min_size(bar.min, egui::vec2(((thumb - 16.0) * used / 100.0).floor(), 4.0)), 0.0, super::list_view::blend(fore, back, 0.5));
                                                }
                                            }
                                        },
                                    );
                                })
                                .response
                                .interact(egui::Sense::click());
                            // 원본 DrawThumbnailItem: DrawThumbnailText가 이름을 줄였으면(+0x65) 커서가 온 뒤 ShowToolTip 형식 3이 온 이름을 보인다.
                            if thumbnails && original.is_some() && cursor_row && !entry.drive {
                                let font = egui::TextStyle::Body.resolve(ui.style());
                                let (shortened, galleys) = ui.fonts_mut(|fonts| {
                                    let shortened = thumbnail_lines(&caption, thumb, 4, &mut |c| fonts.glyph_width(&font, c)).concat() != caption;
                                    let galleys: Vec<_> = if shortened {
                                        super::panes::word_break_lines(&entry.name, thumb, &mut |s| fonts.layout_no_wrap(s.to_owned(), font.clone(), Color32::PLACEHOLDER).size().x)
                                            .into_iter()
                                            .map(|line| fonts.layout_no_wrap(line, font.clone(), Color32::PLACEHOLDER))
                                            .collect()
                                    } else {
                                        Vec::new()
                                    };
                                    (shortened, galleys)
                                });
                                if shortened {
                                    // NDR 칸: 바깥 5, 그림 thumb, 글 첫 줄은 그 밑 2.
                                    let text_top = response.rect.top() + 5.0 + thumb + 2.0;
                                    *self.thumb_tip.borrow_mut() = Some((index, row, response.rect, text_top, thumb, galleys));
                                }
                            }
                            if let (Some(slot), Some(o)) = (focus_slot, original) {
                                // 막대 높이는 그 항목의 글 높이(GetThumbnailTextHeight)로 잰다.
                                let font = egui::TextStyle::Body.resolve(ui.style());
                                let text_height = ui.fonts_mut(|fonts| {
                                    let line_height = fonts.row_height(&font);
                                    thumbnail_text_height(&caption, thumb, 4, 2.0, line_height, entry.drive, &mut |c| fonts.glyph_width(&font, c))
                                });
                                let (top, bottom, line) = super::skin_pane::thumbnail_focus(o.select, o.back, cursor_row, marked, false, list_focused);
                                let bar = super::skin_pane::thumbnail_focus_rect(response.rect, thumb, text_height);
                                ui.painter().set(slot, egui::Shape::Vec(super::skin_pane::rounded_gradient_shapes(bar, 4.0, top, bottom, line)));
                            }
                            // 원본 MouseDown(형식 3): 스킨 테마는 누르는 순간 커서·Shift·Ctrl을 처리한다(표시 칸·끌어 고르기는 없다).
                            if original_click && super::panes::original_thumbnail_press(&mut self.panes[index], row, response.rect, ui) {
                                activate = true;
                            }
                            if response.secondary_clicked() {
                                activate = true;
                                let pane = &mut self.panes[index];
                                super::panes::row_pressed(pane, row, egui::Modifiers::NONE, true, original_click);
                            }
                            if response.clicked() {
                                activate = true;
                                if !original_click {
                                    let modifiers = ui.input(|i| i.modifiers);
                                    let pane = &mut self.panes[index];
                                    super::panes::row_pressed(pane, row, modifiers, false, original_click);
                                }
                            }
                            if super::panes::windows_double_clicked(&response) {
                                self.panes[index].cursor = row;
                                activate = true;
                                open = true;
                            }
                            // 줄 응답에 메뉴를 붙이지 않는다(패널 메뉴 skin_pane::pane_menu).
                            if response.secondary_clicked() {
                                super::skin_pane::request_pane_menu(ui.ctx(), index);
                            }
                        }
                    });
                }
            });
        if let (Some(bar_width), Some(skin)) = (skin_bar, &skin) {
            let needs = output.content_size.y > output.inner_rect.height() + 0.5;
            if ui.data(|d| d.get_temp::<bool>(bar_flag)) != Some(needs) {
                ui.data_mut(|d| d.insert_temp(bar_flag, needs));
                ui.ctx().request_repaint();
            }
            if needs {
                // 원본 UpdateScrollBar(형식 3): 단추 한 번에 한 줄.
                let inner = output.inner_rect;
                let bar = egui::Rect::from_min_max(egui::pos2(inner.right() - bar_width, inner.top()), inner.max);
                if let Some(next) = self.skin_vertical_scrollbar(ui, &ctx, skin, bar_flag.with("bar"), bar, output.state.offset.y, output.content_size.y, inner.height(), stride) {
                    let mut state = output.state;
                    state.offset.y = next;
                    state.store(ui.ctx(), output.id);
                    ui.ctx().request_repaint();
                }
            }
        }
        (activate, open, command)
    }
    /// 원본 패널 폴더 트리(miShowFoldersClick 0x445258 → FUN_0048da38): 패널 안 목록 왼쪽(alLeft TNXPaintPanel 안 TNXShellTreeView).
    /// 뿌리는 셸 바탕 화면, 그 아래는 셸 이름 공간(내 PC → 드라이브 → 폴더, 네트워크, 제어판, 휴지통 …, ndr::shell_tree).
    /// 바탕·글꼴은 목록과 같고(FUN_0048e1ac: 목록 Color·Font, 글 색 GetBasic(1)), 패널 위치가 바뀌면 그 폴더까지 펼쳐 고른다(SetPath).
    /// 고르면(FUN_0048e320) 제어판은 control.exe, 휴지통은 셸로 열고(FUN_00432fe4), 가상 폴더는 셸로 열고, 폴더면 그 패널을 옮긴다.
    pub(super) fn pane_tree_ui(&mut self, ui: &mut egui::Ui, index: usize, rect: egui::Rect) {
        use ndr::shell_tree::{ShellNode, Special, same_key};
        // FUN_0048e1ac: 바탕 = 목록 Color(NXBACK), 글 = GetBasic(1)(NXFORE).
        // 고른 줄: VCL TCustomTreeView 그리기가 고른 항목(CDIS_SELECTED)의 붓을 clHighlight로 두고, FUN_006c9388(트리, 0) =
        // HideSelection 끔(TVS_SHOWSELALWAYS)이라 초점이 없어도 그렇다. OnCustomDrawItem FUN_0048e104는 고른 항목의 글 색을 트리 바탕
        // (Color, +0x7c)이 검정(0)이면 흰색(초점 0xFEFEFE, 아니면 0xFFFFFF), 아니면 트리 글꼴 색(NXFORE)으로 둔다.
        // 고친 것: 전에는 NXSELECT 바탕에 NXBACK 글로 그렸다(원본은 시스템 강조 색).
        let (background, text) = (self.basic_list_color("NXBACK"), self.basic_list_color("NXFORE"));
        let (select, selected_text) = tree_selection_colors(background, text, native::system_color(13));
        ui.painter().rect_filled(rect, 0.0, background);
        let path = self.panes[index].path.clone();
        let local = self.panes[index].archive.is_none() && self.panes[index].remote.is_none() && !super::panes::is_drives(&path);
        // SetPath: 위치가 바뀌면 바탕 화면부터 그 폴더까지(내 PC → 드라이브 → …) 펼치고 그 줄이 보이게 넘긴다.
        let mut reveal = false;
        if local && self.panes[index].tree_synced.as_ref() != Some(&path) {
            for key in ndr::shell_tree::ancestors(&path) {
                if !self.panes[index].tree_open.iter().any(|k| same_key(k, &key)) {
                    self.panes[index].tree_open.insert(key);
                }
            }
            self.panes[index].tree_synced = Some(path.clone());
            self.panes[index].tree_selected = None;
            reveal = true;
        }
        if self.shell_desktop.is_none() {
            self.shell_desktop = Some(ndr::shell_tree::desktop().unwrap_or(ShellNode { key: PathBuf::new(), name: self.tr("바탕 화면", "Desktop").into_owned(), path: None, has_children: true, special: Special::None }));
        }
        let root = self.shell_desktop.clone().unwrap_or(ShellNode { key: PathBuf::new(), name: String::new(), path: None, has_children: true, special: Special::None });
        let font = egui::FontId::new(self.list_font_size, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()));
        let row_height = (ui.fonts_mut(|f| f.row_height(&font)).ceil() + 2.0).max(18.0);
        let mut rows: Vec<(ShellNode, usize)> = Vec::new();
        fn walk(app: &mut App, index: usize, node: &ShellNode, depth: usize, rows: &mut Vec<(ShellNode, usize)>) {
            rows.push((node.clone(), depth));
            if depth > 32 || !app.panes[index].tree_open.iter().any(|k| ndr::shell_tree::same_key(k, &node.key)) {
                return;
            }
            match app.tree_cache.get(&node.key).cloned() {
                Some(children) => {
                    for child in &children {
                        walk(app, index, child, depth + 1, rows);
                    }
                }
                None => app.request_tree_children(index, &node.key),
            }
        }
        walk(self, index, &root, 0, &mut rows);
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("ndr-pane-tree", index)).max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let mut picked: Option<ShellNode> = None;
        let mut toggled: Option<PathBuf> = None;
        let mut shell_menu: Option<PathBuf> = None;
        let mut drop_rows: Vec<(usize, egui::Rect, PathBuf)> = Vec::new();
        // 하위 폴더를 아직 읽는 중이면 고른 줄이 없으므로, 그 줄이 나올 때까지 넘기기를 미룬다.
        let reveal_id = egui::Id::new(("ndr-pane-tree-reveal", index));
        let reveal = reveal || ui.ctx().data(|d| d.get_temp::<bool>(reveal_id)).unwrap_or(false);
        let selected_key = self.panes[index].tree_selected.clone().or_else(|| local.then(|| path.clone()));
        let selected_line = selected_key.as_ref().and_then(|key| rows.iter().position(|(node, _)| same_key(&node.key, key)));
        ui.ctx().data_mut(|d| d.insert_temp(reveal_id, reveal && selected_line.is_none()));
        let mut area = egui::ScrollArea::both().id_salt(("ndr-pane-tree-scroll", index)).auto_shrink([false, false]).max_height(rect.height());
        if let (true, Some(line)) = (reveal, selected_line) {
            let depth = rows[line].1 as f32;
            area = area.vertical_scroll_offset((line as f32 * row_height - rect.height() / 2.0).max(0.0)).horizontal_scroll_offset((depth * 16.0 - 16.0).max(0.0));
        } else if let (true, Some(line)) = (std::mem::take(&mut self.panes[index].tree_scroll_to), selected_line) {
            // 글쇠로 옮긴 줄이 보이는 범위(지난 장면) 밖이면 그 끝에 닿게만 넘긴다(TTreeView처럼).
            let (first, page) = (self.panes[index].tree_first, self.panes[index].tree_page.max(1));
            if line < first {
                area = area.vertical_scroll_offset(line as f32 * row_height);
            } else if line >= first + page {
                area = area.vertical_scroll_offset(((line + 1) as f32 * row_height - rect.height()).max(0.0));
            }
        }
        let mut visible = 0..0;
        area.show_rows(&mut child, row_height, rows.len(), |ui, range| {
            visible = range.clone();
            ui.spacing_mut().item_spacing.y = 0.0;
            for (line, (node, depth)) in rows[range.clone()].iter().enumerate() {
                let galley = ui.painter().layout_no_wrap(node.name.clone(), font.clone(), Color32::PLACEHOLDER);
                let indent = *depth as f32 * 16.0;
                let width = (indent + 16.0 + 18.0 + galley.size().x + 6.0).max(ui.available_width());
                let (row, response) = ui.allocate_exact_size(egui::vec2(width, row_height), egui::Sense::click());
                let selected = selected_line == Some(range.start + line);
                let text_rect = egui::Rect::from_min_size(egui::pos2(row.left() + indent + 34.0, row.top()), egui::vec2(galley.size().x + 4.0, row_height));
                if selected {
                    ui.painter().rect_filled(text_rect, 0.0, select);
                }
                // 펼침 표시(▸/▾): 셸이 하위 폴더가 있다고 한 항목(SFGAO_HASSUBFOLDER)만, 읽어 보니 비었으면 지운다.
                let arrow = egui::Rect::from_min_size(egui::pos2(row.left() + indent, row.top()), egui::vec2(16.0, row_height));
                let open = self.panes[index].tree_open.iter().any(|k| same_key(k, &node.key));
                let expandable = node.has_children && !self.tree_cache.get(&node.key).is_some_and(|c| c.is_empty());
                if expandable {
                    // 목록 글꼴에 ▸·▾ 글자가 없을 수 있어 세모 도형으로 그린다.
                    let c = arrow.center();
                    let points = if open { vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)] } else { vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)] };
                    ui.painter().add(egui::Shape::convex_polygon(points, text, egui::Stroke::NONE));
                }
                if let Some(icon) = self.shell_node_icon(ui.ctx(), node) {
                    ui.painter().image(icon.id(), egui::Rect::from_center_size(egui::pos2(row.left() + indent + 25.0, row.center().y), egui::vec2(16.0, 16.0)), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                ui.painter().galley(egui::pos2(text_rect.left() + 2.0, row.center().y - galley.size().y / 2.0), galley, if selected { selected_text } else { text });
                if response.clicked() {
                    let on_arrow = response.interact_pointer_pos().is_some_and(|p| arrow.contains(p));
                    if on_arrow && expandable {
                        toggled = Some(node.key.clone());
                    } else {
                        picked = Some(node.clone());
                    }
                }
                if response.double_clicked() && expandable {
                    toggled = Some(node.key.clone());
                }
                if let Some(folder) = &node.path {
                    // 원본 FUN_0048da38: 트리도 놓을 곳(TNXDropFileTarget.Register(트리)). 파일 시스템 폴더만.
                    drop_rows.push((index, row.intersect(ui.clip_rect()), folder.clone()));
                    // 원본 FUN_0048e69c(트리 OnContextPopup): 고른 폴더의 셸 메뉴(FUN_00459128).
                    if response.secondary_clicked() {
                        shell_menu = Some(folder.clone());
                    }
                }
            }
        });
        self.folder_rows.extend(drop_rows);
        // 원본 FUN_0048e2e0(트리 패널 그리기): 오른쪽 끝(너비 − 1)에 스킨 색(DAT_008d44c0 + 0x1c, conf.ini [List] lframetree) 세로선.
        if let Some(skin) = self.current_skin() {
            let color = super::skin_pane::skin_color(&skin, "lframetree", Color32::from_gray(0x20));
            ui.painter().vline(rect.right() - 0.5, rect.y_range(), egui::Stroke::new(1.0, color));
        }
        if let Some(path) = shell_menu {
            self.active = index;
            if let Err(error) = ndr::shell_menu::show(&[path]) {
                self.status = error;
            }
        }
        // 다음 장면의 글쇠 처리(tree_keys)가 쓰는 줄·자리.
        {
            let open = &self.panes[index].tree_open;
            let rows: Vec<(ShellNode, usize, bool, bool)> = rows
                .into_iter()
                .map(|(node, depth)| {
                    let expandable = node.has_children && !self.tree_cache.get(&node.key).is_some_and(|c| c.is_empty());
                    let is_open = open.iter().any(|k| same_key(k, &node.key));
                    (node, depth, expandable, is_open)
                })
                .collect();
            let pane = &mut self.panes[index];
            pane.tree_rows = rows;
            pane.tree_rect = Some(rect);
            pane.tree_first = visible.start;
            pane.tree_page = ((rect.height() / row_height).floor() as usize).max(1);
        }
        let ctx = ui.ctx().clone();
        if let Some(key) = toggled {
            self.tree_focus = Some(index);
            self.tree_toggle(index, &key, &ctx);
        }
        if let Some(node) = picked {
            self.tree_focus = Some(index);
            self.tree_select(index, &node, &ctx);
        }
    }

    /// 트리 항목 펼치기·접기. 고른 항목이 접힌 항목 안에 있으면 TTreeView처럼 접힌 항목이 골라진다(OnChange).
    fn tree_toggle(&mut self, index: usize, key: &Path, ctx: &egui::Context) {
        use ndr::shell_tree::same_key;
        let open = &mut self.panes[index].tree_open;
        let before = open.len();
        open.retain(|k| !same_key(k, key));
        if open.len() == before {
            open.insert(key.to_path_buf());
            return;
        }
        let rows = &self.panes[index].tree_rows;
        let Some(at) = rows.iter().position(|(node, ..)| same_key(&node.key, key)) else { return };
        let depth = rows[at].1;
        let selected = self.panes[index].tree_selected.clone().unwrap_or_else(|| self.panes[index].path.clone());
        let inside = rows[at + 1..].iter().take_while(|(_, d, ..)| *d > depth).any(|(node, ..)| same_key(&node.key, &selected));
        if inside {
            let node = rows[at].0.clone();
            self.tree_select(index, &node, ctx);
        }
    }

    /// 원본 FUN_0048e320(트리 OnChange, TCustomTreeView +0x358): 고른 항목이 바뀌면 제어판(CSIDL_CONTROLS)은 control.exe,
    /// 휴지통(CSIDL_BITBUCKET)은 셸로 열고(FUN_00432fe4), 가상 폴더는 셸로 열고, 폴더면 그 패널을 그 폴더로 옮긴다.
    /// 원본 FUN_0048e530(트리 OnEnter): 그 패널이 지금 패널이 된다.
    fn tree_select(&mut self, index: usize, node: &ndr::shell_tree::ShellNode, ctx: &egui::Context) {
        use ndr::shell_tree::{Special, same_key};
        self.active = index;
        self.panes[index].tree_selected = Some(node.key.clone());
        self.panes[index].tree_scroll_to = true;
        match node.special {
            Special::ControlPanel => self.dispatch("controlPanel", ctx),
            Special::RecycleBin => self.dispatch("openRecycle", ctx),
            Special::None if ndr::shell_tree::is_virtual(&node.key) => {
                if let Err(error) = (self.shell_runner)(&node.key, "", Path::new("")) {
                    self.status = error;
                }
            }
            Special::None => {
                if let Some(folder) = node.path.clone().filter(|p| !same_key(p, &self.panes[index].path)) {
                    self.navigate(index, folder, true);
                }
            }
        }
    }

    /// 패널 폴더 트리에 글쇠 초점이 있을 때 TTreeView 글쇠: ↑↓ Home End PgUp PgDn은 고른 항목을 옮기고(그때마다 OnChange),
    /// ←는 접거나 부모로, →는 펼치거나 첫 자식으로, + - 는 펼치기·접기, 글자는 그 글자로 시작하는 다음 항목.
    /// 다른 글쇠(기능키·Esc 등)는 원본처럼 목록과 같은 KeyDown(FUN_0048423c)으로 넘긴다. 처리했으면 true.
    pub(super) fn tree_keys(&mut self, ctx: &egui::Context) -> bool {
        use ndr::shell_tree::same_key;
        let index = self.active;
        if self.tree_focus != Some(index) || !self.settings.flag(if index == 0 { "ShowLeftFolderTree" } else { "ShowRightFolderTree" }, false) {
            return false;
        }
        let rows = self.panes[index].tree_rows.clone();
        if rows.is_empty() {
            return false;
        }
        let selected = self.panes[index].tree_selected.clone().unwrap_or_else(|| self.panes[index].path.clone());
        let current = rows.iter().position(|(node, ..)| same_key(&node.key, &selected));
        let (key, typed) = ctx.input(|i| {
            let key = i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.ctrl && !modifiers.alt => Some(*key),
                _ => None,
            });
            let typed = i.events.iter().find_map(|e| if let egui::Event::Text(t) = e { t.chars().next() } else { None });
            (key, typed.filter(|_| !i.modifiers.ctrl && !i.modifiers.alt))
        });
        let last = rows.len() - 1;
        let at = current.unwrap_or(0);
        let page = self.panes[index].tree_page.max(1);
        let target = match key {
            Some(egui::Key::ArrowUp) => Some(at.saturating_sub(1)),
            Some(egui::Key::ArrowDown) => Some((at + 1).min(last)),
            Some(egui::Key::Home) => Some(0),
            Some(egui::Key::End) => Some(last),
            Some(egui::Key::PageUp) => Some(at.saturating_sub(page - 1)),
            Some(egui::Key::PageDown) => Some((at + page - 1).min(last)),
            Some(egui::Key::ArrowLeft) => {
                let (node, depth, expandable, open) = &rows[at];
                if *expandable && *open {
                    let key = node.key.clone();
                    self.tree_toggle(index, &key, ctx);
                    return true;
                }
                rows[..at].iter().rposition(|(_, d, ..)| *d + 1 == *depth)
            }
            Some(egui::Key::ArrowRight) => {
                let (node, depth, expandable, open) = &rows[at];
                if *expandable && !*open {
                    let key = node.key.clone();
                    self.tree_toggle(index, &key, ctx);
                    return true;
                }
                rows.get(at + 1).filter(|(_, d, ..)| *d == depth + 1).map(|_| at + 1)
            }
            Some(egui::Key::Plus | egui::Key::Minus) => {
                let (node, _, expandable, open) = &rows[at];
                if *expandable && (*open == (key == Some(egui::Key::Minus))) {
                    let key = node.key.clone();
                    self.tree_toggle(index, &key, ctx);
                }
                return true;
            }
            Some(_) | None => {
                // 글자: 지금 항목 다음부터(끝이면 처음부터) 그 글자로 시작하는 항목(TTreeView 증분 찾기의 한 글자).
                let Some(c) = typed.filter(|c| !c.is_control() && *c != ' ') else { return false };
                let lower: String = c.to_lowercase().collect();
                let hit = (1..=rows.len()).map(|step| (at + step) % rows.len()).find(|&i| rows[i].0.name.to_lowercase().starts_with(&lower));
                match hit {
                    Some(i) => Some(i),
                    None => return true,
                }
            }
        };
        if let Some(target) = target.filter(|&t| Some(t) != current) {
            let node = rows[target].0.clone();
            self.tree_select(index, &node, ctx);
        }
        true
    }

    /// 트리의 하위 셸 폴더를 따로 읽는다(숨김 폴더는 그 패널의 숨김 보기를 따른다, 원본 FUN_0048da38 +0x3f8 비트 4).
    fn request_tree_children(&mut self, index: usize, key: &Path) {
        if !self.tree_requests.insert(key.to_path_buf()) {
            return;
        }
        let path = key.to_path_buf();
        let show_hidden = self.panes[index].show_hidden;
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let directories = ndr::shell_tree::children(&path, show_hidden);
            let _ = tx.send(AsyncResult::Tree { path, directories });
        });
    }
}

/// 패널 폴더 트리 고른 줄 (바탕, 글) 색: VCL TCustomTreeView가 고른 항목의 붓을 clHighlight(`highlight` = GetSysColor(13))로 두고,
/// 원본 OnCustomDrawItem FUN_0048e104가 글 색을 트리 바탕이 검정이면 흰색, 아니면 트리 글꼴 색으로 둔다.
pub(super) fn tree_selection_colors(background: Color32, fore: Color32, highlight: (u8, u8, u8)) -> (Color32, Color32) {
    let (r, g, b) = highlight;
    (Color32::from_rgb(r, g, b), if background == Color32::BLACK { Color32::WHITE } else { fore })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_tree_selection_uses_the_system_highlight_like_the_ttreeview() {
        let highlight = (0, 120, 215);
        // 바탕이 검정(원본 처음 값 NXBACK 000000)이면 흰 글, 아니면 트리 글꼴 색(NXFORE). 바탕은 늘 시스템 강조 색.
        assert_eq!(tree_selection_colors(Color32::BLACK, Color32::from_gray(0xC0), highlight), (Color32::from_rgb(0, 120, 215), Color32::WHITE));
        assert_eq!(tree_selection_colors(Color32::WHITE, Color32::from_rgb(1, 2, 3), highlight), (Color32::from_rgb(0, 120, 215), Color32::from_rgb(1, 2, 3)));
    }

    /// 글자 10px, '.'만 2px인 가짜 글꼴.
    fn glyph(c: char) -> f32 {
        if c == '.' { 2.0 } else { 10.0 }
    }

    #[test]
    fn thumbnail_caption_follows_drawthumbnailtext() {
        // 폭 35: 줄마다 3글자. 4줄 안에 다 들어가면 그대로.
        assert_eq!(thumbnail_lines("abcdefghij", 35.0, 4, &mut glyph), ["abc", "def", "ghi", "j"]);
        // 넷째 줄에 남은 글이 넘치면 들어가는 글자 수 − 1부터 줄이며 "..."(여기서는 6px)를 붙인다.
        assert_eq!(thumbnail_lines("abcdefghijklmnop", 35.0, 4, &mut glyph), ["abc", "def", "ghi", "jk..."]);
        assert_eq!(thumbnail_lines("ab", 35.0, 4, &mut glyph), ["ab"]);
        assert!(thumbnail_lines("", 35.0, 4, &mut glyph).is_empty());
    }

    #[test]
    fn thumbnail_height_follows_getthumbnailtextheight() {
        // 줄 수 × (줄 높이 + 2), 최대 4줄, 드라이브 항목은 + 8.
        assert_eq!(thumbnail_text_height("ab", 35.0, 4, 2.0, 15.0, false, &mut glyph), 17.0);
        assert_eq!(thumbnail_text_height("abcdefg", 35.0, 4, 2.0, 15.0, false, &mut glyph), 51.0);
        assert_eq!(thumbnail_text_height("abcdefghijklmnop", 35.0, 4, 2.0, 15.0, false, &mut glyph), 68.0);
        assert_eq!(thumbnail_text_height("ab", 35.0, 4, 2.0, 15.0, true, &mut glyph), 25.0);
    }

    #[test]
    fn captions_narrower_than_one_glyph_still_end() {
        // 칸(8px)이 글자(10px)보다 좁다: 전에는 높이 계산이 끝나지 않았고 이름이 빈 줄로 보였다.
        assert_eq!(thumbnail_text_height("abcdefg", 8.0, 4, 2.0, 15.0, false, &mut glyph), 68.0);
        assert_eq!(thumbnail_lines("abcdefg", 8.0, 4, &mut glyph), ["a", "b", "c", "d"]);
        // 넷째 줄에 "..."를 붙일 자리가 없으면 들어가는 글자만 남긴다(전에는 그 줄이 빠졌다).
        assert_eq!(thumbnail_lines("abcdefghij", 12.0, 4, &mut glyph), ["a", "b", "c", "d"]);
    }
}
