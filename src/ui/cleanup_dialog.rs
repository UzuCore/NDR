//! 원본 TfCleanup(컴퓨터 청소, btnOkClick 0x41b5cc), 임시 폴더 비우기(miEmptyTempClick), 휴지통 비우기(miEmptyRecycleBinClick).
//! 휴지통 비우기는 `App::shell_recycle_emptier`를 거친다(시험은 실제 휴지통을 건드리지 않는다).
//! 차이(HANDOFF): 위치 목록의 Insert/Delete 글쇠는 목록에 초점이 없어도 글 칸이 초점을 갖지 않으면 듣는다. 사용자 지정 검색은
//! 원본(FUN_0041bedc, 창 안에서 lResult에 지금 폴더를 보이며 찾음)과 달리 작은 진행 창으로 찾는다.
use super::grid::{self, Column, GridState, GridView};
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::cleanup::{self, Candidate};
use ndr::msgbox::{IDYES, MB_ICONQUESTION, MB_YESNO};

fn same_path(a: &str, b: &str) -> bool {
    ndr::names::equals_ignore_case(a, b)
}

/// 원본 btnOkClick이 지우기 작업의 +0x844에 넣는, 지우지 않을 파일 이름.
pub(super) const KEEP_NAMES: [&str; 2] = ["desktop.ini", "index.dat"];

/// TfCleanup의 입력. results는 사용자 지정 검색 결과와 확인란(처음 켬).
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
    /// 원본 btnSearchClick(0x41c338): 찾는 동안(btnSearch.Tag = 1, 글 = STOP) 멈춤 표시(+0x434)와 lResult에 보일 지금 폴더.
    pub search_stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub current: String,
}

