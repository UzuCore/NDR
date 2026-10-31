//! Original TfSaveList (Save File List) and miSaveListClick (0x437dbc). Line rules are in ndr::list_export.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::list_export::{self as export, Columns, ExportRow};
use ndr::msgbox::{MB_ICONEXCLAMATION, MB_OK};
use std::fs;

#[derive(Clone, Debug)]
pub(super) struct ListExportForm {
    pub rows: Vec<ExportRow>,
    pub columns: Columns,
    /// rgDelimiter.ItemIndex: 0 tab, 1 comma, 2 eUserDelimiter.
    pub delimiter: usize,
    pub custom: String,
    /// mPreview (editable).
    pub preview: String,
}

impl ListExportForm {
    pub(super) fn new(rows: Vec<ExportRow>) -> Self {
        let mut form = Self { rows, columns: Columns::default(), delimiter: 0, custom: String::new(), preview: String::new() };
        form.rebuild();
        form
    }
    /// FUN_004519a0: refills the preview whenever columns or the delimiter change (cbShowItemClick, rgDelimiterClick, eUserDelimiterChange).
    /// Edits are discarded.
    pub(super) fn rebuild(&mut self) {
        self.preview = export::preview(&self.rows, &self.columns, &export::delimiter(self.delimiter, &self.custom));
    }
}

fn text(language: &compat::Language, key: &str, fallback: &str) -> String {
    language.text(&format!("formSaveList.{key}"), fallback)
}

/// Draws the window. Some(0) save (btnSave), Some(1) cancel (btnCancel).
pub(super) fn list_export_ui(ui: &mut egui::Ui, form: &mut ListExportForm, language: &compat::Language) -> Option<usize> {
    let mut choice = None;
    let mut changed = false;
    // Original TfSaveList (ClientWidth 548 x ClientHeight 461): gbColumns (16,8 305x121: chkFileName (20,28, always on, disabled button),
    // chkExtension(141,28)·chkFileSize(20,48)·chkSizeComma(141,48)·chkDate(20,69)·chkTime(141,69)·chkAttributes(20,92)·
    // chkDescription (141,92)), rgDelimiter (331,8 201x121: tab, comma, other) eUserDelimiter (403,97 101x21, enabled only for other),
    // mPreview (16,143 516x270, scrolls both ways), lHelp (16,418), btnSave (368,427) btnCancel (458,427) 75x25.
    let (area, _) = ui.allocate_exact_size(egui::vec2(548.0, 461.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    use super::original_dialogs::put_left;
    super::original_dialogs::group_box(ui, at(16.0, 8.0, 305.0, 121.0), &text(language, "GroupBox.gbColumns", "Columns to Save"));
    let inside = |x: f32, y: f32, w: f32| at(16.0 + x, 8.0 + y, w, 17.0);
    let raw = |key: &str, fallback: &str| language.raw(&format!("formSaveList.{key}")).unwrap_or_else(|| fallback.to_owned());
    ui.scope_builder(egui::UiBuilder::new().max_rect(inside(20.0, 28.0, 120.0)), |ui| {
        ui.add_enabled(false, egui::Checkbox::new(&mut true, text(language, "Caption.chkFileName", "Filename")));
    });
    let columns = &mut form.columns;
    for (value, x, y, w, key, fallback) in [
        (&mut columns.extension, 141.0, 28.0, 140.0, "Caption.chkExtension", "&Extension"),
        (&mut columns.size, 20.0, 48.0, 120.0, "Caption.chkFileSize", "&Size"),
        (&mut columns.size_comma, 141.0, 48.0, 161.0, "Caption.chkSizeComma", "&Comma in Size"),
        (&mut columns.date, 20.0, 69.0, 120.0, "Caption.chkDate", "&Date"),
        (&mut columns.time, 141.0, 69.0, 120.0, "Caption.chkTime", "&Time"),
        (&mut columns.attributes, 20.0, 92.0, 120.0, "Caption.chkAttributes", "&Attributes"),
        (&mut columns.description, 141.0, 92.0, 120.0, "Caption.chkDescription", "Comme&nt"),
    ] {
        // While the preview memo has focus, letters are memo text and only Alt+letter is an access key.
        changed |= super::original_dialogs::access_checkbox(ui, inside(x, y, w), value, &raw(key, fallback));
    }
    super::original_dialogs::group_box(ui, at(331.0, 8.0, 201.0, 121.0), &text(language, "RadioGroup.rgDelimiter", "Delimiter"));
    let labels = text(language, "RadioGroupItems.rgDelimiter", "Tab;Comma;Other:");
    let labels: Vec<&str> = labels.split(';').collect();
    for (index, (fallback, top)) in [("Tab", 33.0), ("Comma", 66.0), ("Other:", 99.0)].into_iter().enumerate() {
        let rect = at(339.0, top, if index == 2 { 62.0 } else { 185.0 }, 17.0);
        if put_left(ui, rect, egui::RadioButton::new(form.delimiter == index, *labels.get(index).unwrap_or(&fallback))).clicked() && form.delimiter != index {
            form.delimiter = index;
            changed = true;
        }
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(403.0, 97.0, 101.0, 21.0)), |ui| {
        changed |= ui.add_enabled(form.delimiter == 2, egui::TextEdit::singleline(&mut form.custom).desired_width(101.0)).changed();
    });
    if changed {
        form.rebuild();
    }
    let preview = at(16.0, 143.0, 516.0, 270.0);
    ui.painter().rect(preview, 0.0, ui.visuals().extreme_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(preview.shrink(2.0)), |ui| {
        egui::ScrollArea::both().id_salt("list-export-preview").auto_shrink([false, false]).show(ui, |ui| {
            // ScrollBars ssBoth: lines are not wrapped (WordWrap is off when there is a horizontal scroll bar).
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
                let font = egui::TextStyle::Monospace.resolve(ui.style());
                let job = egui::text::LayoutJob::simple(text.as_str().to_owned(), font, ui.visuals().text_color(), f32::INFINITY);
                ui.fonts_mut(|f| f.layout_job(job))
            };
            ui.add(egui::TextEdit::multiline(&mut form.preview).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).frame(false).layouter(&mut layouter));
        });
    });
    put_left(ui, at(16.0, 418.0, 340.0, 15.0), egui::Label::new(text(language, "Caption.lHelp", "You can edit before saving the list")).truncate());
    if ui.put(at(368.0, 427.0, 75.0, 25.0), egui::Button::new(text(language, "Caption.btnSave", "Save"))).clicked() {
        choice = Some(0);
    }
    if ui.put(at(458.0, 427.0, 75.0, 25.0), egui::Button::new(text(language, "Caption.btnCancel", "Cancel"))).clicked() {
        choice = Some(1);
    }
    choice
}

