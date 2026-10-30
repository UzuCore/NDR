//! C# 컴퓨터 청소(RunDiskCleanupAsync), 임시 폴더 비우기(EmptyTemporaryFolderAsync), 휴지통 비우기(EmptyRecycleBin)와
//! CleanupOptionsWindow, CleanupScanProgressWindow, CleanupResultsWindow. 규칙은 ndir::cleanup.
//! 휴지통 비우기는 `App::recycle_emptier`를 거친다(테스트는 실제 휴지통을 건드리지 않는다).
//! 차이(HANDOFF): 위치 목록의 Insert/Delete 글쇠는 목록에 초점이 없어도 글 칸이 초점을 갖지 않으면 듣는다.
use super::grid::{self, Column, GridState, GridView};
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndir::cleanup::{self, Candidate};
use ndir::msgbox::{IDYES, MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_ICONQUESTION, MB_OK, MB_YESNO};

const ERROR_LINES: usize = 500;

fn n0(value: usize) -> String {
    ndir::parity::format_size(value as u64, "B", true).trim_end_matches(" B").to_owned()
}

fn same_path(a: &str, b: &str) -> bool {
    ndir::names::equals_ignore_case(a, b)
}

/// C# CleanupOptions(+ UserDefinedResults).
#[derive(Clone, Debug, Default)]
pub(super) struct CleanupOptions {
    pub system_temp: bool,
    pub internet_temp: bool,
    pub cookies: bool,
    pub recycle: bool,
    pub user_defined: bool,
    pub extensions: String,
    pub locations: Vec<String>,
    pub user_results: Vec<Candidate>,
}

/// CleanupOptionsWindow의 입력. results는 사용자 지정 검색 결과와 확인란(처음 켬).
pub(super) struct CleanupOptionsForm {
    pub system_temp: bool,
    pub internet_temp: bool,
    pub cookies: bool,
    pub recycle: bool,
    pub user_defined: bool,
    pub extensions: String,
    pub location_input: String,
    pub locations: Vec<String>,
    pub selected_location: Option<usize>,
    pub results: Vec<(Candidate, bool)>,
    pub grid: GridState,
    pub searched: bool,
}

impl CleanupOptionsForm {
    /// 고정 드라이브 → 저장한 위치 순으로(같은 경로는 한 번).
    pub(super) fn new(extensions: String, saved: &[String]) -> Self {
        let mut form = Self {
            system_temp: true,
            internet_temp: true,
            cookies: false,
            recycle: true,
            user_defined: false,
            extensions,
            location_input: String::new(),
            locations: vec![],
            selected_location: None,
            results: vec![],
            grid: GridState::new(0),
            searched: false,
        };
        for drive in native::drives() {
            if native::is_fixed_drive(&drive) {
                form.add_unique(&drive.to_string_lossy());
            }
        }
        for path in saved {
            form.add_unique(path);
        }
        form
    }

    /// AddUniqueLocation: 환경 변수를 풀고 전체 경로로.
    fn add_unique(&mut self, value: &str) {
        if value.trim().is_empty() {
            return;
        }
        let expanded = native::expand_environment(value.trim());
        let Ok(path) = std::path::absolute(&expanded) else { return };
        let path = path.to_string_lossy().into_owned();
        if !self.locations.iter().any(|known| same_path(known, &path)) {
            self.locations.push(path);
            self.invalidate();
        }
    }

    fn select_location(&mut self, path: &str) {
        self.selected_location = self.locations.iter().position(|known| same_path(known, path));
    }

    /// InvalidateUserDefinedSearch: 확장자나 위치가 바뀌면 결과를 비우고 다시 검색해야 한다.
    pub(super) fn invalidate(&mut self) {
        if !self.searched && self.results.is_empty() {
            return;
        }
        self.clear_results();
    }

    fn clear_results(&mut self) {
        self.results.clear();
        self.grid = GridState::new(0);
        self.searched = false;
    }

    pub(super) fn set_results(&mut self, results: Vec<Candidate>) {
        self.results = results.into_iter().map(|c| (c, true)).collect();
        self.grid = GridState::new(self.results.len());
        self.searched = true;
    }

    /// ParsePatterns(_extensions.Text)를 위치마다.
    pub(super) fn search_roots(&self) -> Vec<(PathBuf, Vec<String>)> {
        let patterns = cleanup::parse_patterns(&self.extensions);
        self.locations.iter().map(|path| (PathBuf::from(path), patterns.clone())).collect()
    }
}

/// CleanupResultsWindow: 후보 전체, 휴지통 비우기 선택, 표(처음 모두 선택).
pub(super) struct CleanupResultsForm {
    pub candidates: Vec<Candidate>,
    pub recycle: bool,
    pub grid: GridState,
}

impl CleanupResultsForm {
    pub(super) fn new(candidates: Vec<Candidate>, recycle: bool) -> Self {
        let mut grid = GridState::new(candidates.len());
        grid.select_all();
        Self { candidates, recycle, grid }
    }

