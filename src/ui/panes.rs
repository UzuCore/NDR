use super::*;

/// File list ScrollArea: drag and scroll bar stay as-is, the wheel is handled separately by the original rule (App::wheel_scroll).
/// List scrolling: bar only (the wheel uses the original WMMouseWheel handling; drag-to-scroll is not in the original — dragging empty space drags the selected items).
pub(super) const WHEEL_OFF: egui::scroll_area::ScrollSource = egui::scroll_area::ScrollSource { scroll_bar: true, drag: false, mouse_wheel: false };

/// Is this the C# "::drives::" (My PC) view.
pub(super) fn is_drives(path: &Path) -> bool {
    path.as_os_str() == super::remote_window::DRIVES
}

/// Original GetSelAreaAt(0x7dae7c): in non-thumbnail views, the left 12 pixels of the item cell (GetItemRect's left ~ left + 0xc).
/// GetItemRect (0x7daae4): styles 0/1 = column index / rows × column width − horizontal position, row = remainder × (height + gap);
/// style 2 (Details) = −horizontal position, y = (height + gap) × index − top + header; style 3 (Thumbnails) = the stored cell position.
fn selection_area(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(rect.min, egui::pos2((rect.left() + 12.0).min(rect.right()), rect.bottom()))
}

/// How far cell text may run into the right padding before it is shortened. Cell widths come from GDI widths (original Canvas.TextWidth) and the
/// text is drawn by egui, which can be a pixel or two wider; without this slack the widest date, size or extension of a column got "…".
pub(super) const CELL_TEXT_SLACK: f32 = 4.0;

/// Cell text of the original DrawListItem(0x7d8f58): one line inside the cell (left + 5, right − 5; ellipsis … on overflow). Size is right-aligned (GB/MB unit colored),
/// the date cell draws the date left-aligned and the time right-aligned (DateToStr/TimeToStr), the rest left-aligned. Shared by the Details, Auto, and Names views.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_original_cell(ui: &mut egui::Ui, rect: egui::Rect, column: DetailColumn, text: &str, cell_color: Color32, cursor_row: bool, o: &super::skin_pane::ListColors, list_style: u8) {
    let inner = rect.shrink2(egui::vec2(5.0, 0.0));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let layout = |ui: &mut egui::Ui, parts: &[(&str, Color32)]| {
        let mut job = egui::text::LayoutJob::default();
        for (part, part_color) in parts.iter().filter(|(p, _)| !p.is_empty()) {
            super::list_text(*part, list_style).color(*part_color).append_to(&mut job, ui.style(), egui::FontSelection::Default, egui::Align::Center);
        }
        job.wrap = egui::text::TextWrapping { max_width: (inner.width() + CELL_TEXT_SLACK).max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let top = |height: f32| rect.top() + ((rect.height() - height) / 2.0).floor();
    match column {
        DetailColumn::Size => {
            let (number, unit) = if cursor_row { (text, "") } else { super::skin_pane::split_size_text(text) };
            let upper = text.to_uppercase();
            let unit_color = if upper.contains('G') { o.gb } else if upper.contains('M') { o.mb } else { cell_color };
            let galley = layout(ui, &[(number, cell_color), (unit, unit_color)]);
            painter.galley(egui::pos2(inner.right() - galley.size().x, top(galley.size().y)), galley, cell_color);
        }
        // Original DrawListItem field 4: DateToStr goes to the left of the cell, TimeToStr to the right. The gap between them is listing::LIST_DATE_TIME_GAP.
        // Fix: splitting on the first space made "5일 전 23:19" split into "5일" and "전 23:19" (date "5 days ago" + time).
        DetailColumn::Modified if text.contains(ndr::listing::LIST_DATE_TIME_GAP) => {
            let (date, time) = text.split_once(ndr::listing::LIST_DATE_TIME_GAP).unwrap_or((text, ""));
            let date = layout(ui, &[(date, cell_color)]);
            let time = layout(ui, &[(time, cell_color)]);
            // When the cell is narrow and the date and time touch (gap under 6), write them as one line text on the left.
            if date.size().x + time.size().x + 6.0 <= inner.width() {
                painter.galley(egui::pos2(inner.left(), top(date.size().y)), date, cell_color);
                painter.galley(egui::pos2(inner.right() - time.size().x, top(time.size().y)), time, cell_color);
            } else {
                let galley = layout(ui, &[(text, cell_color)]);
                painter.galley(egui::pos2(inner.left(), top(galley.size().y)), galley, cell_color);
            }
        }
        _ => {
            let galley = layout(ui, &[(text, cell_color)]);
            painter.galley(egui::pos2(inner.left(), top(galley.size().y)), galley, cell_color);
        }
    }
}

/// The original DrawListItem(0x7d8f58) draws the name with DT_END_ELLIPSIS | DT_MODIFYSTRING and records whether it was truncated (+0x65).
/// NDR remembers the text layout of the cursor-row name cell, the shown name, and the full name, which the tooltip (original_name_tip) uses.
pub(super) type NameTip = std::cell::RefCell<Option<(usize, usize, egui::Rect, std::sync::Arc<egui::Galley>, std::sync::Arc<egui::Galley>)>>;

/// The original DrawThumbnailItem(0x7da1c4) records whether DrawThumbnailText truncated the name (+0x65). If the cursor cell's name was truncated in this paint,
/// (pane, row, item cell, top of the first line of the name text, thumbnail size, full-name lines split by DT_WORDBREAK).
pub(super) type ThumbTip = std::cell::RefCell<Option<(usize, usize, egui::Rect, f32, f32, Vec<std::sync::Arc<egui::Galley>>)>>;

/// Drive item info (GetDriveName, GetDriveTypeW, and (free, total) for fixed disks).
pub(super) type DriveInfo = (String, u32, Option<(u64, u64)>);

/// Drive list cache shared by pane listings (the original TNXDriveList slot): when it was read, (root, info), and whether a background re-read is in progress.
#[derive(Default)]
pub(super) struct DriveListCache {
    at: Option<Instant>,
    items: Vec<(PathBuf, Option<DriveInfo>)>,
    refreshing: bool,
    /// The functions that filled it (SystemHooks ready_drives, drive_item). If they change (e.g. test fakes), re-read.
    source: Option<(usize, usize)>,
}

impl DriveListCache {
    /// Have the next listing re-read the drives (F5, drive rename, network drive connect/disconnect).
    pub(super) fn invalidate(cache: &std::sync::Arc<std::sync::Mutex<DriveListCache>>) {
        if let Ok(mut guard) = cache.lock() {
            guard.at = None;
        }
    }
}

/// Cached drive list: if read within the last 5 seconds, return it as-is; if expired, return it as-is and re-read once in the background. On the first call, read immediately.
pub(super) fn cached_drive_items(cache: &std::sync::Arc<std::sync::Mutex<DriveListCache>>, ready: fn() -> Vec<PathBuf>, item: fn(&Path) -> Option<DriveInfo>) -> Vec<(PathBuf, Option<DriveInfo>)> {
    let read = move || ready().into_iter().map(|root| { let info = item(&root); (root, info) }).collect::<Vec<_>>();
    let source = Some((ready as usize, item as usize));
    {
        let Ok(mut guard) = cache.lock() else { return read() };
        let at = guard.at.filter(|_| guard.source == source);
        match at {
            Some(at) if at.elapsed() < Duration::from_secs(5) => return guard.items.clone(),
            Some(_) => {
                let items = guard.items.clone();
                if !guard.refreshing {
                    guard.refreshing = true;
                    let cache = cache.clone();
                    thread::spawn(move || {
                        let fresh = read();
                        if let Ok(mut guard) = cache.lock() {
                            guard.items = fresh;
                            guard.at = Some(Instant::now());
                            guard.refreshing = false;
                            guard.source = source;
                        }
                    });
                }
                return items;
            }
            None => {}
        }
    }
    let fresh = read();
    if let Ok(mut guard) = cache.lock() {
        guard.items = fresh.clone();
        guard.at = Some(Instant::now());
        guard.source = source;
    }
    fresh
}

/// Break lines only at word (space) boundaries, like Win32 DrawTextW DT_WORDBREAK. A word longer than the width stays on one line (wider than the area).
pub(super) fn word_break_lines(text: &str, width: f32, measure: &mut dyn FnMut(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        if line.is_empty() || measure(&candidate) <= width {
            line = candidate;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_owned();
        }
    }
    lines.push(line);
    lines
}
#[allow(clippy::too_many_arguments)]
pub(super) fn note_cursor_name(tip: &NameTip, ui: &egui::Ui, pane: usize, row: usize, cell: egui::Rect, shown: &str, full: &str, style: u8) {
    let galley = |text: &str| egui::WidgetText::from(super::list_text(text, style)).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    *tip.borrow_mut() = Some((pane, row, cell, galley(shown), galley(full)));
}

/// Values recorded by the original GetFieldMaxWidth(0x7df9d8): the widest text per column (7 columns in DetailColumn order, list font), sum of name widths (+0x358),
/// the count of non-drive items (list item count − kind 4, the divisor of GetAdjustedNameWidth/GetAdjustedExtWidth), and the extension average (+0x350).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct FieldMax {
    pub max: [f32; 7],
    pub name_sum: f32,
    pub items: usize,
    pub ext_average: f32,
}

