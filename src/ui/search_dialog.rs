//! C# SearchWindow + SearchAsync (find files and folders): search location list (presets, browse, add, up/down/remove, save), name history,
//! category, extension, content, size range, date range, date basis, options. Results become the active pane list like C# (F5 returns to the original list).
//! Rules are ndr::tools::search_advanced. Difference: dates are typed as "yyyy-MM-dd" text instead of a calendar.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use super::tools_dialog::network_share_code;
use ndr::msgbox::{MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_OK};
use ndr::tools::{SearchCategory, SearchCriteria, SearchTimeKind};

const MAXIMUM_PATTERN_HISTORY: usize = 20;

fn search_text(language: &compat::Language, key: &str, en: &str, ko: &str) -> String {
    let fallback = if language.code.eq_ignore_ascii_case("ko") { ko } else { en };
    language.text(&format!("formNDRSearch.{key}"), fallback)
}

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

/// SearchWindow buttons and preset actions.
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

/// NormalizeLocation: strips quotes and spaces and expands environment variables into a full path.
fn normalize_location(value: &str) -> Option<String> {
    let value = value.trim().trim_matches('"');
    if value.is_empty() {
        return None;
    }
    let expanded = native::expand_environment(value);
    std::path::absolute(expanded).ok().map(|p| p.to_string_lossy().into_owned())
}

/// TrySize: None when empty, a non-negative decimal x unit (bytes, KiB, MiB, GiB).
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

