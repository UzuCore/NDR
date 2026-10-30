//! C# ShortcutWindow(NDIR 단축키 설정, 760×560: 검색 칸, 명령/범위/단축키 표(머리글 정렬, 두 번 누르기 = 변경), 변경…/사용 안 함/기본값, 안내 글,
//! 확인·취소)와 ShortcutCaptureWindow(새 단축키 누르기, Esc 취소) + ShowNdirShortcutSettings(KeyboardShortcuts 저장, 상태 줄).
//! 같은 범위(보이는 범위 글)에서 같은 글쇠면 충돌. 저장할 때 기본값과 같으면 KeyboardShortcuts에서 빼고, 사용 안 함은 빈 글.
//! 차이(HANDOFF): 명령 이름은 C# LocalizeCommand의 메뉴 글·영어 대체 글을 따르되 TranslateText(언어 파일 역검색)는 쓰지 않는다.
use super::grid::{self, Column, GridState, GridView};
use super::original_dialogs::{DialogKind, DialogResult};
use super::remote_window::wpf_key;
use super::*;
use ndir::msgbox::{MB_ICONEXCLAMATION, MB_OK};
use ndir::shortcut_bindings::{self as bindings, Definition, Modifiers as KeyMods};
use serde_json::{Map, Value};

pub(super) struct ShortcutRow {
    pub definition: Definition,
    pub command: String,
    pub scope: String,
    /// 빈 글 = 사용 안 함.
    pub raw: String,
}

pub(super) struct NdirShortcutsForm {
    ko: bool,
    pub rows: Vec<ShortcutRow>,
    pub search: String,
    pub grid: GridState,
    disabled_label: String,
    /// 지금 보이는 줄(검색 거르기 뒤).
    visible: Vec<usize>,
}

impl NdirShortcutsForm {
    pub(super) fn gesture_text(&self, row: &ShortcutRow) -> String {
        if row.raw.is_empty() { self.disabled_label.clone() } else { row.raw.clone() }
    }
    fn refresh_view(&mut self) {
        let previous = self.selected();
        let query = self.search.trim().to_lowercase();
        self.visible = (0..self.rows.len())
            .filter(|&i| {
                let row = &self.rows[i];
                query.is_empty() || row.command.to_lowercase().contains(&query) || row.scope.to_lowercase().contains(&query) || self.gesture_text(row).to_lowercase().contains(&query)
            })
            .collect();
        self.grid = GridState::new(self.visible.len());
        if let Some(display) = previous.and_then(|p| self.visible.iter().position(|&v| v == p)) {
            self.grid.click(display, egui::Modifiers::NONE, false);
        }
    }
    /// 고른 줄(rows 번호).
    pub(super) fn selected(&self) -> Option<usize> {
        self.grid.selected_in_order().first().and_then(|&i| self.visible.get(i).copied())
    }
    /// HasConflict: 같은 범위에서 같은 글쇠를 쓰는 다른 줄의 명령.
    pub(super) fn conflict(&self, index: usize, gesture: &str) -> Option<String> {
        let (key, mods) = bindings::parse(gesture)?;
        let row = &self.rows[index];
        self.rows.iter().enumerate().find_map(|(i, other)| {
            if i == index || other.scope != row.scope {
                return None;
            }
            let (other_key, other_mods) = bindings::parse(&other.raw)?;
            (other_key == key && other_mods == mods).then(|| other.command.clone())
        })
    }
}

