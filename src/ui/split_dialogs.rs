//! C# SplitFileWindow / JoinFilesWindow and the SplitFileAsync / JoinPartsAsync flows. Rules are in ndr::split.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{MB_ICONEXCLAMATION, MB_OK};
use std::fs;

/// SplitFileWindow input.
#[derive(Clone, Debug)]
pub(super) struct SplitForm {
    pub source: PathBuf,
    pub length: u64,
    pub path: String,
    pub size: String,
}

/// Original TfFileJoin input.
#[derive(Clone, Debug)]
pub(super) struct JoinForm {
    pub files: Vec<(PathBuf, Option<u64>)>,
    pub selected: Option<usize>,
    pub join_text: bool,
    pub name: String,
}

/// C# JoinFilesOptions.
#[derive(Clone, Debug)]
pub(super) struct JoinOptions {
    pub files: Vec<PathBuf>,
    pub join_text: bool,
    pub name: String,
    pub overwrite_confirmed: bool,
}

fn split_text(language: &compat::Language, key: &str, fallback: &str) -> String {
    language.text(&format!("formFileSplit.{key}"), fallback)
}

fn join_text(language: &compat::Language, key: &str, fallback: &str) -> String {
    language.text(&format!("formFileJoin.{key}"), fallback)
}

/// Exact, grouped byte count in the selected application language.
fn exact_byte_count(language: &compat::Language, bytes: u64) -> String {
    let number = ndr::parity::format_size(bytes, "B", true).trim_end_matches(" B").to_owned();
    let template = language.text("NDR.OP_TRANSFER_CONFLICT_BYTES", "<b>{0}</b> bytes");
    ndr::markup::plain_text(&template.replace("{0}", &number))
}

impl JoinForm {
    /// AddPath: existing files only, the same path (case-insensitive) once.
    pub(super) fn add(&mut self, path: &Path) {
        if !path.is_file() {
            return;
        }
        let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        if self.files.iter().any(|(known, _)| ndr::names::equals_ignore_case(&known.to_string_lossy(), &full.to_string_lossy())) {
            return;
        }
        let length = fs::metadata(&full).ok().map(|m| m.len());
        self.files.push((full, length));
    }

    /// MoveFile(±1).
    pub(super) fn move_selected(&mut self, offset: isize) {
        let Some(index) = self.selected else { return };
        let target = index as isize + offset;
        if target < 0 || target as usize >= self.files.len() {
            return;
        }
        self.files.swap(index, target as usize);
        self.selected = Some(target as usize);
    }

    /// RemoveFile.
    pub(super) fn remove_selected(&mut self) {
        let Some(index) = self.selected else { return };
        self.files.remove(index);
        self.selected = (!self.files.is_empty()).then(|| index.min(self.files.len() - 1));
    }

    /// Exact sum of the part sizes, formatted in the selected language.
    fn size_info(&self, language: &compat::Language) -> String {
        let total = self.files.iter().filter_map(|(_, l)| *l).fold(0u64, u64::saturating_add);
        exact_byte_count(language, total)
    }
}

fn compact_label(ui: &mut egui::Ui, caption: &str, target: egui::Id) {
    let (mut job, access) = super::original_dialogs::access_job_colored(ui, caption, None);
    job.wrap.max_width = ui.available_width();
    job.wrap.break_anywhere = true;
    ui.label(job);
    if ui.is_enabled() && super::original_dialogs::take_dialog_char(ui.ctx(), access) {
        ui.memory_mut(|m| m.request_focus(target));
    }
}

