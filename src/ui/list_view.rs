//! C# Details list (ConfigureGrid columns, FileNameDisplayConverter, ExtensionColorConverter, group headers).
//! Column texts are built once when the list is received or rearranged, like C# (date ages use the value at that time).
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
    /// Sort mode when a header is clicked (the C# SortMemberPath entries that have a sort mode).
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

/// Per list row the column texts (7 cells, DetailColumn order) and name color, plus the row order with group headers inserted.
#[derive(Default)]
pub(super) struct PaneView {
    pub cells: Vec<[String; 7]>,
    pub colors: Vec<Option<(u8, u8, u8)>>,
    pub rows: Vec<DetailRow>,
}

/// CharacterCase of C# FileNameDisplayConverter: 0 as is, 1 lower, 2 upper, 3 first letter upper.
/// SFTP permission bits as list text ("drwxr-xr-x"): original OnGetAttrString (FUN_00485aa0) shows the row's permission text (+0x74) as is on FTP panes,
/// otherwise GetAttrString (width "drwxrwxrwx"). The SFTP adapter builds it in FUN_0045b900: "----------" (0x8ffa42), 'd' for a folder, then
/// r/w/x from bits 0x100..0x1.
pub(super) fn permission_text(is_dir: bool, mode: u32) -> String {
    let mut text = String::from(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 7;
        text.push(if bits & 4 != 0 { 'r' } else { '-' });
        text.push(if bits & 2 != 0 { 'w' } else { '-' });
        text.push(if bits & 1 != 0 { 'x' } else { '-' });
    }
    text
}

/// Original StrCharCase (0x7db714): 1 = CharLowerW, 2 = CharUpperW, 3 = FUN_007d223c (after CharLowerW, only a-z at the start or after a space become upper).
/// Fixed: previously 3 uppercased only the first character of the whole text and left the rest alone.
pub(super) fn character_case(text: &str, mode: i64) -> String {
    match mode {
        1 => text.to_lowercase(),
        2 => text.to_uppercase(),
        3 => {
            let mut previous = ' ';
            text.to_lowercase()
                .chars()
                .map(|c| {
                    let out = if previous == ' ' && c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c };
                    previous = c;
                    out
                })
                .collect()
        }
        _ => text.into(),
    }
}

#[cfg(test)]
mod character_case_tests {
    #[test]
    fn title_case_follows_fun_007d223c() {
        assert_eq!(super::character_case("my REPORT file", 3), "My Report File");
        assert_eq!(super::character_case("a-b c", 3), "A-b C", "only after a space");
        assert_eq!(super::character_case("éclair été", 3), "éclair été", "a-z only");
        assert_eq!((super::character_case("AbC", 1), super::character_case("AbC", 2)), ("abc".to_owned(), "ABC".to_owned()));
    }
}

impl App {
    /// Visible columns, headers and width ratios (C# star widths 2.4, 0.55, 1.0, 0.8, 1.2, 0.55, 1.4).
    /// Column titles = original FIELD_NAME/EXT/SIZE/DATEMODIFIED/ATTR/DESC (TNXCustomFileListView headers). The "type" column exists only in the C# version.
    pub(super) fn detail_columns(&self, _ko: bool) -> Vec<(DetailColumn, String, f32)> {
        let field = |key: &str, fallback: &str| self.language_pack.text(&format!("RES.{key}"), fallback);
        let mut columns = vec![(DetailColumn::Name, field("FIELD_NAME", "Name"), 2.4)];
        if !self.settings.flag("AttachExtensionToName", false) {
            columns.push((DetailColumn::Extension, field("FIELD_EXT", "Ext"), 0.55));
        }
        // The original has no "type" column; with ShowFileType (original showfiletype) it shows the type in an empty description cell.
        columns.extend([
            (DetailColumn::Size, field("FIELD_SIZE", "Size"), 0.8),
            (DetailColumn::Modified, field("FIELD_DATEMODIFIED", "Date modified"), 1.2),
            (DetailColumn::Attributes, field("FIELD_ATTR", "Attr"), 0.55),
            (DetailColumn::Description, field("FIELD_DESC", "Description"), 1.4),
        ]);
        columns
    }

    /// C# GroupHeaderNameConverter. Difference (approved fix): C# Get("Caption", key) cannot find the [formNDRSearch] section of the language file and
    /// shows English names, while Rust uses that section's translation.
    pub(super) fn group_caption(&self, key: &str) -> String {
        match ndr::listing::group_caption_key(key) {
            (label, Some("RES.FOLDERS")) => self.language_pack.text("RES.FOLDERS", label),
            (label, Some(caption)) => self.language_pack.text(&format!("formNDRSearch.Caption.{caption}"), label),
            (label, None) => label.to_owned(),
        }
    }

