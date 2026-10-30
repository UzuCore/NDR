//! C# SearchWindow + SearchAsync(파일 및 폴더 검색): 검색 위치 목록(사전 설정·찾아보기·추가·위/아래/제거·저장), 이름 기록,
//! 범주, 확장자, 내용, 크기 범위, 날짜 범위, 날짜 기준, 옵션. 결과는 C#처럼 활성 패널 목록이 된다(F5로 원래 목록).
//! 규칙은 ndir::tools::search_advanced. 차이: 날짜는 달력 대신 "yyyy-MM-dd" 글자 입력.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use super::tools_dialog::network_share_code;
use ndir::msgbox::{MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_OK};
use ndir::tools::{SearchCategory, SearchCriteria, SearchTimeKind};

const MAXIMUM_PATTERN_HISTORY: usize = 20;

#[derive(Clone, Debug)]
pub(super) struct SearchForm {
    pub root: String,
    pub locations: Vec<String>,
    pub selected_location: Option<usize>,
    pub patterns: String,
    pub history: Vec<String>,
    pub category: usize,
    pub extensions: String,
    pub content: String,
    pub minimum: String,
    pub minimum_unit: usize,
    pub maximum: String,
    pub maximum_unit: usize,
    pub date_from: String,
    pub time_from: String,
    pub date_to: String,
    pub time_to: String,
    pub time_kind: usize,
    pub recursive: bool,
    pub hidden: bool,
    pub files: bool,
    pub folders: bool,
    pub match_case: bool,
    pub custom2: Option<String>,
}

/// SearchWindow 버튼과 사전 설정의 동작.
pub(super) enum SearchAction {
    Search,
    Cancel,
    Browse,
    Add,
    Preset(usize),
    Up,
    Down,
    Remove,
    SaveLocations,
}

/// NormalizeLocation: 따옴표·공백을 떼고 환경 변수를 풀어 전체 경로로.
fn normalize_location(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"');
    if value.is_empty() {
        return None;
    }
    let expanded = native::expand_environment(value);
    std::path::absolute(expanded).ok().map(|p| p.to_string_lossy().into_owned())
}

/// TrySize: 비면 None, 0 이상 소수 × 단위(bytes, KiB, MiB, GiB).
fn size(value: &str, unit: usize) -> Result<Option<u64>, ()> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let amount: f64 = value.trim().parse().map_err(|_| ())?;
    if amount < 0.0 || value.contains(['+', '-', 'e', 'E']) {
        return Err(());
    }
    let multiplier = [1.0, 1024.0, 1024.0 * 1024.0, 1024.0 * 1024.0 * 1024.0][unit.min(3)];
    let bytes = (amount * multiplier).trunc();
    if bytes > i64::MAX as f64 {
        return Err(());
    }
    Ok(Some(bytes as u64))
}

/// TryDateTime: 날짜가 없으면(시간도 없어야) None. 시간이 없으면 시작은 0시, 끝은 그날 마지막 순간. 끝은 분/초 끝까지 포함.
fn date_time(date: &str, time: &str, inclusive_end: bool) -> Result<Option<std::time::SystemTime>, ()> {
    let (date, time) = (date.trim(), time.trim());
    if date.is_empty() {
        return if time.is_empty() { Ok(None) } else { Err(()) };
    }
    let day = ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"].iter().find_map(|f| chrono::NaiveDate::parse_from_str(date, f).ok()).ok_or(())?;
    let local = |value: chrono::NaiveDateTime| value.and_local_timezone(chrono::Local).earliest().map(std::time::SystemTime::from).ok_or(());
    if time.is_empty() {
        let start = local(day.and_hms_opt(0, 0, 0).ok_or(())?)?;
        return Ok(Some(if inclusive_end { start + Duration::from_secs(86_400) - Duration::from_nanos(100) } else { start }));
    }
    let (format, step) = match time.len() {
        5 => ("%H:%M", 60),
        8 => ("%H:%M:%S", 1),
        _ => return Err(()),
    };
    let parsed = local(day.and_time(chrono::NaiveTime::parse_from_str(time, format).map_err(|_| ())?))?;
    Ok(Some(if inclusive_end { parsed + Duration::from_secs(step) - Duration::from_nanos(100) } else { parsed }))
}