    /// UpdateStatus: 선택한 파일 수 / 전체, 선택 크기, 휴지통 비우기 알림.
    pub(super) fn status(&self, ko: bool) -> String {
        let selected = self.grid.selected_in_order();
        let bytes: u64 = selected.iter().map(|&i| self.candidates[i].size).sum();
        let size = ndir::parity::format_size(bytes, "Auto", true);
        let (count, total) = (n0(selected.len()), n0(self.candidates.len()));
        if ko {
            format!("선택한 파일 {count} / {total}개 · {size}{}", if self.recycle { " · 휴지통 비우기도 선택됨" } else { "" })
        } else {
            format!("Selected files {count} / {total} · {size}{}", if self.recycle { " · Empty Recycle Bin is also selected" } else { "" })
        }
    }

    fn cell(&self, row: usize, column: usize) -> String {
        let candidate = &self.candidates[row];
        match column {
            0 => candidate.name.clone(),
            1 => candidate.directory(),
            _ => candidate.size_text(),
        }
    }
}

/// 두 창의 단추와 글쇠.
pub(super) enum CleanupAction {
    Search,
    /// 원본 btnAdd(btnAddClick): 폴더 고르기 창으로 위치를 더한다.
    Browse,
    Remove,
    SelectAll,
    SelectNone,
    Accept,
    Close,
    Copy(String),
}

/// 검색이 끝난 뒤 할 일.
pub(super) enum ScanPurpose {
    /// 옵션 창의 사용자 지정 검색(결과를 창에 넣는다).
    UserDefined,
    /// 정리 흐름(결과 창으로).
    Cleanup(CleanupOptions),
}

/// 정리가 끝난 뒤 알릴 값.
pub(super) struct CleanupDone {
    pub requested: usize,
    pub to_delete: usize,
    pub skipped: usize,
    pub recycle: bool,
}

/// RunDiskCleanupAsync의 검색 위치: Path.GetTempPath(), 인터넷 임시 파일, 쿠키(패턴 없음, 없는 폴더는 뺌).
pub(super) fn cleanup_roots(options: &CleanupOptions) -> Vec<(PathBuf, Vec<String>)> {
    let mut roots: Vec<(PathBuf, Vec<String>)> = vec![];
    if options.system_temp {
        roots.push((std::env::temp_dir(), vec![]));
    }
    if options.internet_temp {
        roots.extend(native::internet_cache_folder().map(|p| (p, vec![])));
    }
    if options.cookies {
        roots.extend(native::cookies_folder().map(|p| (p, vec![])));
    }
    roots
}

/// [formCleanup] 글(단축 문자 표시는 지운다).
fn text(language: &compat::Language, key: &str, fallback: &str) -> String {
    language.text(&format!("formCleanup.{key}"), fallback)
}

/// 결과 표 머리글(ListViewCols.lvResult).
fn result_titles(language: &compat::Language, fallback: &str) -> [String; 3] {
    let joined = text(language, "ListViewCols.lvResult", fallback);
    let mut parts = joined.split(';').map(str::to_owned);
    let defaults: Vec<&str> = fallback.split(';').collect();
    [0, 1, 2].map(|i| parts.next().unwrap_or_else(|| defaults[i].to_owned()))
}

