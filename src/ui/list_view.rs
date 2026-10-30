//! C# 자세히 보기 목록(ConfigureGrid 열, FileNameDisplayConverter, ExtensionColorConverter, 묶음 머리글).
//! 열 글은 C#처럼 목록을 받거나 다시 늘어놓을 때 한 번 만들어 둔다(날짜 나이도 그때 값).
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DetailColumn {
    Name,
    Extension,
    Kind,
    Size,
    Modified,
    Attributes,
    Description,
}

impl DetailColumn {
    pub(super) fn index(self) -> usize {
        self as usize
    }
    /// 머리글을 눌렀을 때의 정렬 방식(C# SortMemberPath 중 정렬 방식이 있는 것).
    pub(super) fn sort_mode(self) -> Option<&'static str> {
        match self {
            DetailColumn::Name => Some("Name"),
            DetailColumn::Extension => Some("Extension"),
            DetailColumn::Size => Some("Size"),
            DetailColumn::Modified => Some("Modified"),
            DetailColumn::Attributes => Some("Attributes"),
            DetailColumn::Kind | DetailColumn::Description => None,
        }
    }
}

pub(super) enum DetailRow {
    Header(String),
    Item(usize),
}

/// 목록 줄마다 열 글(7칸, DetailColumn 순서)과 이름 색, 묶음 머리글을 끼운 줄 순서.
#[derive(Default)]
pub(super) struct PaneView {
    pub cells: Vec<[String; 7]>,
    pub colors: Vec<Option<(u8, u8, u8)>>,
    pub rows: Vec<DetailRow>,
}