fn add_unique(form: &mut SearchForm, value: &str) {
    if let Some(path) = normalize_location(value) {
        if !form.locations.iter().any(|known| ndir::names::equals_ignore_case(known.trim_end_matches('\\'), path.trim_end_matches('\\'))) {
            form.locations.push(path);
        }
    }
}

impl SearchForm {
    pub(super) fn new(initial: &str, pattern: &str, saved: &[String], history: &[String], custom2: Option<String>) -> Self {
        let mut form = Self {
            root: initial.into(),
            locations: vec![],
            selected_location: None,
            patterns: if pattern.trim().is_empty() { "*".into() } else { pattern.into() },
            history: vec![],
            category: 0,
            extensions: "*.*".into(),
            content: String::new(),
            minimum: String::new(),
            minimum_unit: 0,
            maximum: String::new(),
            maximum_unit: 0,
            date_from: String::new(),
            time_from: String::new(),
            date_to: String::new(),
            time_to: String::new(),
            time_kind: 0,
            recursive: true,
            hidden: false,
            files: true,
            folders: false,
            match_case: false,
            custom2,
        };
        for item in history.iter().filter(|p| !p.trim().is_empty()) {
            if !form.history.iter().any(|h| h.eq_ignore_ascii_case(item)) && form.history.len() < MAXIMUM_PATTERN_HISTORY {
                form.history.push(item.clone());
            }
        }
        for path in saved {
            add_unique(&mut form, path);
        }
        add_unique(&mut form, initial);
        form
    }

    /// AddLocation: 있는 폴더(또는 UNC)만, 칸에도 그 경로를 넣는다. 없으면 Err.
    pub(super) fn add_location(&mut self, value: &str) -> Result<(), ()> {
        let path = normalize_location(value).filter(|p| Path::new(p).is_dir() || p.starts_with("\\\\")).ok_or(())?;
        self.root = path.clone();
        add_unique(self, &path);
        self.selected_location = self.locations.iter().position(|l| l.eq_ignore_ascii_case(&path));
        Ok(())
    }

    /// AddAllDrives: 네트워크가 아닌 드라이브 전부.
    pub(super) fn add_all_drives(&mut self) {
        let drives: Vec<String> = native::drives().into_iter().map(|d| d.to_string_lossy().into_owned()).filter(|d| Path::new(d).is_dir() && !native::is_network_drive(Path::new(d))).collect();
        for drive in &drives {
            add_unique(self, drive);
        }
        if let Some(first) = drives.first() {
            self.root = first.clone();
            self.selected_location = self.locations.iter().position(|l| l.eq_ignore_ascii_case(first));
        }
    }

