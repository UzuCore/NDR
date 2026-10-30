//! C# ListExportWindow + SaveListAsync(파일 목록 저장하기). 규칙은 ndr::list_export.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::list_export::{self as export, ExportRow};
use ndr::msgbox::{MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_OK};
use std::fs;

#[derive(Clone, Debug)]
pub(super) struct ListExportForm {
    pub rows: Vec<ExportRow>,
    pub columns: [bool; 7],
    pub thousands: bool,
    /// 0 탭, 1 쉼표, 2 그 밖.
    pub delimiter: usize,
    pub custom: String,
    pub preview: String,
    pub complete: bool,
    pub edited: bool,
}

/// 저장할 때 쓸 값: 고친 미리보기(완전할 때만) 또는 전체 내보내기.
#[derive(Clone, Debug)]
pub(super) struct ListExportOptions {
    pub text: String,
    pub count: usize,
}

impl ListExportForm {
    pub(super) fn new(rows: Vec<ExportRow>) -> Self {
        let mut form = Self { rows, columns: export::COLUMNS.map(|c| c.3), thousands: false, delimiter: 0, custom: String::new(), preview: String::new(), complete: true, edited: false };
        form.rebuild();
        form
    }
    pub(super) fn delimiter_text(&self) -> String {
        match self.delimiter {
            1 => ",".into(),
            2 => self.custom.clone(),
            _ => "\t".into(),
        }
    }
    pub(super) fn selected(&self) -> Vec<&'static str> {
        export::COLUMNS.iter().zip(self.columns).filter(|(_, on)| *on).map(|(c, _)| c.0).collect()
    }
    /// RebuildPreview: 바꾸면 고친 내용은 버린다.
    pub(super) fn rebuild(&mut self) {
        let (text, complete) = export::preview(&self.rows, &self.selected(), &self.delimiter_text(), self.thousands);
        self.preview = text;
        self.complete = complete;
        self.edited = false;
    }
}

fn text(language: &compat::Language, key: &str, fallback: &str) -> String {
    language.text(&format!("formSaveList.{key}"), fallback)
}

/// 창 그리기. Some(0) 저장, Some(1) 취소.
pub(super) fn list_export_ui(ui: &mut egui::Ui, form: &mut ListExportForm, language: &compat::Language, ko: bool) -> Option<usize> {
    let mut choice = None;
    let mut changed = false;
    // 원본 TfSaveList(ClientWidth 548 × ClientHeight 461): gbColumns(16,8 305×121: chkFileName(20,28, 늘 켜짐)·chkExtension(141,28)·
    // chkFileSize(20,48)·chkSizeComma(141,48)·chkDate(20,69)·chkTime(141,69)·chkAttributes(20,92)·chkDescription(141,92)),
    // rgDelimiter(331,8 201×121: 탭·쉼표·그 밖) eUserDelimiter(403,97 101×21), mPreview(16,143 516×270), lHelp(16,418),
    // btnSave(368,427) btnCancel(458,427) 75×25.
    let (area, _) = ui.allocate_exact_size(egui::vec2(548.0, 461.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    use super::original_dialogs::put_left;
    super::original_dialogs::group_box(ui, at(16.0, 8.0, 305.0, 121.0), &text(language, "GroupBox.gbColumns", "Columns to Save"));
    let places = [(20.0, 28.0), (141.0, 28.0), (20.0, 48.0), (20.0, 69.0), (141.0, 69.0), (20.0, 92.0), (141.0, 92.0)];
    form.columns[0] = true;
    for (index, (_, key, fallback, _)) in export::COLUMNS.iter().enumerate() {
        let (x, y) = places[index];
        let rect = at(16.0 + x, 8.0 + y, 120.0, 17.0);
        if index == 0 {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.add_enabled(false, egui::Checkbox::new(&mut true, text(language, key, fallback)));
            });
        } else {
            changed |= put_left(ui, rect, egui::Checkbox::new(&mut form.columns[index], text(language, key, fallback))).changed();
        }
    }
    changed |= put_left(ui, at(157.0, 56.0, 161.0, 17.0), egui::Checkbox::new(&mut form.thousands, text(language, "Caption.chkSizeComma", "Comma in Size"))).changed();
    super::original_dialogs::group_box(ui, at(331.0, 8.0, 201.0, 121.0), &text(language, "RadioGroup.rgDelimiter", "Delimiter"));
    let labels = text(language, "RadioGroupItems.rgDelimiter", "Tab;Comma;Etc.");
    let labels: Vec<&str> = labels.split(';').collect();
    for (index, (fallback, top)) in [("Tab", 33.0), ("Comma", 66.0), ("Other", 99.0)].into_iter().enumerate() {
        let rect = at(339.0, top, if index == 2 { 62.0 } else { 185.0 }, 17.0);
        changed |= put_left(ui, rect, egui::RadioButton::new(form.delimiter == index, *labels.get(index).unwrap_or(&fallback))).clicked().then(|| form.delimiter = index).is_some();
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
            let edit = egui::TextEdit::multiline(&mut form.preview).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).frame(false).interactive(form.complete);
            if ui.add(edit).changed() {
                form.edited = true;
            }
        });
    });
    put_left(ui, at(16.0, 418.0, 340.0, 15.0), egui::Label::new(text(language, "Caption.lHelp", "You can edit before saving the list.")).truncate());
    if !form.complete {
        // C# 판의 안내(원본에는 없음): 미리보기는 일부만 보이고 편집할 수 없다.
        let sample = form.rows.len().min(export::SAMPLE_ROW_LIMIT);
        let total = form.rows.len();
        let note = if ko {
            format!("미리보기에는 전체 {total}개 중 처음 {sample}개만 표시됩니다. 저장하면 전체 항목을 내보내며 미리보기 편집은 비활성화됩니다.")
        } else {
            format!("Preview shows the first {sample} of {total} items. Saving exports all items; editing is disabled.")
        };
        put_left(ui, at(16.0, 436.0, 340.0, 15.0), egui::Label::new(RichText::new(note).small().weak()).truncate());
    }
    if ui.put(at(368.0, 427.0, 75.0, 25.0), egui::Button::new(text(language, "Caption.btnSave", "Save"))).clicked() {
        choice = Some(0);
    }
    if ui.put(at(458.0, 427.0, 75.0, 25.0), egui::Button::new(text(language, "Caption.btnCancel", "Cancel"))).clicked() {
        choice = Some(1);
    }
    choice
}