fn compact_split_ui(ui: &mut egui::Ui, form: &mut SplitForm, language: &compat::Language, enter: bool, width: f32) -> Option<usize> {
    let screen = ui.ctx().content_rect();
    ui.set_width(width);
    ui.set_max_width(width);
    let raw = |key: &str, fallback: &str| language.raw(&format!("formFileSplit.{key}")).unwrap_or_else(|| fallback.to_owned());
    egui::ScrollArea::vertical().id_salt("compact-split-body").max_height((screen.height() - 112.0).max(48.0)).max_width(width).auto_shrink([false, true]).show(ui, |ui| {
        let name = form.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        ui.add(egui::Label::new(format!("{name} ({})", exact_byte_count(language, form.length))).wrap()).on_hover_text(form.source.to_string_lossy());
        let path_id = egui::Id::new("split-path");
        let size_id = egui::Id::new("split-size");
        compact_label(ui, &raw("Caption.lPath", "Save &parts in"), path_id);
        ui.horizontal(|ui| {
            let field = (ui.available_width() - 42.0).max(55.0);
            ui.add_sized([field, 23.0], egui::TextEdit::singleline(&mut form.path).id(path_id).desired_width(field));
            if ui.add_sized([34.0, 23.0], egui::Button::new("...")).clicked() {
                let title = format!("{}{}", language.text("RES.FILESPLIT", "Split File"), language.text("RES.SELECTDEST", " - Select a destination"));
                let initial = Path::new(&form.path).is_dir().then(|| PathBuf::from(&form.path)).or_else(|| form.source.parent().map(Path::to_path_buf));
                if let Some(folder) = ndr::file_dialogs::browse_folder(&title, initial.as_deref()) { form.path = folder.to_string_lossy().into_owned(); }
            }
        });
        compact_label(ui, &raw("Caption.lSplitSize", "Byte&s per part"), size_id);
        ui.horizontal(|ui| {
            let field = (ui.available_width() - 32.0).max(55.0);
            ui.add_sized([field, 23.0], egui::TextEdit::singleline(&mut form.size).id(size_id).desired_width(field));
            ui.menu_button("▼", |ui| {
                egui::ScrollArea::vertical().max_height((screen.height() - 65.0).max(48.0)).show(ui, |ui| {
                    for preset in ndr::split::SPLIT_SIZE_PRESETS { if ui.button(preset).clicked() { form.size = preset.into(); ui.close(); } }
                });
            });
        });
        if let Some(size) = ndr::split::parse_split_size(&form.size) {
            ui.add(egui::Label::new(language.text("RES.SPLITTED_INFO", "Number of splitted files: %d").replace("%d", &ndr::split::part_count(form.length, size).to_string())).wrap());
        }
    });
    ui.separator();
    let (ok, cancel) = super::original_dialogs::compact_input_buttons(ui, &split_text(language, "Caption.btnOk", "OK"), &split_text(language, "Caption.btnCancel", "Cancel"), width);
    if cancel { Some(1) } else if ok || enter { Some(0) } else { None }
}