/// TryDateTime: None without a date (the time must be empty too). Without a time the start is midnight and the end the last moment of that day. The end includes up to the end of the minute/second.
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
        if !form.locations.iter().any(|known| ndr::names::equals_ignore_case(known.trim_end_matches('\\'), path.trim_end_matches('\\'))) {
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

    /// AddLocation: only existing folders (or UNC), putting that path in the field too. Err when missing.
    pub(super) fn add_location(&mut self, value: &str) -> Result<(), ()> {
        let path = normalize_location(value).filter(|p| Path::new(p).is_dir() || p.starts_with("\\\\")).ok_or(())?;
        self.root = path.clone();
        add_unique(self, &path);
        self.selected_location = self.locations.iter().position(|l| l.eq_ignore_ascii_case(&path));
        Ok(())
    }

    /// AddAllDrives: all non-network drives.
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

    /// Check order and texts of Commit (locations -> missing locations -> files/folders -> size -> date). On success the search criteria.
    /// `connect`: C# connectNetworkShare (an error number for UNC, the connect window result on credential failure). Cancelling the connect window (1223) is Err("") = do nothing.
    pub(super) fn criteria(&mut self, language: &compat::Language, connect: &dyn Fn(&str) -> u32) -> Result<SearchCriteria, String> {
        if self.locations.is_empty() {
            let _ = self.add_location(&self.root.clone());
        }
        if self.locations.is_empty() {
            return Err(search_text(language, "Message.NoRoots", "Add at least one search location.", "검색할 위치를 하나 이상 추가하세요."));
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
            let head = search_text(language, "Message.StaleRoots", "These search locations cannot be found. Remove them or choose them again.", "다음 검색 위치를 찾을 수 없습니다. 목록에서 제거하거나 다시 선택하세요.");
            return Err(format!("{head}\n{}", missing.join("\r\n")));
        }
        if !self.files && !self.folders {
            return Err(search_text(language, "Message.NoFileOrFolder", "Select files, folders or both.", "파일 또는 폴더를 하나 이상 선택하세요."));
        }
        let sizes = (size(&self.minimum, self.minimum_unit), size(&self.maximum, self.maximum_unit));
        let (Ok(minimum), Ok(maximum)) = sizes else {
            return Err(search_text(language, "Message.InvalidSize", "Enter sizes of 0 or more; the minimum must not exceed the maximum.", "크기는 0 이상의 숫자로 입력하고 최소 크기는 최대 크기보다 작아야 합니다."));
        };
        if minimum.zip(maximum).is_some_and(|(a, b)| a > b) {
            return Err(search_text(language, "Message.InvalidSize", "Enter sizes of 0 or more; the minimum must not exceed the maximum.", "크기는 0 이상의 숫자로 입력하고 최소 크기는 최대 크기보다 작아야 합니다."));
        }
        let dates = (date_time(&self.date_from, &self.time_from, false), date_time(&self.date_to, &self.time_to, true));
        let date_error = || search_text(language, "Message.InvalidDateTime", "Pick dates and enter times as HH:mm or HH:mm:ss. The start cannot be later than the end.", "날짜를 선택하고 시각은 HH:mm 또는 HH:mm:ss 형식으로 입력하세요. 시작은 끝보다 늦을 수 없습니다.");
        let (Ok(date_from), Ok(date_to)) = dates else { return Err(date_error()) };
        if date_from.zip(date_to).is_some_and(|(a, b)| a > b) {
            return Err(date_error());
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

/// Draws the window.
/// Narrow search form: the existing criteria, validation and search
/// commands are reused. Each group has its own bounded controls.
fn compact_search_ui(ui: &mut egui::Ui, form: &mut SearchForm, language: &compat::Language, enter: bool) -> Option<SearchAction> {
    let t = |key, en, ko| search_text(language, key, en, ko);
    let labels = [language.text("RES.SEARCH", "Search"), language.text("RES.CANCEL", "Cancel")];
    let (mut action, choice) = super::original_dialogs::compact_scroll_form(ui, "compact-search-body", egui::vec2(750.0, 720.0), &labels, |ui| {
        let mut action = None;
        let field = |ui: &mut egui::Ui, label: String, value: &mut String| {
            ui.add(egui::Label::new(label).wrap());
            ui.add_sized([ui.available_width(), 23.0], egui::TextEdit::singleline(value))
        };
        field(ui, t("Caption.lRoot", "Search in", "검색 위치"), &mut form.root);
        ui.horizontal_wrapped(|ui| {
            if ui.add(egui::Button::new(t("Caption.btnBrowse", "Browse…", "찾아보기…")).wrap()).clicked() { action=Some(SearchAction::Browse); }
            if ui.add(egui::Button::new(t("Caption.btnAdd", "Add", "추가")).wrap()).clicked() { action=Some(SearchAction::Add); }
        });
        let presets = [language.text("NDRSearch.AllLocalDrives", "All local drives"), language.text("NDRSearch.DriveC", "C:"), language.text("NDRSearch.DriveD", "D:"), language.text("NDRSearch.DriveE", "E:"), language.text("NDRSearch.DriveF", "F:"), language.text("NDRSearch.Documents", "Documents"), language.text("NDRSearch.Custom2", "Custom 2")];
        egui::ComboBox::from_id_salt("search-presets").width((ui.available_width()-8.0).max(80.0)).selected_text("").show_opaque(ui, |ui| {
            for (i,label) in presets.iter().enumerate() { if ui.selectable_label(false,label).clicked() { action=Some(SearchAction::Preset(i)); } }
        });
        egui::ScrollArea::vertical().id_salt("compact-search-locations").max_height(90.0).min_scrolled_height(0.0).auto_shrink([false,true]).show(ui,|ui| {
            for (i,location) in form.locations.iter().enumerate() {
                if ui.add(egui::Button::selectable(form.selected_location==Some(i),location).wrap()).on_hover_text(location).clicked() { form.selected_location=Some(i); }
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (label,value) in [(t("Caption.btnMoveUp", "Up", "위로"), SearchAction::Up),(t("Caption.btnMoveDown", "Down", "아래로"), SearchAction::Down),(t("Caption.btnRemove", "Remove", "제거"), SearchAction::Remove),(t("Caption.btnSaveLocations", "Save locations", "위치 저장"), SearchAction::SaveLocations)] {
                if ui.add(egui::Button::new(label).wrap()).clicked() { action=Some(value); }
            }
        });
        ui.separator();
        ui.add(egui::Label::new(t("Caption.lName","File name","파일 이름")).wrap());
        ui.horizontal(|ui| {
            let w=if form.history.is_empty(){ui.available_width()}else{(ui.available_width()-34.0).max(40.0)};
            ui.add_sized([w,23.0],egui::TextEdit::singleline(&mut form.patterns));
            if !form.history.is_empty() { ui.menu_button("▼",|ui| { for item in form.history.clone() { if ui.button(&item).clicked() { form.patterns=item;ui.close(); } } }); }
        });
        ui.add(egui::Label::new(t("Caption.lCategory","Category","범주")).wrap());
        let categories=[("Caption.CategoryAll","All files","모든 파일"),("Caption.CategoryDocuments","Documents","문서"),("Caption.CategoryPictures","Pictures and images","사진 및 이미지"),("Caption.CategoryAudio","Audio","오디오"),("Caption.CategoryVideo","Video","동영상"),("Caption.CategoryArchives","Archives","압축 파일"),("Caption.CategoryApplications","Programs","프로그램 파일"),("Caption.CategoryFolders","Folders only","폴더만"),("Caption.CategoryOther","Other files","기타 파일")].map(|(key,en,ko)|t(key,en,ko));
        egui::ComboBox::from_id_salt("search-category").width((ui.available_width()-8.0).max(80.0)).selected_text(&categories[form.category.min(8)]).show_opaque(ui, |ui| {
            for (i,label) in categories.iter().enumerate(){ui.selectable_value(&mut form.category,i,label);}
        });
        field(ui,t("Caption.lExtension","Extensions","확장자"),&mut form.extensions).on_hover_text(t("Hint.extension","e.g. TXT; DOC; *.PDF (empty: every extension)","예: TXT; DOC; *.PDF (비우면 모든 확장자)"));
        field(ui,t("Caption.lContent","Containing text","파일 내용 포함"),&mut form.content);
        ui.separator();
        ui.strong(t("Caption.lSize","Size range","크기 범위"));
        let bytes=language.text("NDR.UI_SIZE_BYTES","bytes");
        let units=[bytes.as_str(),"KiB","MiB","GiB"];
        for (id,label,value,unit) in [("search-min-unit",t("Caption.btnMinimum","minimum","최소"),&mut form.minimum,&mut form.minimum_unit),("search-max-unit",t("Caption.btnMaximum","maximum","최대"),&mut form.maximum,&mut form.maximum_unit)] {
            ui.add(egui::Label::new(label).wrap());
            ui.horizontal(|ui| {
                let w=(ui.available_width()-94.0).max(40.0);
                ui.add_sized([w,23.0],egui::TextEdit::singleline(value));
                egui::ComboBox::from_id_salt(id).width(75.0).selected_text(units[(*unit).min(3)]).show_opaque(ui,|ui| {
                    for (i,u) in units.iter().enumerate(){ui.selectable_value(unit,i,*u);}
                });
            });
        }
        ui.separator();
        ui.strong(t("Caption.lDateRange","Date/time range","날짜/시간 범위"));
        for (label,date,time) in [(t("Caption.btnFrom","from","부터"),&mut form.date_from,&mut form.time_from),(t("Caption.btnTo","to","까지"),&mut form.date_to,&mut form.time_to)] {
            ui.add(egui::Label::new(label).wrap());
            ui.horizontal(|ui| {
                let w=ui.available_width()-ui.spacing().item_spacing.x;
                ui.add_sized([w*0.60,23.0],egui::TextEdit::singleline(date).hint_text("yyyy-MM-dd"));
                ui.add_sized([w*0.40,23.0],egui::TextEdit::singleline(time).hint_text("HH:mm"));
            });
        }
        ui.add(egui::Label::new(t("Caption.lDateKind","Date kind","날짜 기준")).wrap());
        let kinds=[("Caption.TimeAll","Any date","모든 날짜"),("Caption.TimeModified","Date modified","수정한 날짜"),("Caption.TimeCreated","Date created","만든 날짜"),("Caption.TimeAccessed","Date accessed","마지막 액세스 날짜")].map(|(key,en,ko)|t(key,en,ko));
        egui::ComboBox::from_id_salt("search-time-kind").width((ui.available_width()-8.0).max(80.0)).selected_text(&kinds[form.time_kind.min(3)]).show_opaque(ui,|ui| {
            for(i,label) in kinds.iter().enumerate(){ui.selectable_value(&mut form.time_kind,i,label);}
        });
        ui.separator();
        ui.strong(t("Caption.lOptions","Options","검색 옵션"));
        for (value,label) in [(&mut form.recursive,t("Caption.chkRecursive","Include subfolders","하위 폴더 포함")),(&mut form.hidden,t("Caption.chkHidden","Include hidden and system items","숨김·시스템 항목 포함")),(&mut form.files,t("Caption.chkFiles","Files","파일")),(&mut form.folders,t("Caption.chkFolders","Folders","폴더")),(&mut form.match_case,t("Caption.chkMatchCase","Match case","대소문자 구분"))] {
            let mut job=egui::text::LayoutJob::simple(label,egui::TextStyle::Body.resolve(ui.style()),ui.visuals().text_color(),(ui.available_width()-24.0).max(1.0));
            job.wrap.break_anywhere=true;
            ui.add(egui::Checkbox::new(value,job));
        }
        action
    });
    if let Some(choice)=choice { action=Some(if choice==0{SearchAction::Search}else{SearchAction::Cancel}); }
    if enter && action.is_none(){action=Some(SearchAction::Search);}
    action
}

pub(super) fn search_ui(ui: &mut egui::Ui, form: &mut SearchForm, language: &compat::Language, _ko: bool, enter: bool) -> Option<SearchAction> {
    let screen=ui.ctx().content_rect();
    if screen.width()<850.0 || screen.height()<690.0 { return compact_search_ui(ui,form,language,enter); }
    let t = |key, en, ko| search_text(language, key, en, ko);
    let mut action = None;
    ui.set_width(750.0);
    egui::Grid::new("search-form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label(t("Caption.lRoot", "Search in", "검색 위치"));
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut form.root).desired_width(400.0));
                if ui.button(t("Caption.btnBrowse", "Browse…", "찾아보기…")).clicked() {
                    action = Some(SearchAction::Browse);
                }
                if ui.button(t("Caption.btnAdd", "Add", "추가")).clicked() {
                    action = Some(SearchAction::Add);
                }
            });
            let presets = [
                language.text("NDRSearch.AllLocalDrives", "All local drives"),
                language.text("NDRSearch.DriveC", "C:"),
                language.text("NDRSearch.DriveD", "D:"),
                language.text("NDRSearch.DriveE", "E:"),
                language.text("NDRSearch.DriveF", "F:"),
                language.text("NDRSearch.Documents", "Documents"),
                language.text("NDRSearch.Custom2", "Custom 2"),
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
                for (label, value) in [(t("Caption.btnMoveUp", "Up", "위로"), SearchAction::Up), (t("Caption.btnMoveDown", "Down", "아래로"), SearchAction::Down), (t("Caption.btnRemove", "Remove", "제거"), SearchAction::Remove), (t("Caption.btnSaveLocations", "Save locations", "위치 저장"), SearchAction::SaveLocations)] {
                    if ui.button(label).clicked() {
                        action = Some(value);
                    }
                }
            });
        });
        ui.end_row();
        ui.label(t("Caption.lName", "File name", "파일 이름"));
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
        ui.label(t("Caption.lCategory", "Category", "범주"));
        let categories = [
            ("Caption.CategoryAll", "All files", "모든 파일"),
            ("Caption.CategoryDocuments", "Documents", "문서"),
            ("Caption.CategoryPictures", "Pictures and images", "사진 및 이미지"),
            ("Caption.CategoryAudio", "Audio", "오디오"),
            ("Caption.CategoryVideo", "Video", "동영상"),
            ("Caption.CategoryArchives", "Archives", "압축 파일"),
            ("Caption.CategoryApplications", "Programs", "프로그램 파일"),
            ("Caption.CategoryFolders", "Folders only", "폴더만"),
            ("Caption.CategoryOther", "Other files", "기타 파일"),
        ].map(|(key, en, ko)| t(key, en, ko));
        egui::ComboBox::from_id_salt("search-category").width(180.0).selected_text(&categories[form.category.min(8)]).show_opaque(ui, |ui| {
            for (index, label) in categories.iter().enumerate() {
                ui.selectable_value(&mut form.category, index, label);
            }
        });
        ui.end_row();
        ui.label(t("Caption.lExtension", "Extensions", "확장자"));
        ui.add(egui::TextEdit::singleline(&mut form.extensions).desired_width(430.0)).on_hover_text(t("Hint.extension", "e.g. TXT; DOC; *.PDF (empty: every extension)", "예: TXT; DOC; *.PDF (비우면 모든 확장자)"));
        ui.end_row();
        ui.label(t("Caption.lContent", "Containing text", "파일 내용 포함"));
        ui.add(egui::TextEdit::singleline(&mut form.content).desired_width(430.0));
        ui.end_row();
        ui.label(t("Caption.lSize", "Size range", "크기 범위"));
        ui.horizontal(|ui| {
            let bytes = language.text("NDR.UI_SIZE_BYTES", "bytes");
            let units = [bytes.as_str(), "KiB", "MiB", "GiB"];
            ui.add(egui::TextEdit::singleline(&mut form.minimum).desired_width(78.0));
            egui::ComboBox::from_id_salt("search-min-unit").width(70.0).selected_text(units[form.minimum_unit]).show_opaque(ui, |ui| {
                for (index, unit) in units.iter().enumerate() {
                    ui.selectable_value(&mut form.minimum_unit, index, *unit);
                }
            });
            ui.label(format!("  {}   ~   ", t("Caption.btnMinimum", "minimum", "최소")));
            ui.add(egui::TextEdit::singleline(&mut form.maximum).desired_width(78.0));
            egui::ComboBox::from_id_salt("search-max-unit").width(70.0).selected_text(units[form.maximum_unit]).show_opaque(ui, |ui| {
                for (index, unit) in units.iter().enumerate() {
                    ui.selectable_value(&mut form.maximum_unit, index, *unit);
                }
            });
            ui.label(t("Caption.btnMaximum", "maximum", "최대"));
        });
        ui.end_row();
        ui.label(t("Caption.lDateRange", "Date/time range", "날짜/시간 범위"));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut form.date_from).desired_width(100.0).hint_text("yyyy-MM-dd"));
            ui.add(egui::TextEdit::singleline(&mut form.time_from).desired_width(70.0).hint_text("HH:mm"));
            ui.label(format!("{}   ~   ", t("Caption.btnFrom", "from", "부터")));
            ui.add(egui::TextEdit::singleline(&mut form.date_to).desired_width(100.0).hint_text("yyyy-MM-dd"));
            ui.add(egui::TextEdit::singleline(&mut form.time_to).desired_width(70.0).hint_text("HH:mm"));
            ui.label(t("Caption.btnTo", "to", "까지"));
        });
        ui.end_row();
        ui.label(t("Caption.lDateKind", "Date kind", "날짜 기준"));
        let kinds = [
            ("Caption.TimeAll", "Any date", "모든 날짜"),
            ("Caption.TimeModified", "Date modified", "수정한 날짜"),
            ("Caption.TimeCreated", "Date created", "만든 날짜"),
            ("Caption.TimeAccessed", "Date accessed", "마지막 액세스 날짜"),
        ].map(|(key, en, ko)| t(key, en, ko));
        egui::ComboBox::from_id_salt("search-time-kind").width(180.0).selected_text(&kinds[form.time_kind.min(3)]).show_opaque(ui, |ui| {
            for (index, label) in kinds.iter().enumerate() {
                ui.selectable_value(&mut form.time_kind, index, label);
            }
        });
        ui.end_row();
        ui.label(t("Caption.lOptions", "Options", "검색 옵션"));
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut form.recursive, t("Caption.chkRecursive", "Include subfolders", "하위 폴더 포함"));
            ui.checkbox(&mut form.hidden, t("Caption.chkHidden", "Include hidden and system items", "숨김·시스템 항목 포함"));
            ui.checkbox(&mut form.files, t("Caption.chkFiles", "Files", "파일"));
            ui.checkbox(&mut form.folders, t("Caption.chkFolders", "Folders", "폴더"));
            ui.checkbox(&mut form.match_case, t("Caption.chkMatchCase", "Match case", "대소문자 구분"));
        });
        ui.end_row();
    });
    ui.add_space(12.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        if ui.add_sized([82.0, 26.0], egui::Button::new(language.text("RES.CANCEL", "Cancel"))).clicked() {
            action = Some(SearchAction::Cancel);
        }
        if ui.add_sized([90.0, 26.0], egui::Button::new(language.text("RES.SEARCH", "Search"))).clicked() {
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
        search_text(&self.language_pack, "Form.Caption", "Search Files and Folders", "파일 및 폴더 검색")
    }

    fn search_form(&mut self) -> Option<&mut SearchForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::Search(form)) => Some(form),
            _ => None,
        }
    }

    /// C# SearchAsync: not inside archives. Passes the saved locations, name history and work folder 2.
    pub(super) fn search_command(&mut self) {
        let i = self.active;
        if self.panes[i].archive.is_some() {
            let text = search_text(&self.language_pack, "Message.SearchUnavailableInArchive", "Name search is not available inside an archive.", "압축 파일 안에서는 이름 검색을 사용할 수 없습니다.");
            return self.notice_operation(&text);
        }
        let path = self.command_directory(i).to_string_lossy().into_owned();
        let custom2 = self.settings.strings("WorkingFolders").get(1).cloned().filter(|p| !p.trim().is_empty());
        let form = SearchForm::new(&path, &self.toolbar_query.clone(), &self.settings.strings("SavedSearchLocations"), &self.settings.strings("SavedSearchPatterns"), custom2);
        let title = self.search_title();
        self.push_original_dialog(title, DialogKind::Search(Box::new(form)), move |app, result, _| {
            let DialogResult::Search(Some(criteria)) = result else { return };
            app.status = search_text(&app.language_pack, "Status.Searching", "Searching files and folders…", "파일 및 폴더 검색 중…");
            let tx = app.async_tx.clone();
            let root = criteria.roots[0].clone();
            let modal = ModalProgress {
                stop: app.language_pack.text("RES.CANCEL", "Cancel"),
                stopping: None,
                stopping_text: search_text(&app.language_pack, "Progress.Canceling", "Canceling the search…", "검색을 취소하고 있습니다…"),
                kind: ModalKind::Search,
            };
            let title = app.search_progress_title(&criteria.roots);
            app.start_modal_task(title, modal, move |events, cancel| {
                let outcome = ndr::tools::search_advanced_with_progress(&criteria, &cancel, &mut |scanned, matches, path| {
                    let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes: matches as u64, files: scanned as u64 });
                });
                let _ = tx.send(AsyncResult::SearchInPane { pane: i, root, outcome, cancelled: cancel.load(Ordering::Relaxed) });
                Ok(())
            });
        });
    }

    /// SearchProgressWindow title: "searching · N locations" for several locations, "searching · folder name" for one (an empty name for a drive root).
    pub(super) fn search_progress_title(&self, roots: &[PathBuf]) -> String {
        let mut distinct: Vec<String> = vec![];
        for root in roots {
            let text = root.to_string_lossy().into_owned();
            if !distinct.iter().any(|known| known.eq_ignore_ascii_case(&text)) {
                distinct.push(text);
            }
        }
        let searching = search_text(&self.language_pack, "Progress.Searching", "Searching", "검색 중");
        if distinct.len() > 1 {
            let count = ndr::parity::format_size(distinct.len() as u64, "B", true).trim_end_matches(" B").to_owned();
            return format!("{searching} · {count} {}", search_text(&self.language_pack, "Progress.LocationCount", "locations", "개 위치"));
        }
        let root = distinct.first().map(String::as_str).unwrap_or("");
        let trimmed = if root.len() > 3 { root.trim_end_matches(['\\', '/']) } else { root };
        let name = Path::new(trimmed).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        format!("{searching} · {name}")
    }

    /// Handles the window's buttons.
    pub(super) fn search_action(&mut self, action: SearchAction, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let title = self.search_title();
        let warn = |app: &mut App, text: &str| app.system_box(&title, text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        let missing_folder = search_text(&self.language_pack, "Message.MissingFolder", "The folder to add does not exist.", "추가할 폴더가 존재하지 않습니다.");
        match action {
            SearchAction::Cancel => self.answer_dialog(DialogResult::Search(None), ctx),
            SearchAction::Search => {
                let connect_share = self.system.connect_share;
                let path_error = self.system.network_path_error;
                let connect = move |path: &str| network_share_code(path, path_error, connect_share);
                let Some(DialogKind::Search(form)) = self.original_dialogs.last_mut().map(|dialog| &mut dialog.kind) else { return };
                match form.criteria(&self.language_pack, &connect) {
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
                let picker_title = search_text(&self.language_pack, "Dialog.TitleLocation", "Choose a search location", "검색 위치 선택");
                let folder_picker = self.folder_picker;
                let Some(form) = self.search_form() else { return };
                let initial = Path::new(&form.root).is_dir().then(|| PathBuf::from(&form.root));
                if let Some(folder) = folder_picker(&picker_title, initial.as_deref()) {
                    if form.add_location(&folder.to_string_lossy()).is_err() {
                        warn(self, &missing_folder);
                    }
                }
            }
            SearchAction::Add => {
                let Some(form) = self.search_form() else { return };
                let root = form.root.clone();
                if form.add_location(&root).is_err() {
                    warn(self, &missing_folder);
                }
            }
            SearchAction::Preset(index) => {
                let documents = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Documents");
                let unavailable = self.language_pack.text("NDRSearch.Custom2Unavailable", if ko { "작업 폴더 2가 설정되어 있지 않습니다. 설정에서 작업 폴더 2를 지정하세요." } else { "Work folder 2 is not set. Set it in the options." });
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
                    warn(self, &missing_folder);
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
                let text = search_text(&self.language_pack, "Message.LocationsSaved", "The search locations were saved.", "검색 위치 목록을 저장했습니다.");
                self.system_box(&title, &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
            }
        }
    }

    /// Puts the results in the active pane list (clearing the filter; for a different location, switch to that folder first, then insert the results).
    pub(super) fn search_done(&mut self, pane: usize, root: PathBuf, outcome: Result<Vec<Entry>, String>, cancelled: bool) {
        let results = match outcome {
            Err(_) if cancelled => {
                self.status = search_text(&self.language_pack, "Status.Cancelled", "Search canceled.", "검색을 취소했습니다.");
                return;
            }
            Err(error) => {
                // SearchProgressWindow.Run: MessageBox(error, "search", OK, Warning).
                let title = self.language_pack.text("RES.SEARCH", "Search");
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
        let template = self.language_pack.text("formNDRSearch.Status.ResultCount", "검색 결과 {0:N0}개 · F5로 원래 목록 새로고침");
        let count = ndr::parity::format_size(count as u64, "B", true).trim_end_matches(" B").to_owned();
        self.status = template.replace("{0:N0}", &count);
    }
}