    /// Rebuilds list texts, colors and group rows (called at the end of sort_pane).
    pub(super) fn compute_view(&mut self, index: usize) {
        let korean = self.language == "ko";
        let attach = self.settings.flag("AttachExtensionToName", false);
        let case = self.settings.values.get("CharacterCase").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 3);
        let date_format = self.settings.text("DateFormat", "yyyy-MM-dd");
        let time_format = self.settings.text("TimeFormat", "HH:mm");
        // Original default [Display] usedateage=1.
        let use_age = self.settings.flag("UseDateAge", true);
        // GetString of original DateToStr (TODAY / YESTERDAY / DATEAGE).
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
        // Skin theme: original item colors (original_item_color). The three base colors are resolved once.
        let skin = true;
        // Original [Display] showfiletype (default 1).
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
        let sftp_modes = pane.remote.as_ref().filter(|r| r.is_sftp()).and_then(|r| r.modes.lock().ok().map(|m| (r.path.clone(), m.clone())));
        let mut view = PaneView::default();
        let mut shell_types: HashMap<Option<String>, String> = HashMap::new();
        let mut item_colors: HashMap<String, (u8, u8, u8)> = HashMap::new();
        for entry in &pane.entries {
            if entry.drive {
                // Original DrawListItem kind 4: the drive name (item+8) after "%s:". Other cells (extension, size, date, attributes, description) are not drawn.
                let label = pane.drive_infos.get(&entry.path).map(|(name, _, _)| name.as_str()).unwrap_or("");
                let name = if label.is_empty() { entry.name.clone() } else { format!("{} {label}", entry.name) };
                view.cells.push([name, String::new(), String::new(), String::new(), String::new(), String::new(), String::new()]);
                view.colors.push(skin.then(|| ndr::listing::original_item_color(entry, extension_colors, basic)));
                continue;
            }
            let extension = if entry.is_dir || attach { "" } else { ndr::parity::list_extension(&entry.name) };
            let stem = &entry.name[..entry.name.len() - extension.len()];
            let description = if pane.descriptions.is_empty() { String::new() } else { pane.descriptions.get(&entry.name.to_lowercase()).cloned().unwrap_or_default() };
            // Original DrawListItem description cell (field 6, 0x7da043): when the description (+0x10) is empty and showfiletype (+0x384) is on, the file type text (+0xc).
            // The type text of original GetList (0x45a34c) is the shell name (SHGetFileInfoW SHGFI_TYPENAME|USEFILEATTRIBUTES, once per extension,
            // none for ".."). Archive and FTP lists never call SetFileType, so it is empty.
            let kind = if !skin && entry.is_parent() {
                (if korean { "상위 폴더" } else { "Parent folder" }).to_owned()
            } else if !skin {
                ndr::listing::type_name(&entry.name, entry.is_dir, korean)
            } else if pane.archive.is_some() || pane.remote.is_some() || entry.name == ".." {
                String::new()
            } else {
                let key = if entry.is_dir { None } else { Some(ndr::parity::list_extension(&entry.name).to_lowercase()) };
                shell_types.entry(key).or_insert_with_key(|key| native::shell_type_name(key.as_deref().unwrap_or(""), key.is_none())).clone()
            };
            let description = if description.is_empty() && skin && show_file_type { kind.clone() } else { description };
            view.cells.push([
                character_case(stem, case),
                // Original extension cell (item +8 = extension without the dot, letter case follows charcase). The C# theme uses ".ext" like the C# version.
                if entry.is_dir {
                    String::new()
                } else if skin {
                    character_case(ndr::parity::list_extension(&entry.name).trim_start_matches('.'), case)
                } else {
                    ndr::parity::list_extension(&entry.name).to_owned()
                },
                kind,
                // Original DrawListItem field 3 = SizeToStr (0x7df870). Fixed: previously C# FormatSize ("999 B", "1.2 MB").
                if entry.is_dir { String::new() } else { ndr::parity::size_to_str(entry.size, &unit, thousands) },
                ndr::listing::list_date_text(entry.modified, &date_format, &time_format, use_age.then(|| [age_labels[0].as_str(), age_labels[1].as_str(), age_labels[2].as_str()]), now),
                // Original DrawListItem field 5: SFTP list items (+0x78) show the permission text (+0x74), others GetAttrString (0x7b50a4, A/H/R/S in "____").
                // Fixed: previously the C# text ("RHSA", "-").
                match &sftp_modes {
                    Some((directory, modes)) => modes.get(&super::ftp_pane::join_remote(directory, &entry.name)).map(|mode| permission_text(entry.is_dir, *mode)).unwrap_or_default(),
                    None => ndr::listing::attr_string(entry.attributes),
                },
                description,
            ]);
            view.colors.push(if skin {
                // Fixed (speed): instead of scanning the extension color table from the start for every item, look it up once per extension.
                if entry.is_dir {
                    Some(ndr::listing::original_item_color(entry, extension_colors, basic))
                } else {
                    let key = ndr::parity::list_extension(&entry.name).to_ascii_lowercase();
                    Some(*item_colors.entry(key).or_insert_with(|| ndr::listing::original_item_color(entry, extension_colors, basic)))
                }
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

    /// 0x368 of original UpdateDisplay (0x7e01f8): the widest drive item name text, from the item's left edge to the end of that text
    /// (head width + 5 + text, the same margin as `drive_row`). 0 without drive items.
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
                .map(|(_, cells)| fonts.layout_no_wrap(cells[0].clone(), font.clone(), Color32::WHITE).size().x)
                .fold(0.0f32, f32::max)
        });
        App::list_header_width(icons) + 5.0 + widest
    }
}

