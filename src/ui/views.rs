use super::*;

/// Reject empty, malformed and fully transparent icons before installing them
/// into the long-lived texture cache. Shell occasionally returns a usable
/// handle but no visible pixels while its icon handlers are initializing.
pub(super) fn valid_small_icon_pixels(width: usize, height: usize, rgba: &[u8]) -> bool {
    width > 0 && height > 0 && width <= 256 && height <= 256
        && width.checked_mul(height).and_then(|pixels| pixels.checked_mul(4)) == Some(rgba.len())
        && rgba.chunks_exact(4).any(|pixel| pixel[3] != 0)
}

const ICON_RETRY_INTERVAL: Duration = Duration::from_secs(3);

/// Pane folder tree level width (VCL TCustomTreeView Indent default 19, matching the captured original).
const TREE_INDENT: f32 = 19.0;

/// Formats whose shell icon differs per file (executables and icon files whose DefaultIcon is "%1", shortcuts that use the target's icon, etc.).
pub(super) fn has_own_icon(path: &Path) -> bool {
    const OWN: [&str; 10] = ["exe", "lnk", "ico", "cur", "ani", "url", "scr", "cpl", "msc", "appref-ms"];
    path.extension().is_some_and(|e| OWN.iter().any(|o| e.eq_ignore_ascii_case(o)))
}

/// Original DrawThumbnailText(0x7e1998): split into lines by character, as many as fit in the width (GetTextExtentExPointW nFit) each (up to max_lines lines).
/// If the remaining text does not fully fit on the last line, draw it at the first length that fits after appending "...", shrinking from the fitting character count − 1 downward.
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
        // Fix: if the cell is narrower than the first character, only empty lines accumulated. Put at least one character on each line.
        let n = fit(rest, glyph).max(1);
        if (line < max_lines && offset + n < chars.len()) || extent(rest, glyph) <= width {
            lines.push(rest[..n].iter().collect());
            offset += n;
        } else {
            let shortened = (1..n).rev().find_map(|keep| {
                let candidate: String = rest[..keep].iter().chain("...".chars().collect::<Vec<_>>().iter()).collect();
                (extent(&candidate.chars().collect::<Vec<_>>(), glyph) <= width).then_some(candidate)
            });
            // Fix: if there was no room to append "...", the whole last line was dropped. In that case only the fitting characters are shown.
            lines.push(shortened.unwrap_or_else(|| rest[..n].iter().collect()));
            break;
        }
    }
    lines
}