    /// Commit의 검사 순서와 글(위치 → 없는 위치 → 파일/폴더 → 크기 → 날짜). 맞으면 검색 조건.
    /// `connect`: C# connectNetworkShare(UNC면 오류 번호, 자격 증명 실패면 연결 창 결과). 연결 창 취소(1223)면 Err("") = 아무것도 안 함.
    pub(super) fn criteria(&mut self, ko: bool, connect: &dyn Fn(&str) -> u32) -> Result<SearchCriteria, String> {
        if self.locations.is_empty() {
            let _ = self.add_location(&self.root.clone());
        }
        if self.locations.is_empty() {
            return Err(if ko { "검색할 위치를 하나 이상 추가하세요." } else { "Add at least one search location." }.into());
        }
        let mut missing: Vec<String> = Vec::new();
        for path in &self.locations {
            if Path::new(path).is_dir() {
                continue;
            }
            if path.starts_with("\\\\") {
                let code = connect(path);
                if code == 1223 {
                    return Err(String::new());
                }
                if code == 0 && Path::new(path).is_dir() {
                    continue;
                }
                missing.push(if code == 0 { path.clone() } else { format!("{path} (Windows {code}: {})", native::win32_message(code)) });
                continue;
            }
            missing.push(path.clone());
        }
        if !missing.is_empty() {
            let head = if ko { "다음 검색 위치를 찾을 수 없습니다. 목록에서 제거하거나 다시 선택하세요." } else { "These search locations cannot be found. Remove them or choose them again." };
            return Err(format!("{head}\n{}", missing.join("\r\n")));
        }
        if !self.files && !self.folders {
            return Err(if ko { "파일 또는 폴더를 하나 이상 선택하세요." } else { "Select files, folders or both." }.into());
        }
        let sizes = (size(&self.minimum, self.minimum_unit), size(&self.maximum, self.maximum_unit));
        let (Ok(minimum), Ok(maximum)) = sizes else {
            return Err(if ko { "크기는 0 이상의 숫자로 입력하고 최소 크기는 최대 크기보다 작아야 합니다." } else { "Enter sizes of 0 or more; the minimum must not exceed the maximum." }.into());
        };
        if minimum.zip(maximum).is_some_and(|(a, b)| a > b) {
            return Err(if ko { "크기는 0 이상의 숫자로 입력하고 최소 크기는 최대 크기보다 작아야 합니다." } else { "Enter sizes of 0 or more; the minimum must not exceed the maximum." }.into());
        }
        let dates = (date_time(&self.date_from, &self.time_from, false), date_time(&self.date_to, &self.time_to, true));
        let date_error = if ko { "날짜를 선택하고 시각은 HH:mm 또는 HH:mm:ss 형식으로 입력하세요. 시작은 끝보다 늦을 수 없습니다." } else { "Pick dates and enter times as HH:mm or HH:mm:ss. The start cannot be later than the end." };
        let (Ok(date_from), Ok(date_to)) = dates else { return Err(date_error.into()) };
        if date_from.zip(date_to).is_some_and(|(a, b)| a > b) {
            return Err(date_error.into());
        }
        let name_patterns = if self.patterns.trim().is_empty() { "*".to_owned() } else { self.patterns.trim().to_owned() };
        let mut history = vec![name_patterns.clone()];
        for item in &self.history {
            if !history.iter().any(|h| h.eq_ignore_ascii_case(item)) && history.len() < MAXIMUM_PATTERN_HISTORY {
                history.push(item.clone());
            }
        }
        self.history = history;
        Ok(SearchCriteria {
            roots: self.locations.iter().map(PathBuf::from).collect(),
            name_patterns,
            extension_patterns: self.extensions.trim().into(),
            content: self.content.clone(),
            recursive: self.recursive,
            include_hidden: self.hidden,
            include_files: self.files,
            include_folders: self.folders,
            match_case: self.match_case,
            minimum_bytes: minimum,
            maximum_bytes: maximum,
            date_from,
            date_to,
            time_kind: [SearchTimeKind::Any, SearchTimeKind::Modified, SearchTimeKind::Created, SearchTimeKind::Accessed][self.time_kind.min(3)],
            category: [SearchCategory::All, SearchCategory::Documents, SearchCategory::Pictures, SearchCategory::Audio, SearchCategory::Video, SearchCategory::Archives, SearchCategory::Applications, SearchCategory::Folders, SearchCategory::Other][self.category.min(8)],
        })
    }
}