/// C# FileNameDisplayConverter의 CharacterCase: 0 그대로, 1 소문자, 2 대문자, 3 첫 글자만 대문자.
fn character_case(text: &str, mode: i64) -> String {
    match mode {
        1 => text.to_lowercase(),
        2 => text.to_uppercase(),
        3 => {
            let mut chars = text.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
        _ => text.into(),
    }
}

impl App {
    /// 보이는 열과 머리글, 너비 비율(C# star 너비 2.4, 0.55, 1.0, 0.8, 1.2, 0.55, 1.4).
    /// 열 제목 = 원본 FIELD_NAME/EXT/SIZE/DATEMODIFIED/ATTR/DESC(TNXCustomFileListView 머리글). "종류" 열은 C# 판에만 있다.
    pub(super) fn detail_columns(&self, _ko: bool) -> Vec<(DetailColumn, String, f32)> {
        let field = |key: &str, fallback: &str| self.language_pack.text(&format!("RES.{key}"), fallback);
        let mut columns = vec![(DetailColumn::Name, field("FIELD_NAME", "Name"), 2.4)];
        if !self.settings.flag("AttachExtensionToName", false) {
            columns.push((DetailColumn::Extension, field("FIELD_EXT", "Ext"), 0.55));
        }
        // 원본은 "종류" 열을 두지 않고, ShowFileType(원본 showfiletype)이면 빈 설명 칸에 종류를 보인다.
        columns.extend([
            (DetailColumn::Size, field("FIELD_SIZE", "Size"), 0.8),
            (DetailColumn::Modified, field("FIELD_DATEMODIFIED", "Date modified"), 1.2),
            (DetailColumn::Attributes, field("FIELD_ATTR", "Attr"), 0.55),
            (DetailColumn::Description, field("FIELD_DESC", "Description"), 1.4),
        ]);
        columns
    }

    /// C# GroupHeaderNameConverter. 차이(승인된 수정): C#은 Get("Caption", 키)가 언어 파일의 [formNDRSearch] 절을 찾지 못해
    /// 영어 이름을 보이지만, Rust는 그 절의 번역을 쓴다.
    pub(super) fn group_caption(&self, key: &str) -> String {
        match ndr::listing::group_caption_key(key) {
            (label, Some("RES.FOLDERS")) => self.language_pack.text("RES.FOLDERS", label),
            (label, Some(caption)) => self.language_pack.text(&format!("formNDRSearch.Caption.{caption}"), label),
            (label, None) => label.to_owned(),
        }
    }

    /// 목록 글과 색, 묶음 줄을 다시 만든다(sort_pane 끝에서 부른다).
    pub(super) fn compute_view(&mut self, index: usize) {
        let korean = self.language == "ko";
        let attach = self.settings.flag("AttachExtensionToName", false);
        let case = self.settings.values.get("CharacterCase").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 3);
        let date_format = self.settings.text("DateFormat", "yyyy-MM-dd");
        let time_format = self.settings.text("TimeFormat", "HH:mm");
        // 원본 처음 값 [Display] usedateage=1.
        let use_age = self.settings.flag("UseDateAge", true);
        // 원본 DateToStr의 GetString(TODAY / YESTERDAY / DATEAGE).
        let age_labels = [
            self.language_pack.text("RES.TODAY", "Today"),
            self.language_pack.text("RES.YESTERDAY", "Yesterday"),
            self.language_pack.text("RES.DATEAGE", "%d days ago"),
        ];
        let unit = self.settings.text("SizeUnit", "Auto");
        let thousands = self.settings.flag("UseThousandsSeparator", true);
        let group_mode = self.settings.text(&Self::side_key(index, "GroupMode"), "None");
        let category_mode = self.settings.text(&Self::side_key(index, "FolderCategoryMode"), "Auto");
        let show_group_name = self.settings.flag(&Self::side_key(index, "ShowGroupName"), true);
        let now = std::time::SystemTime::now();
        // 스킨 테마: 원본 항목 색(original_item_color). 기본 색 세 개는 한 번만 구한다.
        let skin = true;
        // 원본 [Display] showfiletype(처음 값 1).
        let show_file_type = self.settings.flag("ShowFileType", true);
        let rgb = |key: &str| {
            let c = self.basic_list_color(key);
            (c.r(), c.g(), c.b())
        };
        let (folder, drive, fore) = (rgb("NXFOLDER"), rgb("NXDRIVE"), rgb("NXFORE"));
        let basic = move |key: &str| match key {
            "NXFOLDER" => folder,
            "NXDRIVE" => drive,
            _ => fore,
        };
        let (extension_colors, nexus_colors) = self.color_maps();
        let pane = &self.panes[index];
        let mut view = PaneView::default();
        let mut shell_types: HashMap<Option<String>, String> = HashMap::new();
        for entry in &pane.entries {
            if entry.drive {
                // 원본 DrawListItem 종류 4: "%s:" 뒤에 드라이브 이름(item+8). 다른 칸(확장자·크기·날짜·속성·설명)은 그리지 않는다.
                let label = pane.drive_infos.get(&entry.path).map(|(name, _, _)| name.as_str()).unwrap_or("");
                let name = if label.is_empty() { entry.name.clone() } else { format!("{} {label}", entry.name) };
                view.cells.push([name, String::new(), String::new(), String::new(), String::new(), String::new(), String::new()]);
                view.colors.push(skin.then(|| ndr::listing::original_item_color(entry, extension_colors, basic)));
                continue;
            }
            let extension = if entry.is_dir || attach { "" } else { ndr::parity::dotnet_extension(&entry.name) };
            let stem = &entry.name[..entry.name.len() - extension.len()];
            let description = pane.descriptions.get(&entry.name.to_lowercase()).cloned().unwrap_or_default();
            // 원본 DrawListItem 설명 칸(필드 6, 0x7da043): 설명(+0x10)이 비고 showfiletype(+0x384)이면 파일 종류 글(+0xc).
            // 원본 GetList(0x45a34c)의 종류 글은 셸 이름(SHGetFileInfoW SHGFI_TYPENAME|USEFILEATTRIBUTES, 확장자마다 한 번,
            // ".."는 없음). 압축·FTP 목록은 SetFileType을 부르지 않아 비어 있다.
            let kind = if !skin && entry.is_parent() {
                (if korean { "상위 폴더" } else { "Parent folder" }).to_owned()
            } else if !skin {
                ndr::listing::type_name(&entry.name, entry.is_dir, korean)
            } else if pane.archive.is_some() || pane.remote.is_some() || entry.name == ".." {
                String::new()
            } else {
                let key = if entry.is_dir { None } else { Some(ndr::parity::dotnet_extension(&entry.name).to_lowercase()) };
                shell_types.entry(key).or_insert_with_key(|key| native::shell_type_name(key.as_deref().unwrap_or(""), key.is_none())).clone()
            };
            let description = if description.is_empty() && skin && show_file_type { kind.clone() } else { description };
            view.cells.push([
                character_case(stem, case),
                // 원본 확장자 칸(항목 +8 = 점 없는 확장자, 글자 꼴은 charcase를 따름). C# 테마는 C# 판처럼 ".확장자".
                if entry.is_dir {
                    String::new()
                } else if skin {
                    character_case(ndr::parity::dotnet_extension(&entry.name).trim_start_matches('.'), case)
                } else {
                    ndr::parity::dotnet_extension(&entry.name).to_owned()
                },
                kind,
                if entry.is_dir { String::new() } else { ndr::parity::format_size(entry.size, &unit, thousands) },
                if use_age {
                    ndr::listing::age_date_text(entry.modified, &date_format, &time_format, [&age_labels[0], &age_labels[1], &age_labels[2]], now)
                } else {
                    ndr::listing::date_text(entry.modified, &date_format, &time_format, false, korean, now)
                },
                ndr::listing::attribute_text(entry.attributes),
                description,
            ]);
            view.colors.push(if skin {
                Some(ndr::listing::original_item_color(entry, extension_colors, basic))
            } else {
                ndr::listing::entry_color(entry, extension_colors, nexus_colors)
            });
        }
        let mut previous: Option<String> = None;
        for (row, entry) in pane.entries.iter().enumerate() {
            if group_mode != "None" && show_group_name && !entry.drive {
                let key = ndr::listing::group_key(entry, &group_mode, &category_mode);
                if previous.as_deref() != Some(key.as_str()) {
                    view.rows.push(DetailRow::Header(self.group_caption(&key)));
                    previous = Some(key);
                }
            }
            view.rows.push(DetailRow::Item(row));
        }
        self.panes[index].view = view;
    }

    /// 원본 UpdateDisplay(0x7e01f8)의 0x368: 드라이브 항목 이름 글 가운데 가장 넓은 너비. 이름 단추 왼쪽 끝에서 그 글 끝까지
    /// (단추 안쪽 여백 + 아이콘, 아이콘이 없으면 "▸ "를 붙인 글로 잰다). 드라이브 항목이 없으면 0.
    pub(super) fn drive_text_extent(&self, ui: &egui::Ui, index: usize) -> f32 {
        let pane = &self.panes[index];
        if !pane.entries.iter().any(|e| e.drive) {
            return 0.0;
        }
        let icons = self.settings.flag("ShowIcons", true);
        let font = egui::TextStyle::Button.resolve(ui.style());
        let widest = ui.fonts_mut(|fonts| {
            pane.entries
                .iter()
                .zip(&pane.view.cells)
                .filter(|(e, _)| e.drive)
                .map(|(_, cells)| {
                    let text = if icons { cells[0].clone() } else { format!("  {}", cells[0]) };
                    fonts.layout_no_wrap(text, font.clone(), Color32::WHITE).size().x
                })
                .fold(0.0f32, f32::max)
        });
        ui.spacing().button_padding.x + if icons { 16.0 + ui.spacing().icon_spacing } else { 0.0 } + widest
    }
}