/// Original GetThumbnailTextHeight(0x7e1858): needed line count (up to max_lines) × (line height + spacing), plus 8 for a drive item (kind 4).
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
    /// Icons: none when ShowIcons is off (original SetShowIcon 0x7dc648 stops the image thread); the extension icon is read once per extension (folders use "<DIR>").
    /// Is this a format whose shell icon differs per file: in the has_own_icon fixed list, or the result of asking HKCR once per extension (system.per_file_icon).
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
        // Original ".." item: DrawListItem (0x7d8f58) first draws GetIconIndex(name, attributes) (relative name = SHGFI_USEFILEATTRIBUTES, the folder icon),
        // then the image thread (StartImageThread 0x7defc8, Execute 0x7de870, FUN_007dd51c) replaces it in a local pane with the shell icon of the parent
        // folder itself (PathRemoveFileSpecW of the current folder, SHGFI_SYSICONINDEX | SHGFI_ICON | SHGFI_OVERLAYINDEX), so above a drive root it is
        // that drive's icon. Archive and FTP panes (relative paths) keep the folder icon. Previously the folder icon was used everywhere.
        if entry.is_parent() {
            let path = entry.path.clone();
            let local = self.panes[index].archive.is_none() && self.panes[index].remote.is_none() && path.is_absolute();
            let real_path = self.settings.flag("UseOverlayIcons", true) || path.parent().is_none();
            if self.settings.flag("ShowIcons", true) && local && real_path {
                match self.overlay_icons.get(&path) {
                    Some(Some(texture)) => return Some(texture.clone()),
                    Some(None) | None => self.queue_overlay_icon_if_due(path, true),
                }
            }
            return self.path_icon(ctx, Path::new("A"), true);
        }
        // A .ndr encrypted container looks like a folder in NDR while
        // remaining an ordinary file for OS copy/move/delete operations.
        if self.panes[index].archive.is_none() && self.panes[index].remote.is_none()
            && entry.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ndr")) {
            return self.path_icon(ctx, Path::new("A"), true);
        }
        let (path, directory, drive, attributes) = (entry.path.clone(), entry.is_dir, entry.drive, entry.attributes);
        // Original DrawListItem (0x7d8f58): GetIconIndex of the bare name (extension icon) until the image thread (FUN_007dd51c) stores the item's shell
        // icon by actual path with its overlay index (SHGFI_SYSICONINDEX | SHGFI_ICON | SHGFI_OVERLAYINDEX) for every local item, drives included.
        // Original default value [Operation] overlay=1.
        let overlays = self.settings.flag("UseOverlayIcons", true);
        let local = self.panes[index].archive.is_none() && self.panes[index].remote.is_none() && path.is_absolute();
        // With overlays off, formats with the same icon per extension stay on the extension cache, and only formats whose icon differs per file
        // (fixed list + HKCR DefaultIcon "%1"·shellex\IconHandler formats) are read by real path (without overlay); the result looks the same.
        let own_icon = !directory && local && !overlays && self.per_file_icon_type(&path);
        // A folder is read by real path only when the shell reads desktop.ini (read-only or system attribute, custom folder icon or special folder).
        const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
        const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
        let own_folder_icon = directory && local && !overlays && !drive && attributes & (FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_SYSTEM) != 0;
        let real_path = drive || (overlays && local) || own_icon || own_folder_icon;
        if self.settings.flag("ShowIcons", true) && real_path {
            match self.overlay_icons.get(&path) {
                Some(Some(texture)) => return Some(texture.clone()),
                Some(None) | None => self.queue_overlay_icon_if_due(path.clone(), overlays || drive),
            }
        }
        self.path_icon(ctx, &path, directory)
    }

    /// Drive root shell icon (original FUN_00483108: SHGetFileInfo(root, SHGFI_ICON | SHGFI_SMALLICON)). Before it arrives, the folder icon.
    pub(super) fn drive_icon(&mut self, ctx: &egui::Context, root: &Path) -> Option<egui::TextureHandle> {
        if !self.settings.flag("ShowIcons", true) {
            return None;
        }
        match self.overlay_icons.get(root) {
            Some(Some(texture)) => return Some(texture.clone()),
            Some(None) | None => self.queue_overlay_icon_if_due(root.to_path_buf(), false),
        }
        self.path_icon(ctx, root, true)
    }

    /// C# QueueShellOverlayIcon / EnsureShellOverlayWorker: start an STA worker thread on first call, and send the same path only once.
    /// When overlay is false, the file's shell icon without overlay (native::shell_path_icon).
    fn queue_overlay_icon(&mut self, path: PathBuf, overlay: bool) {
        self.queue_icon_job(path, if overlay { 1 } else { 0 });
    }

    /// Do not permanently cache failed shell icon lookups, but also do not
    /// hammer Explorer on every frame when it is temporarily unavailable.
    fn queue_overlay_icon_if_due(&mut self, path: PathBuf, overlay: bool) {
        if self.overlay_icon_failed_at.get(&path).is_some_and(|at| at.elapsed() < ICON_RETRY_INTERVAL) {
            return;
        }
        self.queue_overlay_icon(path, overlay);
    }

    /// Icon job: 0 = the file's shell icon, 1 = overlay icon, 2 = shell key (desktop or virtual folders in the pane's folder tree).
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

    /// Icon of a pane folder-tree item (TNXShellTreeView: that item's icon from the shell image list). Before it is read, the folder icon.
    /// The running original shows these icons with both the list's showicon=0 and [Tree] tshowicon=0 (that key is the F10 window's), so the
    /// list's ShowIcons does not apply. Fixed: previously the tree lost its icons with the list icons turned off.
    pub(super) fn shell_node_icon(&mut self, ctx: &egui::Context, node: &ndr::shell_tree::ShellNode) -> Option<egui::TextureHandle> {
        match self.overlay_icons.get(&node.key) {
            Some(Some(texture)) => Some(texture.clone()),
            Some(None) => {
                if !self.overlay_icon_failed_at.get(&node.key).is_some_and(|at| at.elapsed() < ICON_RETRY_INTERVAL) {
                    self.queue_icon_job(node.key.clone(), 2);
                }
                self.any_path_icon(ctx, &node.key, true)
            }
            None => {
                self.queue_icon_job(node.key.clone(), 2);
                self.any_path_icon(ctx, &node.key, true)
            }
        }
    }

    /// Small shell icon for a single path (extension cache). Also used as the drag image.
    pub(super) fn path_icon(&mut self, ctx: &egui::Context, path: &Path, directory: bool) -> Option<egui::TextureHandle> {
        if !self.settings.flag("ShowIcons", true) {
            return None;
        }
        self.any_path_icon(ctx, path, directory)
    }

    /// `path_icon` without the list's ShowIcons setting (the pane folder tree always shows icons).
    pub(super) fn any_path_icon(&mut self, ctx: &egui::Context, path: &Path, directory: bool) -> Option<egui::TextureHandle> {
        let key = if directory {
            "<DIR>".to_owned()
        } else {
            path.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default()
        };
        if let Some(cached) = self.file_icons.get(&key) {
            if cached.is_some() {
                return cached.clone();
            }
            if self.file_icon_failed_at.get(&key).is_some_and(|at| at.elapsed() < ICON_RETRY_INTERVAL) {
                return None;
            }
        }
        // Important: the <DIR> cache is shared by *all* pane folders. Passing
        // a real drive root to SHGetFileInfoW(... USEFILEATTRIBUTES) returns a
        // DRIVE icon (C:\ index 158), not the generic folder icon (index 3).
        // That drive icon could poison the <DIR> cache for every archive and
        // FTP folder. Always use a stable, synthetic folder path for this key.
        let sample = if directory { Path::new("folder") } else { path };
        let mut image = (self.system.file_icon)(sample, directory)
            .filter(|(width, height, rgba)| valid_small_icon_pixels(*width, *height, rgba));
        if directory && image.is_none() && !cfg!(test) {
            // Independent Windows stock icon fallback: never let a transient
            // shell handler failure leave all archive folders blank.
            image = native::shell_stock_folder_icon()
                .filter(|(width, height, rgba)| valid_small_icon_pixels(*width, *height, rgba));
        }
        let texture = image.map(|(width, height, rgba)| self.icon_texture(ctx, format!("ndr-file-icon{key}"), width, height, &rgba));
        if texture.is_some() {
            self.file_icon_failed_at.remove(&key);
        } else {
            self.file_icon_failed_at.insert(key.clone(), Instant::now());
            ctx.request_repaint_after(ICON_RETRY_INTERVAL);
        }
        self.file_icons.insert(key, texture.clone());
        texture
    }

    /// Upload the shell icon image, and also build the original dimmed icon (ImageList_DrawEx ILD_BLEND50, rgbFg white = blend the color halfway with white, keeping alpha).
    pub(super) fn icon_texture(&mut self, ctx: &egui::Context, name: String, width: usize, height: usize, rgba: &[u8]) -> egui::TextureHandle {
        let texture = ctx.load_texture(name.clone(), egui::ColorImage::from_rgba_unmultiplied([width, height], rgba), egui::TextureOptions::LINEAR);
        let ghost: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [((p[0] as u16 + 255) / 2) as u8, ((p[1] as u16 + 255) / 2) as u8, ((p[2] as u16 + 255) / 2) as u8, p[3]]).collect();
        let ghost = ctx.load_texture(format!("{name}|ghost"), egui::ColorImage::from_rgba_unmultiplied([width, height], &ghost), egui::TextureOptions::LINEAR);
        self.ghost_icons.insert(texture.id(), ghost);
        texture
    }

    /// Original DrawListItem(0x7d8f58)·DrawThumbnailItem: cut items (+100), hidden attribute (2), and names starting with "." in an SFTP listing (+0x78)
    /// draw the icon dimmed (skin theme). If a dimmed image exists, return it.
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

    /// Original GetListHeaderWidth: header width before the name column (display column 12 + icon 16 + 2, or 14 when icons are hidden).
    pub(super) fn list_header_width(icons: bool) -> f32 {
        if icons { 30.0 } else { 14.0 }
    }

    /// Original DrawListItem(0x7d8f58) name column: the icon is at column left + 12 (the left 12 pixels are the display ▶ column), vertically centered ((item height − 16) / 2).
    /// The name runs from column left + header width (`list_header_width`) + 5 to right − 5 as a single line (ellipsized with …, DT_END_ELLIPSIS when it overflows).
    /// Fix (user request: keep the icon and text from touching the guide lines): previously it was drawn from the egui button inner padding (4 pixels), so it stuck to the column line and pane border.
    /// style = original ListFont style (underline/strikethrough are applied via list_text).
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
        // Skin-theme preview: original DrawThumbnailItem(0x7da1c4)·DrawFocus style 3. The name text color is always NXFORE (Font.Color = GetBasic(1)).
        let skin = self.current_skin();
        let original = Some(self.original_list_colors());
        let original_fore = original.map(|_| self.basic_list_color("NXFORE"));
        // Cursor bar style follows the list's real focus (see pane_ui).
        let list_focused = self.active == index && self.list_has_focus(ui.ctx());
        // Original skin vertical scrollbar (TNXScrollBar, same bar for the preview): shown only when content overflows, and its width narrows the cell area (same as detail view).
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
        // Original UpdateDisplayThumbnails(0x7e1ba0): column width = thumbnail size (+0x3b0, ini thumbsize) + 12 determines the column count,
        // and the leftover width is divided among the columns to widen them. Cell height = size + 12 + text height. The C# version also draws the image at ThumbnailSize.
        let thumb = self.settings.values.get("ThumbnailSize").and_then(|v| v.as_i64()).unwrap_or(120).clamp(32, 512) as f32;
        let tile_width = if thumbnails { thumb + 12.0 } else { 230.0 };
        // C# ResolveColumnCount: for the name list, if Left/RightColumnCount (1~19) use that count; if 0 (auto), divide by the widest name + 14 (+ icon 16) width.
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
        // Original UpdateDisplayThumbnails: row height = the largest GetThumbnailTextHeight in that row (width = thumbnail size, line spacing 2, up to 4 lines) + size + 12.
        let spacing_y = if thumbnails { 0.0 } else { ui.spacing().item_spacing.y };
        let gdi_line = self.list_text_height();
        let heights: Vec<f32> = if thumbnails {
            let font = egui::TextStyle::Body.resolve(ui.style());
            // Line unit = Canvas.TextHeight (GDI tmHeight), like the list rows (App::list_text_height).
            let line_height = gdi_line.unwrap_or_else(|| ui.fonts_mut(|fonts| fonts.row_height(&font)));
            // Fix (speed): previously the name width of every item was measured each frame (15ms per frame for 20,000 items), making the preview view slow.
            // Like the original, remeasure only when the list, column count, or size changes (keyed by item order, name, and drive label).
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
                    // Original GetThumbnailRowHeight (0x7d6920): the tallest GetThumbnailTextHeight (cell width, 2, 4) of the row's items plus the
                    // fixed item height; rows before the last read it from the stored item tops, which come from the same sum.
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
        // Original UpdateScrollBar: the preview (style 3) uses one line per step regardless of the page setting.
        let (by_page, wheel_lines) = if thumbnails { (false, 1) } else { (self.scroll_by_page(index), (self.system.wheel_lines)()) };
        let current = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(egui::Id::new(("tiles", index)))).map_or(0.0, |s| s.offset.y);
        let here = line_at(current);
        let stride = heights.get(here).map_or(26.0, |h| *h) + spacing_y;
        let wheel = Self::wheel_scroll(ui, egui::Id::new(("tiles", index)), height, stride, by_page, wheel_lines);
        // Original DoTargetScroll: while dragging, scroll one line (one page in page mode) at the top or bottom edge.
        let dy = std::mem::take(&mut self.panes[index].drag_scroll.0);
        self.panes[index].drag_scroll.1 = 0;
        let wheel = if dy != 0 {
            let step = if by_page { Self::wheel_step(true, height, stride, 1) } else { stride };
            Some((current + dy as f32 * step).max(0.0))
        } else {
            wheel
        };
        let mut area = egui::ScrollArea::vertical()
            .id_salt(("tiles", index))
            .scroll_source(super::panes::WHEEL_OFF)
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height);
        if skin_bar.is_some() {
            area = area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        }
        // Since each row has a different height, the wheel moves by whole rows.
        let mut offset = match wheel {
            Some(target) if rows > 0 => {
                let steps = ((target - current) / stride).round() as isize;
                tops[(here as isize + steps).clamp(0, rows as isize - 1) as usize]
            }
            Some(target) => target,
            None => current,
        };
        // Original SetItemIndex (style 3)·ArrowKey: scroll so the cursor row is visible, and leave the columns per row and the count of fully visible rows.
        // Original AdjustPage (style 3): if that item's row is off the top, go to the very top; if off the bottom, to the very bottom. A re-read list is not adjusted
        // because the original restores it afterward with SetScrollTop (old position).
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
                        // Original UpdateDisplayThumbnails(0x7e1ba0): cell width = list width / column count, no gap between cells (or between rows).
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
                            // Thumbnail placeholder icon: the list icon, or the shell icon even with ShowIcons off (see DrawThumbnailItem below).
                            let thumb_icon = if !thumbnails {
                                None
                            } else if icon.is_some() {
                                icon.clone()
                            } else {
                                let fallback = self.any_path_icon(ui.ctx(), &entry.path, entry.is_dir);
                                self.row_icon_for_skin(index, row, fallback)
                            };
                            // Original DrawThumbnailItem: for a dimmed item (cut·hidden·SFTP "." name), the preview image uses alpha 0x55.
                            let ghosted = self.row_ghosted(index, row);
                            // Original DrawThumbnailItem(0x7da1c4) kind 4: instead of the name, "%s (%s:)" (volume name, drive letter), with a free-space bar below the text.
                            let drive_info = if thumbnails && entry.drive { self.panes[index].drive_infos.get(&entry.path).cloned() } else { None };
                            let caption = match &drive_info {
                                Some((label, _, _)) => format!("{label} ({}:)", entry.name.trim_end_matches(['\\', ':'])),
                                None => entry.name.clone(),
                            };
                            // Skin theme: for the cursor and marked cells, the original bar (DrawFocus style 3) is laid under the image and text (reserve the slot first, then fill once the cell size is known).
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
                                                // Stack the image, text lines (original each line height + 2), and drive bar with no gap (row height is computed above).
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                // The cell uses the full row height (even cell background and clickable area).
                                                ui.set_min_size(egui::vec2(tile_width - 10.0, row_height - 10.0));
                                            }
                                            if thumbnails {
                                                // Original: for a dimmed item, the image uses alpha 0x55 (AlphaBlend SourceConstantAlpha), and the icon uses ILD_BLEND50 (dimmed icon image). Skin theme only.
                                                let tint = if ghosted { Color32::from_white_alpha(0x55) } else { Color32::WHITE };
                                                if let Some(texture) = self.images.get(&entry.path)
                                                {
                                                    ui.add(egui::Image::new((
                                                        texture.id(),
                                                        egui::vec2(thumb, thumb),
                                                    )).tint(tint));
                                                } else if let Some(icon) = &thumb_icon {
                                                    // Original DrawThumbnailItem (0x7da1c4): until the item has a thumbnail (+0x54 == -1) it draws the shell
                                                    // image list icon (GetIconIndex, size 2) centered in the image square, whatever the list's icon setting.
                                                    // Fixed: NDR wrote "[DIR]"/"[FILE]" there when list icons were off.
                                                    ui.add_sized([thumb, thumb], egui::Image::new((icon.id(), egui::vec2(32.0, 32.0))));
                                                } else {
                                                    ui.add_space(thumb);
                                                }
                                            }
                                            // The skin theme uses NXFORE like the original (regardless of kind); otherwise the C# version's color.
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
                                                    // Original DrawThumbnailText: width = thumbnail size, up to 4 lines, each line centered, line spacing 2.
                                                    let font = egui::TextStyle::Body.resolve(ui.style());
                                                    let (line_height, lines) = ui.fonts_mut(|fonts| (gdi_line.unwrap_or_else(|| fonts.row_height(&font)), thumbnail_lines(&caption, thumb, 4, &mut |c| fonts.glyph_width(&font, c))));
                                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(thumb, lines.len() as f32 * (line_height + 2.0)), egui::Sense::hover());
                                                    for (i, line) in lines.iter().enumerate() {
                                                        ui.painter().text(egui::pos2(rect.center().x, rect.top() + 2.0 + i as f32 * (line_height + 2.0)), egui::Align2::CENTER_TOP, line, font.clone(), name_color);
                                                    }
                                                }
                                                _ => {
                                                    ui.add(egui::Label::new(name).truncate());
                                                }
                                            }
                                            // Original: for a fixed disk (kind 3) whose total size is known, a bar of width thumbnail size − 16 and height 4.
                                            // The total is FUN_007d7e6c(NXFORE, NXBACK, 0.3), and the used portion is (NXFORE, NXBACK, 0.5).
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
                            // Original DrawThumbnailItem: if DrawThumbnailText shortened the name (+0x65), after the cursor arrives ShowToolTip style 3 shows the full name.
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
                                    // NDR cell: outer margin 5, image thumb, text first line is 2 below it.
                                    let text_top = response.rect.top() + 5.0 + thumb + 2.0;
                                    *self.thumb_tip.borrow_mut() = Some((index, row, response.rect, text_top, thumb, galleys));
                                }
                            }
                            if let (Some(slot), Some(o)) = (focus_slot, original) {
                                // The bar height is measured from that item's text height (GetThumbnailTextHeight).
                                let font = egui::TextStyle::Body.resolve(ui.style());
                                let text_height = ui.fonts_mut(|fonts| {
                                    let line_height = gdi_line.unwrap_or_else(|| fonts.row_height(&font));
                                    thumbnail_text_height(&caption, thumb, 4, 2.0, line_height, entry.drive, &mut |c| fonts.glyph_width(&font, c))
                                });
                                let (top, bottom, line) = super::skin_pane::thumbnail_focus(o.select, o.back, cursor_row, marked, false, list_focused);
                                let bar = super::skin_pane::thumbnail_focus_rect(response.rect, thumb, text_height);
                                ui.painter().set(slot, egui::Shape::Vec(super::skin_pane::rounded_gradient_shapes(bar, 4.0, top, bottom, line)));
                            }
                            // Original MouseDown (style 3): the skin theme handles the cursor, Shift, and Ctrl on press (no display column or drag-select).
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
                            // Do not attach a menu to the row response (pane menu skin_pane::pane_menu).
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
                // Original UpdateScrollBar (style 3): one line per button press.
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
    /// Original pane folder tree (miShowFoldersClick 0x445258 → FUN_0048da38): to the left of the list inside the pane (TNXShellTreeView inside the alLeft TNXPaintPanel).
    /// The root is the shell desktop, and below it the shell namespace (My Computer → drives → folders, Network, Control Panel, Recycle Bin …, ndr::shell_tree).
    /// Background and font match the list (FUN_0048e1ac: list Color·Font, text color GetBasic(1)); when the pane location changes, expand to that folder and select it (SetPath).
    /// When selected (FUN_0048e320), Control Panel opens control.exe, the Recycle Bin opens via the shell (FUN_00432fe4), virtual folders open via the shell, and a folder navigates that pane.
    pub(super) fn pane_tree_ui(&mut self, ui: &mut egui::Ui, index: usize, rect: egui::Rect) {
        use ndr::shell_tree::{ShellNode, Special, same_key};
        // FUN_0048e1ac: background = list Color (NXBACK), text = GetBasic(1) (NXFORE).
        // Selected row: VCL TCustomTreeView drawing sets the selected item's (CDIS_SELECTED) brush to clHighlight, and FUN_006c9388(tree, 0) =
        // HideSelection off (TVS_SHOWSELALWAYS), so this holds even without focus. OnCustomDrawItem FUN_0048e104 sets the selected item's text color: if the tree background
        // (Color, +0x7c) is black (0), white (focused 0xFEFEFE, otherwise 0xFFFFFF); otherwise the tree font color (NXFORE).
        // Fix: previously it drew NXFORE text on an NXSELECT background (the original uses the system highlight color).
        let (background, text) = (self.basic_list_color("NXBACK"), self.basic_list_color("NXFORE"));
        let tree_focused = self.tree_focus == Some(index);
        let (selection_box, selected_text) = tree_selection_colors(background, text, native::system_color(13), tree_focused);
        ui.painter().rect_filled(rect, 0.0, background);
        let path = self.panes[index].path.clone();
        let local = self.panes[index].archive.is_none() && self.panes[index].remote.is_none() && !super::panes::is_drives(&path);
        // SetPath: when the location changes, expand from the desktop down to that folder (My Computer → drives → …) and scroll so that row is visible.
        // Original SetPathFromID (0x7ee894, reached from SetPath 0x7ee768 via the Desktop's ParseDisplayName): skipped while already updating or when the selected folder is that folder; otherwise expands
        // every ancestor node that exists (FolderExists), selects the target (or the deepest ancestor found) and makes it visible.
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
            self.shell_desktop = Some(ndr::shell_tree::desktop().unwrap_or(ShellNode { key: PathBuf::new(), name: self.tr("바탕 화면", "Desktop").into_owned(), path: None, has_children: true, special: Special::None, ghosted: false }));
        }
        let root = self.shell_desktop.clone().unwrap_or(ShellNode { key: PathBuf::new(), name: String::new(), path: None, has_children: true, special: Special::None, ghosted: false });
        let font = egui::FontId::new(self.list_font_size, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()));
        let row_height = (ui.fonts_mut(|f| f.row_height(&font)).ceil() + 2.0).max(18.0);
        let mut rows: Vec<(ShellNode, usize)> = Vec::new();
        fn walk(app: &mut App, index: usize, node: &ShellNode, depth: usize, rows: &mut Vec<(ShellNode, usize)>) {
            rows.push((node.clone(), depth));
            if depth > 32 || !app.panes[index].tree_open.iter().any(|k| ndr::shell_tree::same_key(k, &node.key)) {
                return;
            }
            match app.tree_cache.get(&(node.key.clone(), app.panes[index].show_hidden)).cloned() {
                Some(children) => {
                    for child in &children {
                        walk(app, index, child, depth + 1, rows);
                    }
                }
                None => app.request_tree_children(index, &node.key),
            }
        }
        walk(self, index, &root, 0, &mut rows);
        // The tree's own area stops 1 px before the right edge, where FUN_0048e2e0 draws the lframetree line (width - 1); the scroll bar ends next to it.
        let inner_rect = rect.with_max_x(rect.right() - 1.0);
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("ndr-pane-tree", index)).max_rect(inner_rect).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(inner_rect.intersect(ui.clip_rect()));
        let mut picked: Option<ShellNode> = None;
        let mut toggled: Option<PathBuf> = None;
        let mut label_click: Option<PathBuf> = None;
        let mut label_double = false;
        let mut tree_edit_end: Option<bool> = None;
        let mut shell_menu: Option<PathBuf> = None;
        let mut drop_rows: Vec<(usize, egui::Rect, PathBuf)> = Vec::new();
        let mut tree_rows_for_drop: Vec<(usize, egui::Rect, PathBuf, PathBuf, bool)> = Vec::new();
        // While the subfolders are still being read there is no selected row, so defer scrolling until that row appears.
        let reveal_id = egui::Id::new(("ndr-pane-tree-reveal", index));
        let reveal = reveal || ui.ctx().data(|d| d.get_temp::<bool>(reveal_id)).unwrap_or(false);
        let selected_key = self.panes[index].tree_selected.clone().or_else(|| local.then(|| path.clone()));
        let selected_line = selected_key.as_ref().and_then(|key| rows.iter().position(|(node, _)| same_key(&node.key, key)));
        ui.ctx().data_mut(|d| d.insert_temp(reveal_id, reveal && selected_line.is_none()));
        let mut area = egui::ScrollArea::both().id_salt(("ndr-pane-tree-scroll", index)).auto_shrink([false, false]).max_height(rect.height());
        if let (true, Some(line)) = (reveal, selected_line) {
            // TreeView expansion (TVM_EXPAND) scrolls so the expanded folder's items fit, which puts the parent at the top when they overflow (the
            // captured original shows "로컬 디스크 (D:)" as the first row); then the target is made visible. Previously the target was centered
            // and the view scrolled one level right.
            let depth = rows[line].1;
            let page = ((rect.height() / row_height).floor() as usize).max(1);
            let mut top = self.panes[index].tree_first;
            if let Some(parent) = (0..line).rev().find(|&i| rows[i].1 + 1 == depth) {
                let end = (parent + 1..rows.len()).find(|&i| rows[i].1 <= rows[parent].1).unwrap_or(rows.len()) - 1;
                if end + 1 - parent > page || parent < top {
                    top = parent;
                } else if end >= top + page {
                    top = end + 1 - page;
                }
            }
            if line < top {
                top = line;
            } else if line >= top + page {
                top = line + 1 - page;
            }
            let item_left = (depth as f32 - 1.0) * TREE_INDENT + 21.0;
            let horizontal = if item_left > rect.width() - 60.0 { (item_left - rect.width() / 2.0).max(0.0) } else { 0.0 };
            area = area.vertical_scroll_offset(top as f32 * row_height).horizontal_scroll_offset(horizontal);
        } else if let (true, Some(line)) = (std::mem::take(&mut self.panes[index].tree_scroll_to), selected_line) {
            // If the keyboard-moved row is outside the visible range (last frame), scroll only enough to reach that edge (like TTreeView).
            let (first, page) = (self.panes[index].tree_first, self.panes[index].tree_page.max(1));
            if line < first {
                area = area.vertical_scroll_offset(line as f32 * row_height);
            } else if line >= first + page {
                area = area.vertical_scroll_offset(((line + 1) as f32 * row_height - rect.height()).max(0.0));
            }
        } else if self.panes[index].tree_drag_scroll != 0 {
            // Original tree drop effect (0x48e790) scroll indicator + TNXDropTarget.DoTargetScroll(0x7b1e04): WM_VSCROLL one line.
            let dy = std::mem::take(&mut self.panes[index].tree_drag_scroll) as f32;
            let current = egui::scroll_area::State::load(ui.ctx(), child.make_persistent_id(egui::Id::new(("ndr-pane-tree-scroll", index)))).map_or(self.panes[index].tree_first as f32 * row_height, |s| s.offset.y);
            area = area.vertical_scroll_offset((current + dy * row_height).max(0.0));
        }
        let mut visible = 0..0;
        // Original TNXShellTreeView is a Windows TreeView with system scroll bars. Captured on Windows 11: a 17 px 0xF0F0F0 track with a 1 px white
        // edge toward the items and a 2 px 0x858585 thumb line 8 px in (7 px from the outer edge), horizontal bar alike. egui solid bars give the
        // thumb and the reserved width; the track is painted behind them below. Previously the tree used egui's floating bars (hidden until hovered).
        const TREE_TRACK: Color32 = Color32::from_rgb(0xF0, 0xF0, 0xF0);
        const TREE_THUMB: Color32 = Color32::from_rgb(0x85, 0x85, 0x85);
        {
            let style = child.style_mut();
            style.spacing.scroll = egui::style::ScrollStyle { floating: false, bar_width: 2.0, bar_inner_margin: 8.0, bar_outer_margin: 7.0, handle_min_length: 17.0, ..egui::style::ScrollStyle::solid() };
            style.visuals.extreme_bg_color = TREE_TRACK;
            for state in [&mut style.visuals.widgets.inactive, &mut style.visuals.widgets.hovered, &mut style.visuals.widgets.active] {
                state.bg_fill = TREE_THUMB;
                state.corner_radius = egui::CornerRadius::same(1);
            }
        }
        let track_slot = child.painter().add(egui::Shape::Noop);
        let scrolled = area.show_rows(&mut child, row_height, rows.len(), |ui, range| {
            visible = range.clone();
            ui.spacing_mut().item_spacing.y = 0.0;
            for (line, (node, depth)) in rows[range.clone()].iter().enumerate() {
                let galley = ui.painter().layout_no_wrap(node.name.clone(), font.clone(), Color32::PLACEHOLDER);
                // Layout measured in the running original (TTreeView Indent 19): the button cell is the level's first 19 px (glyph centered),
                // the 16 px icon starts 21 px in, the text 42 px in. FUN_0048da38 turns ShowRoot off (FUN_006c9330(tree, 0)), so the root
                // (desktop) has no button cell and level d starts at (d - 1) x 19. Previously 16 px levels from the root, the icon at 17, the text at 36.
                let indent = (*depth as f32 - 1.0) * TREE_INDENT;
                let width = (indent + 42.0 + galley.size().x + 6.0).max(ui.available_width());
                let (row, response) = ui.allocate_exact_size(egui::vec2(width, row_height), egui::Sense::click());
                let selected = selected_line == Some(range.start + line)
                    // Original FUN_0048eb0c: while dragging, the node under the pointer is the DropTarget (TreeView draws it highlighted like the selected item).
                    || (self.file_drag.is_some() && self.tree_drop_hover.as_ref().is_some_and(|(p, key, _)| *p == index && same_key(key, &node.key)));
                let text_rect = egui::Rect::from_min_size(egui::pos2(row.left() + indent + 40.0, row.top()), egui::vec2(galley.size().x + 4.0, row_height));
                if let (true, Some((fill, border))) = (selected, selection_box) {
                    // Themed selection box from 2 px left of the icon to 5 px past the text, the whole row high, 1 px border with rounded corners.
                    let boxed = egui::Rect::from_min_max(egui::pos2(row.left() + indent + 19.0, row.top()), egui::pos2(text_rect.left() + 2.0 + galley.size().x + 5.0, row.bottom()));
                    ui.painter().rect(boxed, 1.0, fill, egui::Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
                }
                // Expand indicator: only for items the shell reported as having subfolders (SFGAO_HASSUBFOLDER); removed if found empty on read.
                // Original CanExpand (0x7ede40): a folder node with no children yet is populated, then HasChildren = child count > 0.
                let arrow = egui::Rect::from_min_size(egui::pos2(row.left() + indent, row.top()), egui::vec2(TREE_INDENT, row_height));
                let open = self.panes[index].tree_open.iter().any(|k| same_key(k, &node.key));
                let expandable = node.has_children && !self.tree_cache.get(&(node.key.clone(), self.panes[index].show_hidden)).is_some_and(|c| c.is_empty());
                if expandable && *depth > 0 {
                    // The themed chevrons of the captured original: collapsed ">" 4 x 8 px, expanded "v" 8 x 4 px, 1 px strokes (tree_glyph_color).
                    let c = egui::pos2(arrow.center().x.floor() + 0.5, arrow.center().y.floor() + 1.5);
                    let points = if open { vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(0.0, 2.0), c + egui::vec2(4.0, -2.0)] } else { vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(2.0, 0.0), c + egui::vec2(-2.0, 4.0)] };
                    ui.painter().add(egui::Shape::line(points, egui::Stroke::new(1.2_f32, tree_glyph_color(background, open))));
                }
                if let Some(icon) = self.shell_node_icon(ui.ctx(), node) {
                    // Original InitNode (0x7ed85c): a ghosted folder (hidden) gets Cut = true, which TTreeView draws with the dimmed icon.
                    let icon = if node.ghosted { self.ghost_icons.get(&icon.id()).cloned().unwrap_or(icon) } else { icon };
                    ui.painter().image(icon.id(), egui::Rect::from_center_size(egui::pos2(row.left() + indent + 29.0, row.center().y), egui::vec2(16.0, 16.0)), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                }
                let editing = self.panes[index].tree_edit.as_ref().is_some_and(|(folder, _)| node.path.as_ref().is_some_and(|p| same_key(p, folder)));
                if editing {
                    // TVN_BEGINLABELEDIT: an edit field in place of the text. Enter·clicking elsewhere = commit, Esc = cancel (TreeView label editing).
                    let edit_rect = egui::Rect::from_min_size(text_rect.min, egui::vec2((galley.size().x + 40.0).max(120.0), row_height));
                    let id = egui::Id::new(("ndr-pane-tree-edit", index));
                    let (escape, enter) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter)));
                    if let Some((_, text)) = self.panes[index].tree_edit.as_mut() {
                        let edit = ui.put(edit_rect, egui::TextEdit::singleline(text).id(id).font(font.clone()).margin(egui::vec2(2.0, 0.0)));
                        if !edit.has_focus() && !edit.lost_focus() && ui.ctx().memory(|m| m.focused().is_none()) {
                            edit.request_focus();
                        }
                        if edit.lost_focus() {
                            tree_edit_end = Some(!escape || enter);
                        }
                    }
                } else {
                    ui.painter().galley(egui::pos2(text_rect.left() + 2.0, row.center().y - galley.size().y / 2.0), galley, if selected { selected_text } else { text });
                }
                if response.clicked() && !editing {
                    let on_arrow = response.interact_pointer_pos().is_some_and(|p| arrow.contains(p));
                    let on_label = response.interact_pointer_pos().is_some_and(|p| text_rect.contains(p));
                    if on_arrow && expandable {
                        toggled = Some(node.key.clone());
                    } else if selected && on_label && tree_renamable(node) && !response.double_clicked() {
                        // TreeView: clicking the text of an already-selected item again starts label editing after the double-click interval passes.
                        label_click = Some(node.key.clone());
                    } else {
                        picked = Some(node.clone());
                    }
                }
                if response.double_clicked() {
                    label_double = true;
                    if expandable {
                        toggled = Some(node.key.clone());
                    }
                }
                if let Some(folder) = &node.path {
                    // Original FUN_0048da38: the tree is also a drop target (TNXDropFileTarget.Register(tree)). Filesystem folders only.
                    drop_rows.push((index, row.intersect(ui.clip_rect()), folder.clone()));
                    tree_rows_for_drop.push((index, row.intersect(ui.clip_rect()), folder.clone(), node.key.clone(), expandable && !open));
                    // Original FUN_0048e69c (tree OnContextPopup): the shell menu of the selected folder (FUN_00459128 → FUN_00458a1c → FUN_004588e0:
                    // TrackPopupMenuEx(0x102) and InvokeCommand, no verb hooks). TNXCustomShellTreeView's own ExecuteCommand/CommandCompleted
                    // (0x7ed684/0x7ed488: "rename" edits the label, "delete" leaves the folder first, "paste"/"rfDesktop" refresh) belong to its built-in popup, which the pane tree does not use.
                    if response.secondary_clicked() {
                        shell_menu = Some(folder.clone());
                    }
                }
            }
        });
        {
            let inner = scrolled.inner_rect;
            let (vertical, horizontal) = (scrolled.content_size.y > inner.height() + 0.5, scrolled.content_size.x > inner.width() + 0.5);
            let bar = 17.0;
            let mut shapes = Vec::new();
            if vertical {
                shapes.push(egui::Shape::rect_filled(egui::Rect::from_min_max(egui::pos2(inner.right(), inner.top()), egui::pos2(inner.right() + bar, inner.bottom())), 0.0, TREE_TRACK));
                shapes.push(egui::Shape::rect_filled(egui::Rect::from_min_max(egui::pos2(inner.right(), inner.top()), egui::pos2(inner.right() + 1.0, inner.bottom())), 0.0, Color32::WHITE));
            }
            if horizontal {
                shapes.push(egui::Shape::rect_filled(egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom()), egui::pos2(inner.right(), inner.bottom() + bar)), 0.0, TREE_TRACK));
                shapes.push(egui::Shape::rect_filled(egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom()), egui::pos2(inner.right(), inner.bottom() + 1.0)), 0.0, Color32::WHITE));
            }
            if vertical && horizontal {
                shapes.push(egui::Shape::rect_filled(egui::Rect::from_min_size(inner.right_bottom(), egui::vec2(bar, bar)), 0.0, TREE_TRACK));
            }
            child.painter().set(track_slot, egui::Shape::Vec(shapes));
        }
        self.folder_rows.extend(drop_rows);
        self.tree_drop_rows.extend(tree_rows_for_drop);
        // Original FUN_0048e2e0 (tree panel drawing): a vertical line in the skin color (DAT_008d44c0 + 0x1c, conf.ini [List] lframetree) at the right edge (width − 1).
        if let Some(skin) = self.current_skin() {
            let color = super::skin_pane::skin_color(&skin, "lframetree", Color32::from_gray(0x3C));
            ui.painter().vline(rect.right() - 0.5, rect.y_range(), egui::Stroke::new(1.0_f32, color));
        }
        if let Some(path) = shell_menu {
            self.active = index;
            if let Err(error) = ndr::shell_menu::show(&[path]) {
                self.status = error;
            }
        }
        // Rows and positions used by the next frame's keyboard handling (tree_keys).
        {
            let open = &self.panes[index].tree_open;
            let rows: Vec<(ShellNode, usize, bool, bool)> = rows
                .into_iter()
                .map(|(node, depth)| {
                    let expandable = node.has_children && !self.tree_cache.get(&(node.key.clone(), self.panes[index].show_hidden)).is_some_and(|c| c.is_empty());
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
        if let Some(commit) = tree_edit_end {
            if commit {
                self.tree_rename_commit(index);
            } else {
                self.panes[index].tree_edit = None;
            }
        }
        if label_double {
            self.panes[index].tree_label_click = None;
        } else if let Some(key) = label_click {
            self.tree_focus = Some(index);
            self.panes[index].tree_label_click = Some((key, Instant::now()));
        }
        if let Some(wait) = self.tree_edit_due(index, Instant::now()) {
            ctx.request_repaint_after(wait);
        }
        if let Some(key) = toggled {
            self.tree_focus = Some(index);
            self.tree_toggle(index, &key, &ctx);
        }
        if let Some(node) = picked {
            self.tree_focus = Some(index);
            self.tree_select(index, &node, &ctx);
        }
    }

    /// Expand/collapse a tree item. If the selected item is inside a collapsed item, the collapsed item becomes selected like TTreeView (OnChange).
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

    /// Original FUN_0048e320 (tree OnChange, TCustomTreeView +0x358): when the selected item changes, Control Panel (CSIDL_CONTROLS) opens control.exe,
    /// the Recycle Bin (CSIDL_BITBUCKET) opens via the shell (FUN_00432fe4), virtual folders open via the shell, and a folder navigates that pane to it.
    /// Original FUN_0048e530 (tree OnEnter): that pane becomes the active pane.
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

    /// After clicking a selected item's text again, start label editing once the double-click time (GetDoubleClickTime) passes (TVN_BEGINLABELEDIT).
    /// If not yet, return the remaining time. Selecting another item in the meantime cancels it.
    pub(super) fn tree_edit_due(&mut self, index: usize, now: Instant) -> Option<Duration> {
        use ndr::shell_tree::same_key;
        let (key, at) = self.panes[index].tree_label_click.clone()?;
        let selected = self.panes[index].tree_selected.clone().unwrap_or_else(|| self.panes[index].path.clone());
        let node = self.panes[index].tree_rows.iter().map(|(node, ..)| node).find(|node| same_key(&node.key, &key)).cloned();
        let Some(node) = node.filter(|n| same_key(&n.key, &selected) && tree_renamable(n)) else {
            self.panes[index].tree_label_click = None;
            return None;
        };
        let wait = Duration::from_millis(native::double_click_metrics().0.max(1) as u64);
        let elapsed = now.saturating_duration_since(at);
        if elapsed < wait {
            return Some(wait - elapsed);
        }
        self.panes[index].tree_label_click = None;
        if let Some(folder) = node.path {
            self.panes[index].tree_edit = Some((folder, node.name));
        }
        None
    }

    /// Original TNXCustomShellTreeView.Edit(0x7edf70, TVN_ENDLABELEDIT): if there is a new name, change it via TNXShellFolder.Rename (shell SetNameOf) and
    /// update the node text. NDR renames with MoveFileW within the same folder (failure shows a rename-operation error dialog), re-reads that part of the tree,
    /// and a pane viewing that folder (or inside it) moves to the new path, while a pane viewing the parent is refreshed. An empty or identical name does nothing.
    pub(super) fn tree_rename_commit(&mut self, index: usize) {
        use ndr::shell_tree::same_key;
        let Some((folder, text)) = self.panes[index].tree_edit.take() else { return };
        let old = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(parent) = folder.parent().map(Path::to_path_buf) else { return };
        if text.is_empty() || text == old {
            return;
        }
        let target = parent.join(&text);
        let error = ndr::names::move_file(&folder.to_string_lossy(), &target.to_string_lossy());
        if error != 0 {
            return self.show_name_operation_error(error, false, |_, _, _| {});
        }
        let moved = |path: &Path| -> Option<PathBuf> {
            let text = path.to_string_lossy();
            let base = folder.to_string_lossy();
            let cut = base.len();
            if text.len() < cut || !text.is_char_boundary(cut) || !ndr::names::equals_ignore_case(&text[..cut], &base) {
                return None;
            }
            let rest = &text[cut..];
            (rest.is_empty() || rest.starts_with('\\')).then(|| PathBuf::from(format!("{}{rest}", target.to_string_lossy())))
        };
        self.tree_cache.retain(|(key, _), _| !same_key(key, &parent) && moved(key).is_none());
        self.tree_requests.retain(|(key, _)| !same_key(key, &parent) && moved(key).is_none());
        for i in 0..self.panes.len() {
            let open: Vec<PathBuf> = self.panes[i].tree_open.iter().cloned().collect();
            self.panes[i].tree_open = open.into_iter().map(|k| moved(&k).unwrap_or(k)).collect();
            if let Some(selected) = self.panes[i].tree_selected.clone().and_then(|k| moved(&k)) {
                self.panes[i].tree_selected = Some(selected);
            }
            let local = self.panes[i].archive.is_none() && self.panes[i].remote.is_none();
            if !local {
                continue;
            }
            let path = self.panes[i].path.clone();
            if let Some(new) = moved(&path) {
                self.panes[i].tree_synced = Some(new.clone());
                self.navigate(i, new, false);
            } else if same_key(&path, &parent) {
                self.refresh(i);
            }
        }
    }

    /// TTreeView keys while keyboard focus is in the pane folder tree: ↑↓ Home End PgUp PgDn move the selected item (OnChange each time),
    /// ← collapses or goes to the parent, → expands or goes to the first child, + - expand/collapse, a letter jumps to the next item starting with that letter.
    /// Other keys (function keys·Esc, etc.) are passed to the same list KeyDown (FUN_0048423c) as the original. Returns true if handled.
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
                // Letter: from the item after the current one (wrapping to the start at the end), find an item starting with that letter (TTreeView single-character incremental search).
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

    /// Read the tree's child shell folders separately (hidden folders follow that pane's hidden view, original FUN_0048da38 +0x3f8 bit 4).
    pub(super) fn request_tree_children(&mut self, index: usize, key: &Path) {
        let show_hidden = self.panes[index].show_hidden;
        if !self.tree_requests.insert((key.to_path_buf(), show_hidden)) {
            return;
        }
        let path = key.to_path_buf();
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let directories = ndr::shell_tree::children(&path, show_hidden);
            let _ = tx.send(AsyncResult::Tree { path, hidden: show_hidden, directories });
        });
    }
}