fn compact_join_ui(ui: &mut egui::Ui, form: &mut JoinForm, language: &compat::Language, first_frame: bool, enter: bool, width: f32) -> Option<usize> {
    use super::original_dialogs::{access_job_colored, compact_button_height, take_dialog_char, vcl_wrapped_button};
    let screen = ui.ctx().content_rect();
    let raw = |key: &str, fallback: &str| language.raw(&format!("formFileJoin.{key}")).unwrap_or_else(|| fallback.to_owned());
    let labels = [raw("Caption.btnAdd", "&Add"), raw("Caption.btnDelete", "&Delete"), raw("Caption.btnUp", "&Up"), raw("Caption.btnDown", "Do&wn")];
    let font = egui::TextStyle::Body.resolve(ui.style());
    let (mut name_job, name_access) = access_job_colored(ui, &raw("Caption.lFileName", "&File name:"), None);
    name_job.wrap.max_width = width - 8.0;
    name_job.wrap.break_anywhere = true;
    let name_height = ui.fonts_mut(|f| f.layout_job(name_job.clone()).size().y).max(15.0) + 2.0;
    let summary = format!("{} · {}", join_text(language, "Caption.lFiles", "Parts in joining order"), form.size_info(language));
    let mut summary_job = egui::text::LayoutJob::simple(summary, font.clone(), ui.visuals().text_color(), width - 8.0);
    summary_job.wrap.break_anywhere = true;
    let summary_height = ui.fonts_mut(|f| f.layout_job(summary_job.clone()).size().y).max(15.0) + 2.0;
    let grid = |cols: usize| {
        let bw = (width - 8.0 - 4.0 * (cols - 1) as f32) / cols as f32;
        let heights: Vec<_> = labels.chunks(cols).map(|r| compact_button_height(ui, r, bw)).collect();
        let h = heights.iter().sum::<f32>() + 4.0 * (heights.len() - 1) as f32;
        (bw, heights, h)
    };
    let four = grid(4);
    let two = grid(2);
    let (cols, (bw, heights, actions_height)) = if four.2 <= two.2 { (4, four) } else { (2, two) };
    let (mut check_job, check_access) = access_job_colored(ui, &raw("Caption.chkIsText", "Par&ts contain text"), None);
    check_job.wrap.max_width = (width - 30.0).max(1.0);
    check_job.wrap.break_anywhere = true;
    let check_height = ui.fonts_mut(|f| f.layout_job(check_job.clone()).size().y).max(17.0) + 3.0;
    let ok = join_text(language, "Caption.btnOk", "OK");
    let cancel = join_text(language, "Caption.btnCancel", "Cancel");
    let confirm_width = (width - 14.0) / 2.0;
    let confirm_height = compact_button_height(ui, &[ok.clone(), cancel.clone()], confirm_width);
    // Only the list scrolls. The output editor and all actions stay fixed;
    // a click on a scrolled row cannot move the focused text field back into view.
    let fixed = name_height + 23.0 + summary_height + actions_height + check_height + confirm_height + 24.0;
    let list_height = (screen.height() - 54.0 - fixed).clamp(23.0, 200.0);
    let (area, _) = ui.allocate_exact_size(egui::vec2(width, fixed + list_height), egui::Sense::hover());
    let at = |x, y, w, h| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let name_id = egui::Id::new("join-file-name");
    super::original_dialogs::put_left(ui, at(4.0, 0.0, width - 8.0, name_height), egui::Label::new(name_job).wrap());
    if ui.is_enabled() && take_dialog_char(ui.ctx(), name_access) { ui.memory_mut(|m| m.request_focus(name_id)); }
    let mut y = name_height + 2.0;
    let edit = ui.put(at(4.0, y, width - 46.0, 23.0), egui::TextEdit::singleline(&mut form.name).id(name_id));
    if first_frame { edit.request_focus(); super::original_dialogs::select_all(ui, name_id, form.name.chars().count()); }
    if ui.put(at(width - 38.0, y, 34.0, 23.0), egui::Button::new("...")).clicked() {
        if let Some(chosen) = ndr::file_dialogs::open_any_file("", None, "", &[]) { form.name = chosen.to_string_lossy().into_owned(); }
    }
    y += 27.0;
    super::original_dialogs::put_left(ui, at(4.0, y, width - 8.0, summary_height), egui::Label::new(summary_job).wrap());
    y += summary_height + 2.0;
    let list_rect = at(4.0, y, width - 8.0, list_height);
    ui.painter().rect(list_rect, 0.0, ui.visuals().extreme_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    let inner = list_rect.shrink(1.0);
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(inner));
        egui::ScrollArea::vertical().id_salt("compact-join-parts").min_scrolled_height(0.0).max_width(inner.width()).max_height(inner.height()).auto_shrink([false, false]).animated(false).show_rows(ui, 20.0, form.files.len(), |ui, range| {
            for i in range {
                let path = form.files[i].0.to_string_lossy();
                let w = ui.available_width();
                let shown = super::original_dialogs::path_ellipsis(ui, &path, (w - 14.0).max(1.0), &font);
                if ui.push_id(i, |ui| ui.add_sized([w, 20.0], egui::Button::selectable(form.selected == Some(i), shown).truncate()).on_hover_text(path).clicked()).inner { form.selected = Some(i); }
            }
        });
    });
    y += list_height + 4.0;
    for (row, bh) in heights.into_iter().enumerate() {
        for col in 0..cols {
            let i = row * cols + col;
            if i >= labels.len() { continue; }
            if vcl_wrapped_button(ui, at(4.0 + col as f32 * (bw + 4.0), y, bw, bh), true, &labels[i]).1 {
                match i {
                    0 => {
                        let title = join_text(language, "Form.Caption", "Join Files");
                        let initial = form.files.first().and_then(|(p, _)| p.parent().map(Path::to_path_buf));
                        for path in ndr::file_dialogs::open_files(&title, initial.as_deref()) { form.add(&path); }
                        if form.selected.is_none() && !form.files.is_empty() { form.selected = Some(0); }
                    }
                    1 => form.remove_selected(),
                    2 => form.move_selected(-1),
                    _ => form.move_selected(1),
                }
            }
        }
        y += bh + 4.0;
    }
    ui.put(at(4.0, y, width - 8.0, check_height), egui::Checkbox::new(&mut form.join_text, check_job));
    if ui.is_enabled() && take_dialog_char(ui.ctx(), check_access) { form.join_text = !form.join_text; }
    y += check_height + 4.0;
    let ok = ui.put(at(4.0, y, confirm_width, confirm_height), egui::Button::new(ok).wrap()).clicked();
    let cancel = ui.put(at(width - confirm_width - 4.0, y, confirm_width, confirm_height), egui::Button::new(cancel).wrap()).clicked();
    if cancel { Some(1) } else if ok || enter { Some(0) } else { None }
}