/// One row of original DrawListItem kind 4 (left aligned, DT_LEFT): a button-like background when selected, the icon ("▸ " when missing), "C: name",
/// then `paint_drive_space`. `label_right` is from the row's left to the end of the widest drive name (App::drive_text_extent).
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
        // Original DrawListItem: the icon at left + 12, the "%s:" text from the head width (GetListHeaderWidth) + 5 (same margin as the name cell).
        if let Some(texture) = icon {
            egui::Image::new((texture.id(), egui::vec2(16.0, 16.0))).paint_at(ui, egui::Rect::from_min_size(egui::pos2(rect.left() + 12.0, rect.top() + ((rect.height() - 16.0) / 2.0).floor()), egui::vec2(16.0, 16.0)));
        }
        let x = rect.left() + App::list_header_width(icon.is_some()) + 5.0;
        // Original DrawListItem kind 4: the drive name uses DT_END_ELLIPSIS (local_64 | 0x8000), so it shortens with "..." when longer than the cell.
        // Fixed: previously characters were cut in half at the cell end.
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), egui::TextStyle::Button.resolve(ui.style()), color);
        job.wrap = egui::text::TextWrapping { max_width: (rect.right() - 5.0 - x).max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let drawn = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley.size());
        painter.galley(drawn.min, galley, color);
        // Underline (4) and strikeout (8) of the original ListFont style.
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

/// Free space of original DrawListItem kind 4 (0x7d8f58): for a fixed disk (type 3) with a known total size, a 70 x min(8, row height) bar drawn
/// 5 px after the end of the drive name cell (`label_right`), its inner 68 px filled by the used ratio. The bar is drawn only when more than 80 px remain on the right,
/// and the "free size (StrFormatByteSizeW) DISKFREE" text after it only when more than 80 px remain after that (clipped 5 px before the right edge).
/// Colors (0x7d9770-0x7d98f2): the border uses the item text color for the cursor item (ItemIndex), otherwise FUN_007d7e6c(NXFORE, NXBACK, 0.3);
/// the fill is always (NXFORE, NXBACK, 0.2); the text uses the item text color for the cursor item, otherwise NXSIZE (size column color).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_drive_space(ui: &egui::Ui, rect: egui::Rect, label_right: f32, kind: u32, sizes: Option<(u64, u64)>, cursor: bool, item_color: Color32, free_label: &str, style: u8) {
    // The original draws only fixed disks (3). Fixed: connected network drives whose size was read (drive_item_info) are drawn too.
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
        // Underline (4) and strikeout (8) of the original ListFont style.
        if style & 4 != 0 {
            text_painter.hline(drawn.x_range(), drawn.bottom() - 1.5, egui::Stroke::new(1.0, text_color));
        }
        if style & 8 != 0 {
            text_painter.hline(drawn.x_range(), drawn.center().y, egui::Stroke::new(1.0, text_color));
        }
    }
}

/// Original FUN_007d7e6c: per channel Round(a x t + b x (1 - t)). Delphi Round rounds half to even.
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
        // Default NXFORE C0C0C0, NXBACK 000000: border 0.3 -> 58 (57.6), fill 0.2 -> 38 (38.4).
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
        // Original FUN_007d223c: the first a-z of each word becomes upper case (C# FileNameDisplayConverter does only the first character).
        assert_eq!(character_case("report final", 3), "Report Final");
        assert_eq!(character_case("", 3), "");
    }
}
