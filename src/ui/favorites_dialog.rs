//! C# ManageFavoritesAsync + FavoritesWindow(즐겨찾기 구성), ShowFavoritesMenu(F11·#FAVORITES·기능키 줄의 즐겨찾기 드롭다운),
//! OpenFavoriteAsync, ShowNavigationHistoryMenu/NavigateHistoryEntryAsync(#SHOWPATHHISTORY). 드롭다운은 `App::popup` 하나.
//! 차이(HANDOFF): 경로변경은 보류 중인 공용 폴더 트리 선택창 대신 Windows 폴더 선택 대화 상자. 단축키 열은 C# 기본값
//! (Shift+F1~F10, Ctrl+Shift+F1~F10)과 KeyboardShortcuts 설정을 보여 준다. 그 글쇠로 여는 것은 keyboard.rs(favoriteOpen:N).
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;

/// 즐겨찾기 칸 수(C#은 20칸을 채워 보인다).
const SLOTS: usize = 20;

/// FavoritesWindow의 입력: (이름, 경로) 칸들(빈 칸은 둘 다 빈 글), 선택한 줄, 현재 폴더.
#[derive(Clone, Debug)]
pub(super) struct FavoritesForm {
    pub items: Vec<(String, String)>,
    pub selected: Option<usize>,
    pub current: String,
    pub gestures: Vec<String>,
}

/// ShortcutBindingService.GetFavoriteGesture: KeyboardShortcuts["favoriteOpenN"], 없으면 Shift+F1~F10, Ctrl+Shift+F1~F10.
pub(super) fn favorite_gesture(settings: &ndir::settings::Settings, index: usize) -> String {
    if index >= SLOTS {
        return String::new();
    }
    let configured = settings.values.get("KeyboardShortcuts").and_then(|v| v.get(format!("favoriteOpen{}", index + 1))).and_then(|v| v.as_str());
    configured.map(str::to_owned).unwrap_or_else(|| if index < 10 { format!("Shift+F{}", index + 1) } else { format!("Ctrl+Shift+F{}", index - 9) })
}

