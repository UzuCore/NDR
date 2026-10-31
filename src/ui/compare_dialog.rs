//! C# CompareAsync / SynchronizeFoldersAsync (MainWindow.ParityFeatures.cs) and FolderComparisonOptionsWindow.
//! Folder comparison is the single original TfCompareFolder window (FolderCompareForm, btnOkClick 0x41d464, btnSaveClick 0x41e2d0).
//! Synchronization follows the C# flow: FolderComparisonProgressWindow, FolderComparisonWindow (sync preview). Rules are in ndr::folder_compare.
//! The modal progress window (modal_progress_ui) is also used by search (SearchProgressWindow).
//! Difference (HANDOFF): a folder present on one side only has kind "folder" (C# wrongly shows "file/folder conflict"); on an English UI the window text and saved results
//! are English too; the sync progress window has no pause button.
use super::grid::{self, Column, GridState, GridView};
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::folder_compare::{self as fc, Difference, DifferenceKind, ExistingBehavior};
use ndr::msgbox::{IDYES, MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_ICONHAND, MB_ICONQUESTION, MB_OK, MB_YESNO};
use std::fs;

/// FolderComparisonWindow column widths (copy, state, kind, relative path (rest), two sizes, two dates).
const COLUMN_WIDTHS: [f32; 8] = [54.0, 120.0, 64.0, 0.0, 95.0, 95.0, 145.0, 145.0];
/// Error rows shown in the TextWindow (C# Take(500)).
const ERROR_LINES: usize = 500;

/// .NET "N0".
fn n0(value: usize) -> String {
    ndr::parity::format_size(value as u64, "B", true).trim_end_matches(" B").to_owned()
}

const DISTINCT_KO: &str = "비교하거나 동기화할 두 폴더는 서로 같거나 포함 관계일 수 없습니다.";
const DISTINCT_EN: &str = "The two folders to compare or synchronize cannot be the same folder or contain each other.";

/// Maps Korean errors of the comparison service to an English UI (known texts only).
fn localized_error(error: &str, ko: bool) -> String {
    if ko {
        return error.into();
    }
    if error == DISTINCT_KO {
        return DISTINCT_EN.into();
    }
    if let Some(path) = error.strip_prefix("폴더를 찾을 수 없습니다: ") {
        return format!("Folder not found: {path}");
    }
    error.into()
}

/// Original result code (lvList row SubItems.Objects[1], 0x41d464): 0 same, 1 different, 2 left only, 3 right only.
pub(super) fn original_code(kind: DifferenceKind) -> u8 {
    match kind {
        DifferenceKind::Same => 0,
        DifferenceKind::Different => 1,
        DifferenceKind::LeftOnly => 2,
        DifferenceKind::RightOnly => 3,
    }
}

/// Original comparison result column text (0x8de484 "=", 0x8de47c "< >", 0x8de460 "<  ", 0x8de46c/0x8de48a "  >").
pub(super) fn result_mark(kind: DifferenceKind) -> &'static str {
    match kind {
        DifferenceKind::Same => "=",
        DifferenceKind::Different => "< >",
        DifferenceKind::LeftOnly => "<  ",
        DifferenceKind::RightOnly => "  >",
    }
}

/// Original lvList row (0x41d464): cell 1 = left list name, cell 2 = result text, cell 3 = right list name. A missing side is empty.
pub(super) fn folder_compare_cell(d: &Difference, column: usize) -> String {
    match column {
        1 if d.kind != DifferenceKind::RightOnly => d.relative.clone(),
        2 => result_mark(d.kind).to_owned(),
        3 if d.kind != DifferenceKind::LeftOnly => d.right_relative.clone(),
        _ => String::new(),
    }
}

/// Header text of original btnSaveClick (0x41e2d0) (0x8de604, one line with tabs).
const REPORT_HEAD: &str = "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//ENhttp://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\">\t<html xmlns=\"http://www.w3.org/1999/xhtml\">\t<head>\t<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\" />\t<title>NexusFile Folder Compare</title>\t<style type=\"text/css\">\tbody,td {font-family:Tahoma,Arial,Verdana;font-size:9pt}\t.onlyleft {color:#008000;padding:4px}\t.onlyright {color:#0000FF;padding:4px}\t.same {color:#666666;padding:4px}\t.diff {color:#FF0000;padding:4px}\t.line1 {background-color:#FFFFFF}\t.line2 {background-color:#F1F5FA}\t.header {background-color:#666666;color:#FFFFFF}\t.headleft {padding-top:2px;padding-bottom:2px;padding-left:12px}\t.headright {padding-top:2px;padding-bottom:2px;padding-right:12px}\t</style>\t</head>\t<body>\t<div style=\"border:1px solid silver;padding:4px;position:absolute\">\t<table border=\"0\" cellspacing=\"0\" cellpadding=\"0\">\t";