impl App {
    /// Text width measured in the list font (original GetTextWidth = Canvas.TextWidth, GDI). egui only when GDI is unavailable.
    /// Previously egui's shaping was used, which differs from GDI by a few pixels per name and changed the original Auto column count.
    pub(super) fn list_text_width(&self, ui: &egui::Ui, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        if let Some(width) = self.gdi_list_widths(&[text]).and_then(|w| w.first().copied()) {
            return width;
        }
        let font = egui::FontId::new(self.list_font_size, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()));
        ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x).ceil()
    }

    /// GDI widths of texts in the list font (face, size, style), as the original Canvas.TextWidth measures them. None without GDI.
    pub(super) fn gdi_list_widths<S: AsRef<str>>(&self, texts: &[S]) -> Option<Vec<f32>> {
        let face = self.settings.text("ListFont", "Segoe UI");
        ndr::native::gdi_text_widths(&face, self.list_font_size.round() as i32, self.list_font_style, texts).map(|w| w.into_iter().map(|w| w as f32).collect())
    }

    /// Values recorded by the original GetFieldMaxWidth(0x7df9d8) (FieldMax). Re-measured only when the listing changes (original local_48[2] cache).
    /// Name is measured on folders/files (kind 1/3), extension on files, size on files whose size is non-zero, and date/description on folders/files.
    /// The size column is never narrower than its header (FIELD_SIZE, original GetString). The extension average is the integer average of non-empty extension widths (+0x350).
    pub(super) fn original_field_max(&self, ui: &egui::Ui, index: usize) -> FieldMax {
        let pane = &self.panes[index];
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (pane.generation, pane.entries.len(), self.list_font_size.to_bits(), pane.view.cells.len(), pane.view.cells.first().map(|c| c[0].clone()), self.language_pack.text("RES.FIELD_SIZE", "Size")).hash(&mut h);
            h.finish()
        };
        let id = egui::Id::new(("ndr-original-field-max", index));
        if let Some((_, fm)) = ui.data(|d| d.get_temp::<(u64, FieldMax)>(id)).filter(|c| c.0 == key) {
            return fm;
        }
        // Fix (performance): previously each item was measured by text layout (layout_no_wrap, building paint shapes), about 60ms for 20,000 files, and the folder
        // watcher stalled that long every time the listing was re-read. The original GetTextWidth is Canvas.TextWidth (GDI): measure the distinct texts in one
        // GDI pass (same font, one DC), falling back to the sum of egui glyph widths without GDI. Previously egui widths were used, a few pixels off per name,
        // which changed the original Auto column count (Documents: two columns in the original, one in NDR).
        let font = egui::FontId::new(self.list_font_size, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()));
        let mut fm = FieldMax::default();
        let (mut ext_sum, mut ext_count) = (0f32, 0usize);
        // (field, text index) per measured cell; texts are deduplicated.
        let mut texts: Vec<String> = Vec::new();
        let mut known: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut cells_measured: Vec<(u8, usize)> = Vec::new();
        let mut add = |field: u8, text: String, cells_measured: &mut Vec<(u8, usize)>| {
            let slot = *known.entry(text.clone()).or_insert_with(|| {
                texts.push(text);
                texts.len() - 1
            });
            cells_measured.push((field, slot));
        };
        for (entry, cells) in pane.entries.iter().zip(&pane.view.cells) {
            if entry.drive {
                continue;
            }
            add(0, cells[0].clone(), &mut cells_measured);
            if !entry.is_dir {
                add(1, cells[1].clone(), &mut cells_measured);
                if entry.size > 0 {
                    add(3, cells[3].clone(), &mut cells_measured);
                }
            }
            // Original GetFieldMaxWidth field 4: GetTextWidth(wsprintf("%s  %s", DateToStr, TimeToStr)) (0x92dba4, two spaces). The cell keeps one
            // LIST_DATE_TIME_GAP between them, so measure that as two spaces. Fixed: one gap left the column too narrow for the date at the left
            // and the time at the right, and the cell fell back to "2009-06-11 05:...".
            add(4, cells[4].replace(ndr::listing::LIST_DATE_TIME_GAP, "  "), &mut cells_measured);
            add(6, cells[6].clone(), &mut cells_measured);
        }
        let widths = self
            .gdi_list_widths(&texts)
            .unwrap_or_else(|| ui.fonts_mut(|fonts| texts.iter().map(|t| t.chars().map(|c| fonts.glyph_width(&font, c)).sum::<f32>().ceil()).collect()));
        for (field, slot) in cells_measured {
            let width = widths[slot];
            match field {
                0 => {
                    fm.max[0] = fm.max[0].max(width);
                    fm.name_sum += width;
                    fm.items += 1;
                }
                1 => {
                    fm.max[1] = fm.max[1].max(width);
                    if width > 0.0 {
                        ext_sum += width;
                        ext_count += 1;
                    }
                }
                field => fm.max[field as usize] = fm.max[field as usize].max(width),
            }
        }
        if ext_count > 0 {
            fm.ext_average = (ext_sum / ext_count as f32).floor();
        }
        fm.max[3] = fm.max[3].max(self.list_text_width(ui, &self.language_pack.text("RES.FIELD_SIZE", "Size")));
        ui.data_mut(|d| d.insert_temp(id, (key, fm)));
        fm
    }

    /// Original GetAdjustedNameWidth(0x7e00a8): longest name + 10. If there are 6 or more non-drive items and "WWWWW" is narrower than that, and
    /// the name average (integer division) × 2 is shorter than the longest name, then Trunc(average × 1.5 + 10) (the 1.5 and 10.0 at 0x7e012c; 0x8c3c8c truncates with FISTP).
    pub(super) fn original_adjusted_name_width(&self, ui: &egui::Ui, fm: &FieldMax) -> f32 {
        let base = fm.max[0] + 10.0;
        if fm.items > 5 && self.list_text_width(ui, "WWWWW") < base {
            let average = (fm.name_sum / fm.items as f32).floor();
            if average * 2.0 < fm.max[0] {
                return (average * 1.5 + 10.0).trunc();
            }
        }
        base
    }

    /// Original GetAdjustedExtWidth(0x7e0134): longest extension + 10. If there are 6 or more non-drive items, there is an extension average, and "WAA" is
    /// narrower than that, and the average × 2 is shorter than the longest extension, then Trunc(average × 1.5 + 10) (the 1.5 and 10.0 at 0x7e01b0).
    pub(super) fn original_adjusted_ext_width(&self, ui: &egui::Ui, fm: &FieldMax) -> f32 {
        let base = fm.max[1] + 10.0;
        if fm.items > 5 && fm.ext_average > 0.0 && self.list_text_width(ui, "WAA") < base && fm.ext_average * 2.0 < fm.max[1] {
            return (fm.ext_average * 1.5 + 10.0).trunc();
        }
        base
    }

    /// Original FitFileNameFields(0x7e0298): name = max(longest name + 10, "WWWWAAAA" + 10), extension = max(longest extension + 10, "WWA" + 10).
    /// If their sum is larger than `room`, when `room` − GetAdjustedExtWidth is less than 21 split the two proportionally (integer division), otherwise extension = GetAdjustedExtWidth,
    /// name = the remainder. (name, extension). When the extension is attached to the name, extension is 0 and name is min(longest name + 10, room); and if that is
    /// narrower than "WWWWAAAA" + 10 and "WWWWAAAA" + 10 fits in room, use "WWWWAAAA" + 10.
    pub(super) fn original_fit_name_fields(&self, ui: &egui::Ui, fm: &FieldMax, room: f32, attach: bool) -> (f32, f32) {
        let eight = self.list_text_width(ui, "WWWWAAAA") + 10.0;
        if attach {
            let natural = fm.max[0] + 10.0;
            let name = if natural <= room { natural } else { room };
            return (if name < eight && eight <= room { eight } else { name.max(0.0) }, 0.0);
        }
        let name = (fm.max[0] + 10.0).max(eight);
        let ext = (fm.max[1] + 10.0).max(self.list_text_width(ui, "WWA") + 10.0);
        if name + ext <= room {
            return (name, ext);
        }
        let adjusted_ext = self.original_adjusted_ext_width(ui, fm);
        if room - adjusted_ext < 21.0 {
            let sum = (name + ext).max(1.0);
            ((name * room / sum).trunc().max(0.0), (ext * room / sum).trunc().max(0.0))
        } else {
            (room - adjusted_ext, adjusted_ext)
        }
    }

    /// Column widths of the original UpdateDisplayDetails(0x7e1374) (skin theme Details view). Each column is GetFieldMaxWidth (widest text) + 10 (0 when there is no text),
    /// and the attributes column is "ARHS" (SFTP uses "drwxrwxrwx") + 10. If their total exceeds `avail` (list width minus the vertical bar), shrink the name with GetAdjustedNameWidth
    /// and the extension with GetAdjustedExtWidth. Then, if there is room left, re-fit name/extension with FitFileNameFields (width minus the other columns)
    /// (with the "WWWWAAAA"/"WWA" minimums), and if it still overflows leave it (scroll horizontally). Leftover width is not filled.
    /// The NDR name column also holds the icon header (GetListHeaderWidth).
    pub(super) fn original_detail_widths(&self, ui: &egui::Ui, index: usize, avail: f32, columns: &[(super::list_view::DetailColumn, String, f32)]) -> Vec<Option<f32>> {
        use super::list_view::DetailColumn as C;
        let fm = self.original_field_max(ui, index);
        let max = fm.max;
        let pane = &self.panes[index];
        let sftp = pane.remote.as_ref().is_some_and(|r| r.is_sftp());
        let header = if self.settings.flag("ShowIcons", true) { 30.0 } else { 14.0 };
        let attach = self.settings.flag("AttachExtensionToName", false);
        let natural = |w: f32| if w >= 1.0 { w + 10.0 } else { 0.0 };
        let attributes = if pane.entries.is_empty() { 0.0 } else { self.list_text_width(ui, if sftp { "drwxrwxrwx" } else { "ARHS" }) + 10.0 };
        let field = |column: C| -> f32 {
            match column {
                C::Name => natural(max[0]),
                C::Extension => natural(max[1]),
                C::Size => natural(max[3]),
                C::Modified => natural(max[4]),
                C::Attributes => attributes,
                C::Description => natural(max[6]),
                C::Kind => 0.0,
            }
        };
        let mut widths: Vec<f32> = columns.iter().map(|c| field(c.0)).collect();
        let position = |column: C| columns.iter().position(|c| c.0 == column);
        let (n, e) = (position(C::Name), position(C::Extension));
        let total = |widths: &[f32]| header + widths.iter().sum::<f32>();
        if total(&widths) > avail {
            if let Some(n) = n {
                widths[n] = self.original_adjusted_name_width(ui, &fm);
            }
            if let Some(e) = e {
                widths[e] = self.original_adjusted_ext_width(ui, &fm);
            }
        }
        let sum = total(&widths);
        if sum < avail {
            let (name_w, ext_w) = (n.map_or(0.0, |n| widths[n]), e.map_or(0.0, |e| widths[e]));
            // Original FitFileNameFields(list width − (0x344 − (name + extension))): 0x344 holds the header, so room = width − header − other columns.
            let room = avail - (sum - name_w - ext_w);
            let (name, ext) = self.original_fit_name_fields(ui, &fm, room, attach || e.is_none());
            if let Some(n) = n {
                widths[n] = name;
            }
            if let Some(e) = e {
                widths[e] = ext;
            }
        }
        if let Some(n) = n {
            widths[n] += header;
        }
        widths.into_iter().map(Some).collect()
    }

    /// Original SetItemIndex(0x7dbdec): if the name of the item the cursor moved to (excluding drives) was drawn truncated, after 600ms (+0x408)
    /// ToolTipTimer (0x7d7838) calls ShowToolTip(0x7d7454) for the cursor item (+0x3e4 tip window +0xc), which draws the full name centered in a TNXToolTipWnd (WS_POPUP | WS_BORDER, background clInfoBk, text clInfoText, list font). Position: item cell
    /// left + GetListHeaderWidth (30 with icons, else 14) − 2, top − 2, width = text width + 10,
    /// height = line height + 4; if off-screen, push it inside. Hides after 3 seconds (TIP_HIDE_MS). Only when the list has focus (active pane, window focus). Skin theme.
    /// The hover timer MouseMoveTimer (0x7d78d4, +0x3ec) is created disabled in the constructor (0x7d4a28) and only ever disabled again
    /// (CMMouseLeave 0x7d5708), so the original shows no tip for the item under the mouse; NDR does the same.
    pub(super) fn original_name_tip(&mut self, ctx: &egui::Context, index: usize) {
        let tip = self.name_tip.borrow_mut().take_if(|(pane, ..)| *pane == index);
        let thumb_tip = self.thumb_tip.borrow_mut().take_if(|(pane, ..)| *pane == index);
        self.name_tip_shown[index] = None;
        let cursor = self.panes[index].cursor;
        let now = Instant::now();
        let since = match self.tip_since[index] {
            Some((row, at)) if row == cursor => at,
            _ => {
                self.tip_since[index] = Some((cursor, now));
                now
            }
        };
        // Original IsHintMsg: hide on the wheel (WM_MOUSEWHEEL).
        if ctx.input(|i| i.raw_scroll_delta != egui::Vec2::ZERO) {
            self.tip_since[index] = Some((cursor, now - Duration::from_secs(60)));
            return;
        }
        let color = |index: i32| {
            let (r, g, b) = native::system_color(index);
            Color32::from_rgb(r, g, b)
        };
        // Original ShowToolTip format 3 (thumbnail view): text position left = item left + (item width − thumbnail size) / 2, width = thumbnail size,
        // measure the height with DrawTextW(DT_CALCRECT | DT_WORDBREAK | DT_CENTER | DT_NOPREFIX) (thumbnail size if narrower), expand by 4 on all sides,
        // and overlap it above the name text (+0x2cc = 0: multi-line centered). NDR aligns it to the first line of the name text inside the cell.
        if let Some((_, row, cell, text_top, thumb, lines)) = thumb_tip {
            let drive = self.panes[index].entries.get(row).is_none_or(|e| e.drive);
            if row != cursor || drive || self.active != index || !self.list_has_focus(ctx) || lines.is_empty() {
                return;
            }
            let (delay, life) = (Duration::from_millis(600), Duration::from_millis(600 + TIP_HIDE_MS));
            let elapsed = now.saturating_duration_since(since);
            if elapsed < delay {
                ctx.request_repaint_after(delay - elapsed);
                return;
            }
            if elapsed >= life {
                return;
            }
            ctx.request_repaint_after(life - elapsed);
            let widest = lines.iter().map(|g| g.size().x).fold(thumb, f32::max);
            let line_height = lines.iter().map(|g| g.size().y).fold(0.0, f32::max);
            let left = (cell.left() + ((cell.width() - thumb) / 2.0).trunc()).round();
            let inner = egui::Rect::from_min_size(egui::pos2(left, text_top), egui::vec2(widest, line_height * lines.len() as f32));
            let mut rect = inner.expand(4.0);
            let (back, border, fore) = (color(24), color(6), color(23));
            let paint = |painter: &egui::Painter, rect: egui::Rect| {
                painter.rect(rect, 0.0, back, egui::Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
                let inner = rect.shrink(4.0);
                for (i, galley) in lines.iter().enumerate() {
                    let at = egui::pos2(inner.center().x - galley.size().x / 2.0, inner.top() + i as f32 * line_height).round();
                    painter.galley(at, galley.clone(), fore);
                }
            };
            if show_tip_window(ctx, index, rect, &paint) {
                self.name_tip_shown[index] = Some(rect);
                return;
            }
            let screen = ctx.content_rect();
            if rect.right() > screen.right() {
                rect = rect.translate(egui::vec2(screen.right() - rect.right(), 0.0));
            }
            if rect.bottom() > screen.bottom() {
                rect = rect.translate(egui::vec2(0.0, screen.bottom() - rect.bottom()));
            }
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("ndr-name-tip", index))));
            paint(&painter, rect);
            self.name_tip_shown[index] = Some(rect);
            return;
        }
        let Some((_, row, cell, shown, full)) = tip else { return };
        let drive = self.panes[index].entries.get(row).is_none_or(|e| e.drive);
        // ShowToolTip runs only while the list has the focus: a dialog opened before the 600 ms timer (F2 rename on a truncated name) leaves no tip,
        // as on the captured original. Previously NDR drew the tip over its in-window dialog.
        if row != cursor || drive || self.active != index || !self.list_has_focus(ctx) {
            return;
        }
        let header = if self.settings.flag("ShowIcons", true) { 30.0 } else { 14.0 };
        if header + shown.size().x + 5.0 <= cell.width() {
            return;
        }
        let (delay, life) = (Duration::from_millis(600), Duration::from_millis(600 + TIP_HIDE_MS));
        let elapsed = now.saturating_duration_since(since);
        if elapsed < delay {
            ctx.request_repaint_after(delay - elapsed);
            return;
        }
        if elapsed >= life {
            return;
        }
        ctx.request_repaint_after(life - elapsed);
        let mut rect = egui::Rect::from_min_size(egui::pos2(cell.left() + header - 2.0, cell.top() - 2.0), egui::vec2(full.size().x + 10.0, cell.height() + 4.0));
        let (back, border, fore) = (color(24), color(6), color(23));
        let paint = |painter: &egui::Painter, rect: egui::Rect| {
            painter.rect(rect, 0.0, back, egui::Stroke::new(1.0_f32, border), egui::StrokeKind::Inside);
            let at = (rect.center() - full.size() / 2.0).round();
            painter.galley(at, full.clone(), fore);
        };
        if show_tip_window(ctx, index, rect, &paint) {
            self.name_tip_shown[index] = Some(rect);
            return;
        }
        let screen = ctx.content_rect();
        if rect.right() > screen.right() {
            rect = rect.translate(egui::vec2(screen.right() - rect.right(), 0.0));
        }
        if rect.bottom() > screen.bottom() {
            rect = rect.translate(egui::vec2(0.0, screen.bottom() - rect.bottom()));
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("ndr-name-tip", index))));
        paint(&painter, rect);
        self.name_tip_shown[index] = Some(rect);
    }
}