/// C# LocalizeCommand(언어 파일 메뉴 글, 없으면 C#의 영어 대체 글).
fn localize_command(definition: &Definition, language: &compat::Language, ko: bool) -> String {
    match definition.id.as_str() {
        "deletePermanent" => return (if ko { "영구 삭제" } else { "Delete permanently" }).into(),
        "cycleSelection" => return (if ko { "파일/폴더 선택 순환" } else { "Cycle file and folder selection" }).into(),
        _ => {}
    }
    // C# fallback = language.TranslateText(definition.Command): 언어 파일 값 역검색.
    let fallback = language.translate_pair(definition.command, definition.command);
    let advanced = match definition.id.as_str() {
        "advancedTabInsert" => "TabSheet.tsInsert",
        "advancedTabReplace" => "TabSheet.tsReplace",
        "advancedTabDelete" => "TabSheet.tsDelete",
        "advancedTabCase" => "TabSheet.tsCase",
        "advancedTabNumber" => "TabSheet.tsNumbering",
        "advancedTabExtension" => "TabSheet.tsExtension",
        _ => "",
    };
    if !advanced.is_empty() {
        return language.text(&format!("formAdvRename.{advanced}"), &fallback);
    }
    let tree = match definition.id.as_str() {
        "treeNewFolder" => "Menu.miCreateNewFolder",
        "treeProperties" => "Menu.miProperties",
        "treeRefresh" => "Menu.miTreeRefresh",
        "treeRefreshRecursive" => "Menu.miSubRefresh",
        "treeFindNext" => "Menu.miFindNext",
        _ => "",
    };
    if !tree.is_empty() {
        return language.text(&format!("formFolderTree.{tree}"), &fallback);
    }
    // language.Menu(명령 id): menu_data에서 그 명령의 원본 메뉴 이름을 찾아 [formMain] 글.
    let menu = |command: &str, fallback: &str| -> String {
        menu_data::ORIGINAL_ACTIONS.iter().find(|(_, action)| *action == command).map(|(name, _)| language.text(&format!("formMain.Menu.{name}"), fallback)).unwrap_or_else(|| fallback.to_owned())
    };
    if !definition.action.is_empty() {
        let action = definition.action;
        let menu_key = if let Some(n) = action.strip_prefix("workOpen:").and_then(|n| n.parse::<usize>().ok()).filter(|n| *n < 5) {
            format!("openWorkFolder{}", n + 1)
        } else if let Some(n) = action.strip_prefix("workSet:").and_then(|n| n.parse::<usize>().ok()).filter(|n| *n < 5) {
            format!("saveWorkFolder{}", n + 1)
        } else if let Some(n) = action.strip_prefix("column:").and_then(|n| n.parse::<usize>().ok()) {
            if (1..=6).contains(&n) { format!("column{n}") } else { String::new() }
        } else if let Some(mode) = action.strip_prefix("sort:") {
            match mode {
                "None" => "sortNone",
                "Name" => "sortName",
                "Extension" => "sortExtension",
                "Size" => "sortSize",
                "Modified" => "sortTime",
                "Attributes" => "sortAttributes",
                "Color" => "sortColor",
                _ => "",
            }
            .to_owned()
        } else if action == "filterOrInstall" {
            String::new()
        } else {
            action.to_owned()
        };
        if !menu_key.is_empty() {
            return menu(&menu_key, &fallback);
        }
    }
    let key = match definition.id.as_str() {
        "copyToOther" => "copyToOther",
        "copyToFolder" => "copyToFolder",
        "moveToOther" => "moveToOther",
        "moveToFolder" => "moveToFolder",
        "refreshF5" | "refreshCtrlR" => "refresh",
        "batchRename" | "advancedRename" => "advancedRename",
        "newFolder" => "newFolder",
        "treeRename" => "rename",
        "historyBack" => "historyBack",
        "historyForward" => "historyForward",
        "newTab" => "newTab",
        "lastDestination" => "lastDestination",
        "deleteF8" | "deleteKey" | "deleteAltD" => "deleteRecycle",
        "shredShiftAltDelete" | "shredShiftAltD" => "shred",
        "open" => "open",
        "copy" => "copy",
        "cut" => "cut",
        "paste" => "paste",
        "selectAll" => "selectAll",
        "extractSelected" => "extractSelected",
        "autoExtract" => "autoExtract",
        "browseFolder" => "browseFolder",
        "favoritesMenu" => "favoritesMenu",
        "shortcutsHelp" => "shortcutsHelp",
        "rename" => "rename",
        "goParent" => "goParent",
        "focusSearch" => "search",
        _ => "",
    };
    // C#: 영어 화면이고 역검색이 바꾸지 못했을 때(fallback == Command)만 영어 대체 글.
    let fallback = if ko || fallback != definition.command {
        fallback
    } else {
        match definition.id.as_str() {
            "historyBack" => "Back",
            "historyForward" => "Forward",
            "addressEdit" => "Edit address",
            "treeRename" => "Rename folder",
            "treeNewFolder" => "Create folder in tree",
            "treeProperties" => "Folder properties",
            "treeRefresh" => "Refresh tree",
            "treeRefreshRecursive" => "Refresh subfolders",
            "treeFindNext" => "Find next folder",
            "newTab" => "New tab",
            "closeTab" => "Close current tab",
            "focusSearch" => "Focus search",
            "refreshCtrlR" | "refreshF5" => "Refresh",
            "deleteF8" | "deleteKey" => "Delete",
            "copyLeft" => "Copy to left panel",
            "copyRight" => "Copy to right panel",
            "moveLeft" => "Move to left panel",
            "moveRight" => "Move to right panel",
            "deleteAltD" => "Delete to Recycle Bin",
            "shredShiftAltDelete" | "shredShiftAltD" => "Shred and delete",
            "toggleSelection" => "Select or deselect item",
            "browseFolder" => "Choose folder",
            "favoritesMenu" => "Favorites menu",
            "shortcutsHelp" => "Shortcut keys",
            _ => definition.command,
        }
        .to_owned()
    };
    if key.is_empty() { fallback } else { menu(key, &fallback) }
}