/// Items in the pane folder tree whose name can be edited: filesystem folders that have a parent (excluding drive roots and virtual folders).
pub(super) fn tree_renamable(node: &ndr::shell_tree::ShellNode) -> bool {
    node.special == ndr::shell_tree::Special::None && !ndr::shell_tree::is_virtual(&node.key) && node.path.as_ref().is_some_and(|p| p.parent().is_some() && p.file_name().is_some())
}

/// Pane folder-tree selected row, as the running original draws it (themed TTreeView on the list background): with the keyboard focus in the
/// tree a box over icon and text filled with the highlight color (GetSysColor(13)) at about 24 % over the background and a 1 px highlight
/// border; without the focus no box at all. Text: OnCustomDrawItem FUN_0048e104 sets white on a black background (0xFEFEFE focused,
/// 0xFFFFFF otherwise), else the tree font color. Returns (Some((fill, border)) when a box is drawn, text color).
/// Fixed: previously a solid highlight bar was drawn whether or not the tree had the focus.
pub(super) fn tree_selection_colors(background: Color32, fore: Color32, highlight: (u8, u8, u8), focused: bool) -> (Option<(Color32, Color32)>, Color32) {
    let (r, g, b) = highlight;
    let blend = |c: u8, bg: u8| ((c as f32 * 0.24) + (bg as f32 * 0.76)).round() as u8;
    let text = if background == Color32::BLACK { if focused { Color32::from_gray(0xFE) } else { Color32::WHITE } } else { fore };
    let boxed = focused.then(|| (Color32::from_rgb(blend(r, background.r()), blend(g, background.g()), blend(b, background.b())), Color32::from_rgb(r, g, b)));
    (boxed, text)
}