impl App {
    /// 저장 단추: 열이 없거나 사용자 구분자가 비면 안내 상자(창 제목, 정보 아이콘)를 띄우고 남긴다.
    pub(super) fn submit_list_export(&mut self, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let Some(dialog) = self.original_dialogs.last() else { return };
        let DialogKind::ListExport(form) = &dialog.kind else { return };
        let problem = if form.selected().is_empty() {
            Some(if ko { "저장할 항목을 하나 이상 선택하세요." } else { "Select at least one column to save." })
        } else if form.delimiter == 2 && form.custom.is_empty() {
            Some(if ko { "사용자 지정 구분자를 입력하세요." } else { "Enter a custom delimiter." })
        } else {
            None
        };
        if let Some(message) = problem {
            let title = dialog.title.clone();
            return self.system_box(&title, message, MB_OK | MB_ICONASTERISK, |_, _, _| {});
        }
        let text = if form.edited && form.complete { form.preview.clone() } else { export::export_text(&form.rows, &form.selected(), &form.delimiter_text(), form.thousands) };
        let count = form.rows.len();
        self.answer_dialog(DialogResult::ListExport(Some(ListExportOptions { text, count })), ctx);
    }

    /// C# SaveListAsync: 활성 패널 목록 → 창 → 저장 대화 상자(.txt, 덮어쓰기 확인) → UTF-8 BOM으로 쓰기.
    /// 차이: Rust 목록에는 ".." 줄이 없어 내보내지 않는다(C#은 상위 폴더 줄도 내보냄).
    pub(super) fn save_list_command(&mut self) {
        let i = self.active;
        let pane = &self.panes[i];
        let rows: Vec<ExportRow> = pane
            .entries
            .iter()
            .enumerate()
            .map(|(row, e)| ExportRow {
                name: e.name.clone(),
                is_dir: e.is_dir,
                size: e.size,
                modified: e.modified,
                attributes: pane.view.cells.get(row).map(|c| c[DetailColumn::Attributes.index()].clone()).unwrap_or_else(|| ndr::listing::attribute_text(e.attributes)),
                description: pane.descriptions.get(&e.name.to_lowercase()).cloned().unwrap_or_default(),
            })
            .collect();
        let folder = (pane.archive.is_none() && pane.path.is_dir()).then(|| pane.path.clone());
        let title = self.language_pack.text("formSaveList.Form.Caption", "Save File List");
        self.push_original_dialog(title.clone(), DialogKind::ListExport(Box::new(ListExportForm::new(rows))), move |app, result, _| {
            let DialogResult::ListExport(Some(options)) = result else { return };
            let documents = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Documents");
            // C# Filter "텍스트 파일 (*.txt)|*.txt"(영어 "Text files (*.txt)"), DefaultExt ".txt", AddExtension.
            let filter = if app.language == "ko" { "텍스트 파일 (*.txt)" } else { "Text files (*.txt)" };
            let Some(mut path) = (app.save_dialog)(&title, Some(folder.as_deref().unwrap_or(&documents)), "", &[(filter, "*.txt")], "txt", true) else { return };
            if path.extension().is_none() {
                path.set_extension("txt");
            }
            let bytes = [&[0xEF, 0xBB, 0xBF][..], options.text.as_bytes()].concat();
            match fs::write(&path, bytes) {
                Ok(()) => {
                    let count = ndr::parity::format_size(options.count as u64, "B", true).trim_end_matches(" B").to_owned();
                    app.status = if app.language == "ko" { format!("목록 {count}개 항목을 저장했습니다.") } else { format!("Saved {count} items.") };
                }
                Err(error) => app.system_box(&title, &error.to_string(), MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
            }
        });
    }
}