/// LocalizeScope: 한국어는 그대로, 영어는 폴더 트리 = "Folder tree", 나머지는 "File panel".
fn localize_scope(scope: &str, ko: bool) -> String {
    if ko { scope.into() } else if scope == "폴더 트리" { "Folder tree".into() } else { "File panel".into() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShortcutsAction {
    Edit,
    Disable,
    Default,
    Ok,
    Cancel,
}

pub(super) fn shortcuts_ui(ui: &mut egui::Ui, form: &mut NdirShortcutsForm, language: &compat::Language, enter: bool, first_frame: bool) -> Option<ShortcutsAction> {
    let ko = form.ko;
    let mut action = None;
    ui.set_width(724.0);
    let mut changed = false;
    ui.horizontal(|ui| {
        let search = menu_data::ORIGINAL_ACTIONS.iter().find(|(_, a)| *a == "search").map(|(name, _)| language.text(&format!("formMain.Menu.{name}"), "Search")).unwrap_or_else(|| "Search".into());
        ui.label(search);
        let response = ui.add(egui::TextEdit::singleline(&mut form.search).desired_width(f32::INFINITY));
        if first_frame {
            response.request_focus();
        }
        changed = response.changed();
    });
    if changed {
        form.refresh_view();
    }
    ui.add_space(10.0);
    let titles: Vec<String> = language.text("formShortcut.ListViewCols.lvShortCut", "Command;Shortcut").split(';').map(str::to_owned).collect();
    let columns = [
        Column::text(titles.first().cloned().unwrap_or_else(|| "Command".into()), 300.0),
        Column::text(if ko { "범위" } else { "Scope" }, 200.0),
        Column::text(titles.get(1).cloned().unwrap_or_else(|| "Shortcut".into()), 210.0),
    ];
    let texts: Vec<[String; 3]> = form.visible.iter().map(|&i| {
        let row = &form.rows[i];
        [row.command.clone(), row.scope.clone(), form.gesture_text(row)]
    }).collect();
    let view = GridView { id: "ndir-shortcuts", columns: &columns, height: 330.0, enabled: true, sortable: true, multi_select: false, keyboard: !ui.ctx().wants_keyboard_input(), context_menu: None, row_color: None, background: None };
    let events = grid::show(ui, &view, &mut form.grid, &|row, column| texts[row][column].clone(), &|_| false);
    if let Some(column) = events.sort {
        form.grid.toggle_sort(column, |a, b| texts[a][column].to_lowercase().cmp(&texts[b][column].to_lowercase()));
    }
    if events.double_clicked.is_some() {
        action = Some(ShortcutsAction::Edit);
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button(if ko { "변경…" } else { "Edit…" }).clicked() {
            action = Some(ShortcutsAction::Edit);
        }
        if ui.button(language.text("formEditShortCut.Caption.btnRemove", "Disable")).clicked() {
            action = Some(ShortcutsAction::Disable);
        }
        if ui.button(if ko { "기본값" } else { "Default" }).clicked() {
            action = Some(ShortcutsAction::Default);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(language.text("formEditShortCut.Caption.btnCancel", "Cancel")).clicked() {
                action = Some(ShortcutsAction::Cancel);
            }
            if ui.button(language.text("formEditShortCut.Caption.btnOk", "OK")).clicked() {
                action = Some(ShortcutsAction::Ok);
            }
        });
    });
    ui.add(egui::Label::new(RichText::new(if ko {
        "폴더 트리 단축키는 트리에 포커스가 있을 때 적용됩니다. 파일 패널에서 설정한 F1–F12 함수 키는 같은 키의 일반 단축키보다 먼저 실행됩니다."
    } else {
        "Folder tree shortcuts apply while the tree has focus. Configured F1–F12 function keys take precedence over the same file panel shortcut."
    }).weak()).wrap());
    if enter && action.is_none() && !ui.ctx().wants_keyboard_input() {
        action = Some(ShortcutsAction::Ok);
    }
    action
}