/// Original TNXToolTipWnd hide time (+0x2c8): the initial value is 5000 (0x7efbd9), but the list constructor changes it to 3000 (0x7d4ef5).
/// Fix: it used to be 5 seconds.
pub(super) const TIP_HIDE_MS: u64 = 3000;

/// The original TNXToolTipWnd is a separate popup window (WS_POPUP | WS_BORDER), so it can extend past the main window, and if it goes off-screen (Screen.Width/Height) it is pushed inside.
/// If the tooltip fits inside the window, returns false (draw it inside the window). If it overflows, show a borderless egui viewport (not activated, pointer passthrough, always on top, no taskbar)
/// at screen coordinates and return true. In environments that cannot draw multiple viewports (e.g. tests), returns false.
pub(super) fn show_tip_window(ctx: &egui::Context, index: usize, rect: egui::Rect, paint: &dyn Fn(&egui::Painter, egui::Rect)) -> bool {
    if ctx.content_rect().contains_rect(rect) || ctx.embed_viewports() {
        return false;
    }
    let (inner, monitor) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size));
    let Some(inner) = inner else { return false };
    let mut at = inner.min + rect.min.to_vec2();
    // The original pushes inside Screen.Width/Height. egui only knows the monitor size, not its origin, so push only when the main window is within that size starting from (0, 0).
    if let Some(monitor) = monitor.filter(|m| inner.min.x >= 0.0 && inner.min.y >= 0.0 && inner.max.x <= m.x + 1.0 && inner.max.y <= m.y + 1.0) {
        at.x = at.x.min(monitor.x - rect.width()).max(0.0);
        at.y = at.y.min(monitor.y - rect.height()).max(0.0);
    }
    let builder = egui::ViewportBuilder::default()
        .with_title("NDR")
        .with_decorations(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_active(false)
        .with_always_on_top()
        .with_mouse_passthrough(true)
        .with_position(at)
        .with_inner_size(rect.size());
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("ndr-name-tip-window", index)), builder, |ctx, _| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let local = egui::Rect::from_min_size(ui.max_rect().min, rect.size());
            paint(ui.painter(), local);
        });
    });
    true
}

/// Is the pointer over this row's selection area (when routing a click/double-click to the selection area).
pub(super) fn pointer_in_selection_area(rect: egui::Rect, ui: &egui::Ui) -> bool {
    ui.input(|i| i.pointer.interact_pos()).is_some_and(|pos| selection_area(rect).contains(pos) && ui.clip_rect().contains(pos))
}

/// Original MouseDown(0x7d63c8)/MouseMove(0x7d65f0)/MouseUp(0x7d6740) (skin theme). Handles the moment the left button is pressed on this row and returns true.
/// - Shift(+Ctrl): add the range from the current cursor row to the pressed row to the selection; drag anchor (+0x3d4) = previous cursor row, last end (+0x3d8) = pressed row.
/// - Ctrl only: toggle the pressed row (if no row is selected and the previous cursor row differs, also select that row). A plain press just moves the cursor.
/// - If in the selection area (left 12 pixels), additionally toggle that row once more and set anchor = end = that row.
/// While dragging the press over another row, when the last end row differs from the anchor, if Ctrl is not held clear all selection (if Ctrl, deselect only the anchor~last-end rows)
/// and select the anchor~row-under-pointer range. The press ends on button release (range_drag is cleared before pane_ui).
/// Is this the paint that begins an item drag. The C# theme uses egui drag_started_by. The skin theme follows the original FUN_004856d8 (list OnMouseMove):
/// while the left or right button is held, if it moved more than 8 pixels horizontally or vertically from the press point, and the press point is not in the selection area (GetSelAreaAt), begin.
/// Whether to begin is checked every paint while not dragging (App receives it once via file_drag).
pub(super) fn drag_begins(response: &egui::Response, button: egui::PointerButton, skin: bool) -> bool {
    if !skin {
        return response.drag_started_by(button);
    }
    if !response.dragged_by(button) {
        return false;
    }
    let (origin, pos) = response.ctx.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos()));
    let (Some(origin), Some(pos)) = (origin, pos) else { return false };
    !selection_area(response.rect).contains(origin) && ((pos.x - origin.x).abs() > 8.0 || (pos.y - origin.y).abs() > 8.0)
}

/// Left button press tracking (windows_double_press): this paint number, the previous press (time, position), and whether the previous press was a second press.
#[derive(Clone, Copy, Default)]
struct PressTrack {
    frame: u64,
    last: Option<(f64, egui::Pos2)>,
    last_double: bool,
}

/// Is the previous (or this paint's) left button press a second press by the Windows standard (WM_LBUTTONDBLCLK). The original controls take it as Windows decides:
/// if it is within GetDoubleClickTime of the prior press and inside an SM_CXDOUBLECLK × SM_CYDOUBLECLK rectangle centered on the prior press, it is a second
/// press, and the press after that is a normal press again. egui looks only at time (a fast double press on different rows also counts), so this is checked alongside it.
/// Counted once per paint (called at the top of App::update and by the list rows).
pub(super) fn windows_double_press(ctx: &egui::Context) -> bool {
    let id = egui::Id::new("ndr-windows-double-press");
    let frame = ctx.cumulative_frame_nr() + 1;
    let (press, time, scale) = ctx.input(|i| {
        let press = i.events.iter().rev().find_map(|e| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some(*pos),
            _ => None,
        });
        (press, i.time, i.pixels_per_point)
    });
    ctx.data_mut(|d| {
        let mut track = d.get_temp::<PressTrack>(id).unwrap_or_default();
        if track.frame != frame {
            track.frame = frame;
            if let Some(pos) = press {
                let (ms, cx, cy) = native::double_click_metrics();
                let (half_x, half_y) = (cx as f32 / scale / 2.0, cy as f32 / scale / 2.0);
                let double = !track.last_double
                    && track.last.is_some_and(|(at, from)| (time - at) * 1000.0 <= ms as f64 && (pos.x - from.x).abs() <= half_x && (pos.y - from.y).abs() <= half_y);
                track.last = Some((time, pos));
                track.last_double = double;
            }
            d.insert_temp(id, track);
        }
        track.last_double
    })
}

/// A click that became a second press by the Windows standard (windows_double_press). egui double_clicked is not used: egui turns a third press within 2 × the interval
/// into a triple click and hides the double click (pressing one item and then double-clicking another within 1 second would not open it),
/// but Windows has no triple click (the press after the second press is a normal press again).
pub(super) fn windows_double_clicked(response: &egui::Response) -> bool {
    let double = windows_double_press(&response.ctx);
    response.clicked() && double
}

pub(super) fn original_press_input(pane: &mut Pane, row: usize, rect: egui::Rect, ui: &egui::Ui) -> bool {
    original_press(pane, row, rect, ui, false)
}

/// The thumbnail (format 3) variant of original_press_input: the original MouseDown does not check the selection area (GetSelAreaAt) in format 3,
/// and MouseMove does not drag-select in format 3 either.
pub(super) fn original_thumbnail_press(pane: &mut Pane, row: usize, rect: egui::Rect, ui: &egui::Ui) -> bool {
    original_press(pane, row, rect, ui, true)
}

fn original_press(pane: &mut Pane, row: usize, rect: egui::Rect, ui: &egui::Ui, thumbnails: bool) -> bool {
    let (pressed, down, pos, moved, modifiers) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos(), i.pointer.delta() != egui::Vec2::ZERO, i.modifiers));
    let double_press = windows_double_press(ui.ctx());
    let Some(pos) = pos.filter(|p| ui.clip_rect().contains(*p)) else { return false };
    // Fix: since raw input is read, if another layer (dialog/message box/menu) is on top, that press is not for the list. Previously the OK button
    // of the Compress window, when sitting over another pane's row, made that pane the active pane and the archive was created in the other pane's folder.
    if ui.ctx().layer_id_at(pos).is_some_and(|layer| layer != ui.layer_id()) {
        return false;
    }
    // Original MouseDown(0x7d63c8): only the selection-changing branch is left-button-only (ssLeft 8); SetItemFocused and clearing the search text happen for any button.
    // Fix: previously a middle-button press did nothing (the right button is handled separately).
    if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Middle)) && rect.contains(pos) {
        pane.item_pressed = true;
        pane.cursor = row;
        return true;
    }
    if pressed && rect.contains(pos) {
        // The original MouseDown(0x7d63c8) does not touch the selection/cursor on the second press of a double click (ssDouble, WM_LBUTTONDBLCLK).
        // The second press within the Windows standard (windows_double_press) is that one. The third press is a normal press again.
        if double_press {
            return true;
        }
        pane.item_pressed = true;
        let before = pane.cursor.min(pane.entries.len().saturating_sub(1));
        if modifiers.shift {
            for j in before.min(row)..=before.max(row) {
                pane.selected.insert(j);
            }
            pane.range_drag = Some((before, row));
        } else if modifiers.ctrl {
            if pane.selected.is_empty() && before != row && before < pane.entries.len() {
                pane.selected.insert(before);
            }
            if !pane.selected.insert(row) {
                pane.selected.remove(&row);
            }
        }
        pane.cursor = row;
        pane.anchor = row;
        if thumbnails {
            pane.range_drag = None;
        } else if selection_area(rect).contains(pos) {
            if !pane.selected.insert(row) {
                pane.selected.remove(&row);
            }
            pane.range_drag = Some((row, row));
        }
        return true;
    }
    if thumbnails {
        return false;
    }
    if let (true, true, true, Some((anchor, last))) = (down, moved, rect.contains(pos), pane.range_drag) {
        if last != anchor {
            if modifiers.ctrl {
                for j in anchor.min(last)..=anchor.max(last) {
                    pane.selected.remove(&j);
                }
            } else {
                pane.selected.clear();
            }
        }
        for j in anchor.min(row)..=anchor.max(row) {
            pane.selected.insert(j);
        }
        pane.range_drag = Some((anchor, row));
    }
    false
}

/// Selection and cursor when a list row is pressed.
/// The skin theme follows the original TNXCustomFileListView.MouseDown(0x7d63c8): a plain press and the right button only move the cursor and leave the selection,
/// Shift(+Ctrl) adds the range from the current cursor to the pressed row to the selection, and Ctrl toggles the pressed row (if no row is selected, also select the previous cursor row).
/// The C# theme follows the C# version (WPF list): a plain press clears the selection, Shift selects from the anchor row, and the right button clears the selection if the row is not selected.
/// What to show when a list row is right-clicked (original FUN_00484e8c).
pub(super) enum ListMenu {
    /// Do not show (archive/FTP pane, no cursor item).
    None,
    /// Original pmMain (`list_context_menu`).
    Main,
    /// Original FUN_00486104: Explorer (shell) menu.
    Shell(Vec<PathBuf>),
}

impl App {
    /// Original list OnContextPopup(FUN_00484e8c): shown only in a local pane (FUN_00481948 = 0). If "Right-click: use Explorer menu"
    /// (chkExplorerMenu, setting +0xd1) is on, or there is no selection and the cursor item is a drive (kind 4), use the Explorer menu (FUN_00486104:
    /// the drive if a drive, otherwise the selected items or the cursor item); otherwise pmMain.
    pub(super) fn list_menu_kind(&self, index: usize) -> ListMenu {
        let pane = &self.panes[index];
        if pane.archive.is_some() || pane.remote.is_some() || pane.loading {
            return ListMenu::None;
        }
        let Some(focused) = pane.entries.get(pane.cursor) else { return ListMenu::None };
        if pane.selected.is_empty() && focused.drive {
            return ListMenu::Shell(vec![focused.path.clone()]);
        }
        if self.settings.flag("UseExplorerMenu", false) {
            let targets = pane.targets();
            return if targets.is_empty() { ListMenu::None } else { ListMenu::Shell(targets) };
        }
        ListMenu::Main
    }

