//! C# Auto view: ResolveAutoColumnCount (the configured column count, otherwise width divided by name width 180-250 + 14 + icon + minimum widths of visible detail columns).
//! With two or more columns, RebuildAutoColumns (each column top to bottom, a group kept in one column, each column scrolls separately, group headers); detail cells
//! inside a column follow ApplyAutoDetailColumns (name minimum min(190, max(96, 42%)), other cells between min and max by header/content width, hidden when out of room).
//! A single-column Auto view uses the same cell width rule (with a header row) like C#. Multiple columns have no header row (same as C#).
use super::list_view::{DetailColumn, DetailRow};
use super::*;

const MINIMUM: [f32; 6] = [42.0, 52.0, 60.0, 94.0, 48.0, 72.0];
const METRICS: [DetailColumn; 6] = [DetailColumn::Extension, DetailColumn::Kind, DetailColumn::Size, DetailColumn::Modified, DetailColumn::Attributes, DetailColumn::Description];

impl App {
    fn metric_shown(&self, metric: usize) -> bool {
        match metric {
            0 => !self.settings.flag("AttachExtensionToName", false),
            1 => false,
            _ => true,
        }
    }

    /// C# ResolveAutoColumnCount.
    pub(super) fn auto_column_count(&self, ui: &egui::Ui, index: usize, width: f32) -> usize {
        let configured = self.settings.values.get(if index == 0 { "LeftColumnCount" } else { "RightColumnCount" }).and_then(|v| v.as_i64()).unwrap_or(0);
        if (1..=19).contains(&configured) {
            return configured as usize;
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let widest = ui.fonts_mut(|fonts| self.panes[index].entries.iter().map(|e| fonts.layout_no_wrap(e.name.clone(), font.clone(), Color32::WHITE).size().x).fold(0.0f32, f32::max));
        let mut item = widest.clamp(180.0, 250.0) + 14.0 + if self.settings.flag("ShowIcons", true) { 16.0 } else { 0.0 };
        for (metric, minimum) in MINIMUM.iter().enumerate() {
            if self.metric_shown(metric) {
                item += minimum;
            }
        }
        ((width.max(1.0) / item.max(1.0)) as usize).clamp(1, 19)
    }

    /// C# ApplyAutoDetailColumns: (name width, per cell Some(width) or hidden). The skin theme uses the original FitVisibleFields.
    fn auto_widths(&self, ui: &egui::Ui, index: usize, column_width: f32, _headers: &[(DetailColumn, String, f32)]) -> (f32, [Option<f32>; 6]) {
        return self.original_auto_fields(ui, index, column_width);
    }

    /// Width of one item used by original UpdateDisplayAuto (0x7e0688) to count columns: GetListHeaderWidth + GetAdjustedNameWidth
    /// + GetAdjustedExtWidth (when the extension is not joined to the name) + the size cell (when "always show size" +0x390 is on).
    pub(super) fn original_auto_item_width(&self, ui: &egui::Ui, index: usize) -> f32 {
        let fm = self.original_field_max(ui, index);
        let header = if self.settings.flag("ShowIcons", true) { 30.0 } else { 14.0 };
        let attach = self.settings.flag("AttachExtensionToName", false);
        let always = self.side_flag(index, "AlwaysShowFolderSize", false);
        let name = self.original_adjusted_name_width(ui, &fm);
        let ext = if attach { 0.0 } else { self.original_adjusted_ext_width(ui, &fm) };
        let size = if always { fm.max[3] + 10.0 } else { 0.0 };
        header + name + ext + size
    }

    /// Original FitVisibleFields (0x7e04f0): within one column width (`column_width`) place name and extension (+ size when always showing size) first, then add size, date, attributes, description
    /// cells in order (widest text + 10) and stop at the first cell that does not fit. Leftover space widens name/extension to their own width via FitFileNameFields;
    /// if short (the column is narrower than one item), name/extension shrink. Returns (name cell width including the icon head, per METRICS cell a width or hidden).
    pub(super) fn original_auto_fields(&self, ui: &egui::Ui, index: usize, column_width: f32) -> (f32, [Option<f32>; 6]) {
        let fm = self.original_field_max(ui, index);
        let max = fm.max;
        let header = if self.settings.flag("ShowIcons", true) { 30.0 } else { 14.0 };
        let attach = self.settings.flag("AttachExtensionToName", false);
        let always = self.side_flag(index, "AlwaysShowFolderSize", false);
        let sftp = self.panes[index].remote.as_ref().is_some_and(|r| r.is_sftp());
        let size = max[3] + 10.0;
        let name0 = self.original_adjusted_name_width(ui, &fm);
        let ext0 = if attach { 0.0 } else { self.original_adjusted_ext_width(ui, &fm) };
        let mut widths = [None; 6];
        let mut width = header + name0 + ext0;
        if always {
            widths[2] = Some(size);
            width += size;
        }
        let (name, ext) = if column_width < width {
            let mut room = column_width - header;
            if always && column_width < size {
                widths[2] = None;
            } else if always {
                room -= size;
            }
            self.original_fit_name_fields(ui, &fm, room, attach)
        } else {
            // Original OnGetFieldMaxWidth (FUN_00485ae8): the widest text of the attribute cell (kind 5) is "drwxrwxrwx" on an FTP pane, otherwise "AHRS"
            // (description cell 6 uses the item +0x10 text).
            let attr = self.list_text_width(ui, if sftp { "drwxrwxrwx" } else { "AHRS" }) + 10.0;
            // Original field order (ResetFields 0x7db770): NAME, EXT, LOCATION, SIZE, DATEMODIFIED, ATTR, DESC. FitVisibleFields walks them in order
            // without checking the visible flag (+0x21), so the hidden LOCATION (+0x21 = 0, list items have empty location text, max width 0) still takes 10.
            let fields = [(None, 10.0), (Some(2usize), size), (Some(3), max[4] + 10.0), (Some(4), attr), (Some(5), max[6] + 10.0)];
            for (slot, w) in fields.into_iter().filter(|(slot, _)| !(always && *slot == Some(2))) {
                if w < 1.0 || width + w >= column_width {
                    break;
                }
                if let Some(slot) = slot {
                    widths[slot] = Some(w);
                }
                width += w;
            }
            if width < column_width {
                self.original_fit_name_fields(ui, &fm, column_width + name0 + ext0 - width, attach)
            } else {
                (name0, ext0)
            }
        };
        if !attach {
            widths[0] = Some(ext);
        }
        (header + name, widths)
    }

    /// Column count setting (LeftColumnCount/RightColumnCount, original +0x388). 0 is automatic.
    pub(super) fn configured_column_count(&self, index: usize) -> usize {
        self.settings.values.get(if index == 0 { "LeftColumnCount" } else { "RightColumnCount" }).and_then(|v| v.as_i64()).filter(|v| (1..=19).contains(v)).unwrap_or(0) as usize
    }

    /// How original UpdateDisplayNames (0x7e0cac) counts columns: list width / (GetListHeaderWidth + GetAdjustedNameWidth + GetAdjustedExtWidth (when not joined)).
    pub(super) fn original_names_fits(&self, ui: &egui::Ui, index: usize, width: f32) -> usize {
        let fm = self.original_field_max(ui, index);
        let header = if self.settings.flag("ShowIcons", true) { 30.0 } else { 14.0 };
        let ext = if self.settings.flag("AttachExtensionToName", false) { 0.0 } else { self.original_adjusted_ext_width(ui, &fm) };
        (width.max(1.0) / (header + self.original_adjusted_name_width(ui, &fm) + ext).max(1.0)) as usize
    }

    /// Name and extension cells of original UpdateDisplayNames (0x7e0cac) (skin theme Names view). The Names view has only these two cells (+0x22: name, extension when not joined).
    /// - Column count set: one column = width / count. With a joined extension, name = column - head. Otherwise its own width (longest text + 10), the Adjusted width on overflow,
    ///   and if it still overflows, name = column - head - extension.
    /// - Everything fits in one column (rows >= items): fit to the list width as above.
    /// - Several columns needed but fewer than 2 fit: one column. If the list is narrower than one item (head + own width, at least "WWWWAAAA"+"WWA"+20 or
    ///   "WWWWAAAA.WWA"+10), name = width - head - Adjusted extension, otherwise its own width.
    /// - Otherwise (several columns): extension = GetAdjustedExtWidth, name = column - head - extension (fills the column).
    ///
    /// Returns (name cell width including the icon head, per METRICS cell a width or hidden).
    pub(super) fn original_names_fields(&self, ui: &egui::Ui, index: usize, width: f32, column_width: f32, rows: usize) -> (f32, [Option<f32>; 6]) {
        let fm = self.original_field_max(ui, index);
        let header = if self.settings.flag("ShowIcons", true) { 30.0 } else { 14.0 };
        let attach = self.settings.flag("AttachExtensionToName", false);
        let count = self.panes[index].entries.len();
        let natural_name = fm.max[0] + 10.0;
        let natural_ext = if attach { 0.0 } else { fm.max[1] + 10.0 };
        let adjusted_name = self.original_adjusted_name_width(ui, &fm);
        let adjusted_ext = if attach { 0.0 } else { self.original_adjusted_ext_width(ui, &fm) };
        let shrink = |item: f32| -> (f32, f32) {
            let (mut name, mut ext) = (natural_name, natural_ext);
            if item < header + name + ext {
                name = adjusted_name;
                ext = adjusted_ext;
                if item < header + name + ext {
                    name = item - header - ext;
                }
            }
            (name, ext)
        };
        let (name, ext) = if self.configured_column_count(index) > 0 {
            if attach { (column_width - header, 0.0) } else { shrink(column_width) }
        } else if count <= rows {
            shrink(width)
        } else if self.original_names_fits(ui, index, width) < 2 {
            let minimum = if attach {
                self.list_text_width(ui, "WWWWAAAA.WWA") + header + 10.0
            } else {
                self.list_text_width(ui, "WWWWAAAA") + header + self.list_text_width(ui, "WWA") + 20.0
            };
            let item = (header + natural_name + natural_ext).max(minimum);
            if width < item { (width - header - adjusted_ext, natural_ext) } else { (natural_name, natural_ext) }
        } else {
            (column_width - header - adjusted_ext, adjusted_ext)
        };
        let mut widths = [None; 6];
        if !attach {
            widths[0] = Some(ext.max(0.0));
        }
        (header + name.max(0.0), widths)
    }

    /// C# RebuildAutoColumns: row indices per column.
    pub(super) fn auto_distribution(&self, index: usize, columns: usize) -> Vec<Vec<usize>> {
        let pane = &self.panes[index];
        let count = pane.entries.len();
        let per_column = count.div_ceil(columns).max(1);
        let mut result = vec![Vec::new(); columns];
        let mode = self.settings.text(&Self::side_key(index, "GroupMode"), "None");
        if mode == "None" {
            for (row, _) in pane.entries.iter().enumerate() {
                result[(row / per_column).min(columns - 1)].push(row);
            }
            return result;
        }
        let category = self.settings.text(&Self::side_key(index, "FolderCategoryMode"), "Auto");
        let mut column = 0;
        let mut row = 0;
        while row < count {
            let key = ndr::listing::group_key(&pane.entries[row], &mode, &category);
            let end = (row..count).find(|r| ndr::listing::group_key(&pane.entries[*r], &mode, &category) != key).unwrap_or(count);
            if column < columns - 1 && !result[column].is_empty() && result[column].len() + (end - row) > per_column {
                column += 1;
            }
            result[column].extend(row..end);
            row = end;
        }
        result
    }

    /// Multiple columns of the Auto view (C# RebuildAutoColumns: one separately scrolling ListBox per column, group headers per column when grouped). Returns (activate, open, command).
    pub(super) fn auto_rows(&mut self, ui: &mut egui::Ui, index: usize, columns: usize, height: f32) -> (bool, bool, Option<&'static str>) {
        let list_style = self.list_font_style;
        let original_click = true;
        let (mut activate, mut open, command) = (false, false, None);
        if self.panes[index].view.cells.len() != self.panes[index].entries.len() {
            self.compute_view(index);
        }
        let headers = self.detail_columns(self.language == "ko");
        let width = ui.available_width();
        let column_width = (width / columns as f32).floor().max(1.0);
        let (name_width, widths) = self.auto_widths(ui, index, column_width, &headers);
        // Row list per column (with group headers inserted, C# ApplyGroupView + GroupStyle).
        let mode = self.settings.text(&Self::side_key(index, "GroupMode"), "None");
        let category = self.settings.text(&Self::side_key(index, "FolderCategoryMode"), "Auto");
        let show_names = self.settings.flag(&Self::side_key(index, "ShowGroupName"), true);
        let lists: Vec<Vec<DetailRow>> = self
            .auto_distribution(index, columns)
            .into_iter()
            .map(|rows| {
                let mut list = Vec::new();
                let mut previous: Option<String> = None;
                for row in rows {
                    if mode != "None" && show_names {
                        let key = ndr::listing::group_key(&self.panes[index].entries[row], &mode, &category);
                        if previous.as_deref() != Some(key.as_str()) {
                            list.push(DetailRow::Header(self.group_caption(&key)));
                            previous = Some(key);
                        }
                    }
                    list.push(DetailRow::Item(row));
                }
                list
            })
            .collect();
        let active = self.active == index;
        let icons_shown = self.settings.flag("ShowIcons", true);
        // Original skin theme: cursor and mark bars (DrawFocus), cell colors (DrawListItem), mark ▶.
        let original = Some(self.original_list_colors());
        // Like other lists: original item height (text height + 2), line spacing (LineSpace) and row lines (ShowRowLines).
        let row_height = self.list_item_height(ui);
        let saved_spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y += self.list_line_space();
        let (row_lines, line_color) = (self.settings.flag("ShowRowLines", false), self.grid_line_color(ui));
        // C# grouped Auto view (each column scrolls separately): keys page like Details, one page = visible rows.
        let cursor = self.panes[index].cursor;
        self.panes[index].page = list_keys::ListPage { style: list_keys::ListStyle::Details, rows: ((height / (row_height + ui.spacing().item_spacing.y)).floor() as usize).max(1), columns: 1, top: cursor, last: cursor };
        self.panes[index].reveal = false;
        self.panes[index].adjust = None;
        let (by_page, wheel_lines) = (self.scroll_by_page(index), (self.system.wheel_lines)());
        let mut begin_drag = None;
        let mut folder_rows = Vec::new();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (number, list) in lists.iter().enumerate() {
                ui.allocate_ui_with_layout(egui::vec2(column_width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(column_width);
                    let wheel = Self::wheel_scroll(ui, egui::Id::new(("auto", index, number)), height, row_height + ui.spacing().item_spacing.y, by_page, wheel_lines);
                    let mut area = egui::ScrollArea::vertical()
                        .id_salt(("auto", index, number))
                        .scroll_source(super::panes::WHEEL_OFF)
                        .auto_shrink([false, false])
                        .max_height(height)
                        .min_scrolled_height(height);
                    if let Some(offset) = wheel {
                        area = area.vertical_scroll_offset(offset);
                    }
                    area
                        .show_rows(ui, row_height, list.len(), |ui, range| {
                            for line in range {
                                let row = match &list[line] {
                                    DetailRow::Header(label) => {
                                        ui.add_sized([column_width, row_height], egui::Label::new(RichText::new(label).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())).color(ui.visuals().weak_text_color())));
                                        continue;
                                    }
                                    DetailRow::Item(row) => *row,
                                };
                                self.queue_folder_size(index, row);
                                let icon = self.file_icon(ui.ctx(), index, row);
                                let icon = self.row_icon_for_skin(index, row, icon);
                                let bar_slot = original.map(|_| ui.painter().add(egui::Shape::Noop));
                                let mut bar_state = None;
                                let response = ui
                                    .horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        let pane = &self.panes[index];
                                        let entry = &pane.entries[row];
                                        let marked = pane.selected.contains(&row);
                                        let cursor_row = pane.cursor == row;
                                        let texts = &pane.view.cells[row];
                                        let default = if entry.is_dir { Color32::from_rgb(228, 188, 85) } else if entry.hidden { Color32::GRAY } else { ui.visuals().text_color() };
                                        let color = pane.view.colors[row].map_or(default, |(r, g, b)| Color32::from_rgb(r, g, b));
                                        bar_state = Some((color, cursor_row, marked));
                                        let name_color = match original {
                                            Some(o) if cursor_row && active => o.back,
                                            _ => color,
                                        };
                                        let response = App::name_cell(ui, egui::vec2(name_width, row_height), icon.as_ref(), icons_shown, &texts[0], name_color, list_style);
                                        if cursor_row && original.is_some() && !entry.drive {
                                            super::panes::note_cursor_name(&self.name_tip, ui, index, row, response.rect, &texts[0], &entry.name, list_style);
                                        }
                                        for (metric, column) in METRICS.iter().enumerate() {
                                            if let Some(w) = widths[metric] {
                                                let folder_size = (*column == DetailColumn::Size && entry.is_dir).then(|| pane.folder_sizes.get(&entry.path).map(|s| s.clone().unwrap_or_else(|| "…".into()))).flatten();
                                                let text = folder_size.as_ref().unwrap_or(&texts[column.index()]);
                                                let metric_color = original.map_or(ui.visuals().weak_text_color(), |o| o.cell(*column, color, entry.attributes, cursor_row, active));
                                                ui.add_sized([w, row_height], egui::Label::new(super::list_text(text.as_str(), list_style).color(metric_color)).truncate());
                                            }
                                        }
                                        response
                                    })
                                    .inner;
                                if let (Some(slot), Some(o), Some((color, cursor_row, marked))) = (bar_slot, original, bar_state) {
                                    let rect = egui::Rect::from_min_size(response.rect.min, egui::vec2(column_width, row_height));
                                    if cursor_row || marked {
                                        let bar = super::skin_pane::focus_bar(color, o.back, o.select, cursor_row, marked, active);
                                        ui.painter().set(slot, egui::Shape::Vec(super::skin_pane::focus_bar_shapes(rect, &bar)));
                                    }
                                    if marked {
                                        ui.painter().extend(super::skin_pane::selection_mark_shapes(rect, o.select));
                                    }
                                }
                                if row_lines {
                                    let y = Self::row_line_y(ui.painter(), response.rect.bottom(), ui.spacing().item_spacing.y);
                                    ui.painter().hline(response.rect.left()..=response.rect.left() + column_width, y, egui::Stroke::new(1.0, line_color));
                                }
                                // Clicking anywhere on the row picks that item (the whole row, not just the name cell; original GetItemAt = the whole item cell).
                                let row_rect = egui::Rect::from_min_size(response.rect.min, egui::vec2(column_width, row_height));
                                let response = ui.interact(row_rect, ui.id().with(("ndr-auto-row", index, row)), egui::Sense::click_and_drag());
                                // Original MouseDown: the skin theme acts on press (the left 12 px of the item cell is the mark cell, Shift drags a range).
                                let in_area = original_click && super::panes::original_press_input(&mut self.panes[index], row, response.rect, ui);
                                let area_hit = original_click && super::panes::pointer_in_selection_area(response.rect, ui);
                                if in_area {
                                    activate = true;
                                }
                                if super::panes::drag_begins(&response, egui::PointerButton::Primary, original_click) && (original_click || self.panes[index].range_drag.is_none()) {
                                    begin_drag = Some((row, false));
                                } else if super::panes::drag_begins(&response, egui::PointerButton::Secondary, original_click) {
                                    begin_drag = Some((row, true));
                                }
                                if self.panes[index].entries[row].is_dir {
                                    folder_rows.push((index, response.rect.intersect(ui.clip_rect()), self.panes[index].entries[row].path.clone()));
                                }
                                if response.clicked() || response.secondary_clicked() {
                                    activate = true;
                                    if !original_click || response.secondary_clicked() {
                                        let modifiers = ui.input(|i| i.modifiers);
                                        let pane = &mut self.panes[index];
                                        super::panes::row_pressed(pane, row, modifiers, response.secondary_clicked(), original_click);
                                    }
                                }
                                if super::panes::windows_double_clicked(&response) && !area_hit {
                                    self.panes[index].cursor = row;
                                    activate = true;
                                    open = true;
                                }
                                // No menu is attached to the row response (pane menu is skin_pane::pane_menu).
                                if response.secondary_clicked() {
                                    super::skin_pane::request_pane_menu(ui.ctx(), index);
                                }
                            }
                        });
                });
            }
        });
        ui.spacing_mut().item_spacing.y = saved_spacing;
        if let Some((row, secondary)) = begin_drag {
            self.begin_file_drag(index, row, secondary, ui.input(|i| i.modifiers));
        }
        self.folder_rows.extend(folder_rows);
        (activate, open, command)
    }
}