/// ShortcutCaptureWindow: 눌린 글쇠의 글(Esc는 부르는 쪽이 취소로 처리한다). 실행 키가 아니면 안내 글.
pub(super) fn capture_ui(ui: &mut egui::Ui, display: &mut String, ko: bool) -> Option<String> {
    ui.set_width(350.0);
    ui.label(if ko { "새 단축키를 누르세요. Esc를 누르면 취소합니다." } else { "Press the new shortcut. Press Esc to cancel." });
    ui.add_space(18.0);
    ui.vertical_centered(|ui| ui.label(RichText::new(display.as_str()).size(20.0)));
    let pressed = ui.ctx().input(|i| {
        i.events.iter().find_map(|event| match event {
            egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if *key != egui::Key::Escape => Some((*key, *modifiers)),
            _ => None,
        })
    });
    let (key, modifiers) = pressed?;
    match wpf_key(key) {
        Some(name) => Some(bindings::format(&name, KeyMods { ctrl: modifiers.ctrl, alt: modifiers.alt, shift: modifiers.shift })),
        None => {
            *display = (if ko { "Ctrl, Alt, Shift와 실행 키를 함께 누르세요." } else { "Press an action key with Ctrl, Alt or Shift." }).into();
            None
        }
    }
}

impl App {
    pub(super) fn ndir_shortcuts_form(&mut self) -> Option<&mut NdirShortcutsForm> {
        self.original_dialogs.iter_mut().rev().find_map(|d| match &mut d.kind {
            DialogKind::NdirShortcuts(form) => Some(form.as_mut()),
            _ => None,
        })
    }

    /// C# ShowNdirShortcutSettings.
    pub(super) fn open_ndir_shortcuts(&mut self) {
        let ko = self.language == "ko";
        let disabled_label = self.language_pack.text("formEditShortCut.Caption.btnRemove", if ko { "사용 안 함" } else { "Disabled" });
        let rows: Vec<ShortcutRow> = bindings::definitions()
            .into_iter()
            .map(|definition| ShortcutRow {
                command: localize_command(&definition, &self.language_pack, ko),
                scope: localize_scope(definition.scope, ko),
                raw: bindings::gesture(&self.settings, &definition.id).unwrap_or_default(),
                definition,
            })
            .collect();
        let mut form = NdirShortcutsForm { ko, rows, search: String::new(), grid: GridState::new(0), disabled_label, visible: Vec::new() };
        form.refresh_view();
        let title = self.language_pack.text("formShortcut.Form.Caption", "NDIR Shortcut Keys");
        self.push_original_dialog(title, DialogKind::NdirShortcuts(Box::new(form)), |app, result, _| {
            let DialogResult::NdirShortcuts(Some(map)) = result else { return };
            app.settings.values.insert("KeyboardShortcuts".into(), Value::Object(map));
            app.save();
            app.status = app.tr("단축키 설정을 저장했습니다.", "Shortcut settings saved.").into_owned();
        });
    }