/// Expand glyph color of the themed tree on `background`: the captured original shows a collapsed ">" as 0x8B and an expanded "v" as 0x1B on
/// black, i.e. 0xA6 at 84 % and 0x26 at 72 % coverage.
pub(super) fn tree_glyph_color(background: Color32, open: bool) -> Color32 {
    let (c, a) = if open { (0x26 as f32, 0.72) } else { (0xA6 as f32, 0.84) };
    let mix = |bg: u8| (c * a + bg as f32 * (1.0 - a)).round() as u8;
    Color32::from_rgb(mix(background.r()), mix(background.g()), mix(background.b()))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_tree_selection_follows_the_captured_original() {
        let highlight = (0, 0x78, 0xD4);
        // Focused on black: fill 001C33 (24 %) and a 0078D4 border, text 0xFEFEFE (pixels measured in the running original).
        assert_eq!(tree_selection_colors(Color32::BLACK, Color32::from_gray(0xC0), highlight, true), (Some((Color32::from_rgb(0, 0x1D, 0x33), Color32::from_rgb(0, 0x78, 0xD4))), Color32::from_gray(0xFE)));
        // Not focused: no box, white text.
        assert_eq!(tree_selection_colors(Color32::BLACK, Color32::from_gray(0xC0), highlight, false), (None, Color32::WHITE));
        // Other backgrounds keep the tree font color.
        assert_eq!(tree_selection_colors(Color32::WHITE, Color32::from_rgb(1, 2, 3), highlight, false).1, Color32::from_rgb(1, 2, 3));
        assert_eq!((tree_glyph_color(Color32::BLACK, false), tree_glyph_color(Color32::BLACK, true)), (Color32::from_gray(0x8B), Color32::from_gray(0x1B)));
    }

    /// A fake font where each glyph is 10px and only '.' is 2px.
    fn glyph(c: char) -> f32 {
        if c == '.' { 2.0 } else { 10.0 }
    }

    #[test]
    fn thumbnail_caption_follows_drawthumbnailtext() {
        // Width 35: 3 characters per line. If it all fits within 4 lines, keep it as is.
        assert_eq!(thumbnail_lines("abcdefghij", 35.0, 4, &mut glyph), ["abc", "def", "ghi", "j"]);
        // If the remaining text overflows the fourth line, append "..." (6px here), shrinking from the fitting character count − 1.
        assert_eq!(thumbnail_lines("abcdefghijklmnop", 35.0, 4, &mut glyph), ["abc", "def", "ghi", "jk..."]);
        assert_eq!(thumbnail_lines("ab", 35.0, 4, &mut glyph), ["ab"]);
        assert!(thumbnail_lines("", 35.0, 4, &mut glyph).is_empty());
    }

    #[test]
    fn thumbnail_height_follows_getthumbnailtextheight() {
        // Line count × (line height + 2), up to 4 lines, plus 8 for a drive item.
        assert_eq!(thumbnail_text_height("ab", 35.0, 4, 2.0, 15.0, false, &mut glyph), 17.0);
        assert_eq!(thumbnail_text_height("abcdefg", 35.0, 4, 2.0, 15.0, false, &mut glyph), 51.0);
        assert_eq!(thumbnail_text_height("abcdefghijklmnop", 35.0, 4, 2.0, 15.0, false, &mut glyph), 68.0);
        assert_eq!(thumbnail_text_height("ab", 35.0, 4, 2.0, 15.0, true, &mut glyph), 25.0);
    }

    #[test]
    fn captions_narrower_than_one_glyph_still_end() {
        // The cell (8px) is narrower than a glyph (10px): previously the height calculation never terminated and the name appeared as blank lines.
        assert_eq!(thumbnail_text_height("abcdefg", 8.0, 4, 2.0, 15.0, false, &mut glyph), 68.0);
        assert_eq!(thumbnail_lines("abcdefg", 8.0, 4, &mut glyph), ["a", "b", "c", "d"]);
        // If there is no room to append "..." on the fourth line, only the fitting characters remain (previously that line was dropped).
        assert_eq!(thumbnail_lines("abcdefghij", 12.0, 4, &mut glyph), ["a", "b", "c", "d"]);
    }
}