impl App {
    /// Height taken by the horizontal bar.
    const FLOW_BAR: f32 = 14.0;

    /// Column layout of original UpdateDisplayAuto (Auto) and UpdateDisplayNames (Names): (rows per column, visible columns, horizontal bar).
    /// One column width is the C# ResolveAutoColumnCount name and cell width for Auto, and the longest name + 14 (+ icon 16) for Names.
    pub(super) fn flow_shape(&self, ui: &egui::Ui, index: usize, width: f32, height: f32, auto: bool) -> (usize, usize, bool) {
        let configured = self.configured_column_count(index);
        let fits = if configured > 0 {
            configured
        } else if auto {
            // Original UpdateDisplayAuto: columns that fit = list width / one item width (original cell width rule, 0 becomes 1, no upper limit).
            ((width.max(1.0) / self.original_auto_item_width(ui, index).max(1.0)) as usize).max(1)
        } else {
            // Original UpdateDisplayNames: list width / (head + Adjusted name + Adjusted extension).
            self.original_names_fits(ui, index, width).max(1)
        };
        // Original UpdateDisplayNames: rows per column = (height + LineSpace) / (item height + LineSpace).
        let space = self.list_line_space();
        let item = self.list_item_height(ui);
        let rows_full = (((height + space) / (item + space)).floor() as usize).max(1);
        // On overflow subtract the horizontal bar. The skin theme uses the original TNXScrollBar thickness (scrollbg width, left by flow_rows), otherwise the egui bar.
        let bar = ui.data(|d| d.get_temp::<f32>(egui::Id::new(("ndr-skin-hbar-width", index)))).unwrap_or(Self::FLOW_BAR);
        let rows_with_bar = (((height - bar + space) / (item + space)).floor() as usize).max(1);
        list_keys::flow_layout(self.panes[index].entries.len(), fits, configured, rows_full, rows_with_bar, auto)
    }