fn trimmed_name(path: &str) -> String {
    let trimmed = if path.len() > 3 { path.trim_end_matches(['\\', '/']) } else { path };
    Path::new(trimmed).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl FavoritesForm {
    /// 이름이나 경로가 빈 항목은 빈 칸, 20칸까지 채운다.
    pub(super) fn new(favorites: Vec<(String, String)>, current: String, settings: &ndir::settings::Settings) -> Self {
        let mut items: Vec<(String, String)> = favorites.into_iter().map(|(name, path)| if name.is_empty() || path.is_empty() { (String::new(), String::new()) } else { (name, path) }).collect();
        while items.len() < SLOTS {
            items.push((String::new(), String::new()));
        }
        let gestures = (0..items.len()).map(|i| favorite_gesture(settings, i)).collect();
        Self { items, selected: None, current, gestures }
    }

    /// AddEntry: 같은 경로가 있으면 그대로, 이름은 마지막 폴더 이름(없으면 경로), 첫 빈 칸(없으면 끝)에 넣고 고른다.
    pub(super) fn add_entry(&mut self, path: &str) {
        if self.items.iter().any(|(_, known)| ndir::names::equals_ignore_case(known, path)) {
            return;
        }
        let name = trimmed_name(path);
        let entry = (if name.is_empty() { path.to_owned() } else { name }, path.to_owned());
        let slot = match self.items.iter().position(|(_, p)| p.is_empty()) {
            Some(slot) => {
                self.items[slot] = entry;
                slot
            }
            None => {
                self.items.push(entry);
                self.items.len() - 1
            }
        };
        self.selected = Some(slot);
    }

    /// RemoveSelected: 앞 20칸은 비우고, 그 뒤는 지운다.
    pub(super) fn remove_selected(&mut self) {
        let Some(index) = self.selected.filter(|i| *i < self.items.len()) else { return };
        if index < SLOTS {
            self.items[index] = (String::new(), String::new());
        } else {
            self.items.remove(index);
        }
        self.selected = Some(index.min(self.items.len() - 1));
    }

    pub(super) fn move_selected(&mut self, delta: isize) {
        let Some(index) = self.selected else { return };
        let next = index as isize + delta;
        if next < 0 || next as usize >= self.items.len() {
            return;
        }
        self.items.swap(index, next as usize);
        self.selected = Some(next as usize);
    }

    /// 저장 결과: 이름이나 경로가 빈 항목은 빈 칸.
    pub(super) fn result(&self) -> Vec<(String, String)> {
        self.items.iter().map(|(name, path)| if name.is_empty() || path.is_empty() { (String::new(), String::new()) } else { (name.clone(), path.clone()) }).collect()
    }
}

/// 창 단추.
pub(super) enum FavoritesAction {
    AddCurrent,
    AddPath,
    Rename,
    ModifyPath,
    Delete,
    Up,
    Down,
    Save,
    Cancel,
}

/// 원본 TfOrgFavorites(ClientWidth 487 × ClientHeight 512): ListView(8,41 469×381, 열 단축키 90·이름 100·경로 250, 없는 경로는 흐리게),
/// btnRename(8,428) btnModifyPath(89,428) btnUp(170,428) btnDown(251,428) 79×25, btnDelete(402,428 75×25), Bevel1(8,464 469),
/// btnSave(319,479) btnCancel(403,479) 75×25. 위 빈 띠(0~41)에는 C# 판에만 있는 "현재 폴더 추가"·"경로 추가" 단추를 둔다.
pub(super) fn favorites_ui(ui: &mut egui::Ui, form: &mut FavoritesForm, language: &compat::Language, ko: bool, enter: bool) -> Option<FavoritesAction> {
    let caption = |key: &str, fallback: &str| language.text(&format!("formOrgFavorites.Caption.{key}"), fallback);
    let mut action = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(487.0, 512.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let buttons = [
        (at(8.0, 8.0, 110.0, 25.0), if ko { "현재 폴더 추가".to_owned() } else { "Add current folder".to_owned() }, FavoritesAction::AddCurrent),
        (at(122.0, 8.0, 110.0, 25.0), if ko { "경로 추가".to_owned() } else { "Add path".to_owned() }, FavoritesAction::AddPath),
        (at(8.0, 428.0, 79.0, 25.0), caption("btnRename", "&Rename"), FavoritesAction::Rename),
        (at(89.0, 428.0, 79.0, 25.0), caption("btnModifyPath", "Chang&e Path"), FavoritesAction::ModifyPath),
        (at(170.0, 428.0, 79.0, 25.0), caption("btnUp", "&Up"), FavoritesAction::Up),
        (at(251.0, 428.0, 79.0, 25.0), caption("btnDown", "Do&wn"), FavoritesAction::Down),
        (at(402.0, 428.0, 75.0, 25.0), caption("btnDelete", "&Delete"), FavoritesAction::Delete),
        (at(319.0, 479.0, 75.0, 25.0), caption("btnSave", "&Save"), FavoritesAction::Save),
        (at(403.0, 479.0, 75.0, 25.0), caption("btnCancel", "&Cancel"), FavoritesAction::Cancel),
    ];
    for (rect, label, value) in buttons {
        if ui.put(rect, egui::Button::new(label)).clicked() {
            action = Some(value);
        }
    }
    ui.painter().hline(area.min.x + 8.0..=area.min.x + 477.0, area.min.y + 464.0, ui.visuals().widgets.noninteractive.bg_stroke);
    let headings = language.text("formOrgFavorites.ListViewCols.ListView", "Shortcut;Name;Path");
    let mut heading = headings.split(';').map(str::to_owned);
    let titles = [heading.next().unwrap_or_else(|| "Shortcut".into()), heading.next().unwrap_or_else(|| "Name".into()), heading.next().unwrap_or_else(|| "Path".into())];
    let widths = [90.0, 100.0, 250.0];
    let visuals = ui.visuals().clone();
    let font = egui::FontId::proportional(13.0);
    let total: f32 = widths.iter().sum();
    let list = at(8.0, 41.0, 469.0, 381.0);
    ui.painter().rect(list, 0.0, visuals.extreme_bg_color, visuals.widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    let header = egui::Rect::from_min_size(list.min + egui::vec2(2.0, 2.0), egui::vec2(total, 20.0));
    let mut x = header.left();
    for (title, width) in titles.iter().zip(widths) {
        let cell = egui::Rect::from_min_size(egui::pos2(x, header.top()), egui::vec2(width, header.height()));
        ui.painter().rect_filled(cell.shrink2(egui::vec2(0.5, 0.0)), 0.0, visuals.widgets.inactive.weak_bg_fill);
        ui.painter().with_clip_rect(cell.shrink2(egui::vec2(4.0, 0.0))).text(cell.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, title, font.clone(), visuals.strong_text_color());
        x += width;
    }
    let mut double = false;
    let rows = egui::Rect::from_min_max(egui::pos2(list.left() + 2.0, header.bottom()), list.max - egui::vec2(2.0, 2.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(rows), |ui| {
        egui::ScrollArea::vertical().id_salt("favorites-list").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, (name, path)) in form.items.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(total, 20.0), egui::Sense::click());
                let selected = form.selected == Some(index);
                if selected {
                    ui.painter().rect_filled(rect, 0.0, visuals.selection.bg_fill);
                }
                let unavailable = !Path::new(path).exists();
                let color = if selected { visuals.selection.stroke.color } else { visuals.text_color() };
                let path_color = if unavailable && !selected { Color32::from_rgb(0xC0, 0xC0, 0xC0).gamma_multiply(0.8) } else { color };
                let gesture = form.gestures.get(index).map(String::as_str).unwrap_or("");
                let mut x = rect.left();
                for ((text, width), cell_color) in [gesture, name.as_str(), path.as_str()].into_iter().zip(widths).zip([color, color, path_color]) {
                    let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(width, 20.0));
                    ui.painter().with_clip_rect(cell.shrink2(egui::vec2(4.0, 0.0))).text(cell.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, text, font.clone(), cell_color);
                    x += width;
                }
                let response = if path.is_empty() { response } else { response.on_hover_text(path.as_str()) };
                if response.clicked() {
                    form.selected = Some(index);
                }
                if super::panes::windows_double_clicked(&response) {
                    form.selected = Some(index);
                    double = true;
                }
            }
        });
    });
    if double {
        action = Some(FavoritesAction::ModifyPath);
    }
    if enter && action.is_none() {
        action = Some(FavoritesAction::Save);
    }
    action
}
/// 드롭다운 메뉴 한 줄.
pub(super) enum PopupItem {
    Header(String),
    Separator,
    Entry { label: String, access: Option<char>, gesture: String, enabled: bool, action: PopupAction },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PopupAction {
    OpenFavorite(String),
    ManageFavorites,
    /// 원본 miAddToFavorites(즐겨찾기 추가).
    AddFavorite,
    History { pane: usize, forward: bool, index: usize },
    /// 원본 mipAddressCopy(주소 복사): 그 패널의 주소를 클립보드로.
    CopyAddress(usize),
    /// 원본 mipAddressEdit(주소 편집): 그 패널의 주소 칸을 편집 상태로.
    EditAddress(usize),
    /// 원본 pmDrop의 복사·이동·바로 가기(DropMenuClick).
    Drop { pane: usize, effect: super::drag_drop::DropEffect, paths: Vec<PathBuf>, destination: PathBuf },
    /// 아무것도 하지 않고 닫기(원본 miDropCancel).
    Close,
    /// 명령 실행에서 실행 파일 기록 후보가 여럿일 때(RunCachedExecutable).
    RunExecutable { path: String, directory: PathBuf },
    /// 원본 도구 모음 Container 단추의 하위 항목(pmToolBarPopup → FUN_00444e04): 그 도구 모음 명령을 실행한다.
    Toolbar(String),
}

/// WPF ContextMenu 자리: 메뉴 위(None) 가운데 또는 마우스 위치. Esc나 바깥 누르기로 닫힌다.
pub(super) struct PopupMenu {
    pub items: Vec<PopupItem>,
    pub position: Option<egui::Pos2>,
    pub highlight: Option<usize>,
    pub opened: bool,
}

/// ConvertLegacyAccessKeysToWpf: "&&"는 '&', "&x"는 x가 단축 문자(표시에서 '&'는 뺀다).
pub(super) fn access_key(label: &str) -> (String, Option<char>) {
    let mut text = String::new();
    let mut access = None;
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.next() {
                Some('&') => text.push('&'),
                Some(next) => {
                    access.get_or_insert(next);
                    text.push(next);
                }
                None => {}
            }
        } else {
            text.push(c);
        }
    }
    (text, access)
}