/// 원본 DrawListItem 종류 4의 한 줄(왼쪽 맞춤, DT_LEFT): 고른 줄이면 단추처럼 바탕, 아이콘(없으면 "▸ "), "C: 이름",
/// 그 뒤 `paint_drive_space`. `label_right`는 줄 왼쪽에서 가장 넓은 드라이브 이름 끝까지(App::drive_text_extent).
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_row(ui: &mut egui::Ui, size: egui::Vec2, icon: Option<&egui::TextureHandle>, text: &str, color: Color32, selected: bool, cursor: bool, label_right: f32, info: Option<(u32, Option<(u64, u64)>)>, free_label: &str, style: u8) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        if selected {
            ui.painter().rect(rect, visuals.corner_radius, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        }
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let mut x = rect.left() + ui.spacing().button_padding.x;
        let shown = match icon {
            Some(texture) => {
                egui::Image::new((texture.id(), egui::vec2(16.0, 16.0))).paint_at(ui, egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 8.0), egui::vec2(16.0, 16.0)));
                x += 16.0 + ui.spacing().icon_spacing;
                text.to_owned()
            }
            None => format!("  {text}"),
        };
        let drawn = painter.text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, shown, egui::TextStyle::Button.resolve(ui.style()), color);
        // 원본 ListFont 모양의 밑줄(4)·취소선(8).
        if style & 4 != 0 {
            painter.hline(drawn.x_range(), drawn.bottom() - 1.5, egui::Stroke::new(1.0, color));
        }
        if style & 8 != 0 {
            painter.hline(drawn.x_range(), drawn.center().y, egui::Stroke::new(1.0, color));
        }
        if let Some((kind, sizes)) = info {
            paint_drive_space(ui, rect, rect.left() + label_right, kind, sizes, cursor, color, free_label, style);
        }
    }
    response
}