/// Draws the split window. Returns Some(0) OK, Some(1) cancel.
pub(super) fn split_ui(ui: &mut egui::Ui, form: &mut SplitForm, language: &compat::Language, enter: bool) -> Option<usize> {
    let mut choice = None;
    // Original TfFileSplit (ClientWidth 469 x ClientHeight 187): lFileName (16,16), lPath (16,59) ePath (103,56 311x21) bBrowse (420,54 41x25),
    // lSplitSize (16,99) cbSplitSize (103,96 138x21, list 1440 KB...100 MB) lSplitCount (260,99), btnOk (303,151) btnCancel (384,151) 75x25.
    let raw = |key: &str, fallback: &str| language.raw(&format!("formFileSplit.{key}")).unwrap_or_else(|| fallback.to_owned());
    let measure = |s: &str| super::original_dialogs::caption_width(ui, s);
    let path_caption = raw("Caption.lPath", "Save &parts in");
    let size_caption = raw("Caption.lSplitSize", "Byte&s per part");
    let label_width = (measure(&path_caption).max(measure(&size_caption)) + 10.0).max(85.0);
    let field = 18.0 + label_width;
    let ok = split_text(language, "Caption.btnOk", "OK");
    let cancel = split_text(language, "Caption.btnCancel", "Cancel");
    let button_width = (measure(&ok).max(measure(&cancel)) + 24.0).max(75.0);
    let width = (field + 366.0).max(button_width * 2.0 + 32.0);
    let screen = ui.ctx().content_rect();
    if width + 40.0 > screen.width() || screen.height() < 265.0 { return compact_split_ui(ui, form, language, enter, (screen.width() - 40.0).clamp(120.0, 469.0)); }
    let (area, _) = ui.allocate_exact_size(egui::vec2(width, 187.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    // Original TntFormShow (0x4236e4): lFileName = "<b>name</b> (size bytes)" (size with thousands separators, FloatToStrF ffNumber 0).
    let name = form.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = exact_byte_count(language, form.length);
    let mut job = egui::text::LayoutJob::default();
    let color = ui.visuals().text_color();
    job.append(&name, 0.0, egui::TextFormat { font_id: egui::FontId::new(13.0, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())), color, ..Default::default() });
    job.append(&format!(" ({bytes})"), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.0), color, ..Default::default() });
    super::original_dialogs::put_left(ui, at(16.0, 16.0, width - 29.0, 15.0), egui::Label::new(job).truncate()).on_hover_text(format!("{} ({bytes})", form.source.display()));
    let (path_id, size_id) = (egui::Id::new("split-path"), egui::Id::new("split-size"));
    super::original_dialogs::access_label(ui, at(16.0, 59.0, label_width, 15.0), &path_caption, path_id);
    ui.put(at(field, 56.0, width - field - 55.0, 21.0), egui::TextEdit::singleline(&mut form.path).id(path_id));
    if ui.put(at(width - 49.0, 54.0, 41.0, 25.0), egui::Button::new("...")).clicked() {
        // Original (0x423b8e): FILESPLIT + SELECTDEST.
        let title = format!("{}{}", language.text("RES.FILESPLIT", "Split File"), language.text("RES.SELECTDEST", " - Select a destination"));
        let initial = Path::new(&form.path).is_dir().then(|| PathBuf::from(&form.path)).or_else(|| form.source.parent().map(Path::to_path_buf));
        if let Some(folder) = ndr::file_dialogs::browse_folder(&title, initial.as_deref()) {
            form.path = folder.to_string_lossy().into_owned();
        }
    }
    super::original_dialogs::access_label(ui, at(16.0, 99.0, label_width, 15.0), &size_caption, size_id);
    ui.put(at(field, 96.0, 116.0, 21.0), egui::TextEdit::singleline(&mut form.size).id(size_id));
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(field + 116.0, 96.0, 22.0, 21.0)), |ui| {
        ui.menu_button("▼", |ui| {
            for preset in ndr::split::SPLIT_SIZE_PRESETS {
                if ui.button(preset).clicked() {
                    form.size = preset.into();
                    ui.close();
                }
            }
        });
    });
    if let Some(size) = ndr::split::parse_split_size(&form.size) {
        let template = language.text("RES.SPLITTED_INFO", "Number of splitted files: %d");
        let count = ndr::split::part_count(form.length, size);
        let summary = template.replace("%d", &count.to_string());
        super::original_dialogs::put_left(ui, at(field + 149.0, 96.0, width - field - 158.0, 46.0), egui::Label::new(&summary).wrap())
            .on_hover_text(summary);
    }
    if ui.put(at(width - 16.0 - 2.0 * button_width, 151.0, button_width, 25.0), egui::Button::new(ok)).clicked() {
        choice = Some(0);
    }
    if ui.put(at(width - 10.0 - button_width, 151.0, button_width, 25.0), egui::Button::new(cancel)).clicked() {
        choice = Some(1);
    }
    if enter {
        choice = Some(0);
    }
    choice
}

