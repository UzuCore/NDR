//! C# 자동(Auto) 보기: ResolveAutoColumnCount(설정한 열 수, 아니면 이름 너비 180~250 + 14 + 아이콘 + 보이는 자세히 열의 최소 너비로 나눈 수),
//! 열이 둘 이상이면 RebuildAutoColumns(열마다 위에서 아래로, 묶음은 한 열에 모아서, 열마다 따로 스크롤, 묶음 머리글), 열 안의 자세히 칸은
//! ApplyAutoDetailColumns(이름 최소 min(190, max(96, 42%)), 나머지 칸은 머리글·내용 너비로 최소~최대, 자리가 모자라면 숨김).
//! 한 열 자동 보기도 C#처럼 같은 칸 너비 규칙(머리글 줄 있음)을 쓴다. 여러 열에는 머리글 줄이 없다(C#과 같음).
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

    /// C# ApplyAutoDetailColumns: (이름 너비, 칸마다 Some(너비) 또는 숨김). 스킨 테마는 원본 FitVisibleFields.
    fn auto_widths(&self, ui: &egui::Ui, index: usize, column_width: f32, _headers: &[(DetailColumn, String, f32)]) -> (f32, [Option<f32>; 6]) {
        return self.original_auto_fields(ui, index, column_width);
    }

    /// 원본 UpdateDisplayAuto(0x7e0688)가 열 수를 셀 때의 한 항목 너비: GetListHeaderWidth + GetAdjustedNameWidth
    /// + GetAdjustedExtWidth(확장자를 이름에 붙이지 않으면) + 크기 칸(늘 크기 보기 +0x390이면).
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

    /// 원본 FitVisibleFields(0x7e04f0): 한 열 너비(`column_width`) 안에 이름·확장자(+ 늘 크기 보기면 크기)부터 두고, 들어가면 크기·날짜·속성·설명
    /// 차례로 칸(가장 넓은 글 + 10)을 넣다가 들어가지 않는 칸에서 멈춘다. 남으면 FitFileNameFields로 이름·확장자를 제 너비로 넓히고,
    /// 모자라면(열이 한 항목보다 좁음) 이름·확장자를 줄인다. (이름 칸 너비(아이콘 머리 포함), METRICS 칸마다 너비 또는 숨김).
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
            let attr = self.list_text_width(ui, if sftp { "drwxrwxrwx" } else { "ARHS" }) + 10.0;
            // 원본 필드 차례(ResetFields 0x7db770): NAME, EXT, LOCATION, SIZE, DATEMODIFIED, ATTR, DESC. FitVisibleFields는 차례대로
            // 돌며 보이기 표시(+0x21)를 보지 않으므로 숨은 LOCATION(+0x21 = 0, 목록 항목의 위치 글은 비어 최대 너비 0)도 10을 쓴다.
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

    /// 열 수 설정(LeftColumnCount/RightColumnCount, 원본 +0x388). 0은 자동.
    pub(super) fn configured_column_count(&self, index: usize) -> usize {
        self.settings.values.get(if index == 0 { "LeftColumnCount" } else { "RightColumnCount" }).and_then(|v| v.as_i64()).filter(|v| (1..=19).contains(v)).unwrap_or(0) as usize
    }

    /// 원본 UpdateDisplayNames(0x7e0cac)가 열 수를 셀 때: 목록 너비 / (GetListHeaderWidth + GetAdjustedNameWidth + GetAdjustedExtWidth(붙이지 않으면)).
    pub(super) fn original_names_fits(&self, ui: &egui::Ui, index: usize, width: f32) -> usize {
        let fm = self.original_field_max(ui, index);
        let header = if self.settings.flag("ShowIcons", true) { 30.0 } else { 14.0 };
        let ext = if self.settings.flag("AttachExtensionToName", false) { 0.0 } else { self.original_adjusted_ext_width(ui, &fm) };
        (width.max(1.0) / (header + self.original_adjusted_name_width(ui, &fm) + ext).max(1.0)) as usize
    }

    /// 원본 UpdateDisplayNames(0x7e0cac)의 이름·확장자 칸(스킨 테마 이름 보기). 이름 보기에는 이 두 칸만 있다(+0x22: 이름, 붙이지 않으면 확장자).
    /// - 열 수 설정: 한 열 = 너비 / 열 수. 확장자를 붙이면 이름 = 한 열 − 머리. 아니면 제 너비(가장 긴 글 + 10), 넘치면 Adjusted 너비,
    ///   그래도 넘치면 이름 = 한 열 − 머리 − 확장자.
    /// - 한 열에 다 들어가면(줄 수 ≥ 항목 수): 위와 같이 목록 너비에 맞춘다.
    /// - 여러 열이 필요하고 들어가는 열이 2 미만: 한 열. 목록 너비가 한 항목(머리 + 제 너비, "WWWWAAAA"+"WWA"+20 또는
    ///   "WWWWAAAA.WWA"+10보다 좁지 않게)보다 좁으면 이름 = 너비 − 머리 − Adjusted 확장자, 아니면 제 너비.
    /// - 그 밖(여러 열): 확장자 = GetAdjustedExtWidth, 이름 = 한 열 − 머리 − 확장자(열을 채운다).
    ///
    /// (이름 칸 너비(아이콘 머리 포함), METRICS 칸마다 너비 또는 숨김).
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

    /// C# RebuildAutoColumns: 열마다 줄 번호들.
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

    /// 자동 보기의 여러 열(C# RebuildAutoColumns: 열마다 따로 스크롤하는 ListBox, 묶음이면 열마다 묶음 머리글). (활성화, 열기, 명령).
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
        // 열마다 줄 목록(묶음 머리글을 끼운다, C# ApplyGroupView + GroupStyle).
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
        // 원본 스킨 테마: 커서·표시 막대(DrawFocus), 칸 색(DrawListItem), 표시 ▶.
        let original = Some(self.original_list_colors());
        // 다른 목록처럼 원본 항목 높이(글 높이 + 2)와 줄 간격(LineSpace), 줄 선(ShowRowLines).
        let row_height = self.list_item_height(ui);
        let saved_spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y += self.list_line_space();
        let (row_lines, line_color) = (self.settings.flag("ShowRowLines", false), self.grid_line_color(ui));
        // C# 묶음 자동 보기(열마다 따로 스크롤): 글쇠는 자세히처럼 한 쪽 = 보이는 줄 수.
        let cursor = self.panes[index].cursor;
        self.panes[index].page = list_keys::ListPage { style: list_keys::ListStyle::Details, rows: ((height / (row_height + ui.spacing().item_spacing.y)).floor() as usize).max(1), columns: 1, top: cursor, last: cursor };
        self.panes[index].reveal = false;
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
                                    let y = response.rect.bottom() + (ui.spacing().item_spacing.y / 2.0).floor() - 0.5;
                                    ui.painter().hline(response.rect.left()..=response.rect.left() + column_width, y, egui::Stroke::new(1.0, line_color));
                                }
                                // 줄 어디를 눌러도 그 항목(이름 칸만이 아니라 줄 전체, 원본 GetItemAt = 항목 칸 전체).
                                let row_rect = egui::Rect::from_min_size(response.rect.min, egui::vec2(column_width, row_height));
                                let response = ui.interact(row_rect, ui.id().with(("ndr-auto-row", index, row)), egui::Sense::click_and_drag());
                                // 원본 MouseDown: 스킨 테마는 누르는 순간 처리(항목 칸 왼쪽 12화소는 표시 칸, Shift는 범위 끌기).
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
                                // 줄 응답에 메뉴를 붙이지 않는다(패널 메뉴 skin_pane::pane_menu).
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
    /// 가로 막대가 차지하는 높이.
    const FLOW_BAR: f32 = 14.0;

    /// 원본 UpdateDisplayAuto(자동)·UpdateDisplayNames(이름)의 열 배치: (한 열의 줄 수, 보이는 열 수, 가로 막대).
    /// 한 열 너비는 자동이면 C# ResolveAutoColumnCount의 이름·칸 너비, 이름이면 가장 긴 이름 + 14(+ 아이콘 16)로 센다.
    pub(super) fn flow_shape(&self, ui: &egui::Ui, index: usize, width: f32, height: f32, auto: bool) -> (usize, usize, bool) {
        let configured = self.configured_column_count(index);
        let fits = if configured > 0 {
            configured
        } else if auto {
            // 원본 UpdateDisplayAuto: 들어가는 열 수 = 목록 너비 / 한 항목 너비(원본 칸 너비 규칙, 0이면 1, 위 한도 없음).
            ((width.max(1.0) / self.original_auto_item_width(ui, index).max(1.0)) as usize).max(1)
        } else {
            // 원본 UpdateDisplayNames: 목록 너비 / (머리 + Adjusted 이름 + Adjusted 확장자).
            self.original_names_fits(ui, index, width).max(1)
        };
        // 원본 UpdateDisplayNames: 한 열의 줄 수 = (높이 + LineSpace) / (항목 높이 + LineSpace).
        let space = self.list_line_space();
        let item = self.list_item_height(ui);
        let rows_full = (((height + space) / (item + space)).floor() as usize).max(1);
        // 넘치면 가로 막대 자리를 뺀다. 스킨 테마는 원본 TNXScrollBar 두께(scrollbg 너비, flow_rows가 남김), 아니면 egui 막대.
        let bar = ui.data(|d| d.get_temp::<f32>(egui::Id::new(("ndr-skin-hbar-width", index)))).unwrap_or(Self::FLOW_BAR);
        let rows_with_bar = (((height - bar + space) / (item + space)).floor() as usize).max(1);
        list_keys::flow_layout(self.panes[index].entries.len(), fits, configured, rows_full, rows_with_bar, auto)
    }

    /// 원본 목록 형식 0(자동)·1(이름) 그리기: 항목 번호 / 한 열의 줄 수 = 열, 나머지 = 줄(GetItemRect)로 위에서 아래로 한 열을 채우고
    /// 다음 열로 흐른다. 넘치면 가로로 넘긴다(UpdateScrollBar: 줄 단위면 한 칸 = 한 열, 쪽 단위면 한 칸 = 보이는 열 수만큼).
    /// 바퀴도 원본 WMMouseWheel처럼 가로 막대를 한 칸씩 옮긴다. 커서를 글쇠로 옮기면 SetItemIndex처럼 그 열이 보이게 넘긴다.
    /// 자동이면 열 안에 C# ApplyAutoDetailColumns처럼 들어가는 칸(확장자·종류·크기·날짜·속성·설명)을 함께 그린다. (활성화, 열기, 명령).
    pub(super) fn flow_rows(&mut self, ui: &mut egui::Ui, index: usize, height: f32, auto: bool) -> (bool, bool, Option<&'static str>) {
        let list_style = self.list_font_style;
        let original_click = true;
        let (mut activate, mut open, command) = (false, false, None);
        if self.panes[index].view.cells.len() != self.panes[index].entries.len() {
            self.compute_view(index);
        }
        let width = ui.available_width();
        // 원본 스킨 가로 막대(TNXScrollBar 가로, 형식 0·1): 두께는 scrollbg 그림 너비. flow_shape가 그만큼 줄 수를 줄인다.
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
        // 원본 UpdateDisplayAuto/Names: 0x344 = 목록 너비 / 보이는 열 수(정수 나눗셈).
        let column_width = (width / columns as f32).floor().max(1.0);
        // 원본 스킨 테마: 커서·표시 막대(DrawFocus), 칸 색(DrawListItem), 표시 ▶.
        let original = Some(self.original_list_colors());
        let total_columns = count.div_ceil(rows).max(1);
        let by_page = self.scroll_by_page(index);
        // 원본 UpdateScrollBar(형식 0·1): 줄 단위면 위치 0..열 수 − 보이는 열 수, 쪽 단위면 위치 = 쪽 번호(0..쪽 수 − 1)라
        // 마지막 쪽도 쪽 처음 열에서 시작한다(오른쪽이 빌 수 있다). 고친 것: 전에는 쪽 단위에서도 열 수 − 보이는 열 수로 잘라
        // 마지막 쪽으로 넘어가지 못하는 크기가 있었다.
        let last_first = list_keys::flow_last_first(total_columns, columns, by_page);
        let snap = |first: usize| if by_page { (first / columns) * columns } else { first };
        let mut first = snap(self.panes[index].first_column.min(last_first));
        // 원본 WMMouseWheel(형식 0·1): 가로 막대 위치 ± SmallChange(줄 단위 = 한 열, 쪽 단위 = 한 쪽). 스킨 테마는 원본처럼 세로 바퀴만
        // (Shift + 바퀴 포함) 받고 기울이는 바퀴(WMMouseHWheel)는 무시한다.
        let area_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, height));
        let (raw, zoom) = ui.input(|i| (if original_click { Self::vertical_wheel(i) } else { i.raw_scroll_delta.y + i.raw_scroll_delta.x }, i.modifiers.ctrl || i.modifiers.command));
        if raw != 0.0 && !zoom && ui.rect_contains_pointer(area_rect) {
            let notches = (raw / ui.ctx().options(|o| o.input_options.line_scroll_speed).max(1.0)).round() as i64;
            let notches = if notches == 0 { raw.signum() as i64 } else { notches };
            let step = if by_page { columns } else { 1 } as i64;
            first = snap((first as i64 - notches * step).clamp(0, last_first as i64) as usize);
        }
        // 원본 SetItemIndex(형식 0·1): 커서 열이 왼쪽 밖이면 그 열부터, 오른쪽 밖이면 그 열이 마지막 열이 되게(쪽 단위는 그 쪽의 처음 열).
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
        // 열로 흐르는 목록의 줄 높이(자세히와 같은 원본 항목 높이).
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
                // 가로 위치가 마지막 시작 열(last_first)까지 닿도록 내용 너비를 잡는다(egui는 내용 − 보이는 너비까지만 넘긴다).
                let content_width = (total_columns as f32 * column_width).max(last_first as f32 * column_width + width);
                ui.set_min_size(egui::vec2(content_width, rows as f32 * pitch));
                let origin = ui.min_rect().min;
                // 보이는 열(columns개) 밖은 그리지 않는다. 고친 것: 목록 너비가 열 수로 나누어떨어지지 않으면 남은 몇 화소에
                // 다음 열이 살짝 보였다.
                let visible = egui::Rect::from_min_max(origin + egui::vec2(viewport.min.x, viewport.min.y), origin + egui::vec2(viewport.min.x + columns as f32 * column_width, viewport.max.y));
                ui.set_clip_rect(ui.clip_rect().intersect(visible));
                let shown_first = ((viewport.min.x / column_width).floor().max(0.0)) as usize;
                let shown_last = ((viewport.max.x / column_width).ceil() as usize).min(total_columns);
                // 원본 Paint(+0x386, 형식 3이 아니면): 보이는 열 사이(i × 항목 너비 − 1)에 위 4부터 아래 − 4까지 NXLINE 세로선.
                if column_lines {
                    let stroke = egui::Stroke::new(1.0, line_color);
                    for column in shown_first.max(1)..shown_last {
                        let x = origin.x + column as f32 * column_width - 0.5;
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
                        // 원본 DrawListItem(+0x387): 항목 아래 bottom + LineSpace / 2 − 1에 가로선.
                        if row_lines {
                            let y = cell.bottom() + ((pitch - item_height) / 2.0).floor() - 0.5;
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
                        // 원본 DrawListItem 종류 4: 드라이브 항목은 칸 너비 전체에 이름과 남은 공간 막대.
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
                                    // 원본 DrawListItem 칸 맞춤(크기 오른쪽, 날짜·시각 양끝, 나머지 왼쪽).
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
                        // 줄 어디를 눌러도 그 항목(원본 GetItemAt = 항목 칸 전체).
                        let _ = response;
                        let response = ui.interact(cell, ui.id().with(("ndr-flow-row", index, row)), egui::Sense::click_and_drag());
                        // 원본 MouseDown: 스킨 테마는 누르는 순간 처리(항목 칸 왼쪽 12화소는 표시 칸, Shift는 범위 끌기).
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
                        // 줄 응답에 메뉴를 붙이지 않는다(패널 메뉴 skin_pane::pane_menu).
                        if response.secondary_clicked() {
                            super::skin_pane::request_pane_menu(ui.ctx(), index);
                        }
                    }
                }
            });
        // 가로 막대를 끌었으면 그 자리(쪽 단위면 쪽 처음)로.
        let dragged = output.state.offset.x;
        if (dragged - first as f32 * column_width).abs() > 0.5 {
            first = snap(((dragged / column_width).round() as usize).min(last_first));
        }
        // 스킨 테마: 원본 가로 TNXScrollBar(넘칠 때만, 목록 아래끝). 단추 한 번 = 한 열(쪽 단위면 보이는 열 수), 바탕 = 한 쪽.
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