    /// Original list OnMouseDown(FUN_00485078): if the left button is pressed while the right button is held (Shift state ssRight), then if "use Explorer menu" is
    /// off, or there is no selection and the cursor item is a drive, show the Explorer (shell) menu (FUN_00486104: local pane only; the drive if a drive, otherwise
    /// the selected items or the cursor item); if on, show pmMain at the press point (the opposite of the right-button menu).
    pub(super) fn rocker_menu_kind(&self, index: usize) -> ListMenu {
        let pane = &self.panes[index];
        let Some(focused) = pane.entries.get(pane.cursor) else { return ListMenu::None };
        let drive_cursor = pane.selected.is_empty() && focused.drive;
        if !self.settings.flag("UseExplorerMenu", false) || drive_cursor {
            if pane.archive.is_some() || pane.remote.is_some() || pane.loading {
                return ListMenu::None;
            }
            if drive_cursor {
                return ListMenu::Shell(vec![focused.path.clone()]);
            }
            let targets = pane.targets();
            return if targets.is_empty() { ListMenu::None } else { ListMenu::Shell(targets) };
        }
        ListMenu::Main
    }
}

pub(super) fn row_pressed(pane: &mut Pane, row: usize, modifiers: egui::Modifiers, secondary: bool, original: bool) {
    if original {
        if !secondary {
            if modifiers.shift {
                let from = pane.cursor.min(pane.entries.len().saturating_sub(1));
                for j in from.min(row)..=from.max(row) {
                    pane.selected.insert(j);
                }
            } else if modifiers.ctrl {
                if pane.selected.is_empty() && pane.cursor != row && pane.cursor < pane.entries.len() {
                    pane.selected.insert(pane.cursor);
                }
                if !pane.selected.insert(row) {
                    pane.selected.remove(&row);
                }
            }
        }
        pane.anchor = row;
    } else if secondary {
        if !pane.selected.contains(&row) {
            pane.selected.clear();
        }
    } else if modifiers.shift {
        for j in pane.anchor.min(row)..=pane.anchor.max(row) {
            pane.selected.insert(j);
        }
    } else if modifiers.ctrl {
        if !pane.selected.insert(row) {
            pane.selected.remove(&row);
        }
    } else {
        pane.selected.clear();
        pane.anchor = row;
    }
    pane.cursor = row;
}