/// Draws the join window. Returns Some(0) OK, Some(1) cancel.
pub(super) fn join_ui(ui: &mut egui::Ui, form: &mut JoinForm, language: &compat::Language, first_frame: bool, enter: bool) -> Option<usize> {
    let mut choice = None;
    // Original TfFileJoin (ClientWidth 501 x ClientHeight 346): lFileName (16,25) eFileName (127,22 323x21) btnBrowse (456,20 33x25),
    // lFiles(16,65) lbFiles(16,84 473×169), btnAdd(16,262) btnDelete(86,262) btnUp(172,262) btnDown(242,262) 64×25,
    // lSizeInfo (right edge 486,267), chkIsText (16,317), btnOk (330,313) btnCancel (411,313) 75x25.
    let raw = |key: &str, fallback: &str| language.raw(&format!("formFileJoin.{key}")).unwrap_or_else(|| fallback.to_owned());
    let caption = raw("Caption.lFileName", "&File name:");
    let labels = [raw("Caption.btnAdd", "&Add"), raw("Caption.btnDelete", "&Delete"), raw("Caption.btnUp", "&Up"), raw("Caption.btnDown", "Do&wn")];
    let measure = |s: &str| super::original_dialogs::caption_width(ui, s);
    let field = (measure(&caption) + 27.0).max(127.0);
    let button_width = labels.iter().map(|s| measure(s) + 20.0).fold(64.0_f32, f32::max);
    let check = raw("Caption.chkIsText", "Par&ts contain text");
    let ok = join_text(language, "Caption.btnOk", "OK");
    let cancel = join_text(language, "Caption.btnCancel", "Cancel");
    let confirm_width = (measure(&ok).max(measure(&cancel)) + 24.0).max(75.0);
    let total = form.size_info(language);
    let width = 501.0_f32.max(field + 374.0).max(4.0 * (button_width + 6.0) + measure(&total) + 40.0)
        .max(measure(&check) + 2.0 * confirm_width + 76.0);
    let screen = ui.ctx().content_rect();
    if width + 40.0 > screen.width() || screen.height() < 420.0 { return compact_join_ui(ui, form, language, first_frame, enter, (screen.width() - 40.0).clamp(120.0, 501.0)); }
    let (area, _) = ui.allocate_exact_size(egui::vec2(width, 346.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let name_id = egui::Id::new("join-file-name");
    super::original_dialogs::access_label(ui, at(16.0, 25.0, field - 19.0, 15.0), &caption, name_id);
    let name_response = ui.put(at(field, 22.0, width - field - 51.0, 21.0), egui::TextEdit::singleline(&mut form.name).id(name_id));
    // eFileName is the first control in the tab order: it has the focus with its text selected when the window opens (seen in the running original).
    if first_frame {
        name_response.request_focus();
        super::original_dialogs::select_all(ui, name_id, form.name.chars().count());
    }
    if ui.put(at(width - 45.0, 20.0, 33.0, 25.0), egui::Button::new("...")).clicked() {
        // Original btnBrowseClick (0x423444): the window's OpenDialog (no Title, Filter or InitialDir) with ofAllowMultiSelect and ofFileMustExist turned off
        // and FileName cleared, then Execute (an open box). The choice goes into eFileName. Being an open box, there is no overwrite prompt (asked separately when joining).
        if let Some(chosen) = ndr::file_dialogs::open_any_file("", None, "", &[]) {
            form.name = chosen.to_string_lossy().into_owned();
        }
    }
    super::original_dialogs::put_left(ui, at(16.0, 65.0, width - 28.0, 15.0), egui::Label::new(join_text(language, "Caption.lFiles", "Parts in joining order")));
    let list = at(16.0, 84.0, width - 28.0, 169.0);
    ui.painter().rect(list, 0.0, ui.visuals().extreme_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink(2.0)), |ui| {
        egui::ScrollArea::vertical().id_salt("join-files").auto_shrink([false, false]).show(ui, |ui| {
            // Original lbFiles holds the full paths (Items.Assign of the data item paths).
            for (index, (path, _)) in form.files.iter().enumerate() {
                if ui.add(egui::Button::selectable(form.selected == Some(index), path.to_string_lossy()).truncate()).on_hover_text(path.to_string_lossy()).clicked() {
                    form.selected = Some(index);
                }
            }
        });
    });
    let button = |ui: &mut egui::Ui, index: usize| super::original_dialogs::vcl_button(ui, at(16.0 + index as f32 * (button_width + 6.0), 262.0, button_width, 25.0), true, &labels[index]).1;
    if button(ui, 0) {
        let title = join_text(language, "Form.Caption", "Join Files");
        let initial = form.files.first().and_then(|(p, _)| p.parent().map(Path::to_path_buf));
        for path in ndr::file_dialogs::open_files(&title, initial.as_deref()) {
            form.add(&path);
        }
        if form.selected.is_none() && !form.files.is_empty() {
            form.selected = Some(0);
        }
    }
    // Original btnDelete/btnUp/btnDown are always enabled; their handlers (0x42332c, 0x423374, 0x4233c8) do nothing without a usable ItemIndex.
    // Previously they were greyed out.
    if button(ui, 1) {
        form.remove_selected();
    }
    if button(ui, 2) {
        form.move_selected(-1);
    }
    if button(ui, 3) {
        form.move_selected(1);
    }
    let total_left = 22.0 + 4.0 * (button_width + 6.0);
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(total_left, 267.0, width - total_left - 15.0, 15.0)).layout(egui::Layout::right_to_left(egui::Align::Min)), |ui| {
        ui.label(form.size_info(language));
    });
    super::original_dialogs::access_checkbox(ui, at(16.0, 317.0, width - 2.0 * confirm_width - 46.0, 17.0), &mut form.join_text, &check);
    if ui.put(at(width - 21.0 - 2.0 * confirm_width, 313.0, confirm_width, 25.0), egui::Button::new(ok)).clicked() {
        choice = Some(0);
    }
    if ui.put(at(width - 15.0 - confirm_width, 313.0, confirm_width, 25.0), egui::Button::new(cancel)).clicked() {
        choice = Some(1);
    }
    if enter {
        choice = Some(0);
    }
    choice
}