/// 원본 TfCleanup(ClientWidth 554 × ClientHeight 507): chkTemp(20,20)·chkInternetTemp(20,43)·chkInternetCookie(20,66)·chkRecycleBin(20,89),
/// chkUserDefined(20,112, TntGroupBox 제목 자리), TntGroupBox(16,115 521×378: lLocation(20,29) lbLocation(20,48 405×55)
/// btnAdd(434,48)·btnDelete(434,79), lExtensions(20,117) cbExtensions(20,136 408×21) btnSearch(434,133), lResult(20,164)
/// lvResult(20,184 489×153, 확인란·파일 이름 140·위치 250·크기 80), btnSelectAll(20,343 85×25)·btnDeselectAll(111,343 84×25)),
/// btnOk(440,16 97×53 "시작"), btnClose(440,84 97×25). 사용자 지정은 켰을 때만 칸이 켜진다. btnAdd는 원본처럼 폴더 고르기 창.
pub(super) fn options_ui(ui: &mut egui::Ui, form: &mut CleanupOptionsForm, language: &compat::Language, ko: bool, enter: bool) -> Option<CleanupAction> {
    use super::original_dialogs::{put_enabled, put_left};
    let _ = ko;
    let t = |key: &str, fallback: &str| text(language, key, fallback);
    let mut action = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(554.0, 507.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    // 그룹 칸을 먼저 그리고(chkUserDefined가 제목 자리를 덮는다), 제목 글은 비워 둔다.
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    ui.painter().rect_stroke(at(16.0, 122.0, 521.0, 371.0), 2.0, stroke, egui::StrokeKind::Inside);
    ui.painter().rect_filled(at(18.0, 112.0, 167.0, 17.0), 0.0, ui.visuals().window_fill);
    for (top, value, key, fallback) in [
        (20.0, &mut form.system_temp, "Caption.chkTemp", "System temporary files"),
        (43.0, &mut form.internet_temp, "Caption.chkInternetTemp", "Internet Explorer temporary files"),
        (66.0, &mut form.cookies, "Caption.chkInternetCookie", "Internet Explorer cookie files"),
        (89.0, &mut form.recycle, "Caption.chkRecycleBin", "Empty Recycle Bin"),
    ] {
        put_left(ui, at(20.0, top, 338.0, 17.0), egui::Checkbox::new(value, t(key, fallback)));
    }
    put_left(ui, at(20.0, 112.0, 161.0, 17.0), egui::Checkbox::new(&mut form.user_defined, t("Caption.chkUserDefined", "User-defined files")));
    let enabled = form.user_defined;
    let has_results = !form.results.is_empty();
    ui.add_enabled_ui(enabled, |ui| {
        put_left(ui, at(36.0, 144.0, 405.0, 15.0), egui::Label::new(t("Caption.lLocation", "Locations:")).truncate());
        let list = at(36.0, 163.0, 405.0, 55.0);
        ui.painter().rect(list, 0.0, ui.visuals().extreme_bg_color, stroke, egui::StrokeKind::Inside);
        ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink(2.0)), |ui| {
            egui::ScrollArea::vertical().id_salt("cleanup-locations").auto_shrink([false, false]).show(ui, |ui| {
                for (index, location) in form.locations.iter().enumerate() {
                    if ui.selectable_label(form.selected_location == Some(index), location).clicked() {
                        form.selected_location = Some(index);
                    }
                }
            });
        });
        if ui.put(at(450.0, 163.0, 75.0, 25.0), egui::Button::new(t("Caption.btnAdd", "Add"))).clicked() {
            action = Some(CleanupAction::Browse);
        }
        if ui.put(at(450.0, 194.0, 75.0, 25.0), egui::Button::new(t("Caption.btnDelete", "Delete"))).clicked() {
            action = Some(CleanupAction::Remove);
        }
        // 원본 lbLocationKeyDown(0x41cac0): 목록에서 Delete = btnDeleteClick, Insert = btnAddClick(0x41bd68).
        // NDIR은 목록에서 줄을 고른 뒤 글 칸에 초점이 없을 때 받는다.
        if action.is_none() && form.selected_location.is_some() && ui.memory(|m| m.focused().is_none()) {
            if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                action = Some(CleanupAction::Remove);
            } else if ui.input(|i| i.key_pressed(egui::Key::Insert)) {
                action = Some(CleanupAction::Browse);
            }
        }
        put_left(ui, at(36.0, 232.0, 405.0, 15.0), egui::Label::new(t("Caption.lExtensions", "Extensions to search:")).truncate());
        if ui.put(at(36.0, 251.0, 408.0, 21.0), egui::TextEdit::singleline(&mut form.extensions)).changed() {
            form.invalidate();
        }
        if ui.put(at(450.0, 248.0, 75.0, 25.0), egui::Button::new(t("Caption.btnSearch", "Search"))).clicked() {
            action = Some(CleanupAction::Search);
        }
        // 원본(0x41c549): 검색이 끝나면 lResult 글을 SEARCHRESULT + Format(" (%d)", 결과 수)로 바꾼다(천 단위 구분 없음). 검색 전은 DFM 글.
        let label = if form.searched {
            format!("{} ({})", language.text("RES.SEARCHRESULT", "Search Result"), form.results.len())
        } else {
            t("Caption.lResult", "Search Result")
        };
        put_left(ui, at(36.0, 279.0, 489.0, 17.0), egui::Label::new(label).truncate());
        let titles = result_titles(language, "Filename;Location;Size(bytes)");
        let columns = [Column::check("", 24.0), Column::text(&titles[0], 126.0), Column::text(&titles[1], 250.0), Column::right(&titles[2], 80.0)];
        let view = GridView { id: "cleanup-user-results", columns: &columns, height: 153.0, enabled: enabled && has_results, sortable: false, multi_select: false, keyboard: true, context_menu: None, row_color: None, background: None };
        let events = ui
            .scope_builder(egui::UiBuilder::new().max_rect(at(36.0, 299.0, 489.0, 153.0)), |ui| {
                let CleanupOptionsForm { results, grid: state, .. } = form;
                let results: &Vec<(Candidate, bool)> = results;
                grid::show(
                    ui,
                    &view,
                    state,
                    &|row, column| match column {
                        1 => results[row].0.name.clone(),
                        2 => results[row].0.directory(),
                        _ => results[row].0.size_text(),
                    },
                    &|row| results[row].1,
                )
            })
            .inner;
        for row in events.toggled {
            form.results[row].1 = !form.results[row].1;
        }
        if events.copy {
            let results = &form.results;
            action = Some(CleanupAction::Copy(form.grid.copy_text(&columns, &|row, column| match column {
                1 => results[row].0.name.clone(),
                2 => results[row].0.directory(),
                _ => results[row].0.size_text(),
            }, &|row| results[row].1)));
        }
        if put_enabled(ui, at(36.0, 458.0, 85.0, 25.0), has_results, egui::Button::new(t("Caption.btnSelectAll", "Select All"))).clicked() {
            action = Some(CleanupAction::SelectAll);
        }
        if put_enabled(ui, at(127.0, 458.0, 84.0, 25.0), has_results, egui::Button::new(t("Caption.btnDeselectAll", "Deselect All"))).clicked() {
            action = Some(CleanupAction::SelectNone);
        }
    });
    // 위치 목록 글쇠: Insert = 찾아보기, Delete = 제거(글 칸이 초점을 가질 때는 그 칸의 글쇠).
    if enabled && !ui.ctx().wants_keyboard_input() {
        let (insert, delete) = ui.input(|i| (i.key_pressed(egui::Key::Insert), i.key_pressed(egui::Key::Delete)));
        if insert {
            action = Some(CleanupAction::Browse);
        } else if delete && form.selected_location.is_some() {
            action = Some(CleanupAction::Remove);
        }
    }
    if ui.put(at(440.0, 16.0, 97.0, 53.0), egui::Button::new(t("Caption.btnOk", "Cleanup"))).clicked() {
        action = Some(CleanupAction::Accept);
    }
    if ui.put(at(440.0, 84.0, 97.0, 25.0), egui::Button::new(t("Caption.btnClose", "Close"))).clicked() {
        action = Some(CleanupAction::Close);
    }
    if enter && action.is_none() {
        action = Some(CleanupAction::Accept);
    }
    action
}
/// CleanupResultsWindow 그리기(1040×660, 작은 화면이면 줄인다).
pub(super) fn results_ui(ui: &mut egui::Ui, form: &mut CleanupResultsForm, language: &compat::Language, ko: bool, enter: bool, size: egui::Vec2) -> Option<CleanupAction> {
    let mut action = None;
    ui.set_width(size.x);
    ui.label(form.status(ko));
    ui.add_space(8.0);
    let titles = result_titles(language, "File name;Path;Size (bytes)");
    let rest = size.x - 16.0;
    let columns = [Column::text(&titles[0], rest * 1.2 / 4.6), Column::text(&titles[1], rest * 2.6 / 4.6), Column::right(&titles[2], rest * 0.8 / 4.6)];
    let view = GridView { id: "cleanup-results", columns: &columns, height: size.y, enabled: true, sortable: true, multi_select: true, keyboard: true, context_menu: None, row_color: None, background: None };
    let events = {
        let CleanupResultsForm { candidates, grid: state, .. } = form;
        let candidates: &Vec<Candidate> = candidates;
        let cell = |row: usize, column: usize| match column {
            0 => candidates[row].name.clone(),
            1 => candidates[row].directory(),
            _ => candidates[row].size_text(),
        };
        grid::show(ui, &view, state, &cell, &|_| false)
    };
    if let Some(column) = events.sort {
        let CleanupResultsForm { candidates, grid: state, .. } = form;
        let key = |row: usize| match column {
            0 => candidates[row].name.clone(),
            1 => candidates[row].directory(),
            _ => candidates[row].size_text(),
        };
        state.toggle_sort(column, |a, b| ndir::listing::compare_text(&key(a), &key(b), false));
    }
    if events.copy {
        action = Some(CleanupAction::Copy(form.grid.copy_text(&columns, &|row, column| form.cell(row, column), &|_| false)));
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui.button(text(language, "Caption.btnSelectAll", "Select All")).clicked() {
            action = Some(CleanupAction::SelectAll);
        }
        if ui.button(text(language, "Caption.btnDeselectAll", "Deselect All")).clicked() {
            action = Some(CleanupAction::SelectNone);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if ui.add_sized([76.0, 26.0], egui::Button::new(language.text("RES.CANCEL", "Cancel"))).clicked() {
                action = Some(CleanupAction::Close);
            }
            let label = if form.candidates.is_empty() && form.recycle { if ko { "휴지통 비우기…".to_owned() } else { "Empty Recycle Bin…".to_owned() } } else { text(language, "Caption.btnOk", "Start") };
            let enabled = !form.candidates.is_empty() || form.recycle;
            if ui.add_enabled(enabled, egui::Button::new(label).min_size(egui::vec2(124.0, 26.0))).clicked() {
                action = Some(CleanupAction::Accept);
            }
        });
    });
    if enter && action.is_none() && (!form.candidates.is_empty() || form.recycle) {
        action = Some(CleanupAction::Accept);
    }
    action
}