    fn shortcut_conflict_box(&mut self, korean: String, english: String) {
        let ko = self.language == "ko";
        let caption = if ko { "단축키 충돌" } else { "Shortcut conflict" };
        self.system_box(caption, if ko { &korean } else { &english }, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
    }

    pub(super) fn ndir_shortcuts_action(&mut self, action: ShortcutsAction, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let Some(form) = self.ndir_shortcuts_form() else { return };
        match action {
            ShortcutsAction::Cancel => self.answer_dialog(DialogResult::NdirShortcuts(None), ctx),
            ShortcutsAction::Disable => {
                if let Some(index) = form.selected() {
                    form.rows[index].raw.clear();
                }
            }
            ShortcutsAction::Default => {
                let Some(index) = form.selected() else { return };
                let default = form.rows[index].definition.default_gesture.clone();
                if let Some(other) = form.conflict(index, &default) {
                    return self.shortcut_conflict_box(format!("기본 단축키가 같은 범위에서 이미 사용 중입니다: {other}"), format!("The default shortcut is already in use in this scope: {other}"));
                }
                form.rows[index].raw = default;
            }
            ShortcutsAction::Edit => {
                let Some(index) = form.selected() else { return };
                let current = form.gesture_text(&form.rows[index]);
                let display = if ko { format!("현재: {current}") } else { format!("Current: {current}") };
                let title = self.language_pack.text("formEditShortCut.Form.Caption", "Edit Shortcut Key");
                self.push_original_dialog(title, DialogKind::ShortcutCapture { display }, move |app, result, _| {
                    let DialogResult::ShortcutCapture(Some(gesture)) = result else { return };
                    let Some(form) = app.ndir_shortcuts_form() else { return };
                    if let Some(other) = form.conflict(index, &gesture) {
                        return app.shortcut_conflict_box(format!("같은 범위에서 이미 사용 중인 단축키입니다: {other}"), format!("That shortcut is already in use in this scope: {other}"));
                    }
                    form.rows[index].raw = gesture;
                });
            }
            ShortcutsAction::Ok => {
                for index in 0..form.rows.len() {
                    if form.rows[index].raw.is_empty() {
                        continue;
                    }
                    if let Some(other) = form.conflict(index, &form.rows[index].raw.clone()) {
                        let command = form.rows[index].command.clone();
                        return self.shortcut_conflict_box(format!("단축키 충돌을 해결한 뒤 저장하세요: {command} / {other}"), format!("Resolve this shortcut conflict before saving: {command} / {other}"));
                    }
                }
                // C#: UpdatedBindings = 지금 KeyboardShortcuts 사본, 기본값과 같으면 빼고 다르면 넣는다(사용 안 함 = 빈 글).
                let mut map: Map<String, Value> = Map::new();
                let existing = self.settings.values.get("KeyboardShortcuts").and_then(Value::as_object).cloned().unwrap_or_default();
                for (key, value) in existing {
                    map.insert(key, value);
                }
                let Some(form) = self.ndir_shortcuts_form() else { return };
                for row in &form.rows {
                    let id = &row.definition.id;
                    map.retain(|key, _| !key.eq_ignore_ascii_case(id));
                    if row.raw != row.definition.default_gesture {
                        map.insert(id.clone(), Value::String(row.raw.clone()));
                    }
                }
                self.answer_dialog(DialogResult::NdirShortcuts(Some(map)), ctx);
            }
        }
    }
}
