//! Original favorites: TfOrgFavorites (Organize Favorites, miOrgFavoritesClick 0x433990), the favorites menu (FUN_0042eedc), opening (FUN_0048d4b4),
//! and the path history menu (#SHOWPATHHISTORY). Drop-downs share the single `App::popup`.
//! Changing a path uses the folder tree window like the original (FUN_0040ca90).
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;

/// Number of favorite slots (original FUN_0044bed8 creates 20 rows).
const SLOTS: usize = 20;

/// ListView of TfOrgFavorites: SubItems per row (none, a name, or name and path) and the selected row (ItemIndex).
#[derive(Clone, Debug)]
pub(super) struct FavoritesForm {
    pub rows: Vec<Vec<String>>,
    pub selected: Option<usize>,
}

/// Original favorites menu (FUN_0042eedc): items 0-9 have "Shift+F%d" (index + 1), items 10-19 "Shift+Ctrl+F%d" (index - 9) shortcuts.
pub(super) fn favorite_gesture(_settings: &ndr::settings::Settings, index: usize) -> String {
    match index {
        0..=9 => format!("Shift+F{}", index + 1),
        10..=19 => format!("Shift+Ctrl+F{}", index - 9),
        _ => String::new(),
    }
}

/// Shortcut column of the organize window (Caption from FUN_0044bed8): "Shift + F%d" (0-9), "Shift+Ctrl+F%d" (10-19).
fn window_gesture(index: usize) -> String {
    if index < 10 { format!("Shift + F{}", index + 1) } else { format!("Shift+Ctrl+F{}", index - 9) }
}

impl FavoritesForm {
    /// FUN_0044bed8: 20 rows. Only slots with both a name and a path (FUN_00410a44) put name and path in SubItems.
    pub(super) fn new(favorites: Vec<(String, String)>) -> Self {
        let mut rows: Vec<Vec<String>> = favorites.into_iter().take(SLOTS).map(|(name, path)| if name.is_empty() || path.is_empty() { vec![] } else { vec![name, path] }).collect();
        rows.resize(SLOTS, vec![]);
        Self { rows, selected: None }
    }

    /// btnDeleteClick (0x44c004): SubItems.Clear on the selected row (the row stays).
    pub(super) fn delete(&mut self) {
        if let Some(row) = self.selected.and_then(|i| self.rows.get_mut(i)) {
            row.clear();
        }
    }

    /// btnUpClick (0x44c698) / btnDownClick (0x44c7c0): swaps SubItems with the neighbor row and moves the selection. The shortcut column stays in place.
    pub(super) fn move_selected(&mut self, down: bool) {
        let Some(index) = self.selected else { return };
        let next = match down {
            true if index < SLOTS - 1 => index + 1,
            false if index > 0 => index - 1,
            _ => return,
        };
        self.rows.swap(index, next);
        self.selected = Some(next);
    }

    /// Confirmation of btnRenameClick (0x44c3bc): adds the name when there are no SubItems, otherwise replaces the name (index 0).
    pub(super) fn set_name(&mut self, index: usize, name: String) {
        let Some(row) = self.rows.get_mut(index) else { return };
        if row.is_empty() { row.push(name) } else { row[0] = name }
    }

    /// Folder chosen in btnModifyPathClick (0x44c058): with no SubItems adds name (PathFindFileNameW) + path. Otherwise fills an empty name,
    /// adds the path when only a name exists, and replaces the path when both exist.
    pub(super) fn set_path(&mut self, index: usize, path: String) {
        let Some(row) = self.rows.get_mut(index) else { return };
        let name = ndr::names::find_file_name(&path);
        if row.is_empty() {
            row.push(name);
            row.push(path);
            return;
        }
        if row[0].is_empty() {
            row[0] = name;
        }
        if row.len() == 1 { row.push(path) } else { row[1] = path }
    }