impl App {
    fn cleanup_title(&self) -> String {
        self.language_pack.text("formCleanup.Form.Caption", "Computer Cleanup")
    }

    fn cleanup_options_form(&mut self) -> Option<&mut CleanupOptionsForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::CleanupOptions(form)) => Some(form),
            _ => None,
        }
    }

    fn cleanup_results_form(&mut self) -> Option<&mut CleanupResultsForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::CleanupResults(form)) => Some(form),
            _ => None,
        }
    }

    /// C# RunDiskCleanupAsync(옵션 창부터). 확인하면 위치·확장자를 설정에 저장한다.
    pub(super) fn cleanup_command(&mut self) {
        let extensions = self.settings.text("UserDefinedCleanupExtensions", cleanup::DEFAULT_EXTENSIONS);
        let form = CleanupOptionsForm::new(extensions, &self.settings.strings("UserDefinedCleanupLocations"));
        let title = self.cleanup_title();
        self.push_original_dialog(title, DialogKind::CleanupOptions(Box::new(form)), |app, result, _| {
            let DialogResult::CleanupOptions(Some(options)) = result else { return };
            app.settings.set_strings("UserDefinedCleanupLocations", options.locations.clone());
            app.settings.set_text("UserDefinedCleanupExtensions", &options.extensions);
            app.save();
            app.run_cleanup(options);
        });
    }

    /// 원본 miEmptyTempClick(0x434344): GetTempPathW → GetLongPathNameW(실패하면 그만), 폴더 크기(StrFormatByteSizeW)와
    /// 파일 수로 TfMsgBox(TITLE_EMPTYTEMP, "", MSG_EMPTYTEMP, MB_YESNO | MB_ICONQUESTION). 예면 지우기 작업(종류 2,
    /// 플래그 0xF0 = 안의 항목만, 묻지 않고, 오류는 건너뛰고 알리지 않음)을 시작하고 두 패널을 새로 읽는다.
    /// C#은 컴퓨터 청소 검색 창을 거쳤다(원본에 없는 흐름).
    pub(super) fn empty_temp_command(&mut self) {
        let Some(folder) = (self.system.temp_folder)() else { return };
        // 셈은 따로 돌린다(원본은 UI 스레드에서 세어 큰 임시 폴더면 잠시 멈춘다). 끝나면 empty_temp_counted가 묻는다.
        self.status = self.tr("임시 폴더 크기를 세고 있습니다…", "Counting the temporary folder…").into_owned();
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let summary = ndir::tools::folder_size_summary(std::slice::from_ref(&folder), &std::sync::atomic::AtomicBool::new(false)).unwrap_or_default();
            let _ = tx.send(AsyncResult::TempCounted { folder, files: summary.files, bytes: summary.total_bytes });
        });
    }

    pub(super) fn empty_temp_counted(&mut self, folder: PathBuf, files: u64, bytes: u64) {
        self.status.clear();
        let title = self.language_pack.text("RES.TITLE_EMPTYTEMP", "Empty a temporary folder");
        let text = self
            .language_pack
            .text("RES.MSG_EMPTYTEMP", "Are you sure you want to empty a temporary folder?\n%d files, %s")
            .replacen("%d", &files.to_string(), 1)
            .replacen("%s", &ndir::native::format_byte_size(bytes), 1);
        self.message_box(&title, "", &text, MB_YESNO | MB_ICONQUESTION, move |app, answer, _| {
            if answer != IDYES {
                return;
            }
            app.start_job("Delete".into(), ndir::fsops::Operation::EmptyFolder { folder }, super::file_operation::JobReport::Delete { recycle: false, skipped: 0 }, None);
            for pane in 0..2 {
                app.refresh(pane);
            }
        });
    }

    /// C# EmptyRecycleBin: 예/아니요(경고) → 비우기 → "휴지통을 비웠습니다.". 실패는 RunSafely 경고 상자.
    pub(super) fn empty_recycle_command(&mut self) {
        // 원본 miEmptyRecycleBinClick(0x433094): SHEmptyRecycleBinW(창, NULL, 0) — 확인·진행은 셸 창, NDIR 글 없음(스킨 테마).
        if self.skin_active() {
            let emptier = self.shell_recycle_emptier;
            thread::spawn(move || {
                let _ = emptier();
            });
            return;
        }
        let title = self.tr("휴지통 비우기", "Empty Recycle Bin").into_owned();
        let text = self.tr("휴지통의 모든 항목을 영구적으로 지울까요?", "Permanently delete every item in the Recycle Bin?").into_owned();
        self.system_box(&title, &text, MB_YESNO | MB_ICONEXCLAMATION, |app, answer, _| {
            if answer == IDYES {
                app.start_recycle_empty(None);
            }
        });
    }

    fn start_recycle_empty(&mut self, cleanup: Option<Vec<PathBuf>>) {
        let (tx, emptier) = (self.async_tx.clone(), self.recycle_emptier);
        thread::spawn(move || {
            let outcome = emptier();
            let _ = tx.send(AsyncResult::RecycleEmptied(cleanup, outcome));
        });
    }

    pub(super) fn recycle_emptied(&mut self, cleanup: Option<Vec<PathBuf>>, outcome: Result<(), String>) {
        let Some(paths) = cleanup else {
            return match outcome {
                Ok(()) => self.status = self.tr("휴지통을 비웠습니다.", "The Recycle Bin was emptied.").into_owned(),
                Err(_) => self.warn("휴지통을 비우지 못했습니다.", "Could not empty the Recycle Bin."),
            };
        };
        if let Err(error) = outcome {
            let text = format!("{}{error}", self.tr("휴지통을 비우지 못했습니다. 파일 정리도 취소했습니다.\n\n", "Could not empty the Recycle Bin. File cleanup was canceled.\n\n"));
            let title = self.tr("파일 정리", "File cleanup").into_owned();
            return self.system_box(&title, &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
        self.cleanup_protected_check(paths, true);
    }

    /// 정리 위치(시스템 임시 폴더, 인터넷 임시 파일, 쿠키)를 검색한다. 위치도 사용자 지정 결과도 휴지통도 없으면 알리기만 한다.
    pub(super) fn run_cleanup(&mut self, options: CleanupOptions) {
        let roots = cleanup_roots(&options);
        if roots.is_empty() && options.user_results.is_empty() && !options.recycle {
            self.status = self.tr("검색할 정리 위치가 없습니다.", "No cleanup folders are available to scan.").into_owned();
            return;
        }
        self.status = self.tr("정리 대상 파일을 검색하고 있습니다…", "Searching cleanup locations…").into_owned();
        self.start_cleanup_scan(roots, ScanPurpose::Cleanup(options));
    }

    /// CleanupScanProgressWindow.Run: 위치가 없으면 창 없이 빈 결과.
    fn start_cleanup_scan(&mut self, roots: Vec<(PathBuf, Vec<String>)>, purpose: ScanPurpose) {
        if roots.is_empty() {
            return self.cleanup_scanned(purpose, Ok(vec![]), false);
        }
        let modal = ModalProgress {
            stop: self.language_pack.text("RES.CANCEL", "Cancel"),
            stopping: None,
            stopping_text: text(&self.language_pack, "Caption.lblCanceling", "Canceling cleanup scan…"),
            kind: ModalKind::Cleanup { scanned: text(&self.language_pack, "Caption.lblScanned", "Items scanned"), matches: text(&self.language_pack, "Caption.lblMatches", "Matches") },
        };
        let title = text(&self.language_pack, "Caption.lblScanTitle", "Scanning cleanup locations");
        let tx = self.async_tx.clone();
        self.start_modal_task(title, modal, move |events, cancel| {
            let outcome = cleanup::scan(&roots, &cancel, &mut |scanned, matches, path| {
                let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes: matches as u64, files: scanned });
            });
            let _ = tx.send(AsyncResult::CleanupScanned(purpose, outcome, cancel.load(Ordering::Relaxed)));
            Ok(())
        });
    }

    pub(super) fn cleanup_scanned(&mut self, purpose: ScanPurpose, outcome: Result<Vec<Candidate>, String>, cancelled: bool) {
        let scan_title = text(&self.language_pack, "Caption.lblScanTitle", "Cleanup scan");
        let found = match outcome {
            Err(_) if cancelled => {
                if matches!(purpose, ScanPurpose::Cleanup(_)) {
                    self.status = self.tr("정리 검색을 취소했습니다.", "Cleanup scan canceled.").into_owned();
                }
                return;
            }
            Err(error) => return self.system_box(&scan_title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
            Ok(found) => found,
        };
        let options = match purpose {
            ScanPurpose::UserDefined => {
                if let Some(form) = self.cleanup_options_form() {
                    form.set_results(found);
                }
                return;
            }
            ScanPurpose::Cleanup(options) => options,
        };
        // 위치 검색 결과 + 고른 사용자 지정 파일(전체 경로 기준 처음 것).
        let mut candidates: Vec<Candidate> = vec![];
        let user = if options.user_defined { options.user_results.clone() } else { vec![] };
        for candidate in found.into_iter().chain(user) {
            if !candidates.iter().any(|known| same_path(&known.full_path.to_string_lossy(), &candidate.full_path.to_string_lossy())) {
                candidates.push(candidate);
            }
        }
        if candidates.is_empty() && !options.recycle {
            self.status = self.tr("정리할 파일이 없습니다.", "No files matched the cleanup filters.").into_owned();
            return;
        }
        let title = text(&self.language_pack, "Caption.lResult", "Cleanup results");
        self.push_original_dialog(title, DialogKind::CleanupResults(Box::new(CleanupResultsForm::new(candidates, options.recycle))), |app, result, _| {
            let DialogResult::CleanupResults(Some((paths, recycle))) = result else { return };
            if recycle {
                app.status = app.tr("휴지통을 비우고 있습니다…", "Emptying the Recycle Bin…").into_owned();
                return app.start_recycle_empty(Some(paths));
            }
            app.cleanup_protected_check(paths, false);
        });
    }

    /// FindProtectedDeleteItems → 항목마다 TfDelConfirm(삭제 흐름과 같은 protected_step) → 정리 삭제.
    fn cleanup_protected_check(&mut self, paths: Vec<PathBuf>, recycle: bool) {
        if paths.is_empty() {
            return self.cleanup_delete(paths, vec![], recycle);
        }
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let items = fsops::find_protected_delete_items(&paths);
            let run = file_commands::DeleteRun { paths, items, index: 0, approve_all: false, skipped: vec![], cleanup: Some(recycle) };
            let _ = tx.send(AsyncResult::ProtectedItems(run));
        });
    }

    /// DeleteCleanupFiles를 진행 창("선택 파일 정리", 일시 정지 없음)으로. 지울 것이 없으면 바로 끝낸다.
    pub(super) fn cleanup_delete(&mut self, paths: Vec<PathBuf>, skipped: Vec<PathBuf>, recycle: bool) {
        let requested = paths.len();
        let skipped_count = skipped.len();
        let targets: Vec<PathBuf> = paths.into_iter().filter(|p| !skipped.iter().any(|s| same_path(&s.to_string_lossy(), &p.to_string_lossy()))).collect();
        let done = CleanupDone { requested, to_delete: targets.len(), skipped: skipped_count, recycle };
        if targets.is_empty() {
            return self.cleanup_finished(done, cleanup::DeleteResult::default(), false);
        }
        // C#은 정리 삭제를 파일 작업 진행 창(일시 정지 없음)으로 보여 준다. 16차에서 ModalKind::Files를 없앴으므로
        // 파일 작업 진행 창(ProgressInfo, 파일 수 막대, allow_pause 없음)으로 돈다.
        let title = self.tr("선택 파일 정리", "Clean selected files").into_owned();
        let preparing = self.tr("작업을 준비하고 있습니다.", "Preparing the operation.").into_owned();
        let total = targets.len() as u64;
        let tx = self.async_tx.clone();
        let started = self.start_task(title, move |events, cancel| {
            let result = cleanup::delete_files(&targets, &cancel, &mut |completed, path| {
                let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes: 0, files: completed });
            });
            let _ = tx.send(AsyncResult::CleanupDeleted(done, result, cancel.load(Ordering::Relaxed)));
            Ok(())
        });
        if started {
            if let Some(job) = self.job.as_mut() {
                job.path = preparing;
                job.progress = Some(super::file_operation::ProgressInfo { total_files: Some(total), ..Default::default() });
            }
        }
    }

    /// 두 패널 새로 고침(지울 파일이 있었으면), 상태 줄, 실패 목록 글 창.
    pub(super) fn cleanup_finished(&mut self, done: CleanupDone, result: cleanup::DeleteResult, cancelled: bool) {
        if done.requested > 0 {
            self.refresh(0);
            self.refresh(1);
        }
        let ko = self.language == "ko";
        let deleted = n0((result.successful as usize).min(done.to_delete));
        let (errors, skipped) = (n0(result.errors.len()), n0(done.skipped));
        self.status = if cancelled || result.cancelled {
            if ko { format!("{deleted}개 파일 정리 후 취소했습니다. 실패 {errors}개, 건너뜀 {skipped}개") } else { format!("Cleanup canceled after {deleted} files. Failed: {errors}, skipped: {skipped}") }
        } else if ko {
            format!("{deleted}개 파일을 정리했습니다. 휴지통 비우기 {}. 실패 {errors}개, 건너뜀 {skipped}개", if done.recycle { "완료" } else { "안 함" })
        } else {
            format!("Cleaned up {deleted} files. Recycle Bin emptying: {}. Failed: {errors}, skipped: {skipped}", if done.recycle { "complete" } else { "not selected" })
        };
        if !result.errors.is_empty() {
            let mut text = result.errors.iter().take(ERROR_LINES).cloned().collect::<Vec<_>>().join("\r\n");
            if result.errors.len() > ERROR_LINES {
                let rest = n0(result.errors.len() - ERROR_LINES);
                text += &if ko { format!("\r\n… 나머지 {rest}개 오류") } else { format!("\r\n… {rest} more errors") };
            }
            let title = self.tr("정리하지 못한 파일", "Files that could not be cleaned up").into_owned();
            self.text = Some(TextDocument { path: None, title, text, editable: false, dirty: false });
        }
        // 원본 FUN_0041c77c: 정리 스레드가 끝나면 MessageBoxW(MSG_CLEANUP_DONE, 정리 창 제목, MB_ICONASTERISK). 취소는 제외.
        if !(cancelled || result.cancelled) {
            let title = self.cleanup_title();
            let text = self.language_pack.text("RES.MSG_CLEANUP_DONE", "Cleanup completed.");
            self.system_box(&title, &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
        }
    }

    /// 두 창의 단추.
    pub(super) fn cleanup_action(&mut self, action: CleanupAction, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let title = self.original_dialogs.last().map(|d| d.title.clone()).unwrap_or_default();
        if self.cleanup_results_form().is_some() {
            return self.cleanup_results_action(action, &title, ctx);
        }
        let folder_picker = self.folder_picker;
        let Some(form) = self.cleanup_options_form() else { return };
        match action {
            CleanupAction::Search => {
                form.clear_results();
                let roots = form.search_roots();
                self.start_cleanup_scan(roots, ScanPurpose::UserDefined);
            }
            CleanupAction::Browse => {
                let picker = if ko { "정리할 위치 선택" } else { "Select cleanup location" };
                let initial = if Path::new(&form.location_input).is_dir() { PathBuf::from(&form.location_input) } else { PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()) };
                if let Some(folder) = folder_picker(picker, Some(&initial)) {
                    let path = folder.to_string_lossy().into_owned();
                    form.add_unique(&path);
                    let full = std::path::absolute(&folder).map(|p| p.to_string_lossy().into_owned()).unwrap_or(path);
                    form.select_location(&full);
                }
            }
            CleanupAction::Remove => {
                if let Some(index) = form.selected_location.filter(|i| *i < form.locations.len()) {
                    form.locations.remove(index);
                    form.invalidate();
                    form.selected_location = (!form.locations.is_empty()).then(|| index.min(form.locations.len() - 1));
                }
            }
            CleanupAction::SelectAll | CleanupAction::SelectNone => {
                let value = matches!(action, CleanupAction::SelectAll);
                form.results.iter_mut().for_each(|row| row.1 = value);
            }
            CleanupAction::Copy(text) => {
                if let Err(error) = (self.clipboard_writer)(&text) {
                    self.status = error;
                }
            }
            CleanupAction::Close => self.answer_dialog(DialogResult::CleanupOptions(None), ctx),
            CleanupAction::Accept => {
                if !(form.system_temp || form.internet_temp || form.cookies || form.recycle || form.user_defined) {
                    let text = if ko { "정리할 항목을 하나 이상 선택하세요." } else { "Select at least one cleanup source." };
                    return self.system_box(&title, text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
                }
                if form.user_defined && !form.searched {
                    let text = if ko { "사용자 지정 위치에서 먼저 검색하세요." } else { "Search the user-defined locations before starting cleanup." };
                    return self.system_box(&title, text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
                }
                let options = CleanupOptions {
                    system_temp: form.system_temp,
                    internet_temp: form.internet_temp,
                    cookies: form.cookies,
                    recycle: form.recycle,
                    user_defined: form.user_defined,
                    extensions: form.extensions.trim().to_owned(),
                    locations: form.locations.clone(),
                    user_results: if form.user_defined { form.results.iter().filter(|r| r.1).map(|r| r.0.clone()).collect() } else { vec![] },
                };
                self.answer_dialog(DialogResult::CleanupOptions(Some(options)), ctx);
            }
        }
    }

    /// CleanupResultsWindow.Commit: 빈 선택 알림 → MSG_CLEANUP 예/아니요 → (휴지통이면) 비우기 확인 → 닫고 결과.
    fn cleanup_results_action(&mut self, action: CleanupAction, title: &str, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let Some(form) = self.cleanup_results_form() else { return };
        match action {
            CleanupAction::SelectAll => form.grid.select_all(),
            CleanupAction::SelectNone => form.grid.clear_selection(),
            CleanupAction::Copy(text) => {
                if let Err(error) = (self.clipboard_writer)(&text) {
                    self.status = error;
                }
            }
            CleanupAction::Accept => {
                let selected: Vec<PathBuf> = form.grid.selected_in_order().into_iter().map(|i| form.candidates[i].full_path.clone()).collect();
                let recycle = form.recycle;
                if selected.is_empty() && !recycle {
                    let text = if ko { "정리할 파일을 하나 이상 선택하세요." } else { "Select at least one file to clean up." };
                    return self.system_box(title, text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
                }
                let title = title.to_owned();
                let confirm_recycle = move |app: &mut App, selected: Vec<PathBuf>, ctx: &egui::Context| {
                    if !recycle {
                        return app.answer_dialog(DialogResult::CleanupResults(Some((selected, false))), ctx);
                    }
                    let count = n0(selected.len());
                    let summary = if app.language == "ko" {
                        format!("모든 드라이브의 휴지통을 먼저 비웁니다.\n{}\n\n휴지통 비우기는 되돌릴 수 없습니다. 계속할까요?", if selected.is_empty() { String::new() } else { format!("\n그 후 선택한 {count}개 파일을 영구 삭제합니다.") })
                    } else {
                        format!("This empties all Recycle Bins on all drives.\n{}\n\nEmptying the Recycle Bin cannot be undone. Continue?", if selected.is_empty() { String::new() } else { format!("\nThen it will permanently delete {count} selected files.") })
                    };
                    let caption = app.tr("휴지통 비우기 확인", "Confirm Empty Recycle Bin").into_owned();
                    app.system_box(&caption, &summary, MB_YESNO | MB_ICONEXCLAMATION, move |app, answer, ctx| {
                        if answer == IDYES {
                            app.answer_dialog(DialogResult::CleanupResults(Some((selected, true))), ctx);
                        }
                    });
                };
                if selected.is_empty() {
                    return confirm_recycle(self, selected, ctx);
                }
                let question = self.language_pack.text("RES.MSG_CLEANUP", "Are you sure you want to delete all selected files?");
                self.system_box(&title, &question, MB_YESNO | MB_ICONQUESTION, move |app, answer, ctx| {
                    if answer == IDYES {
                        confirm_recycle(app, selected, ctx);
                    }
                });
            }
            CleanupAction::Close => self.answer_dialog(DialogResult::CleanupResults(None), ctx),
            _ => {}
        }
    }
}