/// Characters of Path.GetInvalidFileNameChars that may appear in a name (including control characters).
fn invalid_file_name(name: &str) -> bool {
    name.chars().any(|c| c < ' ' || "\"<>|:*?\\/".contains(c))
}

impl App {
    /// SplitFileWindow.Confirm / JoinFilesWindow.Commit: closes with the result when valid, otherwise shows a warning box with the window title and stays.
    pub(super) fn submit_split_or_join(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.original_dialogs.last() else { return };
        let title = dialog.title.clone();
        let outcome = match &dialog.kind {
            DialogKind::Split(form) => match ndr::split::parse_split_size(&form.size) {
                None => Err(self.language_pack.text("NDR.OP_SPLIT_ENTER_A_POSITIVE_SPLIT_SIZE_SUCH_AS_1440_KB", "Enter a positive split size, such as 1440 KB or 100 MB.")),
                Some(_) if form.path.trim().is_empty() => Err(self.language_pack.text("NDR.OP_SPLIT_ENTER_AN_OUTPUT_FOLDER", "Enter an output folder.")),
                Some(size) => match std::path::absolute(form.path.trim()) {
                    Err(_) => Err(self.language_pack.text("NDR.OP_SPLIT_ENTER_A_VALID_OUTPUT_FOLDER", "Enter a valid output folder.")),
                    Ok(folder) if !folder.is_dir() => Err(self.language_pack.text("NDR.OP_SPLIT_THE_OUTPUT_FOLDER_DOES_NOT_EXIST", "The output folder does not exist.")),
                    Ok(folder) => Ok(DialogResult::Split(Some((folder, size, ndr::split::part_count(form.length, size))))),
                },
            },
            DialogKind::Join(form) => {
                let file_name = Path::new(&form.name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if form.files.is_empty() {
                    Err(self.language_pack.text("NDR.OP_SPLIT_ADD_AT_LEAST_ONE_FILE_TO_JOIN", "Add at least one file to join."))
                } else if form.name.trim().is_empty() || form.name.chars().any(|c| c < ' ' || c == '|' || c == '"' || c == '<' || c == '>') || file_name.is_empty() || invalid_file_name(&file_name) {
                    Err(self.language_pack.text("NDR.OP_SPLIT_ENTER_A_VALID_OUTPUT_FILE_NAME", "Enter a valid output file name."))
                } else {
                    Ok(DialogResult::Join(Some(JoinOptions { files: form.files.iter().map(|(p, _)| p.clone()).collect(), join_text: form.join_text, name: form.name.clone(), overwrite_confirmed: false })))
                }
            }
            _ => return,
        };
        match outcome {
            Ok(result) => self.answer_dialog(result, ctx),
            Err(message) => self.system_box(&title, &message, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        }
    }

    /// Original miSplitFileClick (0x43b8b8): opens TfFileSplit only when the active pane is in local mode and the cursor item is a file (kind 3) (otherwise nothing
    /// happens). On OK, splits with a progress window and writes a .crc next to the first piece.
    pub(super) fn split_command(&mut self) {
        let i = self.active;
        let title = self.language_pack.text("formFileSplit.Form.Caption", "Split File");
        let pane = &self.panes[i];
        if pane.archive.is_some() || pane.remote.is_some() {
            return;
        }
        let Some(file) = pane.entries.get(pane.cursor).filter(|e| !e.is_dir && !e.drive && !e.is_parent()).map(|e| e.path.clone()) else { return };
        let length = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        let form = SplitForm {
            path: file.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            size: ndr::split::default_split_size(length).into(),
            source: file.clone(),
            length,
        };
        self.push_original_dialog(title.clone(), DialogKind::Split(form), move |app, result, _| {
            let DialogResult::Split(Some((destination, size, _))) = result else { return };
            let tx = app.async_tx.clone();
            let source = file.clone();
            // C# Run(sourcePath: source folder, destinationPath: output folder, totalFiles: piece count, totalBytes: source size).
            let parts = if size == 0 { None } else { Some(length.div_ceil(size).max(1)) };
            let progress = super::file_operation::ProgressInfo {
                from: source.parent().map(|p| p.to_string_lossy().into_owned()),
                to: Some(destination.to_string_lossy().into_owned()),
                total_files: parts,
                total_bytes: Some(length),
                prefer_bytes: true,
                caption: Some(app.language_pack.text("RES.SPLITTING", "Splitting a file...")),
            };
            let language = app.language_pack.clone();
            let started = app.start_progress_task(title.clone(), progress, move |events, cancel| {
                let outcome = ndr::split::split_file_with_locale(&source, size, &destination, &cancel, &mut |files, bytes, path| {
                    let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes, files });
                }, Some(&language));
                let _ = tx.send(AsyncResult::SplitDone { source, outcome, cancelled: cancel.load(Ordering::Relaxed) });
                Ok(())
            });
            // split_done writes the status text. Even if the end notice (Finished) arrives after SplitDone, it does not overwrite it with the generic "title: N files" text.
            if let Some(job) = app.job.as_mut().filter(|_| started) {
                job.custom_status = true;
            }
        });
    }