    /// btnSaveClick (0x44c8fc): clears all favorites, then stores only rows with two SubItems where both name and path exist into that slot.
    pub(super) fn result(&self) -> Vec<(String, String)> {
        self.rows.iter().map(|row| match row.as_slice() { [name, path] if !name.is_empty() && !path.is_empty() => (name.clone(), path.clone()), _ => (String::new(), String::new()) }).collect()
    }
}

/// Window buttons.
pub(super) enum FavoritesAction {
    Rename,
    ModifyPath,
    Delete,
    Up,
    Down,
    Save,
    Cancel,
}

/// Original TfOrgFavorites (ClientWidth 487 x ClientHeight 512): ListView (8,41 469x381, columns Shortcut 90, Name 100, Path 250, no column clicks,
/// ReadOnly, RowSelect, double click = btnModifyPathClick, missing paths in clSilver), btnRename (8,428) btnModifyPath (89,428) btnUp (170,428)
/// btnDown (251,428) 79x25, btnDelete (402,428 75x25), Bevel1 (8,464 469, top line), btnSave (319,479) btnCancel (403,479, Cancel) 75x25.
/// There is no Default button (Enter does nothing).
pub(super) fn favorites_ui(ui: &mut egui::Ui, form: &mut FavoritesForm, language: &compat::Language) -> Option<FavoritesAction> {
    let caption = |key: &str, fallback: &str| language.raw(&format!("formOrgFavorites.Caption.{key}")).unwrap_or_else(|| fallback.to_owned());
    let mut action = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(487.0, 512.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let buttons = [
        (at(8.0, 428.0, 79.0, 25.0), caption("btnRename", "&Rename"), FavoritesAction::Rename),
        (at(89.0, 428.0, 79.0, 25.0), caption("btnModifyPath", "&Edit"), FavoritesAction::ModifyPath),
        (at(170.0, 428.0, 79.0, 25.0), caption("btnUp", "&Up"), FavoritesAction::Up),
        (at(251.0, 428.0, 79.0, 25.0), caption("btnDown", "Do&wn"), FavoritesAction::Down),
        (at(402.0, 428.0, 75.0, 25.0), caption("btnDelete", "&Delete"), FavoritesAction::Delete),
        (at(319.0, 479.0, 75.0, 25.0), caption("btnSave", "&Save"), FavoritesAction::Save),
        (at(403.0, 479.0, 75.0, 25.0), caption("btnCancel", "&Cancel"), FavoritesAction::Cancel),
    ];
    // The original window has only a ListView and buttons, so typing a letter triggers the access key (VCL CM_DIALOGCHAR).
    for (rect, label, value) in buttons {
        if super::original_dialogs::access_button(ui, rect, &label) && action.is_none() {
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
            for (index, row) in form.rows.iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(total, 20.0), egui::Sense::click());
                let selected = form.selected == Some(index);
                if selected {
                    ui.painter().rect_filled(rect, 0.0, visuals.selection.bg_fill);
                }
                let color = if selected { visuals.selection.stroke.color } else { visuals.text_color() };
                // ListViewAdvancedCustomDrawSubItem (0x44c5b8): the path column is clSilver when there are two SubItems and PathFileExistsW is false.
                let missing = row.len() >= 2 && !Path::new(&row[1]).exists();
                let path_color = if missing && !selected { Color32::from_rgb(0xC0, 0xC0, 0xC0).gamma_multiply(0.8) } else { color };
                let gesture = window_gesture(index);
                let name = row.first().map(String::as_str).unwrap_or("");
                let path = row.get(1).map(String::as_str).unwrap_or("");
                let mut x = rect.left();
                for ((text, width), cell_color) in [gesture.as_str(), name, path].into_iter().zip(widths).zip([color, color, path_color]) {
                    let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(width, 20.0));
                    ui.painter().with_clip_rect(cell.shrink2(egui::vec2(4.0, 0.0))).text(cell.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, text, font.clone(), cell_color);
                    x += width;
                }
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
    action
}
/// One drop-down menu row.
pub(super) enum PopupItem {
    Separator,
    Entry { label: String, access: Option<char>, gesture: String, enabled: bool, checked: bool, action: PopupAction },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum PopupAction {
    OpenFavorite(String),
    ManageFavorites,
    /// Original miAddToFavorites (add to favorites).
    AddFavorite,
    /// One row of the original address bar history menu (FUN_0048d05c): go to the `index`-th folder of the history list (oldest first).
    History { pane: usize, index: usize },
    /// Original mipAddressCopy (copy address): copies that pane's address to the clipboard.
    CopyAddress(usize),
    /// Original mipAddressEdit (edit address): puts that pane's address field into edit mode.
    EditAddress(usize),
    /// Copy, move or shortcut of the original pmDrop (DropMenuClick).
    Drop { pane: usize, effect: super::drag_drop::DropEffect, paths: Vec<PathBuf>, destination: PathBuf },
    /// Close without doing anything (original miDropCancel).
    Close,
    /// When running a command has several executable history candidates (RunCachedExecutable).
    RunExecutable { path: String, directory: PathBuf },
    /// Sub-item of an original toolbar Container button (pmToolBarPopup -> FUN_00444e04): runs that toolbar command.
    Toolbar(String),
}

/// In place of a WPF ContextMenu: centered above the menu (None) or at the mouse position. Esc or an outside click closes it.
pub(super) struct PopupMenu {
    pub items: Vec<PopupItem>,
    pub position: Option<egui::Pos2>,
    pub highlight: Option<usize>,
    pub opened: bool,
}

/// ConvertLegacyAccessKeysToWpf: "&&" is '&', "&x" makes x the access key ('&' is removed from the display).
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

    /// Inside an archive, the folder containing the archive (current folder).
    pub(super) fn favorites_current(&self) -> String {
        let p = &self.panes[self.active];
        match &p.archive {
            Some(archive) => archive.file.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
            None => p.path.to_string_lossy().into_owned(),
        }
    }

    /// Favorite slots from the settings (name, path).
    fn favorite_entries(&self) -> Vec<(String, String)> {
        let names = self.settings.strings("FavoriteNames");
        self.settings.strings("Favorites").into_iter().enumerate().map(|(i, path)| (names.get(i).cloned().unwrap_or_default(), path)).collect()
    }

    /// Original miOrgFavoritesClick (0x433990): shows TfOrgFavorites and on save replaces the favorites (20 slots). The menu is rebuilt after closing
    /// (here the menu is built from the settings every time it opens).
    pub(super) fn manage_favorites(&mut self) {
        let form = FavoritesForm::new(self.favorite_entries());
        let title = self.language_pack.text("formOrgFavorites.Form.Caption", "Organize Favorites");
        self.push_original_dialog(title, DialogKind::Favorites(Box::new(form)), |app, result, _| {
            let DialogResult::Favorites(Some(items)) = result else { return };
            app.settings.set_strings("Favorites", items.iter().map(|(_, p)| p.clone()).collect());
            app.settings.set_strings("FavoriteNames", items.into_iter().map(|(n, _)| n).collect());
            app.save();
        });
    }

    /// Window buttons.
    pub(super) fn favorites_action(&mut self, action: FavoritesAction, ctx: &egui::Context) {
        let Some(form) = self.favorites_form() else { return };
        match action {
            FavoritesAction::Rename => {
                // btnRenameClick (0x44c3bc): nothing without a selected row. TfGetText(title RES.RENAME, no message, initial value = name).
                let Some(index) = form.selected else { return };
                let name = form.rows[index].first().cloned().unwrap_or_default();
                let title = self.language_pack.text("RES.RENAME", "Rename");
                self.prompt(&title, "", &name, None, false, move |app, value, _| {
                    let Some(value) = value else { return };
                    if let Some(form) = app.favorites_form() {
                        form.set_name(index, value);
                    }
                });
            }
            FavoritesAction::ModifyPath => {
                // btnModifyPathClick (0x44c058): FUN_0040ca90 (initial path = the path when there are two SubItems, otherwise empty) - the folder tree window.
                let Some(index) = form.selected else { return };
                let current = form.rows[index].get(1).cloned().unwrap_or_default();
                self.folder_tree_dialog(Path::new(&current), None, move |app, folder, _| {
                    let Some(folder) = folder else { return };
                    let path = folder.to_string_lossy().into_owned();
                    if let Some(form) = app.favorites_form() {
                        form.set_path(index, path);
                    }
                });
            }
            FavoritesAction::Delete => form.delete(),
            FavoritesAction::Up => form.move_selected(false),
            FavoritesAction::Down => form.move_selected(true),
            FavoritesAction::Save => {
                let items = form.result();
                self.answer_dialog(DialogResult::Favorites(Some(items)), ctx);
            }
            FavoritesAction::Cancel => self.answer_dialog(DialogResult::Favorites(None), ctx),
        }
    }

    /// Original favorites menu (pmFavorites, FUN_0042eedc): for each slot with name and path, Caption = the name as is ('&' marks the access key),
    /// shortcut text, and clicking opens that slot. Fixed items follow. Opens centered on the menu bar.
    pub(super) fn show_favorites_menu(&mut self) {
        let mut items = vec![];
        for (index, (name, path)) in self.favorite_entries().into_iter().enumerate().take(SLOTS) {
            if path.is_empty() || name.is_empty() {
                continue;
            }
            let (label, access) = access_key(&name);
            items.push(PopupItem::Entry { label, access, gesture: favorite_gesture(&self.settings, index), enabled: true, checked: false, action: PopupAction::OpenFavorite(path) });
        }
        // Original pmFavorites (DFM): after the favorite rows, fixed items N22 ("-"), miAddToFavorites, miOrgFavorites.
        self.favorites_tail(&mut items);
        self.popup = Some(PopupMenu { items, position: None, highlight: None, opened: true });
    }
    /// Fixed items at the end of the favorites menu: "-", miAddToFavorites (on a local pane), miOrgFavorites.
    fn favorites_tail(&self, items: &mut Vec<PopupItem>) {
        let ko = self.language == "ko";
        items.push(PopupItem::Separator);
        let add = self.language_pack.raw("Menu.miAddToFavorites").unwrap_or_else(|| if ko { "즐겨찾기 추가".into() } else { "Add to Favorites".into() });
        let (label, access) = access_key(&add);
        items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: self.panes[self.active].archive.is_none(), checked: false, action: PopupAction::AddFavorite });
        let organize = self.language_pack.raw("Menu.miOrgFavorites").unwrap_or_else(|| if ko { "즐겨찾기 편집".into() } else { "Organize Favorites".into() });
        let (label, access) = access_key(&organize);
        items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: true, checked: false, action: PopupAction::ManageFavorites });
    }

    /// Address field text ("archive!/prefix" inside an archive, otherwise the current folder). In place of the original TNXPathLabel.GetPath.
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

    /// Original address bar right-click (FUN_0048d10c, pmAddressBar): mipAddressCopy, mipAddressEdit, N16 ("-") then
    /// adds the non-empty favorites (up to 20, names as is, clicking goes to that folder), "-", and copies of miAddToFavorites and miOrgFavorites,
    /// and opens at the mouse position.
    pub(super) fn show_address_bar_menu(&mut self, index: usize, position: egui::Pos2) {
        let ko = self.language == "ko";
        self.active = index;
        let mut items = vec![];
        let copy = self.language_pack.raw("Menu.mipAddressCopy").unwrap_or_else(|| if ko { "주소 복사".into() } else { "Copy Address".into() });
        let (label, access) = access_key(&copy);
        items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: true, checked: false, action: PopupAction::CopyAddress(index) });
        let edit = self.language_pack.raw("Menu.mipAddressEdit").unwrap_or_else(|| if ko { "주소 편집".into() } else { "Edit Address".into() });
        let (label, access) = access_key(&edit);
        items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: true, checked: false, action: PopupAction::EditAddress(index) });
        items.push(PopupItem::Separator);
        let names = self.settings.strings("FavoriteNames");
        for (slot, path) in self.settings.strings("Favorites").into_iter().enumerate().take(20) {
            let name = names.get(slot).cloned().unwrap_or_default();
            // Original FUN_00410a44: an item needs both a name and a path.
            if path.is_empty() || name.is_empty() {
                continue;
            }
            let (label, access) = access_key(&name);
            items.push(PopupItem::Entry { label, access, gesture: String::new(), enabled: true, checked: false, action: PopupAction::OpenFavorite(path) });
        }
        self.favorites_tail(&mut items);
        self.popup = Some(PopupMenu { items, position: Some(position), highlight: None, opened: true });
    }

    /// Original miAddToFavoritesClick (0x433614): on a local pane, TfGetText(TITLE_ADDFAV, MSG_ADDFAV with %s = current folder,
    /// initial value = PathFindFileNameW(folder)). On OK adds (folder, typed name) to the favorites (no duplicate folder check).
    /// %s is shortened like the original with DrawTextW(DT_PATH_ELLIPSIS) to 340 px in the bold font (rectangle 0x8e81e4 = 0,0,340,30).
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
        let initial = ndr::names::find_file_name(&folder);
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

    /// Menu of the original address bar history button (FUN_0048ced4): one row per pane history entry (+0x37c, oldest first up to the last index),
    /// text = "&%d. %s" (index below 10) or "&%c. %s" (index + 'A', so 10 is K), the current position (+8) checked. The original history is one
    /// flat list with a current position (back = position - 1, forward = + 1); NDR lays out its history (back, current folder, forward) in that order.
    pub(super) fn show_history_menu(&mut self, pane: usize, position: Option<egui::Pos2>) {
        let list = self.history_list(pane);
        let current = self.panes[pane].back.len();
        let items: Vec<PopupItem> = list
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let access = if index < 10 { char::from_digit(index as u32, 10) } else { char::from_u32(index as u32 + 0x41) };
                let label = format!("{}. {}", access.map(String::from).unwrap_or_default(), path.to_string_lossy());
                PopupItem::Entry { label, access, gesture: String::new(), enabled: true, checked: index == current, action: PopupAction::History { pane, index } }
            })
            .collect();
        self.active = pane;
        self.popup = Some(PopupMenu { items, position, highlight: None, opened: true });
    }

    /// Original history list order: folders to go back to (oldest first), the current folder, folders to go forward to (nearest first).
    pub(super) fn history_list(&self, pane: usize) -> Vec<PathBuf> {
        let p = &self.panes[pane];
        p.back.iter().cloned().chain(std::iter::once(p.path.clone())).chain(p.forward.iter().rev().cloned()).collect()
    }

    /// Original FUN_0048d4b4 (from the favorites menu FUN_0042f078): if that slot's path exists (PathFileExistsW), moves the active pane to that folder.
    /// Environment variables and "~" are not expanded.
    pub(super) fn open_favorite(&mut self, path: &str) {
        let target = PathBuf::from(path);
        if target.is_dir() {
            self.navigate(self.active, target, true);
        }
    }

    /// Original FUN_0048d05c: goes to the chosen row's folder (FUN_00459968). It is not a history move (back/forward, +0x2e8), so receiving the listing (FUN_00486694)
    /// adds it to the history with FUN_00458348: entries after the current position are dropped and it is appended (unchanged when it equals the current folder). Same as NDR navigate (remembering).
    pub(super) fn navigate_history_entry(&mut self, pane: usize, index: usize) {
        let Some(location) = self.history_list(pane).get(index).cloned() else { return };
        self.active = pane;
        self.navigate(pane, location, true);
    }

    pub(super) fn run_popup_action(&mut self, action: PopupAction) {
        match action {
            PopupAction::OpenFavorite(path) => self.open_favorite(&path),
            PopupAction::ManageFavorites => self.manage_favorites(),
            PopupAction::AddFavorite => self.add_to_favorites_command(),
            PopupAction::History { pane, index } => self.navigate_history_entry(pane, index),
            PopupAction::CopyAddress(pane) => {
                let _ = (self.clipboard_writer)(&self.address_text(pane));
            }
            // Editing needs ctx, so popup_menu_ui handles it.
            PopupAction::EditAddress(pane) => self.active = pane,
            PopupAction::Drop { pane, effect, paths, destination } => self.drop_from_menu(pane, effect, paths, destination),
            PopupAction::Close => {}
            PopupAction::RunExecutable { path, directory } => self.run_cached_executable(&path, &directory),
            // Needs ctx, so popup_menu_ui runs it.
            PopupAction::Toolbar(_) => {}
        }
    }

    /// Draws the drop-down: Up/Down and Enter, access keys, Esc or an outside click closes it.
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
        let area = egui::Area::new(egui::Id::new("ndr-popup-menu")).order(egui::Order::Foreground).fixed_pos(position).show(ctx, |ui| {
            // In the skin theme, the original menu colors (skin_menu_style).
            super::skin_pane::apply_menu_style(ui);
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(300.0);
                // Like VCL menus (AutoLineReduction = maAutomatic), leading, trailing and doubled separators are not drawn
                // (for example N22 of pmFavorites when there are no favorites).
                let visible_separator = |index: usize| {
                    let before = popup.items[..index].iter().rev().find(|i| !matches!(i, PopupItem::Separator)).is_some_and(|i| !matches!(i, PopupItem::Separator));
                    let previous_is_separator = index > 0 && matches!(popup.items[index - 1], PopupItem::Separator);
                    let after = popup.items[index + 1..].iter().any(|i| !matches!(i, PopupItem::Separator));
                    before && after && !previous_is_separator
                };
                for (index, item) in popup.items.iter().enumerate() {
                    match item {
                        PopupItem::Separator => {
                            if visible_separator(index) {
                                ui.separator();
                            }
                        }
                        PopupItem::Entry { label, gesture, enabled, checked, .. } => {
                            // Highlighted row background (mhighlight in the skin): a frameless button paints no background even when selected, so it goes underneath.
                            let backdrop = ui.painter().add(egui::Shape::Noop);
                            let highlighted = popup.highlight == Some(index);
                            // VCL TMenuItem.Checked: the check mark slot before the text (drawn with two lines instead of a glyph the font may lack).
                            let text = format!("     {label}");
                            let button = egui::Button::new(text).shortcut_text(gesture.as_str()).frame(false).selected(highlighted).min_size(egui::vec2(300.0, 22.0));
                            let response = ui.add_enabled(*enabled, button);
                            if *checked {
                                let (x, y) = (response.rect.left() + 8.0, response.rect.center().y);
                                let stroke = egui::Stroke::new(1.5, ui.visuals().text_color());
                                ui.painter().line_segment([egui::pos2(x, y), egui::pos2(x + 3.0, y + 3.0)], stroke);
                                ui.painter().line_segment([egui::pos2(x + 3.0, y + 3.0), egui::pos2(x + 9.0, y - 4.0)], stroke);
                            }
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
        // A drop-down button opens its menu on the press (TNXImageButton/TNXPngButton MouseDown), so the release of that press,
        // possibly a frame later, is not a click outside the new menu.
        popup.opened = popup.opened && ctx.input(|i| i.pointer.any_down() || i.pointer.any_released());
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