    /// Draws original list styles 0 (Auto) and 1 (Names): item index / rows per column = column, remainder = row (GetItemRect), filling one column top to bottom
    /// and flowing into the next. Overflow scrolls horizontally (UpdateScrollBar: by line one step = one column, by page one step = the visible column count).
    /// The wheel moves the horizontal bar one step like the original WMMouseWheel. Moving the cursor by key scrolls its column into view like SetItemIndex.
    /// In Auto, cells that fit in a column (extension, type, size, date, attributes, description) are drawn too, like C# ApplyAutoDetailColumns. Returns (activate, open, command).
    pub(super) fn flow_rows(&mut self, ui: &mut egui::Ui, index: usize, height: f32, auto: bool) -> (bool, bool, Option<&'static str>) {
        let list_style = self.list_font_style;
        let original_click = true;
        let (mut activate, mut open, command) = (false, false, None);
        if self.panes[index].view.cells.len() != self.panes[index].entries.len() {
            self.compute_view(index);
        }
        let width = ui.available_width();
        // Original skin horizontal bar (horizontal TNXScrollBar, styles 0/1): thickness is the scrollbg image width; flow_shape removes that many rows.
        let skin = self.current_skin();
        let ctx = ui.ctx().clone();
        let skin_bar = match &skin {
            Some(s) => self.skin_scrollbar_width(&ctx, s),
            None => None,
        };
        let bar_width_id = egui::Id::new(("ndr-skin-hbar-width", index));
        match skin_bar {
            Some(w) => ui.data_mut(|d| d.insert_temp(bar_width_id, w)),
            None => ui.data_mut(|d| d.remove::<f32>(bar_width_id)),
        }
        let (rows, columns, _) = self.flow_shape(ui, index, width, height, auto);
        let count = self.panes[index].entries.len();
        // Original UpdateDisplayAuto/Names: 0x344 = list width / visible columns (integer division).
        let column_width = (width / columns as f32).floor().max(1.0);
        // Original skin theme: cursor and mark bars (DrawFocus), cell colors (DrawListItem), mark ▶.
        let original = Some(self.original_list_colors());
        let total_columns = count.div_ceil(rows).max(1);
        let by_page = self.scroll_by_page(index);
        // Original UpdateScrollBar (styles 0/1): by line the position is 0..columns - visible columns; by page the position is the page index (0..pages - 1), so
        // the last page also starts at a page's first column (the right side may be empty). Fixed: previously paging also clamped to columns - visible columns,
        // so some sizes could not reach the last page.
        let last_first = list_keys::flow_last_first(total_columns, columns, by_page);
        let snap = |first: usize| if by_page { (first / columns) * columns } else { first };
        let mut first = snap(self.panes[index].first_column.min(last_first));
        // Original WMMouseWheel (styles 0/1): horizontal bar position +/- SmallChange (by line = one column, by page = one page). The skin theme, like the original, takes only
        // the vertical wheel (including Shift + wheel) and ignores the tilt wheel (WMMouseHWheel).
        let area_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, height));
        let (raw, zoom) = ui.input(|i| (if original_click { Self::vertical_wheel(i) } else { i.raw_scroll_delta.y + i.raw_scroll_delta.x }, i.modifiers.ctrl || i.modifiers.command));
        if raw != 0.0 && !zoom && ui.rect_contains_pointer(area_rect) {
            let notches = (raw / ui.ctx().options(|o| o.input_options.line_scroll_speed).max(1.0)).round() as i64;
            let notches = if notches == 0 { raw.signum() as i64 } else { notches };
            let step = if by_page { columns } else { 1 } as i64;
            first = snap((first as i64 - notches * step).clamp(0, last_first as i64) as usize);
        }
        // Original TNXDropTarget.DoTargetScroll (0x7b1e04): while dragging near the left or right edge, WM_HSCROLL one column (a page in page mode).
        let dx = std::mem::take(&mut self.panes[index].drag_scroll.1);
        if dx != 0 {
            let step = if by_page { columns } else { 1 } as i64;
            first = snap((first as i64 + dx as i64 * step).clamp(0, last_first as i64) as usize);
        }
        self.panes[index].drag_scroll.0 = 0;
        // Original AdjustPage (0x7daf24, styles 0/1): by page, that item's page; otherwise start at that column when it is left of view, else keep that column as
        // the last visible column (never below 0). SetItemIndex (reveal) then makes the cursor visible.
        if let Some((item, reload)) = self.panes[index].adjust.take() {
            let column = item.min(count.saturating_sub(1)) / rows;
            first = if by_page {
                (column / columns) * columns
            } else if column < first {
                column
            } else {
                (column + 1).saturating_sub(columns)
            };
            first = first.min(last_first);
            // Reread list: original FUN_00487ce8 then calls SetItemIndex on the cursor item.
            if reload {
                self.panes[index].reveal = true;
            }
        }
        // Original SetItemIndex (styles 0/1): if the cursor column is left of view start there; if right of view make it the last column (by page, that page's first column).
        if std::mem::take(&mut self.panes[index].reveal) {
            let column = self.panes[index].cursor.min(count.saturating_sub(1)) / rows;
            if by_page {
                first = (column / columns) * columns;
            } else if column < first {
                first = column;
            } else if column >= first + columns {
                first = column + 1 - columns;
            }
            first = first.min(last_first);
        }
        let headers = self.detail_columns(self.language == "ko");
        let (name_width, widths) = if auto {
            self.auto_widths(ui, index, column_width, &headers)
        } else if original.is_some() {
            self.original_names_fields(ui, index, width, column_width, rows)
        } else {
            (column_width, [None; 6])
        };
        let drive_extent = self.drive_text_extent(ui, index);
        let free_label = self.language_pack.text("RES.DISKFREE", "Free");
        let icons_shown = self.settings.flag("ShowIcons", true);
        let active = self.active == index;
        let mut begin_drag = None;
        let mut folder_rows = Vec::new();
        // Row height of a column-flowing list (same original item height as Details).
        let item_height = self.list_item_height(ui);
        let pitch = item_height + self.list_line_space();
        let (column_lines, row_lines, line_color) = (self.settings.flag("ShowColumnLines", true), self.settings.flag("ShowRowLines", false), self.grid_line_color(ui));
        let mut area = egui::ScrollArea::horizontal()
            .id_salt(("flow", index))
            .scroll_source(super::panes::WHEEL_OFF)
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_width(width)
            .horizontal_scroll_offset(first as f32 * column_width);
        if skin_bar.is_some() {
            area = area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        }
        let output = area
            .show_viewport(ui, |ui, viewport| {
                // Content width lets the horizontal position reach the last start column (last_first) (egui scrolls only to content - visible width).
                let content_width = (total_columns as f32 * column_width).max(last_first as f32 * column_width + width);
                ui.set_min_size(egui::vec2(content_width, rows as f32 * pitch));
                let origin = ui.min_rect().min;
                // Do not draw beyond the visible columns (`columns`). Fixed: when the list width was not divisible by the column count, the next column
                // peeked into the leftover pixels.
                let visible = egui::Rect::from_min_max(origin + egui::vec2(viewport.min.x, viewport.min.y), origin + egui::vec2(viewport.min.x + columns as f32 * column_width, viewport.max.y));
                ui.set_clip_rect(ui.clip_rect().intersect(visible));
                let shown_first = ((viewport.min.x / column_width).floor().max(0.0)) as usize;
                let shown_last = ((viewport.max.x / column_width).ceil() as usize).min(total_columns);
                // Original Paint (+0x386, not style 3): NXLINE vertical lines between visible columns (i x item width - 1) from top + 4 to bottom - 4.
                if column_lines {
                    let stroke = egui::Stroke::new(1.0, line_color);
                    for column in shown_first.max(1)..shown_last {
                        let x = ui.painter().round_to_pixel_center(origin.x + column as f32 * column_width - 0.5);
                        ui.painter().vline(x, (origin.y + viewport.min.y + 4.0)..=(origin.y + viewport.min.y + height - 4.0), stroke);
                    }
                }
                for column in shown_first..shown_last {
                    for line in 0..rows {
                        let row = column * rows + line;
                        if row >= count {
                            break;
                        }
                        let cell = egui::Rect::from_min_size(origin + egui::vec2(column as f32 * column_width, line as f32 * pitch), egui::vec2(column_width, item_height));
                        // Original DrawListItem (+0x387): horizontal line under the item at bottom + LineSpace / 2 - 1.
                        if row_lines {
                            let y = Self::row_line_y(ui.painter(), cell.bottom(), pitch - item_height);
                            ui.painter().hline(cell.left()..=cell.right(), y, egui::Stroke::new(1.0, line_color));
                        }
                        self.queue_folder_size(index, row);
                        let icon = self.file_icon(ui.ctx(), index, row);
                        let icon = self.row_icon_for_skin(index, row, icon);
                        let pane = &self.panes[index];
                        let entry = &pane.entries[row];
                        let marked = pane.selected.contains(&row);
                        let cursor_row = pane.cursor == row;
                        let texts = &pane.view.cells[row];
                        let default = if entry.is_dir { Color32::from_rgb(228, 188, 85) } else if entry.hidden { Color32::GRAY } else { ui.visuals().text_color() };
                        let color = pane.view.colors[row].map_or(default, |(r, g, b)| Color32::from_rgb(r, g, b));
                        let label = if auto || entry.drive || original.is_some() { texts[0].clone() } else { entry.name.clone() };
                        if let Some(o) = original.filter(|_| cursor_row || marked) {
                            let bar = super::skin_pane::focus_bar(color, o.back, o.select, cursor_row, marked, active);
                            ui.painter().extend(super::skin_pane::focus_bar_shapes(cell, &bar));
                        }
                        let name_color = match original {
                            Some(o) if cursor_row && active => o.back,
                            _ => color,
                        };
                        let framed = false;
                        // Original DrawListItem kind 4: a drive item spans the cell with its name and a free space bar.
                        let name_rect = egui::Rect::from_min_size(cell.min, egui::vec2(if entry.drive { column_width } else { name_width.min(column_width) }, item_height));
                        let response = if entry.drive {
                            let info = pane.drive_infos.get(&entry.path).map(|(_, kind, sizes)| (*kind, *sizes));
                            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(name_rect));
                            super::list_view::drive_row(&mut child, name_rect.size(), icon.as_ref(), &label, name_color, framed, pane.cursor == row, drive_extent, info, &free_label, list_style)
                        } else {
                            if cursor_row && original.is_some() {
                                super::panes::note_cursor_name(&self.name_tip, ui, index, row, name_rect, &label, &entry.name, list_style);
                            }
                            App::name_cell(&mut ui.new_child(egui::UiBuilder::new().max_rect(name_rect)), name_rect.size(), icon.as_ref(), icons_shown, &label, name_color, list_style)
                        };
                        let mut x = name_rect.right();
                        for (metric, column) in METRICS.iter().enumerate().filter(|_| !entry.drive) {
                            if let Some(w) = widths[metric] {
                                let folder_size = (*column == DetailColumn::Size && entry.is_dir).then(|| pane.folder_sizes.get(&entry.path).map(|s| s.clone().unwrap_or_else(|| "…".into()))).flatten();
                                let text = folder_size.as_ref().unwrap_or(&texts[column.index()]);
                                let metric_color = original.map_or(ui.visuals().weak_text_color(), |o| o.cell(*column, color, entry.attributes, cursor_row, active));
                                let metric_rect = egui::Rect::from_min_size(egui::pos2(x, cell.top()), egui::vec2(w, item_height));
                                match &original {
                                    // Original DrawListItem cell alignment (size right, date/time at both ends, the rest left).
                                    Some(o) => super::panes::paint_original_cell(ui, metric_rect, *column, text, metric_color, cursor_row, o, list_style),
                                    None => {
                                        ui.put(metric_rect, egui::Label::new(super::list_text(text.as_str(), list_style).color(metric_color)).truncate());
                                    }
                                }
                                x += w;
                            }
                        }
                        if let Some(o) = original.filter(|_| marked) {
                            ui.painter().extend(super::skin_pane::selection_mark_shapes(cell, o.select));
                        }
                        // Clicking anywhere on the row picks the item. Original GetItemAt (styles 0/1): row = y / (height + LineSpace), so the gap under an item belongs to it, and
                        // the gap below a column's last row (beyond (height + LineSpace) x rows - LineSpace) is no item.
                        let _ = response;
                        let hit = if line + 1 < rows { egui::Rect::from_min_size(cell.min, egui::vec2(column_width, pitch)) } else { cell };
                        let response = ui.interact(hit, ui.id().with(("ndr-flow-row", index, row)), egui::Sense::click_and_drag());
                        // Original MouseDown: the skin theme acts on press (the left 12 px of the item cell is the mark cell, Shift drags a range).
                        let in_area = original_click && super::panes::original_press_input(&mut self.panes[index], row, response.rect, ui);
                        let area_hit = original_click && super::panes::pointer_in_selection_area(response.rect, ui);
                        if in_area {
                            activate = true;
                        }
                        if super::panes::drag_begins(&response, egui::PointerButton::Primary, original_click) && (original_click || self.panes[index].range_drag.is_none()) {
                            begin_drag = Some((row, false));
                        } else if super::panes::drag_begins(&response, egui::PointerButton::Secondary, original_click) {
                            begin_drag = Some((row, true));
                        }
                        if self.panes[index].entries[row].is_dir {
                            folder_rows.push((index, cell.intersect(ui.clip_rect()), self.panes[index].entries[row].path.clone()));
                        }
                        if response.clicked() || response.secondary_clicked() {
                            activate = true;
                            if !original_click || response.secondary_clicked() {
                                let modifiers = ui.input(|i| i.modifiers);
                                let pane = &mut self.panes[index];
                                super::panes::row_pressed(pane, row, modifiers, response.secondary_clicked(), original_click);
                            }
                        }
                        if super::panes::windows_double_clicked(&response) && !area_hit {
                            self.panes[index].cursor = row;
                            activate = true;
                            open = true;
                        }
                        // No menu is attached to the row response (pane menu is skin_pane::pane_menu).
                        if response.secondary_clicked() {
                            super::skin_pane::request_pane_menu(ui.ctx(), index);
                        }
                    }
                }
            });
        // If the horizontal bar was dragged, go to that position (the page start when paging).
        let dragged = output.state.offset.x;
        if (dragged - first as f32 * column_width).abs() > 0.5 {
            first = snap(((dragged / column_width).round() as usize).min(last_first));
        }
        // Skin theme: original horizontal TNXScrollBar (only on overflow, at the bottom of the list). One button press = one column (visible columns when paging), track = one page.
        if let (Some(bar_height), Some(skin)) = (skin_bar, &skin) {
            if output.content_size.x > output.inner_rect.width() + 0.5 {
                let inner = output.inner_rect;
                let bar = egui::Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - bar_height), inner.max);
                let line = if by_page { columns as f32 * column_width } else { column_width };
                let page = columns as f32 * column_width;
                if let Some(next) = self.skin_horizontal_scrollbar(ui, &ctx, skin, egui::Id::new(("ndr-skin-hbar", index)), bar, first as f32 * column_width, output.content_size.x, page, line) {
                    first = snap(((next / column_width).round() as usize).min(last_first));
                    ui.ctx().request_repaint();
                }
            }
        }
        let pane = &mut self.panes[index];
        pane.first_column = first;
        pane.page = list_keys::ListPage { style: list_keys::ListStyle::Flow, rows, columns, top: first * rows, last: ((first + columns) * rows).min(count).saturating_sub(1) };
        if let Some((row, secondary)) = begin_drag {
            self.begin_file_drag(index, row, secondary, ui.input(|i| i.modifiers));
        }
        self.folder_rows.extend(folder_rows);
        (activate, open, command)
    }
}