impl App {
    pub(super) fn refresh(&mut self, index: usize) {
        // C# RefreshPaneAsync: discard that pane's quick-find input.
        self.typed_search[index] = TypedSearch::default();
        // The original turns off TNXShellChangeNotify during its own operation (SetActive(0) = FreeThread), re-reads after the operation, then turns it back on (SetActive(1) =
        // ResetThread, new FindFirstChangeNotificationW). So changes caused by its own operation do not trigger another re-read 1.5 seconds later.
        // NDR re-opens the watch every time it re-reads: it discards the old watch's notifications (folder_changed) and discards pending changes too (original Clear).
        // Fix: previously, 1.5 seconds after a new folder or rename, the same folder was re-read once more.
        self.panes[index].change_due = None;
        if let Some(wanted) = self.watch_wanted(index) {
            let ctx = self.egui_ctx.clone();
            self.arm_folder_watch(index, Some(wanted), &ctx);
        }
        // Pane folder tree: also re-read that folder's subfolder list (so newly created or deleted folders appear in the tree).
        // Original TNXCustomShellTreeView.Refresh(node) (0x7ee25c) rebuilds the node from a fresh TNXShellFolder (PopulateNode), keeps the
        // expanded state of children that still exist (FolderExists) and restores the selected and top nodes. NDR drops the cached
        // child list of that folder; expansion is kept by key in `tree_open`, so the same rows reopen.
        let current = self.panes[index].path.clone();
        self.tree_cache.retain(|(key, _), _| !ndr::shell_tree::same_key(key, &current));
        self.tree_requests.retain(|(key, _)| !ndr::shell_tree::same_key(key, &current));
        // The original refresh resets the selection to the name (apply_listing), so keep the current selection until the new listing arrives.
        let pane = &mut self.panes[index];
        pane.generation += 1;
        pane.loading = true;
        let generation = pane.generation;
        let path = pane.path.clone();
        let show_hidden = pane.show_hidden;
        let archive = pane.archive.clone();
        let remote = pane.remote.clone();
        let tx = self.listing_tx.clone();
        let include_legacy = self.settings.flag("UseDescriptionFile", true);
        let local = archive.is_none() && remote.is_none();
        // C# ListLocal("::drives::"): a folder item (name = "C:\") for each ready drive. Empty if ShowDriveList is off.
        let show_drive_list = self.settings.flag("ShowDriveList", true);
        let drives_view = local && is_drives(&path);
        // Original FUN_00486474: if "show disk drive list in the list" (ini showdrivelist, initial value 1 = 0x46eed9) is on,
        // append an item for each TNXDriveList drive (attribute 0x40000 → kind 4) to the end of the normal folder listing. The read happens on the listing thread.
        let append_drives = local && !drives_view && show_drive_list;
        let ready_drives = self.system.ready_drives;
        let drive_item = self.system.drive_item;
        let drive_list = self.drive_list.clone();
        // Tests keep fake drives per thread, so they bypass the cache (the cache itself is the cached_drive_items test).
        let drive_items = move |cache: &std::sync::Arc<std::sync::Mutex<DriveListCache>>| {
            if cfg!(test) { ready_drives().into_iter().map(|root| { let info = drive_item(&root); (root, info) }).collect() } else { cached_drive_items(cache, ready_drives, drive_item) }
        };
        // The drives of the My PC view are read on the calling thread as before (immediately if cached). Tests have fake drives per thread, so
        // the appended drives are also read on the calling thread.
        let this_pc: Option<Vec<(PathBuf, Option<DriveInfo>)>> = (drives_view || (cfg!(test) && append_drives)).then(|| if show_drive_list { drive_items(&drive_list) } else { Vec::new() });
        // Original listing workers are TMJFThread (Nmjfthreads: constructor 0x462814, Execute 0x462abc = OnStart, then OnExecute while
        // looping and not terminated, then OnFinish; SyncFunction 0x462ed0 / SyncDataFunction 0x462f6c = Synchronize, Terminate 0x462d34,
        // SuspendIfNotTerminated 0x462e48, destructor 0x462a1c), and results reach the window through TMessageTrap (Nmjfcore: constructor
        // 0x45ff70, RegisterMessageEvent 0x4602b0 / UnregisterMessageEvent 0x460850 keep a message → handler std::map (0x460478 / 0x460a24),
        // Dispatch 0x46091c calls the handler, destructor 0x460204). Here a std::thread sends the listing over a channel and wakes egui.
        thread::spawn(move || {
            // The original TNXDriveList is created once at startup and held (it does not re-query drives per listing). NDR changed the per-pane-listing
            // drive read into a cache (use as-is within 5 seconds, use as-is after and re-read in the background). So even if a drive's volume info arrives late,
            // like a disconnected network drive, you don't wait for it every time you change folders.
            let listed = match this_pc {
                Some(items) => items,
                None if append_drives => drive_items(&drive_list),
                None => Vec::new(),
            };
            let roots: Vec<PathBuf> = listed.iter().map(|(root, _)| root.clone()).collect();
            let drives = drives_view.then(|| roots.clone());
            let appended: Vec<PathBuf> = if append_drives { roots } else { Vec::new() };
            // The original My PC drive item's TNXDriveInfo (name, kind, and size for fixed disks).
            let drive_infos = listed.into_iter().filter_map(|(root, info)| info.map(|info| (root, info))).collect();
            let result = if let Some(remote) = remote {
                super::ftp_pane::list_remote(&remote, show_hidden)
            } else if let Some(drives) = drives {
                Ok(drives
                    .into_iter()
                    .map(|root| Entry { name: root.to_string_lossy().into_owned(), path: root, is_dir: true, size: 0, modified: None, hidden: false, attributes: 0x10, drive: false })
                    .collect())
            } else if let Some(location) = archive {
                ndr::archive::list(
                    &location.file,
                    if location.password.is_empty() {
                        None
                    } else {
                        Some(&location.password)
                    },
                )
                .map(|items| {
                    let mut entries = std::collections::BTreeMap::new();
                    // The current folder's own time, by the same rule as its row one level up: its folder entry, otherwise the first entry below it.
                    // The original ".." of an archive subfolder shows that time; previously NDR used the archive file's time at every level.
                    let current = location.prefix.trim_end_matches('/');
                    let (mut own_time, mut first_below) = (None, None);
                    for e in items {
                        // Fix: on Windows 7-Zip reports with \, like "dir\sub\c.txt", so entering an archive subfolder ("dir/") looked empty.
                        let key = e.key.replace('\\', "/");
                        if !current.is_empty() {
                            if key.trim_end_matches('/') == current {
                                own_time = Some(e.modified);
                            } else if first_below.is_none() && key.starts_with(&location.prefix) {
                                first_below = Some(e.modified);
                            }
                        }
                        if let Some(rest) = key.strip_prefix(&location.prefix) {
                            let mut pieces = rest.split('/');
                            if let Some(name) = pieces.next() {
                                if name.is_empty() {
                                    continue;
                                }
                                let folder = pieces.next().is_some() || e.is_dir;
                                // If it is the item itself, use its time. A folder derived from an entry below it (the archive has no folder entry) uses, like the original FUN_00405e60,
                                // the time (+0x414/0x418) of the first entry below that created the folder, and switches to the folder entry's time when a folder entry appears.
                                let slot = entries.entry(name.to_owned()).or_insert(Entry {
                                    path: PathBuf::from(format!("{}{name}", location.prefix)),
                                    name: name.to_owned(),
                                    is_dir: folder,
                                    size: if folder { 0 } else { e.size },
                                    modified: e.modified,
                                    hidden: false,
                                    attributes: if folder { 0x10 } else { 0 },
                                    drive: false,
                                });
                                if rest.trim_end_matches('/') == name {
                                    slot.modified = e.modified;
                                }
                            }
                        }
                    }
                    let mut listed: Vec<Entry> = entries.into_values().collect();
                    if let Some(modified) = own_time.or(first_below) {
                        listed.insert(0, Entry::parent_item(PathBuf::from(".."), modified));
                    }
                    listed
                })
            } else {
                fsops::list_dir(&path, show_hidden).map(|mut entries| {
                    // The drive item name is "%s:" (letter + ':'), like the original DrawListItem.
                    entries.extend(appended.iter().map(|root| Entry {
                        name: root.to_string_lossy().trim_end_matches('\\').to_owned(),
                        path: root.clone(),
                        is_dir: true,
                        size: 0,
                        modified: None,
                        hidden: false,
                        attributes: 0x10,
                        drive: true,
                    }));
                    entries
                })
            };
            // C# ListLocal: descriptions are read together with the listing (descript.dat always, descript.ion when UseDescriptionFile).
            let descriptions = if local && result.is_ok() && !is_drives(&path) {
                compat::read_descriptions(&path, include_legacy).map(|map| map.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect()).unwrap_or_default()
            } else {
                HashMap::new()
            };
            let _ = tx.send(Listing {
                pane: index,
                generation,
                result,
                descriptions,
                drive_infos,
            });
        });
        // The original GetList(0x45a34c) reads the listing in place (the view stays as-is while reading, the new listing when done). The skin theme waits up to
        // 3 seconds for local and archive listings to switch without a "loading" paint. If it takes longer (network, etc.) it reads separately as before and switches when done. FTP does not wait.
        if self.panes[index].remote.is_none() {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut auth = Vec::new();
            while self.panes[index].loading && self.panes[index].generation == generation {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                match self.listing_rx.recv_timeout(left) {
                    Ok(listing) => self.apply_listing(listing, &mut auth),
                    Err(_) => break,
                }
            }
            self.auth_pending.extend(auth);
        }
    }
    pub(super) fn navigate(&mut self, index: usize, path: PathBuf, remember: bool) {
        // Going to a local folder ends the pane's FTP mode (original FUN_0048d6e4: disconnect, then to a local folder).
        if let Some(remote) = self.panes[index].remote.as_ref() {
            if remote.busy.load(Ordering::Relaxed) {
                self.status = self.language_pack.text("RES.FTP_CANNOTDISCONNECT", "Cannot be disconnected while any operation is working.");
                return;
            }
            let active = self.active;
            self.active = index;
            self.ftp_disconnect();
            self.active = active;
        }
        let pane = &mut self.panes[index];
        // Skip if the same location. Fix: previously only the path was compared, so leaving an archive top with ".." (the archive was dropped but the pane path
        // stayed the folder that holds that archive file) did not re-read, so the archive listing remained and those items were treated as local files and couldn't even be copied.
        // Skip only when the currently shown listing (listed) is exactly that local folder.
        let same_listing = pane.archive.is_none() && pane.remote.is_none() && pane.listed.as_deref() == Some(format!("dir|{}", path.display()).to_lowercase().as_str());
        if pane.path == path && !pane.loading && same_listing {
            return;
        }
        // Original pre-location-change event (location +0x38 = FUN_0048631c): if that pane's notification window (TfMessage, +0x30c) is open, close it and clear the selection
        // (the selection is cleared by the new listing).
        if self.pane_message.as_ref().is_some_and(|(_, pane, _)| *pane == index) {
            self.pane_message = None;
        }
        let pane = &mut self.panes[index];
        if remember {
            pane.back.push(pane.path.clone());
            pane.forward.clear();
            // Original history list capacity 20 (FUN_00480d58 → FUN_0045806c(…, 0x14)): when full, remove the oldest (FUN_00458348).
            if pane.back.len() > 19 {
                let extra = pane.back.len() - 19;
                pane.back.drain(..extra);
            }
        }
        // Fix (user request): when going to the current folder's parent via Back, the address bar, etc., place the cursor on the subfolder you came from (so it's visible).
        // The original FUN_00486694 does this only when going with ".." (1) or "\" (2), and Back (FUN_0048d58c) returned to the first item.
        if pane.archive.is_none() && self.pending_focus[index].is_none() {
            if let Ok(rest) = pane.path.strip_prefix(&path) {
                self.pending_focus[index] = rest.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned());
            }
        }
        let pane = &mut self.panes[index];
        pane.path = path.clone();
        pane.address = path.to_string_lossy().into_owned();
        let local = pane.archive.is_none();
        if local {
            pane.tabs[pane.tab] = path.clone();
            // C# NavigateLocalAsync: RememberDriveFolder.
            self.remember_drive_folder(&path);
        }
        self.refresh(index);
    }
    pub(super) fn parent(&mut self) {
        let i = self.active;
        if self.panes[i].remote.is_some() {
            return self.ftp_parent(i);
        }
        // Original FUN_00443f14 0x32 · FUN_00486694 (go up = 1): going up places the cursor on the folder you came from (or the archive file when leaving an archive).
        if let Some(location) = self.panes[i].archive.as_mut() {
            if location.prefix.is_empty() {
                let file = location.file.clone();
                self.panes[i].archive = None;
                self.pending_focus[i] = file.file_name().map(|n| n.to_string_lossy().into_owned());
                if let Some(parent) = file.parent() {
                    self.navigate(i, parent.to_path_buf(), false);
                } else {
                    self.refresh(i);
                }
            } else {
                let trimmed = location.prefix.trim_end_matches('/').to_owned();
                let (up, child) = trimmed.rsplit_once('/').map_or((String::new(), trimmed.clone()), |(p, c)| (format!("{p}/"), c.to_owned()));
                location.prefix = up;
                self.pending_focus[i] = Some(child);
                self.refresh(i);
            }
            return;
        }
        let current = self.panes[i].path.clone();
        if current.parent().is_some() && !is_drives(&current) {
            self.pending_focus[i] = current.file_name().map(|n| n.to_string_lossy().into_owned());
        }
        // Original FUN_00459ba8: if at the root (IsRoot), do nothing.
        if current.parent().is_none() || is_drives(&current) {
            return;
        }
        let Some(parent) = current.parent().map(Path::to_path_buf) else { return };
        {
            self.navigate(
                i,
                if parent.as_os_str().is_empty() {
                    PathBuf::from("/")
                } else {
                    parent
                },
                true,
            );
        }
    }
    pub(super) fn open_current(&mut self) {
        let i = self.active;
        let Some(entry) = self.panes[i].entries.get(self.panes[i].cursor).cloned() else {
            return;
        };
        // Original ".." item: go to the parent location (local/archive/FTP).
        if entry.is_parent() {
            return self.parent();
        }
        if self.panes[i].remote.is_some() {
            // Pane FTP mode: if a folder, go into it. Files are handled only via download (F5/miFTPGet).
            if entry.is_dir {
                self.ftp_open_dir(i, &entry.name);
            }
            return;
        }
        if entry.is_dir {
            if let Some(location) = self.panes[i].archive.as_mut() {
                location.prefix = format!("{}/", entry.path.to_string_lossy().replace('\\', "/"));
                self.refresh(i);
            } else {
                self.navigate(i, entry.path, true);
            }
            return;
        }
        if self.panes[i].archive.is_some() {
            // C# OpenItemAsync: an archive item is extracted to a temp folder and opened, and if the extracted item is an archive, go into it.
            self.open_archive_entry(i, &entry);
            return;
        }
        // C# OpenItemAsync: IsArchive (format table) → open the first volume with GetArchiveFirstVolumePath.
        // A format the bundled 7-Zip cannot read (.ace) is extracted with an external program like C# TryGetExternalArchiveProgram and goes to that folder.
        if ndr::archive::archive_format(&entry.path).is_some() && !ndr::archive::bundled_can_browse(&entry.path) {
            self.open_with_external_archive_program(i, ndr::archive::first_volume_path(&entry.path), true);
        } else if ndr::archive::archive_format(&entry.path).is_some() {
            self.panes[i].archive = Some(ArchiveLocation {
                file: ndr::archive::first_volume_path(&entry.path),
                prefix: String::new(),
                password: String::new(),
            });
            self.refresh(i);
        } else if self.settings.flag("PlayWaveInProgram", true) && entry.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
            // Original 0x483c79 (playwav, initial value on 0x46f088): if the extension is wav, stop with PlaySoundW(NULL), then PlaySoundW(file, 0x22003). Not opened via the shell.
            (self.system.play_wave)(None);
            (self.system.play_wave)(Some(&entry.path));
        } else if let Some(folder) = entry.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")).then(|| (self.system.shortcut_target)(&entry.path)).flatten().filter(|(_, folder)| *folder).map(|(target, _)| target) {
            // Original FUN_00483640 (open file): if a shortcut's (.lnk) target (GetShortcutTarget) is a folder, after GetLongPathNameW point that pane to that folder.
            // Fix: previously it was opened via the shell and a new Explorer window popped up.
            let folder = std::path::absolute(&folder).unwrap_or(folder);
            self.navigate(i, native::long_path(&folder).unwrap_or(folder), true);
        } else {
            // C# OpenItemAsync: RememberOpenedExecutable(.exe/.lnk), then run.
            let mut rows = self.settings.strings("ExecutableHistory");
            ndr::command_line::remember_opened_executable(&mut rows, &entry.path.to_string_lossy());
            self.settings.set_strings("ExecutableHistory", rows);
            if let Err(e) = native::shell_open(&entry.path, None, "open") {
                self.status = e;
            }
        }
    }
    /// Pane setting name ("Left…"/"Right…").
    pub(super) fn side_key(index: usize, name: &str) -> String {
        format!("{}{name}", if index == 0 { "Left" } else { "Right" })
    }
    /// Original TNXScrollStep (scrollstep1/2, initial value 2 = 0x46f20f): 1 = line step, 2 = page step ("page-wise scroll").
    pub(super) fn scroll_by_page(&self, index: usize) -> bool {
        self.settings.values.get(&Self::side_key(index, "ScrollStep")).and_then(|v| v.as_i64()).unwrap_or(2) != 1
    }
    /// Original WMMouseWheel(0x7d5ba0)/UpdateScrollBar(0x7db270): page step scrolls by the number of rows visible in one page, line step by Mouse.WheelScrollLines rows.
    pub(super) fn wheel_step(by_page: bool, view_height: f32, row_height: f32, wheel_lines: u32) -> f32 {
        let rows = if by_page { (view_height / row_height).floor().max(1.0) } else { wheel_lines.max(1) as f32 };
        rows * row_height
    }
    /// The original WMMouseWheel(0x7d5ba0) looks only at WM_MOUSEWHEEL and does not mask Shift (Shift + wheel scrolls the same direction). The tilt wheel (WM_MOUSEHWHEEL,
    /// WMMouseHWheel 0x7d5c94) is only received and moves nothing. egui converts Shift + wheel to horizontal (x), so with Shift the wheel is x.
    pub(super) fn vertical_wheel(input: &egui::InputState) -> f32 {
        if input.modifiers.shift { input.raw_scroll_delta.x } else { input.raw_scroll_delta.y }
    }
    /// The new vertical offset after converting this frame's wheel over the list (height `height`, current cursor position) by the original rule. None if not scrolled.
    /// The list ScrollArea turns off the egui wheel (WHEEL_OFF) and feeds this value as vertical_scroll_offset.
    pub(super) fn wheel_scroll(ui: &egui::Ui, salt: egui::Id, height: f32, row_height: f32, by_page: bool, wheel_lines: u32) -> Option<f32> {
        let area = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
        let (raw, zoom) = ui.input(|i| (Self::vertical_wheel(i), i.modifiers.ctrl || i.modifiers.command));
        if raw == 0.0 || zoom || !ui.rect_contains_pointer(area) {
            return None;
        }
        let notches = raw / ui.ctx().options(|o| o.input_options.line_scroll_speed).max(1.0);
        let current = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(salt)).map_or(0.0, |s| s.offset.y);
        Some((current - notches * Self::wheel_step(by_page, height, row_height, wheel_lines)).max(0.0))
    }
    /// Original SetScrollTop (format 2, page step 0xc5 == 2): vertical position = page number × rows per page × row height (0..page count − 1).
    /// SetScrollTop 0x7e1ee4: otherwise the top is clamped to content − client height (Details add the header), never below 0, then the
    /// scroll bar position follows (in pages when paging) and the list repaints.
    /// Snap the wheel-moved position to the nearest page boundary. The last page also starts at a page boundary (the bottom may look empty).
    pub(super) fn page_snap(offset: f32, page_px: f32, pages: usize) -> f32 {
        if page_px <= 0.0 {
            return offset.max(0.0);
        }
        let page = (offset / page_px).round().max(0.0) as usize;
        page.min(pages.saturating_sub(1)) as f32 * page_px
    }
    pub(super) fn side_text(&self, index: usize, name: &str, default: &str) -> String {
        self.settings.text(&Self::side_key(index, name), default)
    }
    pub(super) fn side_flag(&self, index: usize, name: &str, default: bool) -> bool {
        self.settings.flag(&Self::side_key(index, name), default)
    }
    /// Extension colors (ExtensionColors) and, when using a color map, the NexusFile element colors (NexusFileColorMapValues).
    pub(super) fn color_maps(&self) -> (Option<&serde_json::Map<String, serde_json::Value>>, Option<&serde_json::Map<String, serde_json::Value>>) {
        let extension = self.settings.values.get("ExtensionColors").and_then(|v| v.as_object());
        let nexus = self.settings.flag("UseNexusFileColorMap", false).then(|| self.settings.values.get("NexusFileColorMapValues").and_then(|v| v.as_object())).flatten();
        (extension, nexus)
    }
    /// Original SetLineSpace (+0x364, ini linespace 0~32): the pixels added between rows.
    pub(super) fn list_line_space(&self) -> f32 {
        self.settings.values.get("ListLineSpace").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 32) as f32
    }
    /// Original UpdateDisplayDetails/UpdateDisplayNames (+0x340): item height = the list font's text height + 2, and at least 16 when icons are shown (+0x385).
    /// The text height is VCL Canvas.TextHeight, i.e. the GDI tmHeight of the list font (Tahoma 16 px: 19, rows 21 px apart on the captured original);
    /// egui's own line height (19.3) is only the fallback. Previously the rounded-up egui height made every row 1 px taller (22 px).
    pub(super) fn list_item_height(&self, ui: &egui::Ui) -> f32 {
        let text = self.list_text_height().unwrap_or_else(|| ui.text_style_height(&egui::TextStyle::Body).ceil()) + 2.0;
        if self.settings.flag("ShowIcons", true) { text.max(16.0) } else { text }
    }
    /// VCL Canvas.TextHeight of the list font (GDI tmHeight), the unit of the original list, header and thumbnail text layouts. None without GDI.
    pub(super) fn list_text_height(&self) -> Option<f32> {
        let face = self.settings.text("ListFont", "Segoe UI");
        ndr::native::gdi_text_height(&face, self.list_font_size.round() as i32, self.list_font_style).map(|h| h as f32)
    }
    /// Original DrawListItem (+0x387): an NXLINE horizontal line (integer division, thickness 1) on the pixel row below the item (bottom + LineSpace / 2 − 1).
    /// Here the center of that pixel row (bottom + floor(LineSpace / 2) − 0.5) is aligned to a screen pixel center.
    /// Fix: when the line position fell between pixels (non-integer list position/screen scale), the 1-pixel line blurred across two rows.
    pub(super) fn row_line_y(painter: &egui::Painter, item_bottom: f32, line_space: f32) -> f32 {
        let y = item_bottom + (line_space / 2.0).floor() - 0.5;
        painter.round_to_pixel_center(y)
    }
    /// Line color of the original SetShowGridColumn (+0x386)/SetShowGridRow (+0x387): GetBasic(13) NXLINE. When not using a color map, the theme's separator color.
    pub(super) fn grid_line_color(&self, ui: &egui::Ui) -> Color32 {
        let (_, nexus) = self.color_maps();
        nexus
            .and_then(|map| map.get("NXLINE"))
            .and_then(|v| v.as_str())
            .and_then(ndr::styles::parse_color)
            .map(|[r, g, b, _]| Color32::from_rgb(r, g, b))
            .unwrap_or(ui.visuals().widgets.noninteractive.bg_stroke.color)
    }
    /// C# SortPaneItems + ReorderPane: reorder by the pane settings (sort mode, descending, folder sort, logical sort, grouping),
    /// keeping the selection and cursor on the same items.
    pub(super) fn sort_pane(&mut self, index: usize) {
        let options = ndr::listing::SortOptions {
            folder_mode: self.side_text(index, "FolderSortMode", "Auto"),
            folder_descending: self.side_flag(index, "FolderSortDescending", false),
            file_mode: self.side_text(index, "SortMode", "Name"),
            file_descending: self.side_flag(index, "SortDescending", false),
            logical: self.settings.flag("UseLogicalSort", true),
        };
        let group_mode = self.side_text(index, "GroupMode", "None");
        let category_mode = self.side_text(index, "FolderCategoryMode", "Auto");
        let group_descending = self.side_flag(index, "GroupDescending", false);
        let (extension_colors, nexus_colors) = self.color_maps();
        let (extension_colors, nexus_colors) = (extension_colors.cloned(), nexus_colors.cloned());
        let color = |entry: &Entry| ndr::listing::color_sort_key(entry, extension_colors.as_ref(), nexus_colors.as_ref());
        let pane = &mut self.panes[index];
        let cursor = pane.entries.get(pane.cursor).map(|e| e.path.clone());
        let marked: HashSet<PathBuf> = pane.selected.iter().filter_map(|i| pane.entries.get(*i).map(|e| e.path.clone())).collect();
        // The original ".." item comes first regardless of sorting.
        let (mut parents, rest): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut pane.entries).into_iter().partition(|e| e.is_parent());
        pane.entries = rest;
        ndr::listing::sort_entries(&mut pane.entries, &options, &color);
        ndr::listing::sort_groups(&mut pane.entries, &group_mode, &category_mode, group_descending);
        // The original drive items (kind 4) sit at the end of the list in TNXDriveList order (alphabetical), regardless of sorting.
        let (mut items, mut drives): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut pane.entries).into_iter().partition(|e| !e.drive);
        drives.sort_by(|a, b| a.path.cmp(&b.path));
        items.append(&mut drives);
        parents.append(&mut items);
        pane.entries = parents;
        pane.selected = pane.entries.iter().enumerate().filter(|(_, e)| marked.contains(&e.path)).map(|(i, _)| i).collect();
        if let Some(cursor) = cursor {
            pane.cursor = pane.entries.iter().position(|e| e.path == cursor).unwrap_or(0);
        }
        self.compute_view(index);
    }
    /// Original SetSortType/SetSortDescend (0x7dbb28/0x7dba3c): changing the sort runs the list's FileListChanged → AdjustPage(old cursor index),
    /// then SetItemIndex (make visible) to the cursor name's new position. SetFolderSortType 0x7dbc14 does the same for the folder sort
    /// (remembers the focused name, re-sorts, SetItemIndex to that name).
    /// Fix: previously, if the cursor item moved due to sorting, the view stayed put and it could be off-screen.
    pub(super) fn resort_pane(&mut self, index: usize) {
        let old = self.panes[index].cursor;
        self.sort_pane(index);
        let pane = &mut self.panes[index];
        if !pane.entries.is_empty() {
            pane.adjust = Some((old.min(pane.entries.len() - 1), false));
            pane.reveal = true;
        }
    }
    pub(super) fn sort_command(&mut self, command: &str) -> bool {
        let i = self.active;
        let set = |app: &mut App, name: &str, value: &str| app.settings.set_text(&Self::side_key(i, name), value);
        let toggle = |app: &mut App, name: &str| {
            let key = Self::side_key(i, name);
            let old = app.settings.flag(&key, name == "ShowGroupName");
            app.settings.set_flag(&key, !old);
        };
        let sort_modes = ["None", "Name", "Extension", "Size", "Modified", "Attributes", "Color"];
        if let Some(mode) = command.strip_prefix("sort:") {
            if !sort_modes.contains(&mode) {
                return true;
            }
            set(self, "SortMode", mode);
        } else if let Some(mode) = command.strip_prefix("folderSort:") {
            if mode == "None" || !sort_modes.contains(&mode) {
                return true;
            }
            set(self, "FolderSortMode", mode);
        } else if let Some(mode) = command.strip_prefix("groupMode:") {
            if !["None", "Extension", "Size", "Time", "Category"].contains(&mode) {
                return true;
            }
            set(self, "GroupMode", mode);
        } else if let Some(mode) = command.strip_prefix("folderCategory:") {
            if !["Auto", "Default"].contains(&mode) {
                return true;
            }
            set(self, "FolderCategoryMode", mode);
        } else {
            match command {
                "sortDescending" => toggle(self, "SortDescending"),
                "folderSortDescending" => toggle(self, "FolderSortDescending"),
                "groupDescending" => toggle(self, "GroupDescending"),
                "groupShowName" => toggle(self, "ShowGroupName"),
                "sortNone" | "sortName" | "sortExtension" | "sortSize" | "sortTime" | "sortAttributes" | "sortColor" => {
                    let mode = match command {
                        "sortNone" => "None",
                        "sortExtension" => "Extension",
                        "sortSize" => "Size",
                        "sortTime" => "Modified",
                        "sortAttributes" => "Attributes",
                        "sortColor" => "Color",
                        _ => "Name",
                    };
                    // Original miSortByClick(0x431c88): re-choosing the current sort kind (except None) flips descending (skin theme).
                    let current = self.side_text(i, "SortMode", "Name");
                    if mode != "None" && current == mode {
                        toggle(self, "SortDescending");
                    } else {
                        set(self, "SortMode", mode);
                    }
                }
                _ => return false,
            }
        }
        self.resort_pane(i);
        true
    }
    pub(super) fn pane_ui(&mut self, ui: &mut egui::Ui, index: usize, ctx: &egui::Context) {
        // Original pane (TNXFramePanel): pressing anywhere in the pane (including empty space, the address bar, and the info bar) makes that pane the active pane.
        // Exclude presses on an upper layer like a menu or dialog, and measure the position with this frame's pane area (ui.max_rect).
        // Fix: it used to measure with the previous frame's pane_rects, but update cleared those before drawing panes, so pressing empty space didn't change the pane.
        let area = ui.max_rect();
        if let Some(pos) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None }) {
            let on_background = ctx.layer_id_at(pos).is_none_or(|layer| layer.order == egui::Order::Background);
            if on_background && area.contains(pos) && self.original_dialogs.is_empty() && self.msgboxes.is_empty() {
                self.active = index;
            }
        }
        let active = self.active == index;
        // Cursor bar style (DrawFocus focused/unfocused) follows the list's real focus: the original draws the unfocused bar when the window is
        // inactive or the focus is in the address field or a dialog. Previously NDR used the focused bar for the active pane at all times.
        let list_focus = active && self.list_has_focus(ctx);
        // Original pane: border lframe*, and the address bar (TNXPathLabel) sits above the list (original FUN_00481978). If the skin cannot be read, do not draw.
        let Some(skin_now) = self.current_skin() else { return };
        let skin = Some(skin_now);
        // Original MouseUp(0x7d6740): releasing the left button ends the selection-area drag.
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.panes[index].range_drag = None;
        }
        let stroke = match &skin {
            Some(skin) => self.skin_frame_stroke(skin, index),
            None => egui::Stroke::NONE,
        };
        // Original pane frame: only a border (lframewidth), with no rounded corners or inner padding. egui 0.33 Frame already reserves the stroke
        // width (total margin = inner margin + stroke width), so no inner margin: the address bar starts on the row right below the border.
        // Previously an inner margin of the stroke width left a 1 px gap inside the border on every side (captured original: none).
        let frame = egui::Frame::new();
        // The original tab bar (TNXTabSet) sits outside the frame panel (+0x330), attached right above the pane (FUN_00481fb8 SetParent(pane)).
        let outer_spacing = ui.spacing().item_spacing.y;
        if let Some(skin) = &skin {
            ui.spacing_mut().item_spacing.y = 0.0;
            self.skin_tab_set(ui, index, ctx, skin);
        }
        let shown = frame
            .stroke(stroke)
            .show(ui, |ui| {
                // The original frame's inner controls (address bar, info bar, list, file info bar) stick together with Align, no gaps. Fix: previously egui's default gap (3 pixels)
                // slipped in between the folder info bar and the list, and between the list and the file info bar.
                let _ = outer_spacing;
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut activate = false;
                let mut open = false;
                let mut command = None;
                let mut begin_drag = None;
                let mut folder_rows: Vec<(usize, egui::Rect, PathBuf)> = Vec::new();
                // Inside the original frame panel, top to bottom: the drive button row (when on), the address bar, the folder info bar, the list (no gaps between them).
                if let Some(skin) = &skin {
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        self.skin_drive_bar(ui, index, ctx, skin);
                        self.skin_address_bar(ui, index, ctx, skin);
                        self.skin_folder_info_bar(ui, index, skin);
                    });
                }
                // Original FUN_0048da38 (miShowFoldersClick 0x445258, Alt+F3): per pane, to the left of the list (alLeft, between the address/folder-info bars above and the file info bar
                // below) a folder tree (width = setting +0x48 + pane × 4, ini treewidth1/2), with a 3-pixel splitter bar to its right (MinSize 50; when dragged,
                // FUN_0048e2c0 keeps the width). The list takes the rest to the right.
                let outer: &mut egui::Ui = ui;
                let full = outer.available_rect_before_wrap();
                let tree_shown = self.settings.flag(if index == 0 { "ShowLeftFolderTree" } else { "ShowRightFolderTree" }, false);
                let tree_key = if index == 0 { "LeftFolderTreeWidth" } else { "RightFolderTreeWidth" };
                let tree_width = if tree_shown { (self.settings.values.get(tree_key).and_then(|v| v.as_f64()).unwrap_or(120.0) as f32).clamp(50.0, (full.width() - 53.0).max(50.0)) } else { 0.0 };
                let splitter = if tree_shown { 3.0 } else { 0.0 };
                if !tree_shown {
                    self.panes[index].tree_rect = None;
                }
                if tree_shown {
                    let spacing = outer.spacing().item_spacing.y;
                    let below = self.skin_file_info_height() + spacing + self.ftp_log_reserve(index, spacing);
                    let height = (full.height() - below).max(80.0);
                    let tree_rect = egui::Rect::from_min_size(full.min, egui::vec2(tree_width, height));
                    self.pane_tree_ui(outer, index, tree_rect);
                    let split = egui::Rect::from_min_size(egui::pos2(tree_rect.right(), full.top()), egui::vec2(splitter, height));
                    let drag = outer.interact(split, egui::Id::new(("ndr-pane-tree-split", index)), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                    if drag.dragged() {
                        if let Some(pointer) = drag.interact_pointer_pos() {
                            let width = (pointer.x - full.left()).clamp(50.0, (full.width() - 53.0).max(50.0)).round();
                            self.settings.values.insert(tree_key.into(), serde_json::json!(width as f64));
                        }
                    }
                    if drag.drag_stopped() {
                        self.save();
                    }
                }
                let mut list_ui = outer.new_child(egui::UiBuilder::new().id_salt("ndr-pane-list").max_rect(egui::Rect::from_min_max(egui::pos2(full.left() + tree_width + splitter, full.top()), full.max)).layout(egui::Layout::top_down(egui::Align::Min)));
                let ui = &mut list_ui;
                // C# ConfigureGrid columns: Name, Extension (when not attached to the name), Kind (ShowFileType), Size, Modified date, Attributes, Description.
                let columns = self.detail_columns(self.language == "ko");
                // Original initial value viewstyle1/2 = 0 (Auto view).
                let view = self.settings.text(if index == 0 { "LeftViewStyle" } else { "RightViewStyle" }, "Auto");
                let tiles = matches!(view.as_str(), "Thumbnails" | "viewThumbnails");
                let names = matches!(view.as_str(), "Names" | "viewNames");
                let grouped = self.side_text(index, "GroupMode", "None") != "None";
                // Original UpdateDisplayAuto: if there is more than one column or a single column does not fit (horizontal bar), a column-flowing list (no header);
                // if one column fits, a single column like Details. Group view (C# version only) uses C# RebuildAutoColumns (per-column scrolling) as-is.
                let (auto_columns, auto_bar) = if matches!(view.as_str(), "Auto" | "viewAuto") {
                    if grouped {
                        (self.auto_column_count(ui, index, ui.available_width()), false)
                    } else {
                        let (_, columns, bar) = self.flow_shape(ui, index, ui.available_width(), (ui.available_height() - 60.0).max(80.0), true);
                        (columns, bar)
                    }
                } else {
                    (1, false)
                };
                // Original DrawItems(0x7da82c): Auto(0) and Names(1) views are always column-flowing lists, and the header (DrawListHeader) is drawn only in Details(2).
                // A single item change goes through DrawItem(TNXFileItem) 0x7ded00 (that row painted off screen with DrawItem(TCanvas) 0x7d7ad4,
                // clipped to the client area and below the Details header); egui repaints the whole list each frame instead.
                // So in the skin theme, even when it fits one column it is a flowing list (no header, with the UpdateDisplayAuto cell width).
                let auto_view = matches!(view.as_str(), "Auto" | "viewAuto");
                let auto_flow = !grouped && (auto_columns > 1 || auto_bar || (auto_view && skin.is_some()));
                // Original skin vertical bar (TNXScrollBar): shown only when overflowing, and the list shrinks by its width. Whether it overflows is measured from the last paint.
                let skin_bar = match &skin {
                    Some(s) => self.skin_scrollbar_width(ctx, s),
                    None => None,
                };
                let bar_flag = egui::Id::new(("ndr-skin-vbar", index));
                let bar_reserve = match skin_bar {
                    Some(w) if ui.data(|d| d.get_temp::<bool>(bar_flag)).unwrap_or(false) => w,
                    _ => 0.0,
                };
                // Cell widths: Details view uses the C# star ratio; single-column Auto view uses C# ApplyAutoDetailColumns (name = the rest, cells are content width, hidden when there is not enough room).
                let cell_widths: Vec<Option<f32>> = if tiles || names || auto_columns > 1 || auto_flow {
                    Vec::new()
                } else {
                    self.original_detail_widths(ui, index, ui.available_width() - bar_reserve, &columns)
                };
                // Original UpdateScrollBar(format 2, 0x7db270): if the sum of cell widths (0x370) is larger than the list width (GetRealClientWidth, width minus the vertical bar),
                // show the horizontal bar (0x334, thickness 12) and the horizontal position (0x378) is 0..sum − width; otherwise the horizontal position = 0. AdjustPage(format 2) also leaves it at 0.
                // Fix: previously you could not scroll horizontally, so the right columns stayed cut off and could not be seen.
                let detail_content: f32 = cell_widths.iter().flatten().sum();
                let detail_avail = ui.available_width() - bar_reserve;
                let detail_hbar = skin_bar.filter(|_| !cell_widths.is_empty() && detail_content > detail_avail + 0.5);
                let detail_x = {
                    let pane = &mut self.panes[index];
                    if pane.adjust.is_some() {
                        pane.detail_x = 0.0;
                    }
                    let max_x = if detail_hbar.is_some() { (detail_content - detail_avail).max(0.0) } else { 0.0 };
                    pane.detail_x = pane.detail_x.clamp(0.0, max_x);
                    pane.detail_x
                };
                if !tiles && !names && auto_columns == 1 && !auto_flow {
                    let current = self.side_text(index, "SortMode", "Name");
                    let font = egui::FontId::new(self.list_font_size, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()));
                    let clicked = self.skin_list_header(ui, index, &columns, &cell_widths, font, detail_x);
                    if let Some(mode) = clicked {
                        // Original header(0x484d88): the same mode flips descending; a different mode only sets the sort type (descending stays).
                        let descending = Self::side_key(index, "SortDescending");
                        let old = self.settings.flag(&descending, false);
                        let flip = if current == mode { !old } else { old };
                        self.settings.set_flag(&descending, flip);
                        self.settings.set_text(&Self::side_key(index, "SortMode"), mode);
                        self.resort_pane(index);
                    }
                }
                if let Some(error) = &self.panes[index].error {
                    ui.colored_label(Color32::LIGHT_RED, self.archive_error_text(error));
                }
                if self.panes[index].loading {
                    ui.spinner();
                }
                // Like C#, the filter is applied when the listing arrives, so the visible rows = the whole list (matching the selection and operation targets).
                let visible: Vec<usize> = (0..self.panes[index].entries.len()).collect();
                let reserve = self.skin_file_info_height() + ui.spacing().item_spacing.y;
                // Original FTP-mode pane: a splitter bar and a log box below the list (FUN_0048d7ac).
                let reserve = reserve + self.ftp_log_reserve(index, ui.spacing().item_spacing.y);
                let height = (ui.available_height() - reserve).max(80.0);
                let ftp_log_room = height + self.panes[index].ftp_log_height;
                // Original/C# version ListFont/ListFontSize: only the list (name/cell text, row-height calculation) uses that font and size. The header, address bar, etc. use the normal font.
                let saved_text_styles = ui.style().text_styles.clone();
                let list_font_size = self.list_font_size;
                let list_style = self.list_font_style;
                for style in [egui::TextStyle::Body, egui::TextStyle::Button] {
                    if let Some(font) = ui.style_mut().text_styles.get_mut(&style) {
                        font.family = egui::FontFamily::Name(super::LIST_FONT_FAMILY.into());
                        font.size = list_font_size;
                    }
                }
                let row_height = self.list_item_height(ui);
                let list_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
                // Original WMSize(0x7d5814)/SetListStyle(0x7dc6d4) → AdjustPage: when the list size or view style changes, reposition to follow the cursor.
                let shape = (list_rect.size(), view.clone());
                let pane = &mut self.panes[index];
                if pane.list_shape.as_ref().is_some_and(|old| (old.0 - shape.0).length() > 0.5 || old.1 != shape.1) && !pane.entries.is_empty() {
                    pane.adjust = Some((pane.cursor.min(pane.entries.len() - 1), false));
                }
                pane.list_shape = Some(shape);
                pane.list_rect = Some(list_rect);
                // User request (2026-10-31 16:49): the wheel scrolls the list like the original WMMouseWheel(0x7d5ba0) (one page at a time when set to page step).
                // The "wheel = move cursor up/down" from r138~r160 was reverted. Each view's wheel_scroll handles it.
                if tiles {
                    let(a,o,c)=self.tile_rows(ui,index,&visible,height,view.contains("humb"));activate|=a;open|=o;if c.is_some(){command=c;}
                } else if names || auto_flow {
                    let (a, o, c) = self.flow_rows(ui, index, height, !names);
                    activate |= a;
                    open |= o;
                    if c.is_some() {
                        command = c;
                    }
                } else if auto_columns > 1 {
                    let (a, o, c) = self.auto_rows(ui, index, auto_columns, height);
                    activate |= a;
                    open |= o;
                    if c.is_some() {
                        command = c;
                    }
                } else {
                if self.panes[index].view.cells.len() != self.panes[index].entries.len() {
                    self.compute_view(index);
                }
                // Original drive item (kind 4): the end of the name cell (the widest drive name) and the DISKFREE text.
                let drive_extent = self.drive_text_extent(ui, index);
                let free_label = self.language_pack.text("RES.DISKFREE", "Free");
                let icons_shown = self.settings.flag("ShowIcons", true);
                let view_cache = std::mem::take(&mut self.panes[index].view);
                let (rows, texts) = (&view_cache.rows, &view_cache);
                let width = ui.available_width() - bar_reserve;
                // Original GetItemRect(format 2): the row starts at −horizontal position and its width is the sum of cell widths (the list width when narrower than the list). When the horizontal bar is shown,
                // the row area is lowered by its thickness.
                let view_width = width;
                let width = width.max(detail_content);
                let height = match detail_hbar {
                    Some(bar) => (height - bar).max(row_height),
                    None => height,
                };
                // Original UpdateDisplayDetails: row stride = item height + LineSpace(+0x364). egui adds item_spacing.y per row, so use that as LineSpace.
                // Fix: previously egui's default gap (3 pixels) was added, so rows were sparser than the original and pressing that gap hit no item.
                let line_space = self.list_line_space();
                let saved_spacing = ui.spacing().item_spacing.y;
                ui.spacing_mut().item_spacing.y = line_space;
                let (row_lines, line_color) = (self.settings.flag("ShowRowLines", false), self.grid_line_color(ui));
                // show_rows stacks rows adding item_spacing.y each, so position math uses that stride too.
                let stride = row_height + ui.spacing().item_spacing.y;
                let by_page = self.scroll_by_page(index);
                let page_rows = ((height / stride).floor() as usize).max(1);
                let pages = rows.len().div_ceil(page_rows).max(1);
                let page_px = page_rows as f32 * stride;
                // Original SetScrollTop(page step): a wheel-moved position is a page boundary (page number × one page).
                let wheel = Self::wheel_scroll(ui, egui::Id::new(("files", index)), height, stride, by_page, (self.system.wheel_lines)()).map(|o| if by_page { Self::page_snap(o, page_px, pages) } else { o });
                // Original DoTargetScroll: while dragging, at the top/bottom edge scroll one line (one page in page step) at a time.
                let dy = std::mem::take(&mut self.panes[index].drag_scroll.0);
                self.panes[index].drag_scroll.1 = 0;
                let wheel = if dy != 0 {
                    let step = if by_page { page_px } else { stride };
                    let now_offset = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(egui::Id::new(("files", index)))).map_or(0.0, |s| s.offset.y);
                    let moved = (now_offset + dy as f32 * step).max(0.0);
                    Some(if by_page { Self::page_snap(moved, page_px, pages) } else { moved })
                } else {
                    wheel
                };
                let mut area = egui::ScrollArea::both()
                    .id_salt(("files", index))
                    .scroll_source(WHEEL_OFF)
                    .auto_shrink([false, false])
                    .max_height(height)
                    .min_scrolled_height(height);
                if skin_bar.is_some() {
                    area = area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
                }
                // Original SetItemIndex(format 2): when the cursor row moved by a key goes off the top, bring it to the top; off the bottom, bring it to the bottom.
                // In page step, bring the first row of the page the cursor is on to the top ((cursor / rows per page) × rows per page).
                // The page info (visible row count, the top/bottom visible items) is used by ArrowKey's PageUp/PageDown.
                let current = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(egui::Id::new(("files", index)))).map_or(0.0, |s| s.offset.y);
                let mut offset = wheel.unwrap_or(current);
                // Original AdjustPage(format 2): horizontal position = 0. In page step, bring the first row of the page that item is on to the top; otherwise top when it goes off the top,
                // bottom when it goes off the bottom. A re-read listing is not scrolled vertically because the original then reverts with SetScrollTop(old position).
                if let Some((item, reload)) = self.panes[index].adjust.take() {
                    if self.panes[index].detail_x != 0.0 {
                        self.panes[index].detail_x = 0.0;
                        ui.ctx().request_repaint();
                    }
                    if let Some(line) = rows.iter().position(|r| matches!(r, DetailRow::Item(i) if *i == item)).filter(|_| !reload) {
                        let (top, bottom) = (line as f32 * stride, line as f32 * stride + row_height);
                        if by_page {
                            offset = (line / page_rows) as f32 * page_px;
                        } else if top < offset {
                            offset = top;
                        } else if bottom > offset + height {
                            offset = bottom - height;
                        }
                    }
                }
                if std::mem::take(&mut self.panes[index].reveal) && wheel.is_none() {
                    let cursor = self.panes[index].cursor;
                    if let Some(line) = rows.iter().position(|r| matches!(r, DetailRow::Item(i) if *i == cursor)) {
                        let (top, bottom) = (line as f32 * stride, line as f32 * stride + row_height);
                        if top < offset || bottom > offset + height {
                            offset = if by_page {
                                (line / page_rows) as f32 * page_px
                            } else if top < offset {
                                top
                            } else {
                                bottom - height
                            };
                        }
                    }
                }
                let item = |r: &DetailRow| if let DetailRow::Item(i) = r { Some(*i) } else { None };
                let first_line = (offset / stride - 0.01).ceil().max(0.0) as usize;
                let last_line = (((offset + height - row_height).max(0.0) / stride).floor() as usize).max(first_line);
                let top_item = rows.iter().skip(first_line).find_map(item).unwrap_or(0);
                let last_item = rows.iter().take(last_line + 1).rev().find_map(item).unwrap_or(top_item).max(top_item);
                self.panes[index].page = list_keys::ListPage { style: list_keys::ListStyle::Details, rows: page_rows, columns: 1, top: top_item, last: last_item };
                if (offset - current).abs() > 0.5 {
                    area = area.vertical_scroll_offset(offset);
                }
                area = area.horizontal_scroll_offset(self.panes[index].detail_x);
                // In page step with multiple pages, pad empty rows so the last page also starts at a page boundary (the original leaves the area below the last page empty).
                let drawn = if by_page && pages > 1 { rows.len().max(pages * page_rows + 2) } else { rows.len() };
                // Original skin theme: the cursor/selection bar (DrawFocus), the cell colors (DrawListItem), and the selection mark ▶ (InitSelMarkBitmap).
                let original = Some(self.original_list_colors());
                let output = area
                    .show_rows(ui, row_height, drawn, |ui, range| {
                        // So it can scroll horizontally, the content width = the row width (sum of cell widths or the list width). egui's inner width includes the vertical bar area,
                        // so add that much to bring the last cell up to just before the vertical bar.
                        ui.set_min_width(width + bar_reserve);
                        for visible_index in range {
                            let Some(detail_row) = rows.get(visible_index) else { continue };
                            let row = match detail_row {
                                DetailRow::Header(label) => {
                                    ui.add_sized([width, row_height], egui::Label::new(RichText::new(label).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())).color(ui.visuals().weak_text_color())));
                                    continue;
                                }
                                DetailRow::Item(row) => *row,
                            };
                            // C# QueueFolderSizeCalculation: when a visible folder row appears.
                            self.queue_folder_size(index, row);
                            let icon = self.file_icon(ui.ctx(), index, row);
                            let icon = self.row_icon_for_skin(index, row, icon);
                            let pane = &mut self.panes[index];
                            let entry = &pane.entries[row];
                            let selected = pane.selected.contains(&row);
                            let default = if entry.is_dir {
                                Color32::from_rgb(228, 188, 85)
                            } else if entry.hidden {
                                Color32::GRAY
                            } else {
                                ui.visuals().text_color()
                            };
                            let color = texts.colors[row].map_or(default, |(r, g, b)| Color32::from_rgb(r, g, b));
                            let cells = &texts.cells[row];
                            let cursor_row = pane.cursor == row;
                            let bar_slot = original.map(|_| ui.painter().add(egui::Shape::Noop));
                            let framed = false;
                            // The cursor row text is NXBACK only on the focused bar (DrawFocus); without list focus the bar is a faint fill and the text keeps
                            // the item color. Previously an active pane without focus drew NXBACK text on the faint bar, which hid the name.
                            let name_color = match original {
                                Some(o) if cursor_row && list_focus => o.back,
                                _ => color,
                            };
                            let line = ui
                                .horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    // Original DrawListItem kind 4: a drive item draws, on one line regardless of columns, the name ("C: name") and the remaining space.
                                    if entry.drive {
                                        let info = pane.drive_infos.get(&entry.path).map(|(_, kind, sizes)| (*kind, *sizes));
                                        return super::list_view::drive_row(ui, egui::vec2(width, row_height), icon.as_ref(), &cells[0], name_color, framed, pane.cursor == row, drive_extent, info, &free_label, list_style);
                                    }
                                    let mut name_response = None;
                                    for ((column, _, _), cell) in columns.iter().zip(&cell_widths) {
                                        let Some(cell) = *cell else { continue };
                                        let folder_size = (*column == DetailColumn::Size && entry.is_dir).then(|| pane.folder_sizes.get(&entry.path).map(|s| s.clone().unwrap_or_else(|| "…".into()))).flatten();
                                        let text = folder_size.as_ref().unwrap_or(&cells[column.index()]);
                                        let size = [cell, row_height];
                                        if *column == DetailColumn::Name {
                                            name_response = Some(App::name_cell(ui, egui::vec2(cell, row_height), icon.as_ref(), icons_shown, text, name_color, list_style));
                                            if cursor_row && original.is_some() {
                                                if let Some(response) = &name_response {
                                                    note_cursor_name(&self.name_tip, ui, index, row, response.rect, text, &entry.name, list_style);
                                                }
                                            }
                                        } else if let Some(o) = original {
                                            // Original DrawListItem: an uncounted folder's RES DIR text keeps the item color (counted sizes use the size colors).
                                            let color_column = if *column == DetailColumn::Size && entry.is_dir && folder_size.is_none() { DetailColumn::Name } else { *column };
                                            let cell_color = o.cell(color_column, color, entry.attributes, cursor_row, list_focus);
                                            let (rect, _) = ui.allocate_exact_size(egui::vec2(cell, row_height), egui::Sense::hover());
                                            paint_original_cell(ui, rect, *column, text, cell_color, cursor_row, &o, list_style);
                                        } else {
                                            ui.add_sized(size, egui::Label::new(super::list_text(text.as_str(), list_style).color(if *column == DetailColumn::Description { Color32::from_rgb(0x36, 0x82, 0xAF) } else { ui.visuals().weak_text_color() })).truncate());
                                        }
                                    }
                                    name_response.expect("the name column is always shown")
                                });
                            if let (Some(slot), Some(o)) = (bar_slot, original) {
                                let rect = egui::Rect::from_min_size(line.response.rect.left_top(), egui::vec2(width, row_height));
                                if cursor_row || selected {
                                    let bar = super::skin_pane::focus_bar(color, o.back, o.select, cursor_row, selected, list_focus);
                                    ui.painter().set(slot, egui::Shape::Vec(super::skin_pane::focus_bar_shapes(rect, &bar)));
                                }
                                if selected {
                                    ui.painter().extend(super::skin_pane::selection_mark_shapes(rect, o.select));
                                }
                            }
                            // Original DrawListItem(+0x387): an NXLINE horizontal line below the item (bottom + LineSpace / 2 − 1). Here it is in the middle of the gap between rows.
                            if row_lines {
                                let y = Self::row_line_y(ui.painter(), line.response.rect.bottom(), ui.spacing().item_spacing.y);
                                let left = line.response.rect.left();
                                ui.painter().hline(left..=left + width, y, egui::Stroke::new(1.0_f32, line_color));
                            }
                            // Pressing anywhere on the row hits that item (the whole row, not just the name cell). Original GetItemAt(format 2): index = (y + position + LineSpace − header)
                            // / (height + LineSpace), so the gap above the row (LineSpace) also belongs to that item.
                            let gap = ui.spacing().item_spacing.y;
                            let row_rect = egui::Rect::from_min_size(line.response.rect.left_top() - egui::vec2(0.0, gap), egui::vec2(width, row_height + gap));
                            let response = ui.interact(row_rect, ui.id().with(("ndr-detail-row", index, row)), egui::Sense::click_and_drag());
                            // C# BeginFileDragTracking: dragging with the left button drags the item. The original also accepts a right-button drag and opens pmDrop on drop.
                            let folder_path = entry.is_dir.then(|| entry.path.clone());
                            // Original MouseDown: the skin theme handles the moment of the press (the left 12 pixels of the row are the selection area, Shift is a range drag).
                            let in_press = skin.is_some() && original_press_input(pane, row, response.rect, ui);
                            let area_hit = skin.is_some() && pointer_in_selection_area(response.rect, ui);
                            // Original (skin): even during a range drag after a Shift press, moving more than 8 pixels starts a drag (FUN_004856d8) (not a selection-area range drag).
                            if drag_begins(&response, egui::PointerButton::Primary, skin.is_some()) && (skin.is_some() || pane.range_drag.is_none()) {
                                begin_drag = Some((row, false));
                            } else if drag_begins(&response, egui::PointerButton::Secondary, skin.is_some()) {
                                begin_drag = Some((row, true));
                            }
                            if let Some(path) = folder_path {
                                let visual = egui::Rect::from_min_size(line.response.rect.left_top(), egui::vec2(width, row_height));
                                folder_rows.push((index, visual.intersect(ui.clip_rect()), path));
                            }
                            if response.clicked() {
                                activate = true;
                                if skin.is_none() {
                                    let modifiers = ui.input(|i| i.modifiers);
                                    row_pressed(pane, row, modifiers, false, false);
                                }
                            }
                            if in_press {
                                activate = true;
                            }
                            // Original DblClick(0x7dd1ec): does not open from the selection area. A double press follows the Windows standard (time and position).
                            if super::panes::windows_double_clicked(&response) && !area_hit {
                                pane.cursor = row;
                                activate = true;
                                open = true;
                            }
                            // WPF DataGrid: the right button selects that row (keeps the selection if the row is already selected). The skin theme moves only the cursor, like the original.
                            if response.secondary_clicked() {
                                row_pressed(pane, row, egui::Modifiers::NONE, true, skin.is_some());
                                activate = true;
                            }
                            // Do not attach a menu to the row response (the pane menu is skin_pane::pane_menu).
                            if response.secondary_clicked() {
                                super::skin_pane::request_pane_menu(ui.ctx(), index);
                            }
                        }
                    });
                // Original SetScrollTop(page step): dragging and releasing the bar lands on the nearest page boundary (page number).
                if by_page && pages > 1 && !ui.input(|i| i.pointer.any_down()) {
                    let snapped = Self::page_snap(output.state.offset.y, page_px, pages);
                    if (snapped - output.state.offset.y).abs() > 0.5 {
                        let mut state = output.state;
                        state.offset.y = snapped;
                        state.store(ui.ctx(), output.id);
                        ui.ctx().request_repaint();
                    }
                }
                if let (Some(bar_width), Some(skin)) = (skin_bar, &skin) {
                    let needs = output.content_size.y > output.inner_rect.height() + 0.5;
                    if ui.data(|d| d.get_temp::<bool>(bar_flag)) != Some(needs) {
                        ui.data_mut(|d| d.insert_temp(bar_flag, needs));
                        ui.ctx().request_repaint();
                    }
                    if needs {
                        let inner = output.inner_rect;
                        let bar = egui::Rect::from_min_max(egui::pos2(inner.right() - bar_width, inner.top()), inner.max);
                        if let Some(next) = self.skin_vertical_scrollbar(ui, ctx, skin, bar_flag.with("bar"), bar, output.state.offset.y, output.content_size.y, inner.height(), stride) {
                            let mut state = output.state;
                            state.offset.y = next;
                            state.store(ui.ctx(), output.id);
                            ui.ctx().request_repaint();
                        }
                    }
                }
                // Original horizontal TNXScrollBar(format 2): below the list, width = GetRealClientWidth, one step 10, one page = the list width.
                self.panes[index].detail_x = output.state.offset.x.clamp(0.0, (detail_content - view_width).max(0.0));
                if let (Some(bar), Some(skin)) = (detail_hbar, &skin) {
                    let (strip, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), bar), egui::Sense::hover());
                    let rect = egui::Rect::from_min_size(strip.min, egui::vec2(view_width, bar));
                    if let Some(next) = self.skin_horizontal_scrollbar(ui, ctx, skin, egui::Id::new(("ndr-skin-detail-hbar", index)), rect, self.panes[index].detail_x, detail_content, view_width, 10.0) {
                        self.panes[index].detail_x = next.clamp(0.0, (detail_content - view_width).max(0.0));
                        ui.ctx().request_repaint();
                    }
                }
                self.panes[index].view = view_cache;
                ui.spacing_mut().item_spacing.y = saved_spacing;
                }
                ui.style_mut().text_styles = saved_text_styles;
                // The list right-button menu (every paint, regardless of whether rows were drawn). The original OnContextPopup(FUN_00484e8c) chooses what to show.
                // The key (Apps, MousePos -1, -1) is for the current pane only: if there is no selection and the cursor item is "..", do not show it, and the position is the bottom-left of the cursor item's cell.
                let mouse_menu = super::skin_pane::take_pane_menu_request(ctx, index);
                let key_menu = index == self.active && std::mem::take(&mut self.pending_context_key);
                let parent_only = self.panes[index].selected.is_empty() && self.panes[index].entries.get(self.panes[index].cursor).is_some_and(|e| e.is_parent());
                // Original FUN_00485078: the left button pressed while the right button is held (after moving the cursor to the pressed row) → rocker_menu_kind.
                let rocker = ui.rect_contains_pointer(list_rect) && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary) && i.pointer.button_down(egui::PointerButton::Secondary));
                let kind = if rocker {
                    self.rocker_menu_kind(index)
                } else if mouse_menu || (key_menu && !parent_only) {
                    self.list_menu_kind(index)
                } else {
                    ListMenu::None
                };
                let mouse_menu = mouse_menu || rocker;
                let open_menu = match kind {
                    ListMenu::Main if !mouse_menu => {
                        let cursor = self.panes[index].cursor;
                        let at = self.name_tip.borrow().as_ref().filter(|tip| tip.0 == index && tip.1 == cursor).map(|tip| tip.2.left_bottom());
                        Some(at.unwrap_or(list_rect.left_top()))
                    }
                    ListMenu::Main => Some(ctx.input(|i| i.pointer.interact_pos()).unwrap_or(list_rect.left_top())),
                    ListMenu::Shell(paths) => {
                        if let Err(error) = ndr::shell_menu::show(&paths) {
                            self.status = error;
                        }
                        None
                    }
                    ListMenu::None => None,
                };
                super::skin_pane::pane_menu(ui, index, open_menu, |ui| {
                    if let Some(chosen) = self.list_context_menu(ui, index) {
                        command = Some(chosen);
                    }
                });
                self.panes[index].drop_drive_marks();
                self.original_name_tip(ctx, index);
                // Reflect the list area into the outer Ui, and place what sits below it (the FTP log box, the file info bar) across the full pane width.
                let used = ui.min_rect();
                let ui = outer;
                ui.advance_cursor_after_rect(egui::Rect::from_min_max(full.min, egui::pos2(full.right(), used.bottom().max(full.top()))));
                self.ftp_log_ui(ui, index, ftp_log_room);
                if let Some(skin) = &skin {
                    self.skin_file_info_bar(ui, index, skin);
                } else {
                    self.address_bar(ui, index, ctx);
                    self.info_bars(ui, index);
                }
                if activate {
                    self.active = index;
                }
                // Original FUN_004856d8: even when dragging more than 8 pixels from a press on empty space, if there are selected items (GetSelCount > 0) it drags those items (skin theme).
                if begin_drag.is_none() && skin.is_some() && self.file_drag.is_none() && !self.panes[index].selected.is_empty() && ctx.dragged_id().is_none() {
                    let (origin, now, primary, secondary) = ui.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos(), i.pointer.primary_down(), i.pointer.secondary_down()));
                    if let (Some(origin), Some(now)) = (origin, now) {
                        let moved = (now - origin).abs();
                        if (primary || secondary) && list_rect.contains(origin) && (moved.x > 8.0 || moved.y > 8.0) {
                            begin_drag = Some((self.panes[index].cursor, secondary && !primary));
                        }
                    }
                }
                if let Some((row, secondary)) = begin_drag {
                    self.begin_file_drag(index, row, secondary, ui.input(|i| i.modifiers));
                }
                self.folder_rows.extend(folder_rows);
                if open {
                    self.open_current();
                }
                if let Some(command) = command {
                    self.dispatch(command, ctx);
                }
            });
        ui.spacing_mut().item_spacing.y = outer_spacing;
        self.pane_rects[index] = shown.response.rect;
    }
}