    pub(super) fn split_done(&mut self, source: PathBuf, outcome: Result<ndr::split::SplitResult, String>, cancelled: bool) {
        let title = self.language_pack.text("formFileSplit.Form.Caption", "Split File");
        let result = match outcome {
            Err(_) if cancelled => {
                self.status = self.language_pack.text("NDR.OP_SPLIT_FILE_SPLIT_CANCELED", "File split canceled.");
                return;
            }
            Err(error) => return self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
            Ok(result) => result,
        };
        if let Some(first) = result.parts.first() {
            if let Err(error) = ndr::split::write_split_metadata(first, &source, result.source_size, &result.crc32) {
                self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
        }
        let count = ndr::parity::format_size(result.parts.len() as u64, "B", true).trim_end_matches(" B").to_owned();
        self.status = super::file_commands::operation_format(&self.language_pack, "NDR.OP_SPLIT_CREATED_PARTS", "Created {0} parts.", &[&count]);
        self.refresh(self.active);
        self.completion_notice_operation(self.language_pack.text("NDR.OP_SPLIT_FILE_SPLIT_COMPLETED", "File split completed"), self.status.clone());
    }

    /// C# JoinPartsAsync: the chosen files (with one, all numbered pieces). A relative name is joined to the current folder. When the name exists
    /// (and overwriting was not agreed), it reports instead of joining.
    /// Original miJoinFileClick (0x43bec8): opens TfFileJoin only when the active pane is in local mode and there are data items (marks, otherwise the cursor item, FUN_00485bbc)
    /// (otherwise nothing happens). lbFiles holds those items as is; on OK the marks are saved as the last selection and cleared.
    pub(super) fn join_command(&mut self) {
        let i = self.active;
        if self.panes[i].archive.is_some() || self.panes[i].remote.is_some() {
            return;
        }
        let inputs: Vec<PathBuf> = self.command_targets(i).into_iter().filter(|p| !p.is_dir()).collect();
        if inputs.is_empty() {
            return;
        }
        let base = self.command_directory(i);
        let mut form = JoinForm { files: vec![], selected: None, join_text: false, name: ndr::split::suggest_joined_file_name(&inputs[0]) };
        for input in &inputs {
            form.add(input);
        }
        let title = self.language_pack.text("formFileJoin.Form.Caption", "Join Files");
        self.push_original_dialog(title.clone(), DialogKind::Join(form), move |app, result, _| {
            let DialogResult::Join(Some(options)) = result else { return };
            // Original: on OK, CopySelection (last selection) + ClearSelection.
            app.original_take_pane_marks(i);
            let typed = Path::new(&options.name);
            let destination = std::path::absolute(if typed.is_absolute() { typed.to_path_buf() } else { base.join(typed) }).unwrap_or_else(|_| base.join(typed));
            if (destination.is_file() && !options.overwrite_confirmed) || destination.is_dir() {
                let text = super::file_commands::operation_format(&app.language_pack, "NDR.OP_JOIN_EXISTS", "A file with this name already exists. Nothing was joined.\n{0}", &[&destination.display()]);
                return app.system_box(&title, &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
            let tx = app.async_tx.clone();
            let task_title = app.language_pack.text("formFileJoin.Form.Caption", if app.language.eq_ignore_ascii_case("ko") { "파일 합치기" } else { "Join Files" });
            // Resolve the numbered sequence in the worker, before publishing any
            // totals. A single selected .001 can stand for many input files.
            let progress = super::file_operation::ProgressInfo {
                from: options.files.first().and_then(|f| super::file_operation::source_folder(f)),
                to: destination.parent().map(|p| p.to_string_lossy().into_owned()),
                total_files: None,
                total_bytes: None,
                prefer_bytes: true,
                caption: Some(app.language_pack.text("RES.JOINING", "Joining files...")),
            };
            let language = app.language_pack.clone();
            let started = app.start_progress_task(task_title, progress, move |events, cancel| {
                // A single numbered piece (".001", ".part1") joins all of its pieces in order, as the original does (joining big.001 alone
                // rebuilt the whole big.bin). Previously only that one piece was copied.
                let files = match options.files.as_slice() {
                    [only] => {
                        let parts = ndr::split::find_numbered_parts(only);
                        if parts.len() > 1 && ndr::split::has_continuous_numbered_parts(&parts) { parts } else { options.files.clone() }
                    }
                    _ => options.files.clone(),
                };
                let count = files.len();
                let totals = super::file_operation::flat_totals(&files);
                let _ = events.send(Event::Totals { files: Some(count as u64), bytes: totals.map(|t| t.1) });
                let outcome = ndr::split::join_ordered_files_with_locale(&files, &destination, options.join_text, options.overwrite_confirmed, &cancel, &mut |files, bytes, path| {
                    let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes, files });
                }, Some(&language));
                let _ = tx.send(AsyncResult::JoinDone { count, outcome: outcome.map(|_| ()), cancelled: cancel.load(Ordering::Relaxed) });
                Ok(())
            });
            // join_done writes the status text (same reason as the split above).
            if let Some(job) = app.job.as_mut().filter(|_| started) {
                job.custom_status = true;
            }
        });
    }

    pub(super) fn join_done(&mut self, count: usize, outcome: Result<(), String>, cancelled: bool) {
        match outcome {
            Err(_) if cancelled => self.status = self.language_pack.text("NDR.OP_SPLIT_FILE_JOIN_CANCELED", "File join canceled."),
            Err(error) => {
                let title = self.language_pack.text("formFileJoin.Form.Caption", "Join Files");
                self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
            Ok(()) => {
                let count = ndr::parity::format_size(count as u64, "B", true).trim_end_matches(" B").to_owned();
                self.status = super::file_commands::operation_format(&self.language_pack, "NDR.OP_JOIN_CREATED_FILES", "Joined {0} files.", &[&count]);
                self.refresh(self.active);
                self.completion_notice_operation(self.language_pack.text("NDR.OP_SPLIT_FILE_JOIN_COMPLETED", "File join completed"), self.status.clone());
            }
        }
    }
}