/// 창 그리기.
pub(super) fn search_ui(ui: &mut egui::Ui, form: &mut SearchForm, language: &compat::Language, ko: bool, enter: bool) -> Option<SearchAction> {
    // C# LanguageService.TranslateControls: 창 글(한국어)을 ko.lang에서 찾아 그 키의 지금 언어 글로([formNDIRSearch] 등). 없으면 그대로.
    let _ = ko;
    let t = |ko_text: &str, en: &str| language.translate_pair(ko_text, en);
    let mut action = None;
    ui.set_width(750.0);
    egui::Grid::new("search-form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label(t("검색 위치", "Search in"));
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut form.root).desired_width(400.0));
                if ui.button(t("찾아보기…", "Browse…")).clicked() {
                    action = Some(SearchAction::Browse);
                }
                if ui.button(t("추가", "Add")).clicked() {
                    action = Some(SearchAction::Add);
                }
            });
            let presets = [
                language.text("NDIRSearch.AllLocalDrives", &t("모든 로컬 드라이브", "All local drives")),
                language.text("NDIRSearch.DriveC", "C:"),
                language.text("NDIRSearch.DriveD", "D:"),
                language.text("NDIRSearch.DriveE", "E:"),
                language.text("NDIRSearch.DriveF", "F:"),
                language.text("NDIRSearch.Documents", "Documents"),
                language.text("NDIRSearch.Custom2", &t("사용자지정2", "Custom 2")),
            ];
            egui::ComboBox::from_id_salt("search-presets").width(300.0).selected_text("").show_opaque(ui, |ui| {
                for (index, label) in presets.iter().enumerate() {
                    if ui.selectable_label(false, label).clicked() {
                        action = Some(SearchAction::Preset(index));
                    }
                }
            });
            egui::ScrollArea::vertical().id_salt("search-locations").max_height(100.0).min_scrolled_height(100.0).show(ui, |ui| {
                ui.set_width(400.0);
                for (index, location) in form.locations.iter().enumerate() {
                    if ui.selectable_label(form.selected_location == Some(index), location).clicked() {
                        form.selected_location = Some(index);
                    }
                }
            });
            ui.horizontal(|ui| {
                for (label, value) in [(t("위로", "Up"), SearchAction::Up), (t("아래로", "Down"), SearchAction::Down), (t("제거", "Remove"), SearchAction::Remove), (t("위치 저장", "Save locations"), SearchAction::SaveLocations)] {
                    if ui.button(label).clicked() {
                        action = Some(value);
                    }
                }
            });
        });
        ui.end_row();
        ui.label(t("파일 이름", "File name"));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut form.patterns).desired_width(400.0));
            if !form.history.is_empty() {
                ui.menu_button("▼", |ui| {
                    for item in form.history.clone() {
                        if ui.button(&item).clicked() {
                            form.patterns = item;
                            ui.close();
                        }
                    }
                });
            }
        });
        ui.end_row();
        ui.label(t("범주", "Category"));
        let categories = if ko { ["모든 파일", "문서", "사진 및 이미지", "오디오", "동영상", "압축 파일", "프로그램 파일", "폴더만", "기타 파일"] } else { ["All files", "Documents", "Pictures and images", "Audio", "Video", "Archives", "Programs", "Folders only", "Other files"] };
        egui::ComboBox::from_id_salt("search-category").width(180.0).selected_text(categories[form.category.min(8)]).show_opaque(ui, |ui| {
            for (index, label) in categories.iter().enumerate() {
                ui.selectable_value(&mut form.category, index, *label);
            }
        });
        ui.end_row();
        ui.label(t("확장자", "Extensions"));
        ui.add(egui::TextEdit::singleline(&mut form.extensions).desired_width(430.0)).on_hover_text(t("예: TXT; DOC; *.PDF (비우면 모든 확장자)", "e.g. TXT; DOC; *.PDF (empty: every extension)"));
        ui.end_row();
        ui.label(t("파일 내용 포함", "Containing text"));
        ui.add(egui::TextEdit::singleline(&mut form.content).desired_width(430.0));
        ui.end_row();
        ui.label(t("크기 범위", "Size range"));
        ui.horizontal(|ui| {
            let units = ["bytes", "KiB", "MiB", "GiB"];
            ui.add(egui::TextEdit::singleline(&mut form.minimum).desired_width(78.0));
            egui::ComboBox::from_id_salt("search-min-unit").width(70.0).selected_text(units[form.minimum_unit]).show_opaque(ui, |ui| {
                for (index, unit) in units.iter().enumerate() {
                    ui.selectable_value(&mut form.minimum_unit, index, *unit);
                }
            });
            ui.label(format!("  {}   ~   ", t("최소", "minimum")));
            ui.add(egui::TextEdit::singleline(&mut form.maximum).desired_width(78.0));
            egui::ComboBox::from_id_salt("search-max-unit").width(70.0).selected_text(units[form.maximum_unit]).show_opaque(ui, |ui| {
                for (index, unit) in units.iter().enumerate() {
                    ui.selectable_value(&mut form.maximum_unit, index, *unit);
                }
            });
            ui.label(t("최대", "maximum"));
        });
        ui.end_row();
        ui.label(t("날짜/시간 범위", "Date/time range"));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut form.date_from).desired_width(100.0).hint_text("yyyy-MM-dd"));
            ui.add(egui::TextEdit::singleline(&mut form.time_from).desired_width(70.0).hint_text("HH:mm"));
            ui.label(format!("{}   ~   ", t("부터", "from")));
            ui.add(egui::TextEdit::singleline(&mut form.date_to).desired_width(100.0).hint_text("yyyy-MM-dd"));
            ui.add(egui::TextEdit::singleline(&mut form.time_to).desired_width(70.0).hint_text("HH:mm"));
            ui.label(t("까지", "to"));
        });
        ui.end_row();
        ui.label(t("날짜 기준", "Date kind"));
        let kinds = if ko { ["모든날짜", "수정한 날짜", "만든 날짜", "마지막 액세스 날짜"] } else { ["Any date", "Date modified", "Date created", "Date accessed"] };
        egui::ComboBox::from_id_salt("search-time-kind").width(180.0).selected_text(kinds[form.time_kind.min(3)]).show_opaque(ui, |ui| {
            for (index, label) in kinds.iter().enumerate() {
                ui.selectable_value(&mut form.time_kind, index, *label);
            }
        });
        ui.end_row();
        ui.label(t("검색 옵션", "Options"));
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut form.recursive, t("하위 폴더 포함", "Include subfolders"));
            ui.checkbox(&mut form.hidden, t("숨김·시스템 항목 포함", "Include hidden and system items"));
            ui.checkbox(&mut form.files, t("파일", "Files"));
            ui.checkbox(&mut form.folders, t("폴더", "Folders"));
            ui.checkbox(&mut form.match_case, t("대소문자 구분", "Match case"));
        });
        ui.end_row();
    });
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        if ui.add_sized([82.0, 26.0], egui::Button::new(t("취소", "Cancel"))).clicked() {
            action = Some(SearchAction::Cancel);
        }
        if ui.add_sized([90.0, 26.0], egui::Button::new(t("검색", "Search"))).clicked() {
            action = Some(SearchAction::Search);
        }
    });
    if enter && action.is_none() {
        action = Some(SearchAction::Search);
    }
    action
}