impl Drop for CleanupOptionsForm {
    /// 창을 닫으면 찾기도 멈춘다.
    fn drop(&mut self) {
        if let Some(stop) = &self.search_stop {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

impl CleanupOptionsForm {
    /// 원본 TntFormShow: 고정 드라이브(FUN_0041bbe8, GetDriveTypeW = 3) → 저장한 위치 순으로(같은 경로는 한 번).
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
            search_stop: None,
            current: String::new(),
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

    /// 환경 변수를 풀고 전체 경로로(같은 경로는 한 번).
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

    /// 확장자나 위치가 바뀌면 결과를 비우고 다시 검색해야 한다.
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

    /// 위치마다 cbExtensions의 확장자(";"로 나눔).
    pub(super) fn search_roots(&self) -> Vec<(PathBuf, Vec<String>)> {
        let patterns = cleanup::parse_patterns(&self.extensions);
        self.locations.iter().map(|path| (PathBuf::from(path), patterns.clone())).collect()
    }
}

/// 창의 단추와 글쇠.
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
    use super::original_dialogs::put_left;
    let _ = ko;
    let t = |key: &str, fallback: &str| text(language, key, fallback);
    // VCL Caption 그대로('&' 단축 문자).
    let r = |key: &str, fallback: &str| language.raw(&format!("formCleanup.{key}")).unwrap_or_else(|| fallback.to_owned());
    use super::original_dialogs::{access_checkbox, vcl_button};
    let mut action = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(554.0, 507.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    // 그룹 칸을 먼저 그리고(chkUserDefined가 제목 자리를 덮는다), 제목 글은 비워 둔다.
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    ui.painter().rect_stroke(at(16.0, 122.0, 521.0, 371.0), 2.0, stroke, egui::StrokeKind::Inside);
    ui.painter().rect_filled(at(18.0, 112.0, 167.0, 17.0), 0.0, ui.visuals().window_fill);
    for (top, value, key, fallback) in [
        (20.0, &mut form.system_temp, "Caption.chkTemp", "System &Temporary Files"),
        (43.0, &mut form.internet_temp, "Caption.chkInternetTemp", "&Internet Explorer Temporary Files"),
        (66.0, &mut form.cookies, "Caption.chkInternetCookie", "Internet Explorer &Cookies"),
        (89.0, &mut form.recycle, "Caption.chkRecycleBin", "Empty &Recycle Bin"),
    ] {
        access_checkbox(ui, at(20.0, top, 338.0, 17.0), value, &r(key, fallback));
    }
    access_checkbox(ui, at(20.0, 112.0, 161.0, 17.0), &mut form.user_defined, &r("Caption.chkUserDefined", "&User Defined Files"));
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
        if vcl_button(ui, at(450.0, 163.0, 75.0, 25.0), true, &r("Caption.btnAdd", "&Add")).1 {
            action = Some(CleanupAction::Browse);
        }
        if vcl_button(ui, at(450.0, 194.0, 75.0, 25.0), true, &r("Caption.btnDelete", "&Delete")).1 {
            action = Some(CleanupAction::Remove);
        }
        // 원본 lbLocationKeyDown(0x41cac0): 목록에서 Delete = btnDeleteClick, Insert = btnAddClick(0x41bd68).
        // NDR은 목록에서 줄을 고른 뒤 글 칸에 초점이 없을 때 받는다.
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
        // 원본 btnSearchClick: 찾는 동안 단추 글은 STOP(누르면 멈춤).
        let search_caption = if form.search_stop.is_some() { language.raw("RES.STOP").unwrap_or_else(|| "Stop".into()) } else { r("Caption.btnSearch", "&Search") };
        if vcl_button(ui, at(450.0, 248.0, 75.0, 25.0), true, &search_caption).1 {
            action = Some(CleanupAction::Search);
        }
        // 원본(0x41c549): 찾는 동안 lResult = 들어간 폴더(epPathEllipsis), 끝나면 SEARCHRESULT + Format(" (%d)", 결과 수)
        // (천 단위 구분 없음). 검색 전은 DFM 글.
        let label = if form.search_stop.is_some() {
            super::original_dialogs::path_ellipsis(ui, &form.current, 489.0, &egui::FontId::proportional(13.0))
        } else if form.searched {
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
        // 원본 chkUserDefinedClick(0x41bcc0): btnSelectAll·btnDeselectAll도 확인란만 따른다(결과가 없어도 켜짐).
        if vcl_button(ui, at(36.0, 458.0, 85.0, 25.0), true, &r("Caption.btnSelectAll", "Select All(&Z)")).1 && has_results {
            action = Some(CleanupAction::SelectAll);
        }
        if vcl_button(ui, at(127.0, 458.0, 84.0, 25.0), true, &r("Caption.btnDeselectAll", "Deselect All(&X)")).1 && has_results {
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
    // 원본 btnSearchClick: 찾는 동안 btnOk(+0x418)는 꺼진다.
    if vcl_button(ui, at(440.0, 16.0, 97.0, 53.0), form.search_stop.is_none(), &r("Caption.btnOk", "Start")).1 {
        action = Some(CleanupAction::Accept);
    }
    if vcl_button(ui, at(440.0, 84.0, 97.0, 25.0), true, &r("Caption.btnClose", "Close")).1 {
        action = Some(CleanupAction::Close);
    }
    if enter && action.is_none() {
        action = Some(CleanupAction::Accept);
    }
    action
}
impl App {
    fn cleanup_title(&self) -> String {
        self.language_pack.text("formCleanup.Form.Caption", "Disk Clean-Up")
    }

    fn cleanup_options_form(&mut self) -> Option<&mut CleanupOptionsForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::CleanupOptions(form)) => Some(form),
            _ => None,
        }
    }

    /// 원본 컴퓨터 청소: TfCleanup을 띄운다. btnClose로 닫으면 사용자 지정 위치·확장자를 설정에 남긴다.
    pub(super) fn cleanup_command(&mut self) {
        let extensions = self.settings.text("UserDefinedCleanupExtensions", cleanup::DEFAULT_EXTENSIONS);
        let form = CleanupOptionsForm::new(extensions, &self.settings.strings("UserDefinedCleanupLocations"));
        let title = self.cleanup_title();
        self.push_original_dialog(title, DialogKind::CleanupOptions(Box::new(form)), |app, result, _| {
            let DialogResult::CleanupOptions(Some((locations, extensions))) = result else { return };
            app.settings.set_strings("UserDefinedCleanupLocations", locations);
            app.settings.set_text("UserDefinedCleanupExtensions", &extensions);
            app.save();
        });
    }

    /// 원본 miEmptyTempClick(0x434344): GetTempPathW → GetLongPathNameW(실패하면 그만), 폴더 크기(StrFormatByteSizeW)와
    /// 파일 수로 TfMsgBox(TITLE_EMPTYTEMP, "", MSG_EMPTYTEMP, MB_YESNO | MB_ICONQUESTION). 예면 지우기 작업(종류 2,
    /// 플래그 0xF0 = 안의 항목만, 묻지 않고, 오류는 건너뛰고 알리지 않음)을 시작하고 두 패널을 새로 읽는다.
    pub(super) fn empty_temp_command(&mut self) {
        let Some(folder) = (self.system.temp_folder)() else { return };
        // 셈은 따로 돌린다(원본은 UI 스레드에서 세어 큰 임시 폴더면 잠시 멈춘다). 끝나면 empty_temp_counted가 묻는다.
        self.status = self.tr("임시 폴더 크기를 세고 있습니다…", "Counting the temporary folder…").into_owned();
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let summary = ndr::tools::folder_size_summary(std::slice::from_ref(&folder), &std::sync::atomic::AtomicBool::new(false)).unwrap_or_default();
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
            .replacen("%s", &ndr::native::format_byte_size(bytes), 1);
        self.message_box(&title, "", &text, MB_YESNO | MB_ICONQUESTION, move |app, answer, _| {
            if answer != IDYES {
                return;
            }
            app.start_job("Delete".into(), ndr::fsops::Operation::EmptyFolder { folder }, super::file_operation::JobReport::Delete { recycle: false, skipped: 0 }, None);
            for pane in 0..2 {
                app.refresh(pane);
            }
        });
    }

    /// 원본 miEmptyRecycleBinClick(0x433094): SHEmptyRecycleBinW(창, NULL, 0) — 확인·진행은 셸 창, NDR 글 없음.
    pub(super) fn empty_recycle_command(&mut self) {
        let emptier = self.shell_recycle_emptier;
        thread::spawn(move || {
            let _ = emptier();
        });
    }

    /// 원본 btnSearchClick(0x41c338, Tag 0): 결과를 비우고, btnOk를 끄고, 단추 글을 STOP으로 바꾼 뒤 위치마다 FUN_0041bedc로
    /// 찾는다(창은 그대로 쓸 수 있고 lResult에 지금 폴더). 다시 누르면(Tag 1) 멈춘다(+0x434). 끝나면 cleanup_scanned.
    fn start_cleanup_search(&mut self) {
        let tx = self.async_tx.clone();
        let Some(form) = self.cleanup_options_form() else { return };
        if let Some(stop) = &form.search_stop {
            stop.store(true, Ordering::Relaxed);
            return;
        }
        form.clear_results();
        let roots = form.search_roots();
        let stop = Arc::new(AtomicBool::new(false));
        form.search_stop = Some(stop.clone());
        form.current = roots.first().map(|(p, _)| p.to_string_lossy().into_owned()).unwrap_or_default();
        thread::spawn(move || {
            let mut last = Instant::now();
            let found = cleanup::search_until_stopped(&roots, &stop, &mut |folder| {
                // 글은 화면이 따라갈 만큼만 보낸다.
                if last.elapsed() >= Duration::from_millis(50) {
                    last = Instant::now();
                    let _ = tx.send(AsyncResult::CleanupSearchFolder(folder.to_string_lossy().into_owned()));
                }
            });
            let _ = tx.send(AsyncResult::CleanupScanned(found));
        });
    }

    /// 찾는 중 들어간 폴더(lResult).
    pub(super) fn cleanup_search_folder(&mut self, folder: String) {
        if let Some(form) = self.cleanup_options_form().filter(|f| f.search_stop.is_some()) {
            form.current = folder;
        }
    }

    /// 찾기가 끝나거나 멈춤: 찾은 것을 lvResult에 넣고(모두 확인 표시) 단추를 되돌린다.
    pub(super) fn cleanup_scanned(&mut self, found: Vec<Candidate>) {
        if let Some(form) = self.cleanup_options_form() {
            form.search_stop = None;
            form.current.clear();
            form.set_results(found);
        }
    }

    /// 원본 btnOkClick(0x41b5cc): MessageBoxW(MSG_CLEANUP, 창 제목, MB_YESNO | MB_ICONQUESTION). 예면 chkRecycleBin이면
    /// SHEmptyRecycleBinW(셸 확인 창), 지우기 작업(종류 2, 플래그 0xF0)에 chkTemp = 임시 폴더, chkInternetCookie = 쿠키 폴더,
    /// chkInternetTemp = 인터넷 임시 파일 폴더(안의 항목만), chkUserDefined = 확인 표시한 검색 결과 파일을 넣고 desktop.ini·index.dat는
    /// 남긴다. 창은 그대로 두고, 작업이 끝나면 MSG_CLEANUP_DONE을 알린다(JobReport::Cleanup).
    fn start_cleanup(&mut self, title: String) {
        let temp = (self.system.temp_folder)();
        let special = self.system.special_folder;
        let Some(form) = self.cleanup_options_form() else { return };
        let recycle = form.recycle;
        let mut folders = Vec::new();
        if form.system_temp {
            folders.extend(temp);
        }
        if form.cookies {
            folders.extend(special(0x21));
        }
        if form.internet_temp {
            folders.extend(special(0x20));
        }
        let files: Vec<PathBuf> = if form.user_defined { form.results.iter().filter(|(_, checked)| *checked).map(|(c, _)| c.full_path.clone()).collect() } else { vec![] };
        let question = self.language_pack.text("RES.MSG_CLEANUP", "Are you sure you want to delete all selected files?");
        self.system_box(&title, &question, MB_YESNO | MB_ICONQUESTION, move |app, answer, _| {
            if answer != IDYES {
                return;
            }
            if recycle {
                let emptier = app.shell_recycle_emptier;
                thread::spawn(move || {
                    let _ = emptier();
                });
            }
            let keep_names = KEEP_NAMES.iter().map(|n| (*n).to_owned()).collect();
            app.start_job("Delete".into(), ndr::fsops::Operation::Cleanup { folders, files, keep_names }, super::file_operation::JobReport::Cleanup, None);
        });
    }

    /// 창의 단추.
    pub(super) fn cleanup_action(&mut self, action: CleanupAction, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let title = self.original_dialogs.last().map(|d| d.title.clone()).unwrap_or_default();
        let folder_picker = self.folder_picker;
        let Some(form) = self.cleanup_options_form() else { return };
        match action {
            CleanupAction::Search => self.start_cleanup_search(),
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
            CleanupAction::Close => {
                let saved = (form.locations.clone(), form.extensions.trim().to_owned());
                self.answer_dialog(DialogResult::CleanupOptions(Some(saved)), ctx);
            }
            CleanupAction::Accept => self.start_cleanup(title),
        }
    }
}