impl App {
    /// Original btnSaveClick (0x452030): when the preview has lines, opens dlgSave (DefaultExt txt, Filter "*.txt", overwrite prompt)
    /// and on choice Lines.SaveToFile(TEncoding.Unicode). Closing the dialog still closes the window as confirmed (ModalResult 1).
    pub(super) fn submit_list_export(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.original_dialogs.last() else { return };
        let DialogKind::ListExport(form) = &dialog.kind else { return };
        if let Some(bytes) = export::file_bytes(&form.preview) {
            // DFM dlgSave.Title is only the Korean "파일 목록 저장". Fixed: English uses the window title (Save File List).
            let title = if self.language == "ko" { "파일 목록 저장".to_owned() } else { dialog.title.clone() };
            if let Some(mut path) = (self.save_dialog)(&title, None, "", &[("*.txt", "*.txt")], "txt", true) {
                if path.extension().is_none() {
                    path.set_extension("txt");
                }
                if let Err(error) = fs::write(&path, bytes) {
                    let caption = dialog.title.clone();
                    self.system_box(&caption, &error.to_string(), MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                }
            }
        }
        self.answer_dialog(DialogResult::ListExport(Some(())), ctx);
    }

    /// Original miSaveListClick (0x437dbc): without marks passes the items of SelectAll (with folders, without drives and ".."), otherwise the marked items,
    /// to the window. When it closes confirmed, the marks are saved as the "last selection" and cleared (CopySelection + ClearSelection). Without marks
    /// only the temporary selection is cleared (the list marks stay).
    pub(super) fn save_list_command(&mut self) {
        let i = self.active;
        let pane = &self.panes[i];
        let marked = !pane.selected.is_empty();
        let mut rows: Vec<usize> = if marked { pane.selected.iter().copied().collect() } else { (0..pane.entries.len()).collect() };
        rows.sort_unstable();
        let rows: Vec<ExportRow> = rows
            .into_iter()
            .filter_map(|row| pane.entries.get(row))
            .filter(|e| !e.drive && !e.is_parent())
            .map(|e| {
                let size = if e.is_dir { pane.folder_bytes.get(&e.path).copied().unwrap_or(e.size) } else { e.size };
                let description = pane.descriptions.get(&e.name.to_lowercase()).cloned().unwrap_or_default();
                ExportRow::new(&e.name, e.is_dir, size, e.modified, e.attributes, description)
            })
            .collect();
        let title = self.language_pack.text("formSaveList.Form.Caption", "Save File List");
        self.push_original_dialog(title, DialogKind::ListExport(Box::new(ListExportForm::new(rows))), move |app, result, _| {
            if marked && matches!(result, DialogResult::ListExport(Some(()))) {
                app.original_take_pane_marks(i);
            }
        });
    }
}