impl App {
    fn search_title(&self) -> String {
        self.tr("파일 및 폴더 검색", "Search Files and Folders").into_owned()
    }

    fn search_form(&mut self) -> Option<&mut SearchForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::Search(form)) => Some(form),
            _ => None,
        }
    }

    /// C# SearchAsync: 압축 파일 안은 안 됨. 저장한 위치·이름 기록·작업 폴더 2를 넘긴다.
    pub(super) fn search_command(&mut self) {
        let i = self.active;
        if self.panes[i].archive.is_some() {
            return self.notice("압축 파일 안에서는 이름 검색을 사용할 수 없습니다.", "Name search is not available inside an archive.");
        }
        let path = self.panes[i].path.to_string_lossy().into_owned();
        let custom2 = self.settings.strings("WorkingFolders").get(1).cloned().filter(|p| !p.trim().is_empty());
        let form = SearchForm::new(&path, &self.toolbar_query.clone(), &self.settings.strings("SavedSearchLocations"), &self.settings.strings("SavedSearchPatterns"), custom2);
        let title = self.search_title();
        self.push_original_dialog(title, DialogKind::Search(Box::new(form)), move |app, result, _| {
            let DialogResult::Search(Some(criteria)) = result else { return };
            app.status = app.tr("파일 및 폴더 검색 중…", "Searching files and folders…").into_owned();
            let tx = app.async_tx.clone();
            let root = criteria.roots[0].clone();
            let modal = ModalProgress {
                stop: app.tr("취소", "Cancel").into_owned(),
                stopping: None,
                stopping_text: app.tr("검색을 취소하고 있습니다…", "Canceling the search…").into_owned(),
                kind: ModalKind::Search,
            };
            let title = app.search_progress_title(&criteria.roots);
            app.start_modal_task(title, modal, move |events, cancel| {
                let outcome = ndir::tools::search_advanced_with_progress(&criteria, &cancel, &mut |scanned, matches, path| {
                    let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes: matches as u64, files: scanned as u64 });
                });
                let _ = tx.send(AsyncResult::SearchInPane { pane: i, root, outcome, cancelled: cancel.load(Ordering::Relaxed) });
                Ok(())
            });
        });
    }

    /// SearchProgressWindow 제목: 위치가 여럿이면 "검색 중 · N 개 위치", 하나면 "검색 중 · 폴더 이름"(드라이브 루트는 빈 이름).
    pub(super) fn search_progress_title(&self, roots: &[PathBuf]) -> String {
        let mut distinct: Vec<String> = vec![];
        for root in roots {
            let text = root.to_string_lossy().into_owned();
            if !distinct.iter().any(|known| known.eq_ignore_ascii_case(&text)) {
                distinct.push(text);
            }
        }
        let searching = self.tr("검색 중", "Searching");
        if distinct.len() > 1 {
            let count = ndir::parity::format_size(distinct.len() as u64, "B", true).trim_end_matches(" B").to_owned();
            return format!("{searching} · {count} {}", self.tr("개 위치", "locations"));
        }
        let root = distinct.first().map(String::as_str).unwrap_or("");
        let trimmed = if root.len() > 3 { root.trim_end_matches(['\\', '/']) } else { root };
        let name = Path::new(trimmed).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        format!("{searching} · {name}")
    }

    /// 창의 단추 처리.
    pub(super) fn search_action(&mut self, action: SearchAction, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let title = self.search_title();
        let warn = |app: &mut App, text: &str| app.system_box(&title, text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        match action {
            SearchAction::Cancel => self.answer_dialog(DialogResult::Search(None), ctx),
            SearchAction::Search => {
                let connect_share = self.system.connect_share;
                let path_error = self.system.network_path_error;
                let connect = move |path: &str| network_share_code(path, path_error, connect_share);
                let Some(form) = self.search_form() else { return };
                match form.criteria(ko, &connect) {
                    Ok(criteria) => {
                        let history = form.history.clone();
                        self.settings.set_strings("SavedSearchPatterns", history);
                        self.save();
                        self.answer_dialog(DialogResult::Search(Some(criteria)), ctx);
                    }
                    Err(message) if message.is_empty() => {}
                    Err(message) => warn(self, &message),
                }
            }
            SearchAction::Browse => {
                let picker_title = self.tr("검색 위치 선택", "Choose a search location").into_owned();
                let folder_picker = self.folder_picker;
                let Some(form) = self.search_form() else { return };
                let initial = Path::new(&form.root).is_dir().then(|| PathBuf::from(&form.root));
                if let Some(folder) = folder_picker(&picker_title, initial.as_deref()) {
                    if form.add_location(&folder.to_string_lossy()).is_err() {
                        warn(self, if ko { "추가할 폴더가 존재하지 않습니다." } else { "The folder to add does not exist." });
                    }
                }
            }
            SearchAction::Add => {
                let Some(form) = self.search_form() else { return };
                let root = form.root.clone();
                if form.add_location(&root).is_err() {
                    warn(self, if ko { "추가할 폴더가 존재하지 않습니다." } else { "The folder to add does not exist." });
                }
            }
            SearchAction::Preset(index) => {
                let documents = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Documents");
                let unavailable = self.language_pack.text("NDIRSearch.Custom2Unavailable", if ko { "작업 폴더 2가 설정되어 있지 않습니다. 설정에서 작업 폴더 2를 지정하세요." } else { "Work folder 2 is not set. Set it in the options." });
                let Some(form) = self.search_form() else { return };
                let outcome = match index {
                    0 => {
                        form.add_all_drives();
                        Ok(())
                    }
                    1..=4 => form.add_location(&format!("{}:\\", ["C", "D", "E", "F"][index - 1])),
                    5 => form.add_location(&documents.to_string_lossy()),
                    _ => match form.custom2.clone() {
                        Some(path) => form.add_location(&path),
                        None => return self.system_box(&title, &unavailable, MB_OK | MB_ICONASTERISK, |_, _, _| {}),
                    },
                };
                if outcome.is_err() {
                    warn(self, if ko { "추가할 폴더가 존재하지 않습니다." } else { "The folder to add does not exist." });
                }
            }
            SearchAction::Up | SearchAction::Down => {
                let offset: isize = if matches!(action, SearchAction::Up) { -1 } else { 1 };
                let Some(form) = self.search_form() else { return };
                if let Some(index) = form.selected_location {
                    let next = index as isize + offset;
                    if next >= 0 && (next as usize) < form.locations.len() {
                        form.locations.swap(index, next as usize);
                        form.selected_location = Some(next as usize);
                    }
                }
            }
            SearchAction::Remove => {
                let Some(form) = self.search_form() else { return };
                if let Some(index) = form.selected_location.filter(|i| *i < form.locations.len()) {
                    form.locations.remove(index);
                    form.selected_location = (!form.locations.is_empty()).then(|| index.min(form.locations.len() - 1));
                }
            }
            SearchAction::SaveLocations => {
                let Some(form) = self.search_form() else { return };
                let locations = form.locations.clone();
                self.settings.set_strings("SavedSearchLocations", locations);
                self.save();
                let text = self.tr("검색 위치 목록을 저장했습니다.", "The search locations were saved.").into_owned();
                self.system_box(&title, &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
            }
        }
    }

    /// 결과를 활성 패널 목록으로(필터는 비우고, 위치가 다르면 그 폴더로 바꾼 뒤 결과를 넣는다).
    pub(super) fn search_done(&mut self, pane: usize, root: PathBuf, outcome: Result<Vec<Entry>, String>, cancelled: bool) {
        let results = match outcome {
            Err(_) if cancelled => {
                self.status = self.tr("검색을 취소했습니다.", "Search canceled.").into_owned();
                return;
            }
            Err(error) => {
                // SearchProgressWindow.Run: MessageBox(오류, "검색", OK, Warning).
                let title = self.tr("검색", "Search").into_owned();
                return self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
            Ok(results) => results,
        };
        let p = &mut self.panes[pane];
        if p.path != root || p.archive.is_some() {
            p.back.push(p.path.clone());
            p.forward.clear();
            p.path = root.clone();
            p.address = root.to_string_lossy().into_owned();
            p.archive = None;
        }
        p.filter.clear();
        p.generation += 1;
        p.loading = false;
        p.entries = results;
        p.selected.clear();
        p.cursor = 0;
        p.error = None;
        let count = p.entries.len();
        self.sort_pane(pane);
        let template = self.language_pack.text("formNDIRSearch.Status.ResultCount", "검색 결과 {0:N0}개 · F5로 원래 목록 새로고침");
        let count = ndir::parity::format_size(count as u64, "B", true).trim_end_matches(" B").to_owned();
        self.status = template.replace("{0:N0}", &count);
    }
}