impl App {
    fn favorites_form(&mut self) -> Option<&mut FavoritesForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::Favorites(form)) => Some(form),
            _ => None,
        }
    }

    /// 압축 파일 안이면 압축 파일이 있는 폴더(C# ActiveState의 현재 폴더).
    pub(super) fn favorites_current(&self) -> String {
        let p = &self.panes[self.active];
        match &p.archive {
            Some(archive) => archive.file.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
            None => p.path.to_string_lossy().into_owned(),
        }
    }

    /// 설정의 즐겨찾기(이름이 없으면 마지막 폴더 이름).
    fn favorite_entries(&self) -> Vec<(String, String)> {
        let names = self.settings.strings("FavoriteNames");
        self.settings.strings("Favorites").into_iter().enumerate().map(|(i, path)| (names.get(i).cloned().unwrap_or_else(|| trimmed_name(&path)), path)).collect()
    }

    /// C# ManageFavoritesAsync: 저장하면 Favorites/FavoriteNames를 바꾸고 설정을 저장한다.
    pub(super) fn manage_favorites(&mut self) {
        let form = FavoritesForm::new(self.favorite_entries(), self.favorites_current(), &self.settings);
        let title = self.language_pack.text("formOrgFavorites.Form.Caption", "Organize Favorites");
        self.push_original_dialog(title, DialogKind::Favorites(Box::new(form)), |app, result, _| {
            let DialogResult::Favorites(Some(items)) = result else { return };
            app.settings.set_strings("Favorites", items.iter().map(|(_, p)| p.clone()).collect());
            app.settings.set_strings("FavoriteNames", items.into_iter().map(|(n, _)| n).collect());
            app.save();
        });
    }

    /// 창 단추.
    pub(super) fn favorites_action(&mut self, action: FavoritesAction, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let Some(form) = self.favorites_form() else { return };
        match action {
            FavoritesAction::AddCurrent => {
                let current = form.current.clone();
                if Path::new(&current).is_dir() {
                    form.add_entry(&current);
                }
            }
            FavoritesAction::AddPath => {
                let current = form.current.clone();
                let (title, message) = if ko { ("즐겨찾기 추가", "폴더 경로") } else { ("Add favorite", "Folder path") };
                self.prompt(title, message, &current, None, false, |app, value, _| {
                    let Some(value) = value.filter(|v| !v.trim().is_empty() && Path::new(v).is_dir()) else { return };
                    let full = std::path::absolute(&value).map(|p| p.to_string_lossy().into_owned()).unwrap_or(value);
                    if let Some(form) = app.favorites_form() {
                        form.add_entry(&full);
                    }
                });
            }
            FavoritesAction::Rename => {
                let Some(index) = form.selected else { return };
                let name = form.items[index].0.clone();
                let (title, message) = if ko { ("즐겨찾기 이름", "표시 이름") } else { ("Favorite name", "Display name") };
                self.prompt(title, message, &name, None, false, move |app, value, _| {
                    let Some(value) = value else { return };
                    if let Some(form) = app.favorites_form() {
                        form.items[index].0 = value;
                        form.selected = Some(index);
                    }
                });
            }
            FavoritesAction::ModifyPath => {
                let Some(index) = form.selected else { return };
                let current = form.items[index].1.clone();
                let title = self.language_pack.text("formFolderTree.Form.Caption", "Select Folder");
                let initial = Path::new(&current).is_dir().then(|| PathBuf::from(&current));
                let Some(folder) = (self.folder_picker)(&title, initial.as_deref()) else { return };
                let path = folder.to_string_lossy().into_owned();
                if let Some(form) = self.favorites_form() {
                    let item = &mut form.items[index];
                    if item.0.is_empty() {
                        item.0 = trimmed_name(&path);
                    }
                    item.1 = path;
                    form.selected = Some(index);
                }
            }
            FavoritesAction::Delete => form.remove_selected(),
            FavoritesAction::Up => form.move_selected(-1),
            FavoritesAction::Down => form.move_selected(1),
            FavoritesAction::Save => {
                let items = form.result();
                self.answer_dialog(DialogResult::Favorites(Some(items)), ctx);
            }
            FavoritesAction::Cancel => self.answer_dialog(DialogResult::Favorites(None), ctx),
        }
    }

    /// C# ShowFavoritesMenu: 이름이 있는 즐겨찾기마다 "&N. 이름"(이름에 '&'가 있거나 10번째부터는 이름 그대로)과 단축키 글,
    /// 없으면 "즐겨찾기가 없습니다"(끔), 구분선, "즐겨찾기 관리…". 메뉴 줄 가운데에 연다.
    pub(super) fn show_favorites_menu(&mut self) {
        let names = self.settings.strings("FavoriteNames");
        let mut items = vec![];
        for (index, path) in self.settings.strings("Favorites").into_iter().enumerate() {
            let stored = names.get(index).cloned().unwrap_or_default();
            if path.is_empty() || stored.is_empty() {
                continue;
            }
            let mut name = if stored.trim().is_empty() { trimmed_name(&path) } else { stored };
            if name.trim().is_empty() {
                name = path.clone();
            }
            let legacy = if name.contains('&') || index >= 9 { name } else { format!("&{}. {name}", index + 1) };
            let (label, access) = access_key(&legacy);
            items.push(PopupItem::Entry { label, access, gesture: favorite_gesture(&self.settings, index), enabled: true, action: PopupAction::OpenFavorite(path) });
        }
        // 원본 pmFavorites(DFM): 즐겨찾기 줄들 다음에 고정 항목 N22("-"), miAddToFavorites, miOrgFavorites.
        // (C#의 "즐겨찾기가 없습니다" 줄은 원본에 없다.)
        self.favorites_tail(&mut items);
        self.popup = Some(PopupMenu { items, position: None, highlight: None, opened: true });
    }

    /// 즐겨찾기 메뉴 끝의 고정 항목: "-", miAddToFavorites(로컬 패널일 때), miOrgFavorites.
    fn favorites_tail(&self, items: &mut Vec<PopupItem>) {
        let ko = self.language == "ko";
        items.push(PopupItem::Separator);
        let add = self.language_pack.raw("Menu.miAddToFavorites").unwrap_or_else(|| if ko { "즐겨찾기 추가".into() } else { "Add to Favorites".into() });
        let (label, access) = access_key(&add);
        items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: self.panes[self.active].archive.is_none(), action: PopupAction::AddFavorite });
        let organize = self.language_pack.raw("Menu.miOrgFavorites").unwrap_or_else(|| if ko { "즐겨찾기 편집".into() } else { "Organize Favorites".into() });
        let (label, access) = access_key(&organize);
        items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: true, action: PopupAction::ManageFavorites });
    }

    /// 주소 칸의 글(압축 안이면 "압축!/접두", 아니면 지금 폴더). 원본 TNXPathLabel.GetPath 자리.
    pub(super) fn address_text(&self, index: usize) -> String {
        let pane = &self.panes[index];
        if let Some(remote) = &pane.remote {
            return remote.address();
        }
        match &pane.archive {
            Some(archive) => format!("{}!/{}", archive.file.display(), archive.prefix),
            None => pane.path.to_string_lossy().into_owned(),
        }
    }

    /// 원본 주소 표시줄 오른쪽 단추(FUN_0048d10c, pmAddressBar): mipAddressCopy, mipAddressEdit, N16("-") 다음에
    /// 비어 있지 않은 즐겨찾기(최대 20개, 이름 그대로, 누르면 그 폴더로), "-", miAddToFavorites·miOrgFavorites 복제를 붙여
    /// 마우스 위치에 연다.
    pub(super) fn show_address_bar_menu(&mut self, index: usize, position: egui::Pos2) {
        let ko = self.language == "ko";
        self.active = index;
        let mut items = vec![];
        let copy = self.language_pack.raw("Menu.mipAddressCopy").unwrap_or_else(|| if ko { "주소 복사".into() } else { "Copy Address".into() });
        let (label, access) = access_key(&copy);
        items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: true, action: PopupAction::CopyAddress(index) });
        let edit = self.language_pack.raw("Menu.mipAddressEdit").unwrap_or_else(|| if ko { "주소 편집".into() } else { "Edit Address".into() });
        let (label, access) = access_key(&edit);
        items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: true, action: PopupAction::EditAddress(index) });
        items.push(PopupItem::Separator);
        let names = self.settings.strings("FavoriteNames");
        for (slot, path) in self.settings.strings("Favorites").into_iter().enumerate().take(20) {
            let name = names.get(slot).cloned().unwrap_or_default();
            // 원본 FUN_00410a44: 이름과 경로가 둘 다 있어야 항목을 만든다.
            if path.is_empty() || name.is_empty() {
                continue;
            }
            let (label, access) = access_key(&name);
            items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: true, action: PopupAction::OpenFavorite(path) });
        }
        self.favorites_tail(&mut items);
        self.popup = Some(PopupMenu { items, position: Some(position), highlight: None, opened: true });
    }

    /// 원본 miAddToFavoritesClick(0x433614): 로컬 패널일 때 TfGetText(TITLE_ADDFAV, MSG_ADDFAV의 %s = 지금 폴더,
    /// 처음 값 = PathFindFileNameW(폴더)). 확인이면 (폴더, 입력한 이름)을 즐겨찾기에 더한다(같은 폴더 검사 없음).
    /// %s는 원본처럼 굵은 글꼴 340 화소에 맞춰 DrawTextW(DT_PATH_ELLIPSIS)로 줄인 글(사각형 0x8e81e4 = 0,0,340,30).
    pub(super) fn add_to_favorites_command(&mut self) {
        let pane = &self.panes[self.active];
        if pane.archive.is_some() {
            return;
        }
        let folder = pane.path.to_string_lossy().into_owned();
        let shown = (self.system.compact_path)(&folder, 340);
        let title = self.language_pack.text("RES.TITLE_ADDFAV", "Add to Favorites");
        let message = self
            .language_pack
            .text("RES.MSG_ADDFAV", "Adding a following folder to favorites.\nType a name for this folder.\n<b><c0000FF>%s</c></b>")
            .replace("%s", &shown);
        let initial = ndir::names::find_file_name(&folder);
        self.prompt(&title, &message, &initial, None, false, move |app, name, _| {
            let Some(name) = name else { return };
            let mut paths = app.settings.strings("Favorites");
            let mut names = app.settings.strings("FavoriteNames");
            names.resize(paths.len(), String::new());
            match (0..paths.len()).find(|&slot| paths[slot].is_empty() && names[slot].is_empty()) {
                Some(slot) => {
                    paths[slot] = folder;
                    names[slot] = name;
                }
                None => {
                    paths.push(folder);
                    names.push(name);
                }
            }
            app.settings.set_strings("Favorites", paths);
            app.settings.set_strings("FavoriteNames", names);
            app.save();
        });
    }

    /// C# ShowNavigationHistoryMenu: "뒤로 이동"(가까운 것부터), 구분선, "앞으로 이동". 둘 다 비면 열지 않는다.
    pub(super) fn show_history_menu(&mut self, pane: usize, position: Option<egui::Pos2>) {
        let mut items = vec![];
        let sections = [(self.tr("뒤로 이동", "Back").into_owned(), &self.panes[pane].back, false), (self.tr("앞으로 이동", "Forward").into_owned(), &self.panes[pane].forward, true)];
        for (title, history, forward) in sections {
            if history.is_empty() {
                continue;
            }
            if forward && !self.panes[pane].back.is_empty() {
                items.push(PopupItem::Separator);
            }
            items.push(PopupItem::Header(title));
            for (index, path) in history.iter().rev().enumerate() {
                items.push(PopupItem::Entry { label: path.to_string_lossy().into_owned(), access: None, gesture: String::new(), enabled: true, action: PopupAction::History { pane, forward, index } });
            }
        }
        if items.is_empty() {
            return;
        }
        self.active = pane;
        self.popup = Some(PopupMenu { items, position, highlight: None, opened: true });
    }

    /// C# OpenFavoriteAsync: ResolveLocalNavigationPath(환경 변수, "~", 상대 경로는 현재 폴더 기준)가 있는 폴더면 활성 패널을 옮긴다.
    pub(super) fn open_favorite(&mut self, path: &str) {
        let expanded = native::expand_environment(path.trim().trim_matches('"'));
        let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default());
        let expanded = if expanded == "~" {
            home.clone()
        } else if let Some(rest) = expanded.strip_prefix("~\\").or_else(|| expanded.strip_prefix("~/")) {
            home.join(rest)
        } else {
            PathBuf::from(expanded)
        };
        let base = PathBuf::from(self.favorites_current());
        let target = std::path::absolute(base.join(&expanded)).unwrap_or(expanded);
        if target.is_dir() {
            self.navigate(self.active, target, true);
        }
    }

    /// C# NavigateHistoryEntryAsync: 현재 위치를 반대쪽에 넣고, 고른 것까지 건너뛴 것들도 반대쪽으로 옮긴 뒤 그 위치로.
    pub(super) fn navigate_history_entry(&mut self, pane: usize, forward: bool, index: usize) {
        let p = &mut self.panes[pane];
        let (source, target) = if forward { (&mut p.forward, &mut p.back) } else { (&mut p.back, &mut p.forward) };
        if index >= source.len() {
            return;
        }
        target.push(p.path.clone());
        for _ in 0..index {
            if let Some(skipped) = source.pop() {
                target.push(skipped);
            }
        }
        let Some(location) = source.pop() else { return };
        self.active = pane;
        self.navigate(pane, location, false);
    }

    pub(super) fn run_popup_action(&mut self, action: PopupAction) {
        match action {
            PopupAction::OpenFavorite(path) => self.open_favorite(&path),
            PopupAction::ManageFavorites => self.manage_favorites(),
            PopupAction::AddFavorite => self.add_to_favorites_command(),
            PopupAction::History { pane, forward, index } => self.navigate_history_entry(pane, forward, index),
            PopupAction::CopyAddress(pane) => {
                let _ = (self.clipboard_writer)(&self.address_text(pane));
            }
            // 편집은 ctx가 필요해 popup_menu_ui에서 처리한다.
            PopupAction::EditAddress(pane) => self.active = pane,
            PopupAction::Drop { pane, effect, paths, destination } => self.drop_from_menu(pane, effect, paths, destination),
            PopupAction::Close => {}
            PopupAction::RunExecutable { path, directory } => self.run_cached_executable(&path, &directory),
            // ctx가 필요해 popup_menu_ui에서 실행한다.
            PopupAction::Toolbar(_) => {}
        }
    }

    /// 드롭다운 그리기: 위/아래·Enter, 단축 문자, Esc·바깥 누르기로 닫기.
    pub(super) fn popup_menu_ui(&mut self, ctx: &egui::Context) {
        let Some(mut popup) = self.popup.take() else { return };
        let entries: Vec<usize> = popup.items.iter().enumerate().filter(|(_, item)| matches!(item, PopupItem::Entry { enabled: true, .. })).map(|(i, _)| i).collect();
        let (escape, up, down, enter, typed) = ctx.input(|i| {
            let typed = i.events.iter().find_map(|e| if let egui::Event::Text(t) = e { t.chars().next() } else { None });
            (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter), typed)
        });
        let mut chosen = None;
        if !entries.is_empty() && (up || down) {
            let position = popup.highlight.and_then(|h| entries.iter().position(|&e| e == h));
            let next = match (position, down) {
                (None, true) => 0,
                (None, false) => entries.len() - 1,
                (Some(p), true) => (p + 1) % entries.len(),
                (Some(p), false) => (p + entries.len() - 1) % entries.len(),
            };
            popup.highlight = Some(entries[next]);
        }
        if enter {
            chosen = popup.highlight;
        }
        if let Some(c) = typed {
            chosen = popup.items.iter().position(|item| matches!(item, PopupItem::Entry { access: Some(a), enabled: true, .. } if a.eq_ignore_ascii_case(&c))).or(chosen);
        }
        let screen = ctx.content_rect();
        let position = popup.position.unwrap_or(egui::pos2(screen.center().x - 150.0, screen.top() + 28.0));
        let area = egui::Area::new(egui::Id::new("ndir-popup-menu")).order(egui::Order::Foreground).fixed_pos(position).show(ctx, |ui| {
            // 스킨 테마면 원본 메뉴 색(skin_menu_style).
            super::skin_pane::apply_menu_style(ui);
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(300.0);
                // VCL 메뉴(AutoLineReduction = maAutomatic)처럼 맨 앞·맨 뒤·겹친 구분선은 그리지 않는다
                // (예: 즐겨찾기가 없을 때 pmFavorites의 N22).
                let visible_separator = |index: usize| {
                    let before = popup.items[..index].iter().rev().find(|i| !matches!(i, PopupItem::Separator)).is_some_and(|i| !matches!(i, PopupItem::Separator));
                    let previous_is_separator = index > 0 && matches!(popup.items[index - 1], PopupItem::Separator);
                    let after = popup.items[index + 1..].iter().any(|i| !matches!(i, PopupItem::Separator));
                    before && after && !previous_is_separator
                };
                for (index, item) in popup.items.iter().enumerate() {
                    match item {
                        PopupItem::Header(title) => {
                            ui.label(RichText::new(title).weak());
                        }
                        PopupItem::Separator => {
                            if visible_separator(index) {
                                ui.separator();
                            }
                        }
                        PopupItem::Entry { label, gesture, enabled, .. } => {
                            // 고른 줄 바탕(스킨이면 mhighlight): 테 없는 단추는 selected여도 바탕을 칠하지 않으므로 뒤에 깐다.
                            let backdrop = ui.painter().add(egui::Shape::Noop);
                            let highlighted = popup.highlight == Some(index);
                            let button = egui::Button::new(label.as_str()).shortcut_text(gesture.as_str()).frame(false).selected(highlighted).min_size(egui::vec2(300.0, 22.0));
                            let response = ui.add_enabled(*enabled, button);
                            if highlighted {
                                ui.painter().set(backdrop, egui::Shape::rect_filled(response.rect, 0.0, ui.visuals().selection.bg_fill));
                            }
                            if response.hovered() && *enabled {
                                popup.highlight = Some(index);
                            }
                            if response.clicked() {
                                chosen = Some(index);
                            }
                        }
                    }
                }
            });
        });
        let outside = !popup.opened && area.response.clicked_elsewhere();
        popup.opened = false;
        if let Some(index) = chosen {
            if let Some(PopupItem::Entry { action, enabled: true, .. }) = popup.items.get(index) {
                let action = action.clone();
                if let PopupAction::EditAddress(pane) = action {
                    self.active = pane;
                    return self.begin_address_edit(pane, ctx);
                }
                if let PopupAction::Toolbar(command) = &action {
                    return self.toolbar_command(command, ctx);
                }
                return self.run_popup_action(action);
            }
        }
        if escape || outside {
            return;
        }
        self.popup = Some(popup);
    }
}