/// Name placed in HTML (the original inserts it as is; NDR escapes &, <, >).
fn html_text(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Original lvList column widths (lvListResize 0x41e210): column 0 is 30, the result column (DFM 100, 80 when narrower than 30), the two name columns half of the rest each.
pub(super) const LIST_CLIENT_WIDTH: f32 = 617.0;
pub(super) const RESULT_COLUMN_WIDTH: f32 = 100.0;

/// Comparison job (background thread): stop request and progress percentage (original (left + right index) x 100 / total).
#[derive(Clone, Default)]
pub(super) struct CompareTask {
    pub cancel: std::sync::Arc<AtomicBool>,
    pub percent: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

/// State of the original TfCompareFolder (ClientWidth 640 x ClientHeight 505).
/// left/right are the folders compared; headers are the paths without the trailing "\" like original FUN_0041d280 (lvList Columns[1]/[3].Caption).
pub(super) struct FolderCompareForm {
    pub left: PathBuf,
    pub right: PathBuf,
    pub left_caption: String,
    pub right_caption: String,
    /// chkRecursive (DFM off).
    pub recursive: bool,
    /// chkCompareContents (DFM Checked; NDR takes the initial value from C# UseContentComparison).
    pub contents: bool,
    /// lvList rows (original item order: the two lists aligned by name).
    pub rows: Vec<Difference>,
    pub grid: GridState,
    pub running: Option<CompareTask>,
    /// btnSave.Enabled (DFM False, enabled after one comparison).
    pub compared: bool,
    /// lResult text (FOLDERCOMPARE_RESULT). None shows the ProgressBar in its place.
    pub result: Option<String>,
    /// Number of unreadable items (C# version warning, not in the original).
    pub issues: usize,
}

/// TfCompareFolder buttons.
pub(super) enum FolderCompareAction {
    Start,
    Stop,
    Save,
    Close,
}

impl FolderCompareForm {
    pub(super) fn new(left: PathBuf, right: PathBuf, contents: bool) -> Self {
        let caption = |path: &Path| {
            let text = path.to_string_lossy();
            text.strip_suffix('\\').unwrap_or(&text).to_owned()
        };
        Self {
            left_caption: caption(&left),
            right_caption: caption(&right),
            left,
            right,
            recursive: false,
            contents,
            rows: Vec::new(),
            grid: GridState::new(0),
            running: None,
            compared: false,
            result: None,
            issues: 0,
        }
    }

    /// lvList cell text: 0 empty, 1 left name, 2 comparison result, 3 right name (the right list as is; a missing side is empty).
    pub(super) fn cell(&self, row: usize, column: usize) -> String {
        folder_compare_cell(&self.rows[row], column)
    }

    /// Original lvListColumnClick (0x41e098) -> lvListCompare (0x41de00): column 0 does not sort, column 1 left name, column 2 result code then left name,
    /// column 3 right name (lstrcmpW). Always ascending.
    pub(super) fn sort(&mut self, column: usize) {
        if column == 0 || column > 3 {
            return;
        }
        let name = |row: usize, c: usize| self.cell(row, c);
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        order.sort_by(|&a, &b| match column {
            2 => original_code(self.rows[a].kind).cmp(&original_code(self.rows[b].kind)).then_with(|| ndr::listing::compare_text(&name(a, 1), &name(b, 1), false)),
            c => ndr::listing::compare_text(&name(a, c), &name(b, c), false),
        });
        self.grid.order = order;
        self.grid.sort = Some((column, false));
    }

    /// Original FUN_0041e0b4: FOLDERCOMPARE_RESULT with the count per result code (same, different, left only, right only).
    pub(super) fn result_text(&self, template: &str) -> String {
        let mut counts = [0usize; 4];
        for row in &self.rows {
            counts[original_code(row.kind) as usize] += 1;
        }
        counts.iter().fold(template.to_owned(), |text, n| text.replacen("%d", &n.to_string(), 1))
    }

    /// Original btnSaveClick: header text, header row (left and right paths), rows in display order (even line1, odd line2, class per result), footer.
    /// TStringList.SaveToFile(TEncoding.UTF8): CRLF per line; the caller adds the UTF-8 BOM.
    pub(super) fn report_html(&self) -> String {
        let mut lines = vec![
            REPORT_HEAD.to_owned(),
            format!(
                "<tr class=\"header\"><td align=\"right\" class=\"headleft\">{}</td><td>&nbsp;</td><td class=\"headright\">{}</td></tr>",
                html_text(&self.left_caption),
                html_text(&self.right_caption)
            ),
        ];
        for (index, &row) in self.grid.order.iter().enumerate() {
            let class = ["same", "diff", "onlyleft", "onlyright"][original_code(self.rows[row].kind) as usize];
            lines.push(if index % 2 == 0 { "<tr class=\"line1\">" } else { "<tr class=\"line2\">" }.to_owned());
            lines.push(format!("<td align=\"right\" class=\"{class}\">{}</td>", html_text(&self.cell(row, 1))));
            lines.push(format!("<td width=\"50\" align=\"center\" class=\"{class}\">{}</td>", html_text(&self.cell(row, 2))));
            lines.push(format!("<td class=\"{class}\">{}</td>", html_text(&self.cell(row, 3))));
            lines.push("</tr>".to_owned());
        }
        lines.push("</table></div></body>\r\n</html>".to_owned());
        lines.iter().map(|line| format!("{line}\r\n")).collect()
    }
}


/// What to do after the comparison (locals of C# CompareAsync / SynchronizeFoldersAsync).
pub(super) struct CompareRun {
    pub left: PathBuf,
    pub right: PathBuf,
    /// For synchronization, Some(whether the source is the left side).
    pub source_is_left: Option<bool>,
}

/// FolderComparisonRow: one difference, the "copy" checkbox (initially on), display texts (state, kind, relative path, two sizes, two dates).
#[derive(Clone, Debug)]
pub(super) struct ComparisonRow {
    pub difference: Difference,
    pub include: bool,
    pub cells: [String; 7],
}

/// FolderComparisonWindow state. rows keep the original order (saving and syncing use it, like C#); display order and selection live in grid.
/// grid column indices are visible columns (in the sync preview 0 is "copy").
pub(super) struct ComparisonForm {
    pub left: String,
    pub right: String,
    pub rows: Vec<ComparisonRow>,
    pub issues: Vec<String>,
    pub select_for_sync: bool,
    pub grid: GridState,
}

/// Result window buttons and keys.
pub(super) enum ComparisonAction {
    SelectAll,
    SelectNone,
    Save,
    Commit,
    Close,
    /// Ctrl+C: the selected rows (display order) as tab-separated text (DataGrid copy, no header).
    Copy(String),
}

impl ComparisonForm {
    pub(super) fn new(left: String, right: String, differences: Vec<Difference>, select_for_sync: bool, issues: Vec<String>, ko: bool) -> Self {
        let rows: Vec<ComparisonRow> = differences
            .into_iter()
            .map(|d| {
                let cells = [
                    fc::status_text(d.kind, ko).to_owned(),
                    fc::type_text(&d, ko).to_owned(),
                    d.relative.clone(),
                    fc::size_text(d.left_modified, d.left_dir, d.left_size, ko),
                    fc::size_text(d.right_modified, d.right_dir, d.right_size, ko),
                    fc::time_text(d.left_modified),
                    fc::time_text(d.right_modified),
                ];
                ComparisonRow { difference: d, include: true, cells }
            })
            .collect();
        let grid = GridState::new(rows.len());
        Self { left, right, rows, issues, select_for_sync, grid }
    }

    /// Original FUN_0041e0b4: wsprintf(FOLDERCOMPARE_RESULT, same, different, left only, right only).
    pub(super) fn original_result(&self, language: &compat::Language) -> String {
        let count = |kind: DifferenceKind| self.rows.iter().filter(|r| r.difference.kind == kind).count().to_string();
        let mut text = language.text(
            "RES.FOLDERCOMPARE_RESULT",
            "<c666666>Indentical: <b>%d</b></c>, <cFF0000>Different: <b>%d</b></c>, <c008000>Only in left: <b>%d</b></c>, <c0000FF>Only in right: <b>%d</b></c>",
        );
        for value in [count(DifferenceKind::Same), count(DifferenceKind::Different), count(DifferenceKind::LeftOnly), count(DifferenceKind::RightOnly)] {
            text = text.replacen("%d", &value, 1);
        }
        text
    }

    /// UpdateStatus.
    pub(super) fn status(&self, ko: bool) -> String {
        let count = |kind: DifferenceKind| self.rows.iter().filter(|r| r.difference.kind == kind).count();
        let changed = self.rows.len() - count(DifferenceKind::Same);
        let selected = self.rows.iter().filter(|r| r.include).count();
        match (self.select_for_sync, ko) {
            (true, true) => format!("차이 {}개 · 선택 {} / {}개 · 읽지 못함 {}개", n0(changed), n0(selected), n0(self.rows.len()), n0(self.issues.len())),
            (true, false) => format!("{} different · {} / {} selected · {} unreadable", n0(changed), n0(selected), n0(self.rows.len()), n0(self.issues.len())),
            (false, true) => format!("전체 {}개 · 같음 {} · 차이 {}개 · 읽지 못함 {}개", n0(self.rows.len()), n0(count(DifferenceKind::Same)), n0(changed), n0(self.issues.len())),
            (false, false) => format!("{} total · {} same · {} different · {} unreadable", n0(self.rows.len()), n0(count(DifferenceKind::Same)), n0(changed), n0(self.issues.len())),
        }
    }

    /// Visible columns (the "copy" checkbox column only in the sync preview). The relative path column takes the remaining width.
    pub(super) fn columns(&self, ko: bool, width: f32) -> Vec<Column> {
        let titles = fc::column_titles(ko);
        let range = if self.select_for_sync { 0..8 } else { 1..8 };
        let fixed: f32 = range.clone().map(|c| COLUMN_WIDTHS[c]).sum();
        range
            .map(|c| match c {
                0 => Column::check(titles[0], COLUMN_WIDTHS[0]),
                3 => Column::text(titles[3], (width - fixed - 16.0).max(160.0)),
                _ => Column::text(titles[c], COLUMN_WIDTHS[c]),
            })
            .collect()
    }

    /// Text of a visible column index (empty for the checkbox column).
    fn cell(&self, row: usize, column: usize) -> String {
        cell_text(&self.rows, self.select_for_sync, row, column)
    }

    /// DataGrid header sort: by visible text (copy column: off < on).
    pub(super) fn sort_column(&mut self, column: usize) {
        let (rows, sync) = (&self.rows, self.select_for_sync);
        self.grid.toggle_sort(column, |a, b| match (sync, column) {
            (true, 0) => rows[a].include.cmp(&rows[b].include),
            _ => ndr::listing::compare_text(&cell_text(rows, sync, a, column), &cell_text(rows, sync, b, column), false),
        });
    }

    /// Ctrl+C text (selected rows in display order, tabs, CRLF, checkboxes as True/False).
    pub(super) fn copy_text(&self) -> String {
        self.grid.copy_text(&self.columns(true, 0.0), &|row, column| self.cell(row, column), &|row| self.rows[row].include)
    }

    pub(super) fn included(&self) -> Vec<Difference> {
        self.rows.iter().filter(|r| r.include).map(|r| r.difference.clone()).collect()
    }
}

fn cell_text(rows: &[ComparisonRow], sync: bool, row: usize, column: usize) -> String {
    match (sync, column) {
        (true, 0) => String::new(),
        (true, c) => rows[row].cells[c - 1].clone(),
        (false, c) => rows[row].cells[c].clone(),
    }
}

/// Original lvListAdvancedCustomDrawItem (0x41dd6c): Font.Color by result (Data of SubItems 1).
/// Same 0x808080, different clRed, left only clGreen (0x008000), right only clBlue (TColor 0xFF0000 = RGB 0000FF).
pub(super) fn result_color(kind: DifferenceKind) -> Color32 {
    match kind {
        DifferenceKind::Same => Color32::from_rgb(0x80, 0x80, 0x80),
        DifferenceKind::Different => Color32::from_rgb(0xFF, 0x00, 0x00),
        DifferenceKind::LeftOnly => Color32::from_rgb(0x00, 0x80, 0x00),
        DifferenceKind::RightOnly => Color32::from_rgb(0x00, 0x00, 0xFF),
    }
}

/// Original TfCompareFolder (ClientWidth 640 x ClientHeight 505) at the original positions: ProgressBar (8,12 624x16, hidden after comparing) and
/// lResult in its place (12,12, FOLDERCOMPARE_RESULT), lvList (8,44 621x375, white background, columns 30, left path (right aligned), result (centered),
/// right path), chkRecursive (8,425), chkCompareContents (8,448), btnOk/btnStop (390,475) btnSave (471,475) btnCancel (552,475) 75x25.
pub(super) fn folder_compare_ui(ui: &mut egui::Ui, form: &mut FolderCompareForm, language: &compat::Language, text_color: Color32, enter: bool) -> Option<FolderCompareAction> {
    let text = |key: &str, fallback: &str| language.text(&format!("formCompareFolder.{key}"), fallback).replace('&', "");
    let mut action = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(640.0, 505.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let running = form.running.clone();
    match (&form.result, &running) {
        (Some(result), None) => {
            super::original_dialogs::put_left(ui, at(12.0, 12.0, 620.0, 18.0), egui::Label::new(super::original_dialogs::markup_job(result, 12.0, text_color, 620.0)));
        }
        _ => {
            let percent = running.as_ref().map_or(0, |task| task.percent.load(Ordering::Relaxed).min(100));
            ui.put(at(8.0, 12.0, 624.0, 16.0), egui::ProgressBar::new(percent as f32 / 100.0).desired_height(16.0).corner_radius(0.0));
        }
    }
    let side = ((LIST_CLIENT_WIDTH - RESULT_COLUMN_WIDTH - 30.0) / 2.0).floor();
    let result_title = language.text("formCompareFolder.ListViewCols.lvList", ";;Result;").split(';').nth(2).unwrap_or("Result").to_owned();
    let columns = [Column::text("", 30.0), Column::right(form.left_caption.clone(), side), Column::center(result_title, RESULT_COLUMN_WIDTH), Column::text(form.right_caption.clone(), side)];
    let events = ui
        .scope_builder(egui::UiBuilder::new().max_rect(at(8.0, 44.0, 621.0, 375.0)), |ui| {
            ui.painter().rect_stroke(at(8.0, 44.0, 621.0, 375.0), 0.0, egui::Stroke::new(1.0_f32, Color32::from_gray(160)), egui::StrokeKind::Inside);
            let FolderCompareForm { rows, grid: state, .. } = form;
            let rows: &Vec<Difference> = rows;
            let color = |row: usize| Some(result_color(rows[row].kind));
            let view = GridView { id: "folder-compare", columns: &columns, height: 375.0 - 26.0, enabled: true, sortable: true, multi_select: false, keyboard: true, context_menu: None, row_color: Some(&color), background: Some(Color32::WHITE) };
            let cell = |row: usize, column: usize| folder_compare_cell(&rows[row], column);
            grid::show(ui, &view, state, &cell, &|_| false)
        })
        .inner;
    if let Some(column) = events.sort {
        form.sort(column);
    }
    super::original_dialogs::put_left(ui, at(8.0, 425.0, 609.0, 17.0), egui::Checkbox::new(&mut form.recursive, text("Caption.chkRecursive", "Include Subfolders(&R)")));
    super::original_dialogs::put_left(ui, at(8.0, 448.0, 609.0, 17.0), egui::Checkbox::new(&mut form.contents, text("Caption.chkCompareContents", "&Compare Files Contents - If you uncheck this, compare only file time")));
    // btnOk and btnStop share one position (both Default). Only btnStop shows while comparing.
    if running.is_some() {
        if ui.put(at(390.0, 475.0, 75.0, 25.0), egui::Button::new(text("Caption.btnStop", "Stop"))).clicked() || enter {
            action = Some(FolderCompareAction::Stop);
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    } else if ui.put(at(390.0, 475.0, 75.0, 25.0), egui::Button::new(text("Caption.btnOk", "Compare"))).clicked() || enter {
        action = Some(FolderCompareAction::Start);
    }
    let save = form.compared && running.is_none();
    if ui.add_enabled_ui(save, |ui| ui.put(at(471.0, 475.0, 75.0, 25.0), egui::Button::new(text("Caption.btnSave", "Save")))).inner.clicked() {
        action = Some(FolderCompareAction::Save);
    }
    if ui.put(at(552.0, 475.0, 75.0, 25.0), egui::Button::new(text("Caption.btnCancel", "Close"))).clicked() {
        action = Some(FolderCompareAction::Close);
    }
    action
}

/// Draws FolderComparisonWindow: left/right paths, unreadable warning, status line, table (header sort, row selection, copy checkboxes), buttons.
pub(super) fn comparison_ui(ui: &mut egui::Ui, form: &mut ComparisonForm, language: &compat::Language, ko: bool, enter: bool, size: egui::Vec2) -> Option<ComparisonAction> {
    let t = |ko_text: &'static str, en: &'static str| if ko { ko_text } else { en };
    let mut action = None;
    ui.set_width(size.x);
    ui.add(egui::Label::new(format!("{}: {}", t("왼쪽", "Left"), form.left)).truncate());
    ui.add(egui::Label::new(format!("{}: {}", t("오른쪽", "Right"), form.right)).truncate());
    if !form.issues.is_empty() {
        ui.add_space(4.0);
        let n = n0(form.issues.len());
        ui.add(egui::Label::new(if ko { format!("주의: {n}개 항목을 읽지 못했습니다. 비교 결과가 부분적입니다.") } else { format!("Warning: {n} items could not be read. The comparison is partial.") }).wrap());
    }
    ui.add_space(10.0);
    // Original FUN_0041e0b4: counts per kind (same, different, left only, right only) as FOLDERCOMPARE_RESULT (NXLabel markup).
    let summary = form.original_result(language);
    ui.label(super::original_dialogs::markup_job(&summary, 12.0, ui.visuals().text_color(), size.x));
    ui.label(form.status(ko));
    ui.add_space(6.0);
    let columns = form.columns(ko, size.x);
    let view = GridView { id: "compare-grid", columns: &columns, height: size.y, enabled: true, sortable: true, multi_select: true, keyboard: true, context_menu: None, row_color: None, background: None };
    let events = {
        let ComparisonForm { rows, grid: state, select_for_sync, .. } = form;
        let (rows, sync): (&Vec<ComparisonRow>, bool) = (rows, *select_for_sync);
        // Only the comparison result window uses per-result text colors like the original (the sync preview keeps the C# window).
        let color = |row: usize| (!sync).then(|| result_color(rows[row].difference.kind));
        let view = GridView { row_color: Some(&color), background: (!sync).then_some(Color32::WHITE), ..view };
        grid::show(ui, &view, state, &|row, column| cell_text(rows, sync, row, column), &|row| rows[row].include)
    };
    for row in events.toggled {
        form.rows[row].include = !form.rows[row].include;
    }
    if let Some(column) = events.sort {
        form.sort_column(column);
    }
    if events.copy {
        action = Some(ComparisonAction::Copy(form.copy_text()));
    }
    ui.add_space(9.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        let close = if form.select_for_sync { t("선택 항목 동기화", "Synchronize selected").to_owned() } else { language.text("formCompareFolder.Caption.btnCancel", "Close") };
        if ui.add_sized([116.0, 26.0], egui::Button::new(close)).clicked() {
            action = Some(if form.select_for_sync { ComparisonAction::Commit } else { ComparisonAction::Close });
        }
        if ui.add_sized([84.0, 26.0], egui::Button::new(language.text("formCompareFolder.Caption.btnSave", "Save Result"))).clicked() {
            action = Some(ComparisonAction::Save);
        }
        if form.select_for_sync {
            if ui.add_sized([84.0, 26.0], egui::Button::new(t("모두 해제", "Clear all"))).clicked() {
                action = Some(ComparisonAction::SelectNone);
            }
            if ui.add_sized([84.0, 26.0], egui::Button::new(t("모두 선택", "Select all"))).clicked() {
                action = Some(ComparisonAction::SelectAll);
            }
        }
    });
    // Only the sync preview has a default button (IsDefault).
    if enter && form.select_for_sync && action.is_none() {
        action = Some(ComparisonAction::Commit);
    }
    action
}

impl App {
    fn comparison_form(&mut self) -> Option<&mut ComparisonForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::Comparison(form)) => Some(form),
            _ => None,
        }
    }

    /// C#: both panes must be ordinary folders (not inside archives or remote), and must not be equal or contain each other (RunSafely warning box).
    fn compare_roots(&mut self, synchronize: bool) -> Option<(PathBuf, PathBuf)> {
        // Original FUN_0043c602 (compare folders): only when both panes are in local mode (0).
        let local = |p: &Pane| p.archive.is_none() && p.remote.is_none();
        if !local(&self.panes[0]) || !local(&self.panes[1]) {
            if synchronize {
                self.warn("폴더 동기화는 일반 폴더에서 사용할 수 없습니다.", "Folder synchronization is only available for regular folders.");
            } else {
                self.warn("폴더 비교는 일반 폴더에서 사용할 수 없습니다.", "Folder comparison is only available for regular folders.");
            }
            return None;
        }
        let (left, right) = (self.panes[0].path.clone(), self.panes[1].path.clone());
        if fc::ensure_distinct_roots(&left, &right).is_err() {
            self.warn(DISTINCT_KO, DISTINCT_EN);
            return None;
        }
        Some((left, right))
    }

    /// Original FUN_0043c602 -> FUN_0041d280: opens TfCompareFolder with the folders of the two local panes (comparison starts with the window's start button).
    pub(super) fn compare_command(&mut self) {
        let Some((left, right)) = self.compare_roots(false) else { return };
        let title = self.language_pack.text("formCompareFolder.Form.Caption", "Compare Folders");
        let form = FolderCompareForm::new(left, right, self.settings.flag("UseContentComparison", true));
        self.push_original_dialog(title, DialogKind::FolderCompare(Box::new(form)), |_, _, _| {});
    }

    fn folder_compare_form(&mut self) -> Option<&mut FolderCompareForm> {
        self.original_dialogs.iter_mut().rev().find_map(|d| if let DialogKind::FolderCompare(form) = &mut d.kind { Some(form.as_mut()) } else { None })
    }

    /// TfCompareFolder buttons: btnOkClick (0x41d464) starts, btnStopClick (0x41e2c4) stops, btnSaveClick (0x41e2d0) saves HTML, close.
    pub(super) fn folder_compare_action(&mut self, action: FolderCompareAction, ctx: &egui::Context) {
        match action {
            FolderCompareAction::Start => {
                let tx = self.async_tx.clone();
                let Some(form) = self.folder_compare_form() else { return };
                if form.running.is_some() {
                    return;
                }
                let task = CompareTask::default();
                form.running = Some(task.clone());
                form.result = None;
                form.rows.clear();
                form.grid = GridState::new(0);
                let (left, right, recursive, contents) = (form.left.clone(), form.right.clone(), form.recursive, form.contents);
                std::thread::spawn(move || {
                    let outcome = fc::compare_partial(&left, &right, recursive, contents, &task.cancel, &mut |report| {
                        if let (done, Some(total)) = report.counts() {
                            if total > 0 {
                                task.percent.store(done * 100 / total, Ordering::Relaxed);
                            }
                        }
                    });
                    // Original: stopping inside the comparison loop keeps the rows so far (Ok + stopped). Stopping while reading the lists is Err.
                    let cancelled = task.cancel.load(Ordering::Relaxed) || matches!(outcome, Ok((_, _, true)));
                    let outcome = outcome.map(|(rows, issues, _)| (rows, issues));
                    let _ = tx.send(AsyncResult::CompareDone(CompareRun { left, right, source_is_left: None }, outcome, cancelled));
                });
            }
            FolderCompareAction::Stop => {
                if let Some(task) = self.folder_compare_form().and_then(|form| form.running.as_ref()) {
                    task.cancel.store(true, Ordering::Relaxed);
                }
            }
            FolderCompareAction::Save => self.save_folder_compare(),
            FolderCompareAction::Close => {
                if let Some(task) = self.folder_compare_form().and_then(|form| form.running.take()) {
                    task.cancel.store(true, Ordering::Relaxed);
                }
                self.answer_dialog(DialogResult::FolderCompare, ctx);
            }
        }
    }

    /// After comparing (after the original loop): result text (FUN_0041e0b4), btnSave enabled, ProgressBar hidden, btnOk shown again.
    /// On stop the original keeps the rows inserted so far but NDR shows an empty list. If the window is already closed, only the status line.
    fn folder_compare_done(&mut self, outcome: Result<(Vec<Difference>, Vec<String>), String>, cancelled: bool) {
        let ko = self.language == "ko";
        let template = self.language_pack.text(
            "RES.FOLDERCOMPARE_RESULT",
            "<c666666>Indentical: <b>%d</b></c>, <cFF0000>Different: <b>%d</b></c>, <c008000>Only in left: <b>%d</b></c>, <c0000FF>Only in right: <b>%d</b></c>",
        );
        let open = self.folder_compare_form().is_some_and(|form| form.running.is_some());
        if !open {
            if cancelled {
                self.status = self.tr("폴더 비교를 취소했습니다.", "Folder comparison canceled.").into_owned();
            }
            return;
        }
        let error = match outcome {
            Err(_) if cancelled => {
                let form = self.folder_compare_form().unwrap();
                form.running = None;
                form.compared = true;
                form.result = Some(form.result_text(&template));
                self.status = self.tr("폴더 비교를 취소했습니다.", "Folder comparison canceled.").into_owned();
                return;
            }
            Err(error) => error,
            Ok((differences, issues)) => {
                let changed = differences.iter().filter(|d| d.kind != DifferenceKind::Same).count();
                let unreadable = issues.len();
                let form = self.folder_compare_form().unwrap();
                form.running = None;
                form.grid = GridState::new(differences.len());
                form.rows = differences;
                form.issues = unreadable;
                form.compared = true;
                form.result = Some(form.result_text(&template));
                self.status = match (ko, unreadable) {
                    _ if cancelled => self.tr("폴더 비교를 취소했습니다.", "Folder comparison canceled.").into_owned(),
                    (true, 0) => format!("비교 완료 · 변경 {}개", n0(changed)),
                    (true, n) => format!("비교 완료 · 변경 {}개 · 읽지 못함 {}개", n0(changed), n0(n)),
                    (false, 0) => format!("Comparison complete · {} changed", n0(changed)),
                    (false, n) => format!("Comparison complete · {} changed · {} unreadable", n0(changed), n0(n)),
                };
                return;
            }
        };
        if let Some(form) = self.folder_compare_form() {
            form.running = None;
        }
        let text = localized_error(&error, ko);
        self.system_box("NDR", &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
    }

    /// Original btnSaveClick: SaveDialog (FileName "compare result.html", Filter "HTML File (*.html)", DefaultExt html, overwrite prompt) ->
    /// TStringList.SaveToFile(name, TEncoding.UTF8).
    fn save_folder_compare(&mut self) {
        let Some(form) = self.folder_compare_form() else { return };
        if !form.compared || form.running.is_some() {
            return;
        }
        let text = form.report_html();
        let Some(path) = (self.save_dialog)("", None, "compare result.html", &[("HTML File (*.html)", "*.html")], "html", true) else { return };
        if let Err(error) = fs::write(&path, [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat()) {
            let title = self.original_dialogs.last().map(|d| d.title.clone()).unwrap_or_default();
            self.system_box(&title, &error.to_string(), MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
    }

    /// C# SynchronizeFoldersAsync: direction -> content comparison mode -> compare (with subfolders) -> preview -> existing item handling -> confirm -> copy.
    pub(super) fn synchronize_command(&mut self) {
        let Some((left, right)) = self.compare_roots(true) else { return };
        let ko = self.language == "ko";
        let title = self.tr("폴더 동기화 방향", "Synchronization direction").into_owned();
        let message = self.tr(
            "어느 쪽 파일로 다른 폴더를 갱신할까요? 동기화는 파일을 복사하고 대상에서 추가 파일을 삭제하지 않습니다.",
            "Which side should update the other folder? Synchronization copies files and does not delete extra files from the destination.",
        );
        let directions = if ko { ["왼쪽 → 오른쪽", "오른쪽 → 왼쪽"] } else { ["Left → Right", "Right → Left"] };
        self.choice_dialog(&title, &message, directions.map(String::from).to_vec(), 0, move |app, direction, _| {
            let Some(direction) = direction else { return };
            let source_is_left = direction == 0;
            let ko = app.language == "ko";
            let title = app.tr("파일 내용 비교", "Compare file contents").into_owned();
            let message = app.tr("크기와 수정 시간 외에 파일 내용까지 비교할까요?", "Compare file contents in addition to size and modification time?").into_owned();
            let modes = if ko { ["크기·수정 날짜", "파일 내용 포함"] } else { ["Size and modified date", "Include file contents"] };
            let initial = usize::from(app.settings.flag("UseContentComparison", true));
            app.choice_dialog(&title, &message, modes.map(String::from).to_vec(), initial, move |app, mode, _| {
                let Some(mode) = mode else { return };
                let title = app.tr("동기화 비교", "Synchronization compare").into_owned();
                app.start_comparison(title, CompareRun { left, right, source_is_left: Some(source_is_left) }, true, mode == 1);
            });
        });
    }

    /// FolderComparisonProgressWindow (stop button, "preparing folder comparison...").
    fn start_comparison(&mut self, title: String, run: CompareRun, recursive: bool, contents: bool) {
        let ko = self.language == "ko";
        let modal = ModalProgress {
            stop: self.language_pack.text("formCompareFolder.Caption.btnStop", "Stop"),
            stopping: Some(self.tr("중단 중…", "Stopping…").into_owned()),
            stopping_text: self.tr("현재 작업을 안전하게 멈추는 중…", "Stopping the current work safely…").into_owned(),
            kind: ModalKind::Message,
        };
        let preparing = self.tr("폴더 비교 준비 중…", "Preparing the folder comparison…").into_owned();
        let tx = self.async_tx.clone();
        let (left, right) = (run.left.clone(), run.right.clone());
        let started = self.start_modal_task(title, modal, move |events, cancel| {
            let outcome = fc::compare(&left, &right, recursive, contents, &cancel, &mut |report| {
                let (completed, total) = report.counts();
                let _ = events.send(Event::Progress { path: PathBuf::from(report.message(ko)), bytes: total.unwrap_or(0) as u64, files: completed as u64 });
            });
            let _ = tx.send(AsyncResult::CompareDone(run, outcome, cancel.load(Ordering::Relaxed)));
            Ok(())
        });
        if started {
            if let Some(job) = self.job.as_mut() {
                job.path = preparing;
            }
        }
    }

    pub(super) fn compare_done(&mut self, run: CompareRun, outcome: Result<(Vec<Difference>, Vec<String>), String>, cancelled: bool) {
        let ko = self.language == "ko";
        if run.source_is_left.is_none() {
            return self.folder_compare_done(outcome, cancelled);
        }
        let (differences, issues) = match outcome {
            Err(_) if cancelled => {
                self.status = self.tr("폴더 동기화를 취소했습니다.", "Folder synchronization canceled.").into_owned();
                return;
            }
            Err(error) => {
                let text = localized_error(&error, ko);
                return self.system_box("NDR", &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
            Ok(result) => result,
        };
        let (left, right) = (run.left.to_string_lossy().into_owned(), run.right.to_string_lossy().into_owned());
        let Some(source_is_left) = run.source_is_left else { return };
        if !issues.is_empty() {
            // Stop when some items could not be read so nothing is copied wrongly.
            let mut text = self.tr(
                "일부 항목을 읽지 못해 비교 결과가 완전하지 않습니다. 잘못된 파일 복사를 막기 위해 동기화를 시작하지 않았습니다.\n\n",
                "Some items could not be read, so the comparison is incomplete. Synchronization did not start, to avoid copying the wrong files.\n\n",
            )
            .into_owned();
            text += &issues.iter().take(ERROR_LINES).cloned().collect::<Vec<_>>().join("\r\n");
            if issues.len() > ERROR_LINES {
                let rest = n0(issues.len() - ERROR_LINES);
                text += &if ko { format!("\r\n… 나머지 {rest}개 오류") } else { format!("\r\n… {rest} more errors") };
            }
            let title = self.tr("폴더 동기화 중단", "Folder synchronization stopped").into_owned();
            self.text = Some(TextDocument { title, text });
            self.status = if ko { format!("동기화 중단 · 읽지 못한 항목 {}개", n0(issues.len())) } else { format!("Synchronization stopped · {} unreadable items", n0(issues.len())) };
            return;
        }
        let plan: Vec<Difference> = differences
            .into_iter()
            .filter(|d| d.kind == DifferenceKind::Different || d.kind == if source_is_left { DifferenceKind::LeftOnly } else { DifferenceKind::RightOnly })
            .collect();
        if plan.is_empty() {
            self.status = self.tr("동기화할 차이가 없습니다.", "There are no differences to synchronize.").into_owned();
            return;
        }
        let title = self.tr("동기화 미리보기", "Synchronization preview").into_owned();
        let form = ComparisonForm::new(left, right, plan, true, vec![], ko);
        self.push_original_dialog(title, DialogKind::Comparison(Box::new(form)), move |app, result, _| {
            let DialogResult::Comparison(Some(selected)) = result else { return };
            app.choose_existing_behavior(run, source_is_left, selected);
        });
    }

    /// Choose existing item handling -> confirm "run synchronization" (yes/no).
    fn choose_existing_behavior(&mut self, run: CompareRun, source_is_left: bool, selected: Vec<Difference>) {
        let ko = self.language == "ko";
        let title = self.tr("기존 항목 처리", "Existing items").into_owned();
        let message = self.tr("대상에 같은 경로가 있을 때 처리할 방법을 선택하세요.", "Choose what to do when the destination already has the same path.").into_owned();
        let options = if ko {
            ["기존 항목 건너뛰기", "기존 파일 덮어쓰기", "원본이 최신이면 덮어쓰기", "파일/폴더 형식 충돌도 교체"]
        } else {
            ["Skip existing items", "Overwrite existing files", "Overwrite if the source is newer", "Also replace file/folder type conflicts"]
        };
        self.choice_dialog(&title, &message, options.map(String::from).to_vec(), 0, move |app, choice, _| {
            let Some(choice) = choice else { return };
            let behavior = [ExistingBehavior::Skip, ExistingBehavior::Overwrite, ExistingBehavior::OverwriteIfSourceIsNewer, ExistingBehavior::OverwriteAndReplaceTypeConflicts][choice.min(3)];
            let (source, destination) = if source_is_left { (run.left, run.right) } else { (run.right, run.left) };
            let count = n0(selected.len());
            let text = if app.language == "ko" {
                format!("선택한 파일 {count}개를\n{}\n에서\n{}\n으로 동기화할까요? 대상의 추가 파일은 삭제하지 않습니다.", source.display(), destination.display())
            } else {
                format!("Synchronize the {count} selected files from\n{}\nto\n{}\n? Extra files in the destination are not deleted.", source.display(), destination.display())
            };
            let caption = app.tr("동기화 실행", "Run synchronization").into_owned();
            app.system_box(&caption, &text, MB_YESNO | MB_ICONQUESTION, move |app, answer, _| {
                if answer == IDYES {
                    app.start_sync(source, destination, selected, source_is_left, behavior);
                }
            });
        });
    }

    /// FileOperationProgressWindow ("synchronize folders", counted by source-side files): CopyDifferencesAsync(UseRecycleBin).
    fn start_sync(&mut self, source: PathBuf, destination: PathBuf, selected: Vec<Difference>, source_is_left: bool, behavior: ExistingBehavior) {
        let files = selected.iter().filter(|d| !if source_is_left { d.left_dir } else { d.right_dir }).count() as u64;
        let progress = super::file_operation::ProgressInfo {
            from: Some(source.to_string_lossy().into_owned()),
            to: Some(destination.to_string_lossy().into_owned()),
            total_files: Some(files),
            total_bytes: None,
            prefer_bytes: false,
            caption: None,
        };
        let recycle = self.settings.flag("UseRecycleBin", true);
        let tx = self.async_tx.clone();
        let title = self.tr("폴더 동기화", "Synchronize folders").into_owned();
        let started = self.start_progress_task(title, progress, move |events, cancel| {
            let outcome = fc::copy_differences(&source, &destination, &selected, source_is_left, behavior, recycle, &cancel, &mut |copied, bytes, path| {
                let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes, files: copied });
            });
            let _ = tx.send(AsyncResult::SyncDone(outcome, cancel.load(Ordering::Relaxed)));
            Ok(())
        });
        if started {
            if let Some(job) = self.job.as_mut() {
                // The text after finishing is written by sync_done.
                job.custom_status = true;
            }
        }
    }

    /// After refreshing both panes: cancel, fatal error (ReportFileOperationResult: "synchronization error" box), done status line and error window.
    pub(super) fn sync_done(&mut self, outcome: Result<fc::SyncResult, String>, cancelled: bool) {
        self.refresh(0);
        self.refresh(1);
        let ko = self.language == "ko";
        if cancelled {
            self.status = self.tr("동기화를 취소했습니다. 이미 완료된 항목은 유지됩니다.", "Synchronization canceled. Items already completed are kept.").into_owned();
            return;
        }
        let result = match outcome {
            Ok(result) => result,
            Err(error) => {
                let caption = self.tr("동기화 오류", "Synchronization error").into_owned();
                self.system_box(&caption, &error, MB_OK | MB_ICONHAND, |_, _, _| {});
                self.status = self.tr("동기화 작업을 마치지 못했습니다.", "The synchronization could not be completed.").into_owned();
                return;
            }
        };
        let (copied, created, skipped, errors) = (n0(result.copied as usize), n0(result.created_directories as usize), n0(result.skipped_existing as usize), n0(result.errors.len()));
        self.status = if ko {
            format!("동기화 완료 · 파일 {copied}개 복사 · 폴더 {created}개 생성 · 건너뜀 {skipped}개 · 오류 {errors}개")
        } else {
            format!("Synchronization complete · {copied} files copied · {created} folders created · {skipped} skipped · {errors} errors")
        };
        if !result.errors.is_empty() {
            let mut text = result.errors.iter().take(ERROR_LINES).cloned().collect::<Vec<_>>().join("\r\n");
            if result.errors.len() > ERROR_LINES {
                let rest = n0(result.errors.len() - ERROR_LINES);
                text += &if ko { format!("\r\n나머지 {rest}개 오류") } else { format!("\r\n{rest} more errors") };
            }
            let title = self.tr("동기화 오류", "Synchronization errors").into_owned();
            self.text = Some(TextDocument { title, text });
        }
    }

    /// Result window buttons.
    pub(super) fn comparison_action(&mut self, action: ComparisonAction, ctx: &egui::Context) {
        match action {
            ComparisonAction::SelectAll | ComparisonAction::SelectNone => {
                let value = matches!(action, ComparisonAction::SelectAll);
                if let Some(form) = self.comparison_form() {
                    form.rows.iter_mut().for_each(|row| row.include = value);
                }
            }
            ComparisonAction::Close => self.answer_dialog(DialogResult::Comparison(None), ctx),
            ComparisonAction::Commit => {
                let Some(form) = self.comparison_form() else { return };
                let selected = form.included();
                if selected.is_empty() {
                    let title = self.original_dialogs.last().map(|d| d.title.clone()).unwrap_or_default();
                    let text = self.tr("동기화할 파일을 하나 이상 선택하세요.", "Select at least one file to synchronize.").into_owned();
                    return self.system_box(&title, &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
                }
                self.answer_dialog(DialogResult::Comparison(Some(selected)), ctx);
            }
            ComparisonAction::Save => self.save_comparison(),
            ComparisonAction::Copy(text) => {
                if let Err(error) = (self.clipboard_writer)(&text) {
                    self.status = error;
                }
            }
        }
    }

    /// SaveResults: HTML (.html, window title as title) or TSV (any other extension), UTF-8 BOM. The save dialog uses the C# filter list as is.
    fn save_comparison(&mut self) {
        let ko = self.language == "ko";
        let title = self.original_dialogs.last().map(|d| d.title.clone()).unwrap_or_default();
        let dialog_title = self.tr("폴더 비교 결과 저장", "Save folder comparison").into_owned();
        let filters: [(&str, &str); 4] = if ko {
            [("HTML 파일 (*.html)", "*.html"), ("TSV 파일 (*.tsv)", "*.tsv"), ("텍스트 파일 (*.txt)", "*.txt"), ("모든 파일 (*.*)", "*.*")]
        } else {
            [("HTML files (*.html)", "*.html"), ("TSV files (*.tsv)", "*.tsv"), ("Text files (*.txt)", "*.txt"), ("All files (*.*)", "*.*")]
        };
        let Some(path) = (self.save_dialog)(&dialog_title, None, "NDR-folder-comparison.html", &filters, "html", true) else { return };
        let Some(form) = self.comparison_form() else { return };
        let rows: Vec<(Difference, bool)> = form.rows.iter().map(|r| (r.difference.clone(), r.include)).collect();
        let text = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("html")) {
            fc::report_html(&title, &form.left, &form.right, &rows, &form.issues, form.select_for_sync, ko)
        } else {
            fc::report_tsv(&rows, &form.issues, ko)
        };
        if let Err(error) = fs::write(&path, [&[0xEF, 0xBB, 0xBF][..], text.as_bytes()].concat()) {
            self.system_box(&title, &error.to_string(), MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
    }

    /// Modal progress window (in place of FolderComparisonProgressWindow / SearchProgressWindow / sync FileOperationProgressWindow).
    /// A button or Esc (close) only requests cancel; the window closes when the job ends.
    pub(super) fn modal_progress_ui(&mut self, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let Some(job) = self.job.as_ref() else { return };
        let Some(modal) = job.modal.as_ref() else { return };
        let stopping = job.cancel.load(Ordering::Relaxed);
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let (status, detail, fraction): (String, Option<String>, Option<f32>) = match &modal.kind {
            ModalKind::Message => (job.path.clone(), None, (job.bytes > 0).then(|| job.files.min(job.bytes) as f32 / job.bytes as f32)),
            ModalKind::Search => {
                let counts = if ko { format!("검색 항목 {} · 결과 {}", n0(job.files as usize), n0(job.bytes as usize)) } else { format!("Scanned {} · Results {}", n0(job.files as usize), n0(job.bytes as usize)) };
                (format!("{counts}\n{}", job.path), None, None)
            }

        };
        let status = if stopping { modal.stopping_text.clone() } else { status };
        super::keep_opaque(ctx, egui::Id::new("ndr-modal-progress"));
        let progress_id = egui::Id::new("ndr-modal-progress");
        super::movable_modal(ctx, progress_id).show(ctx, |ui| {
            ui.set_width(424.0);
            let title = ui.label(RichText::new(&job.title).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())));
            super::drag_title(ui, progress_id, title.rect.with_max_x(title.rect.left() + 424.0));
            ui.separator();
            ui.add(egui::Label::new(status).wrap());
            if let Some(detail) = detail {
                ui.label(detail);
            }
            ui.add_space(10.0);
            let bar = match fraction {
                Some(value) => egui::ProgressBar::new(value),
                None => egui::ProgressBar::new(0.0).animate(true),
            };
            ui.add(bar.desired_height(14.0));
            ui.add_space(14.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let label = if stopping { modal.stopping.clone().unwrap_or_else(|| modal.stop.clone()) } else { modal.stop.clone() };
                if ui.add_enabled(!stopping, egui::Button::new(label).min_size(egui::vec2(88.0, 26.0))).clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_colors_follow_lvlistadvancedcustomdrawitem() {
        // 0x41dd6c: 0 -> 0x808080, 1 -> clRed (0x0000FF), 2 -> clGreen (0x008000), else clBlue (0xFF0000). TColor is BGR.
        assert_eq!(result_color(DifferenceKind::Same), Color32::from_rgb(128, 128, 128));
        assert_eq!(result_color(DifferenceKind::Different), Color32::from_rgb(255, 0, 0));
        assert_eq!(result_color(DifferenceKind::LeftOnly), Color32::from_rgb(0, 128, 0));
        assert_eq!(result_color(DifferenceKind::RightOnly), Color32::from_rgb(0, 0, 255));
    }
}