/// 원본 DrawListItem 종류 4의 남은 공간(0x7d8f58): 고정 디스크(종류 3)이고 전체 크기를 알 때, 드라이브 이름 칸 끝(`label_right`)에서
/// 5px 띄어 70 × min(8, 줄 높이) 막대를 그리고 안쪽 68px에 쓴 비율만큼 채운다. 막대는 오른쪽에 80px 넘게 남을 때만,
/// 막대 뒤 "여유 크기(StrFormatByteSizeW) DISKFREE" 글은 그 뒤로 80px 넘게 남을 때만 그린다(오른쪽 5px 앞에서 자른다).
/// 색(0x7d9770~0x7d98f2): 테두리는 커서 항목(ItemIndex)이면 항목 글자 색, 아니면 FUN_007d7e6c(NXFORE, NXBACK, 0.3),
/// 채움은 늘 (NXFORE, NXBACK, 0.2), 글은 커서 항목이면 항목 글자 색, 아니면 NXSIZE(크기 열 색).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_drive_space(ui: &egui::Ui, rect: egui::Rect, label_right: f32, kind: u32, sizes: Option<(u64, u64)>, cursor: bool, item_color: Color32, free_label: &str, style: u8) {
    // 원본은 고정 디스크(3)만. 고친 것: 크기를 읽은 WSL 네트워크 드라이브(drive_item_info)도 그린다.
    let Some((free, total)) = sizes.filter(|_| kind == 3 || kind == 4) else { return };
    let room = rect.right() - label_right;
    if total == 0 || room <= 79.0 {
        return;
    }
    let used = total.saturating_sub(free);
    let fill = ((used as u128 * 100 / total as u128) as f32 * 68.0 / 100.0).floor();
    let height = rect.height().min(8.0);
    let left = (label_right + 5.0).round();
    let top = rect.top() + ((rect.height() - height) / 2.0).floor();
    let fore = ui.visuals().text_color();
    let back = ui.visuals().extreme_bg_color;
    let outline = if cursor { item_color } else { blend(fore, back, 0.3) };
    let fill_color = blend(fore, back, 0.2);
    let text_color = if cursor { item_color } else { ui.visuals().weak_text_color() };
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let outer = egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(70.0, height));
    painter.rect_stroke(outer, 0.0, egui::Stroke::new(1.0, outline), egui::StrokeKind::Inside);
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(left + 1.0, top + 1.0), egui::vec2(fill, (height - 2.0).max(0.0))), 0.0, fill_color);
    if room - 80.0 > 80.0 {
        let text_rect = egui::Rect::from_min_max(egui::pos2(left + 81.0, rect.top()), egui::pos2(rect.right() - 5.0, rect.bottom()));
        let text_painter = painter.with_clip_rect(text_rect.intersect(ui.clip_rect()));
        let drawn = text_painter.text(
            egui::pos2(text_rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{} {free_label}", native::format_byte_size(free)),
            egui::TextStyle::Button.resolve(ui.style()),
            text_color,
        );
        // 원본 ListFont 모양의 밑줄(4)·취소선(8).
        if style & 4 != 0 {
            text_painter.hline(drawn.x_range(), drawn.bottom() - 1.5, egui::Stroke::new(1.0, text_color));
        }
        if style & 8 != 0 {
            text_painter.hline(drawn.x_range(), drawn.center().y, egui::Stroke::new(1.0, text_color));
        }
    }
}

/// 원본 FUN_007d7e6c: 채널마다 Round(a × t + b × (1 − t)). Delphi Round는 반올림 짝수.
pub(super) fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) * t + (1.0 - t) * f32::from(y)).round_ties_even().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

#[cfg(test)]
mod tests {
    use super::character_case;

    #[test]
    fn drive_bar_blend_follows_fun_007d7e6c() {
        use super::Color32;
        // 기본 NXFORE C0C0C0, NXBACK 000000: 테두리 0.3 → 58(57.6), 채움 0.2 → 38(38.4).
        let (fore, back) = (Color32::from_rgb(0xC0, 0xC0, 0xC0), Color32::BLACK);
        assert_eq!(super::blend(fore, back, 0.3), Color32::from_rgb(58, 58, 58));
        assert_eq!(super::blend(fore, back, 0.2), Color32::from_rgb(38, 38, 38));
        assert_eq!(super::blend(Color32::WHITE, Color32::BLACK, 0.5), Color32::from_rgb(128, 128, 128));
    }
    #[test]
    fn character_case_follows_filenamedisplayconverter() {
        assert_eq!(character_case("Report Final", 0), "Report Final");
        assert_eq!(character_case("Report Final", 1), "report final");
        assert_eq!(character_case("Report Final", 2), "REPORT FINAL");
        assert_eq!(character_case("report final", 3), "Report final");
        assert_eq!(character_case("", 3), "");
    }
}
