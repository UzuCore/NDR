//! C# MainWindow.HandleKeyboardAsync(파일 목록 글쇠) 순서:
//! Esc(EscapeKeyAction) → Shift+글자 드라이브(UseAlternateDriveHandling, 영어가 아닌 입력 언어) → 원본 단축키 표(shortcut.ini,
//! 사용자가 KeyboardShortcuts로 바꾼 NDR 글쇠가 맞으면 건너뜀) → NDR 단축키(ShortcutBindingService) → 기능키 → 나머지 NDR 단축키 →
//! Space(지금 항목 표시 전환) → 동작이 있는 정의(legacy*, favoriteOpenN). 그 뒤 목록 이동(위/아래/Home/End/Tab, egui에는 목록 상자가 없다).
//! 함께: CycleFileSelection, ToggleFocusedItemSelection, ExecuteShortcutActionAsync, ExecuteMenuCommandAsync의 favoriteOpen:/workOpen:/
//! workSet:/column:, RememberDriveFolder/RestoreLastDriveFolder, 드라이브 단추(ShowDriveButtons, 준비된 드라이브, 볼륨 이름 도움말).
//! 차이(HANDOFF): 글 입력 칸에 초점이 있으면 목록 글쇠를 하나도 받지 않는다(C#은 뒤로·앞으로 등 몇 개를 받는다).
use super::remote_window::wpf_key;
use super::*;

impl App {
    /// 원본 주 창은 글쇠를 늘 목록(TNXCustomFileListView)이 받는다(단추는 TabStop 없음). egui는 Tab이나 화살표로 단추에 글쇠 초점을
    /// 옮기고, 초점 단추에서 Enter·Space를 누르기로 바꾸며, 초점이 있는 동안 wants_keyboard_input이 참이라 목록 글쇠가 끊긴다.
    /// 그래서 주 창(바탕 층)의 글 입력 칸이 아닌 위젯이 초점을 가지면 놓게 하고, 아무것도 초점이 없으면 egui의 초점 이동을 없앤다.
    /// 떠 있는 창·메뉴(가운데·앞 층)의 초점은 건드리지 않는다.
    pub(super) fn drop_widget_focus(&mut self, ctx: &egui::Context) {
        if let Some(id) = ctx.memory(|m| m.focused()) {
            let text_edit = egui::text_edit::TextEditState::load(ctx, id).is_some();
            let background = ctx.read_response(id).is_none_or(|r| r.layer_id.order == egui::Order::Background);
            if text_edit || !background {
                return;
            }
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }

    pub(super) fn keyboard(&mut self, ctx: &egui::Context) {
        self.menu_letter = None;
        self.menu_targets_seen = std::mem::take(self.menu_targets.get_mut());
        if self.job.as_ref().is_some_and(|j| !j.finished && j.progress.is_some())
            || !self.msgboxes.is_empty()
            || !self.original_dialogs.is_empty()
            || self.job.as_ref().is_some_and(|j| j.modal.is_some())
            || self.popup.is_some()
        {
            return;
        }
        // 원본 ApplicationEvents1Message(0x442984): WM_XBUTTONDOWN(0x20b)의 XBUTTON1이면 HistoryBack, XBUTTON2면 HistoryForward.
        // (NDR은 대화 상자·진행 창이 떠 있는 동안에는 받지 않는다.)
        let (back, forward) = ctx.input(|i| (i.pointer.button_pressed(egui::PointerButton::Extra1), i.pointer.button_pressed(egui::PointerButton::Extra2)));
        if back {
            self.dispatch("back", ctx);
        } else if forward {
            self.dispatch("forward", ctx);
        }
        // 메뉴가 열려 있으면(원본 Windows 메뉴 모드) 글쇠는 패널로 가지 않는다. 친 글자는 그 메뉴의 접근 글자 항목을 고른다.
        if egui::Popup::is_any_open(ctx) {
            self.menu_letter = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Text(t) = e { t.chars().next() } else { None })).map(|c| c.to_ascii_lowercase());
            // Windows 메뉴: 같은 접근 글자의 항목이 둘 이상이면 실행하지 않고 다음 항목으로 선택만 옮긴다(Enter가 실행).
            if let Some(letter) = self.menu_letter {
                let same: Vec<egui::Id> = self.menu_targets_seen.iter().filter(|(c, _)| *c == letter).map(|(_, id)| *id).collect();
                if same.len() > 1 {
                    let focused = ctx.memory(|m| m.focused());
                    let next = focused.and_then(|f| same.iter().position(|id| *id == f)).map_or(0, |i| (i + 1) % same.len());
                    self.menu_focus.set(Some(same[next]));
                    self.menu_letter = None;
                    ctx.request_repaint();
                }
            }
            return;
        }
        self.drop_widget_focus(ctx);
        // 원본 단축키 표의 숫자판 글쇠(raw_input_hook이 모아 둠).
        for (original, shift) in std::mem::take(&mut self.pending_numpad) {
            self.run_original_shortcut(&original, shift, ctx);
        }
        // 원본 FormShortCut(0x42b36c)은 파일 목록에 초점이 있을 때만 단축키를 본다: 글 입력 칸에 초점이 있으면 목록 글쇠를 받지 않는다.
        if ctx.wants_keyboard_input() {
            return;
        }
        // 원본 KeyDown(0x7d7338) → ArrowKey의 [ ]가 먼저, 그다음 WMChar의 빠른 찾기(같은 글자도 받는다).
        self.bracket_keys(ctx);
        // 원본 TNXCustomFileListView 빠른 찾기(WMChar 0x7d5a18 → QuickSearch 0x7d5f9c).
        if self.original_typed_search(ctx) {
            return;
        }
        let pressed = pressed_key(ctx);
        if let Some((key, modifiers)) = pressed {
            if self.shortcut_key(key, modifiers, ctx) {
                return;
            }
            // 원본 TToolBar(tbFile … tbHelp) CM_DIALOGCHAR: 단축키가 아닌 Alt+글자는 그 접근 글자의 메뉴 단추를 누른다.
            if modifiers.alt && !modifiers.ctrl && !modifiers.shift {
                let name = key.name();
                if name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric() {
                    self.menu_access = name.chars().next().map(|c| c.to_ascii_lowercase());
                    return;
                }
            }
        }
        self.list_navigation(ctx);
    }

    /// 작업 폴더 N 지정(원본 FUN_0042fa5c 자리): 지정, 저장, C# 상태 글 "작업 폴더 N 저장됨".
    pub(super) fn set_work_folder(&mut self, index: usize, path: String) {
        let mut folders = self.settings.strings("WorkingFolders");
        folders.resize(5, String::new());
        folders[index] = path;
        self.settings.set_strings("WorkingFolders", folders);
        self.save();
        self.status = if self.language == "ko" { format!("작업 폴더 {} 저장됨", index + 1) } else { format!("Work folder {} saved", index + 1) };
    }

    /// 원본 단축키 표(shortcut.ini)로 고른 메뉴 항목·내부 명령. None이면 넘긴다.
    fn original_shortcut(&mut self, key: egui::Key, modifiers: egui::Modifiers, ctx: &egui::Context) -> bool {
        let custom = self.settings.values.get("OriginalShortcutMapText").and_then(|v| v.as_str());
        let shortcuts = compat::ShortcutMap::parse(custom);
        let key_name = match key {
            egui::Key::ArrowLeft => "Left",
            egui::Key::ArrowRight => "Right",
            egui::Key::ArrowUp => "Up",
            egui::Key::ArrowDown => "Down",
            egui::Key::Space => "Spacebar",
            other => other.name(),
        };
        let text = format!("{}{}{}{key_name}", if modifiers.ctrl { "Ctrl+" } else { "" }, if modifiers.shift { "Shift+" } else { "" }, if modifiers.alt { "Alt+" } else { "" });
        let Some(original) = shortcuts.resolve(compat::text_to_shortcut(&text)) else { return false };
        self.run_original_shortcut(&original, modifiers.shift, ctx)
    }

    /// 원본 단축키 표에서 숫자판 글쇠(VK_NUMPAD0~9, VK_MULTIPLY·ADD·SUBTRACT·DECIMAL·DIVIDE = "Num 0"…"Num /")에 걸린 항목.
    pub(super) fn numpad_shortcut(&self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> Option<String> {
        let name = match vk {
            0x60..=0x69 => format!("Num {}", vk - 0x60),
            0x6a => "Num *".into(),
            0x6b => "Num +".into(),
            0x6d => "Num -".into(),
            0x6e => "Num .".into(),
            0x6f => "Num /".into(),
            _ => return None,
        };
        let text = format!("{}{}{}{name}", if ctrl { "Ctrl+" } else { "" }, if shift { "Shift+" } else { "" }, if alt { "Alt+" } else { "" });
        let custom = self.settings.values.get("OriginalShortcutMapText").and_then(|v| v.as_str());
        compat::ShortcutMap::parse(custom).resolve(compat::text_to_shortcut(&text))
    }

    /// 글쇠를 목록이 받지 않는 상태(대화 상자·메시지 상자·진행 창·메뉴가 열려 있음). keyboard의 첫 검사와 같다.
    pub(super) fn keyboard_blocked(&self, ctx: &egui::Context) -> bool {
        self.job.as_ref().is_some_and(|j| !j.finished && j.progress.is_some())
            || !self.msgboxes.is_empty()
            || !self.original_dialogs.is_empty()
            || self.job.as_ref().is_some_and(|j| j.modal.is_some())
            || self.popup.is_some()
            || egui::Popup::is_any_open(ctx)
    }

    /// 원본 단축키 표가 고른 항목을 실행한다(메뉴 항목 또는 내부 명령).
    fn run_original_shortcut(&mut self, original: &str, shift: bool, ctx: &egui::Context) -> bool {
        // C# OriginalMenuItemAction의 특수 항목: 모두 선택은 CycleFileSelection, 보이지 않는 항목은 글쇠만 삼킨다.
        match original {
            "miSelectAll" => {
                self.cycle_selection(shift);
                return true;
            }
            "miSearchFile" | "miArcMove" | "miArcInstall" => return true,
            // 원본 miFind(Ctrl+F) = 찾기 입력 상자(TfGetText). C# 판은 같은 글쇠로 도구 모음 빠른 찾기 칸에 초점을 주므로,
            // 그 칸이 도구 모음에 있을 때만 C# 판처럼 하고, 없으면(원본 도구 모음) 원본 찾기 상자를 연다.
            "miFind" if !self.toolbar.items.iter().any(|i| i.kind == ndr::commands::ToolbarKind::QuickSearch) => {
                self.dispatch("find", ctx);
                return true;
            }
            _ => {}
        }
        if let Some((_, action)) = menu_data::ORIGINAL_ACTIONS.iter().find(|(name, _)| *name == original) {
            self.dispatch(action, ctx);
            return true;
        }
        self.original_internal(original, ctx)
    }

    /// 한 번 눌린 글쇠를 원본 차례로 처리한다: 원본 설정 esckey(Esc 동작)·hanshdrive(영어가 아닌 입력 언어의 Shift+글자 드라이브),
    /// 그다음 원본 FormShortCut(단축키 표, 기능키 표). 처리했으면 true.
    pub(super) fn shortcut_key(&mut self, key: egui::Key, modifiers: egui::Modifiers, ctx: &egui::Context) -> bool {
        let Some(name) = wpf_key(key) else { return false };
        if key == egui::Key::Escape {
            match self.settings.values.get("EscapeKeyAction").and_then(|v| v.as_i64()) {
                Some(0) => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                Some(1) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Some(2) => self.dispatch("command", ctx),
                Some(3) => self.panes[self.active].selected.clear(),
                Some(4) => self.dispatch("goParent", ctx),
                _ => return false,
            }
            return true;
        }
        // UseAlternateDriveHandling: 영어가 아닌 입력 언어에서 Shift+글자 = 그 드라이브(마지막 폴더 복원).
        if modifiers.shift && !(modifiers.ctrl || modifiers.alt)
            && self.settings.flag("UseAlternateDriveHandling", false)
            && name.len() == 1
            && name.as_bytes()[0].is_ascii_uppercase()
            && !native::english_input_language()
        {
            let root = PathBuf::from(format!("{name}:\\"));
            if root.is_dir() {
                self.navigate_drive(self.active, root);
                return true;
            }
        }
        // 원본 FormShortCut(0x42b36c): 메뉴 항목·내부 명령 단축키(shortcut.ini 표, 메뉴 IsShortCut)가 먼저.
        if self.original_shortcut(key, modifiers, ctx) {
            return true;
        }
        // 원본 즐겨찾기 메뉴 항목의 단축키(FUN_0042eedc, 메뉴 IsShortCut): 0~9번 Shift+F1~F10, 10~19번 Shift+Ctrl+F1~F10.
        // 항목은 이름이 있는 즐겨찾기만 만들어지므로 빈 칸의 글쇠는 넘긴다.
        if modifiers.shift && !modifiers.alt && name.starts_with('F') {
            if let Some(n) = name[1..].parse::<usize>().ok().filter(|n| (1..=10).contains(n)) {
                let index = if modifiers.ctrl { n + 9 } else { n - 1 };
                let (paths, names) = (self.settings.strings("Favorites"), self.settings.strings("FavoriteNames"));
                if paths.get(index).is_some_and(|p| !p.is_empty()) && names.get(index).is_some_and(|n| !n.is_empty()) {
                    self.prefixed_command(&format!("favoriteOpen:{index}"));
                    return true;
                }
            }
        }
        // 그다음 기능키 표([FuncKey]: F1~F12와 TfPreferences 기능키 목록의 Ctrl+F1~F11 "^F1"~"^F11", FUN_0047cc18 → FUN_0044381c).
        if !(modifiers.ctrl || modifiers.alt || modifiers.shift) && name.starts_with('F') && name[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) && self.invoke_function_key(&name, ctx) {
            return true;
        }
        if modifiers.ctrl && !(modifiers.alt || modifiers.shift) && name.starts_with('F') && name[1..].parse::<u8>().is_ok_and(|n| (1..=11).contains(&n)) && self.invoke_function_key(&format!("^{name}"), ctx) {
            return true;
        }
        false
    }

    /// ExecuteMenuCommandAsync의 favoriteOpen:/workOpen:/workSet:/column:. 처리했으면 true.
    pub(super) fn prefixed_command(&mut self, id: &str) -> bool {
        // 원본 메뉴 이름(menu_data의 openWorkFolderN/saveWorkFolderN/columnN) = C# OriginalMenuItemAction의 workOpen:/workSet:/column:.
        let alias = |prefix: &str| id.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=6).contains(n));
        if let Some(n) = alias("openWorkFolder").filter(|n| *n <= 5) {
            return self.prefixed_command(&format!("workOpen:{}", n - 1));
        }
        if let Some(n) = alias("saveWorkFolder").filter(|n| *n <= 5) {
            return self.prefixed_command(&format!("workSet:{}", n - 1));
        }
        if let Some(n) = alias("column") {
            return self.prefixed_command(&format!("column:{n}"));
        }
        let ko = self.language == "ko";
        let number = |prefix: &str| id.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok());
        if let Some(index) = number("favoriteOpen:") {
            let paths = self.settings.strings("Favorites");
            let names = self.settings.strings("FavoriteNames");
            if index < 20 && paths.get(index).is_some_and(|p| !p.is_empty()) && names.get(index).is_some_and(|n| !n.is_empty()) {
                let path = paths[index].clone();
                self.open_favorite(&path);
            }
            return true;
        }
        if let Some(index) = number("workOpen:") {
            let path = self.settings.strings("WorkingFolders").get(index).cloned().unwrap_or_default();
            if !path.trim().is_empty() && Path::new(&path).is_dir() {
                self.navigate(self.active, PathBuf::from(path), true);
            } else {
                let text = if ko { format!("작업 폴더 {}이 설정되지 않았습니다.", index + 1) } else { format!("Work folder {} is not set.", index + 1) };
                self.system_box("NDR", &text, ndr::msgbox::MB_OK, |_, _, _| {});
            }
            return true;
        }
        if let Some(index) = number("workSet:") {
            if index < 5 {
                let pane = &self.panes[self.active];
                let path = match &pane.archive {
                    Some(archive) => archive.file.parent().map(Path::to_path_buf).unwrap_or_default(),
                    None => pane.path.clone(),
                };
                let path = path.to_string_lossy().into_owned();
                // 원본 FUN_0042ff58: 이미 지정된 작업 폴더면 TfMsgBox(WORKFOLDER, "", MSG_UNSETWORKFOLDER의 %s = 지금 값,
                // MB_YESNO | MB_ICONQUESTION), 예일 때만 바꾼다. 비어 있으면 바로 지정한다.
                let existing = self.settings.strings("WorkingFolders").get(index).cloned().unwrap_or_default();
                if existing.is_empty() {
                    self.set_work_folder(index, path);
                } else {
                    let title = self.language_pack.text("RES.WORKFOLDER", "Work Folder");
                    let text = self
                        .language_pack
                        .text("RES.MSG_UNSETWORKFOLDER", "A following folder is already assigned to this work folder. You want to replace this with current folder?\n%s")
                        .replace("%s", &existing);
                    self.message_box(&title, "", &text, ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONQUESTION, move |app, answer, _| {
                        if answer == ndr::msgbox::IDYES {
                            app.set_work_folder(index, path);
                        }
                    });
                }
            }
            return true;
        }
        if let Some(count) = id.strip_prefix("column:").and_then(|n| n.parse::<i64>().ok()) {
            let key = if self.active == 0 { "LeftColumnCount" } else { "RightColumnCount" };
            self.settings.values.insert(key.into(), serde_json::json!(count.clamp(0, 19)));
            self.save();
            return true;
        }
        false
    }

    /// C# CycleFileSelection(원본 miSelectAllClick 0x4311c4).
    pub(super) fn cycle_selection(&mut self, shift: bool) {
        let select_folders = self.settings.flag("SelectFolders", false);
        let pane = &mut self.panes[self.active];
        let files: Vec<usize> = (0..pane.entries.len()).filter(|&i| !pane.entries[i].is_dir).collect();
        // 원본 SelectAll(0x7dcb00)은 종류 3(파일)과, 폴더도 고를 때 종류 1(폴더)만 고른다(드라이브 항목 제외).
        let folders: Vec<usize> = (0..pane.entries.len()).filter(|&i| pane.entries[i].is_dir && !pane.entries[i].drive && !pane.entries[i].is_parent()).collect();
        if pane.selected.is_empty() {
            let include_folders = files.is_empty() || (select_folders ^ shift);
            pane.selected = (0..pane.entries.len()).filter(|&i| !pane.entries[i].drive && !pane.entries[i].is_parent() && (include_folders || !pane.entries[i].is_dir)).collect();
            return;
        }
        // ".." 줄은 원본처럼 고르지 않는다(+0x67).
        let no_folder_marked = !folders.is_empty() && folders.iter().all(|i| !pane.selected.contains(i));
        let all_files_marked = files.iter().all(|i| pane.selected.contains(i));
        if no_folder_marked && all_files_marked {
            pane.selected.extend(folders);
        } else {
            pane.selected.clear();
        }
    }

    /// C# ToggleFocusedItemSelection: 지금 항목의 표시만 바꾼다(커서는 그대로).
    pub(super) fn toggle_focused_selection(&mut self) {
        let pane = &mut self.panes[self.active];
        if pane.cursor >= pane.entries.len() {
            return;
        }
        if !pane.selected.insert(pane.cursor) {
            pane.selected.remove(&pane.cursor);
        }
    }

    /// C# BeginAddressEdit: 주소 칸에 지금 경로(압축 안이면 "압축!/접두")를 넣고 초점·전체 선택.
    pub(super) fn begin_address_edit(&mut self, index: usize, ctx: &egui::Context) {
        let text = self.address_text(index);
        let pane = &mut self.panes[index];
        pane.address_editing = true;
        pane.address = text;
        let id = egui::Id::new(("ndr-address", index));
        ctx.memory_mut(|memory| memory.request_focus(id));
        let length = pane.address.chars().count();
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(length))));
        state.store(ctx, id);
    }

    /// C# RestoreLastDriveFolder: SaveDriveFolders이고 드라이브 루트면 그 드라이브에서 마지막으로 연 폴더(있으면).
    pub(super) fn restore_drive_folder(&self, path: PathBuf) -> PathBuf {
        if !self.settings.flag("SaveDriveFolders", true) {
            return path;
        }
        let full = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
        let is_root = full.parent().is_none();
        if !is_root {
            return path;
        }
        let root = full.to_string_lossy().into_owned();
        let last = self.settings.values.get("LastFoldersByDrive").and_then(|v| v.as_object()).and_then(|m| {
            m.iter().find(|(key, _)| key.eq_ignore_ascii_case(&root)).and_then(|(_, value)| value.as_str()).map(str::to_owned)
        });
        match last {
            Some(last) if Path::new(&last).is_dir() => std::path::absolute(&last).unwrap_or_else(|_| PathBuf::from(last)),
            _ => path,
        }
    }

    /// C# RememberDriveFolder: SaveDriveFolders면 LastFoldersByDrive[루트] = 전체 경로.
    pub(super) fn remember_drive_folder(&mut self, path: &Path) {
        if !self.settings.flag("SaveDriveFolders", true) || super::panes::is_drives(path) {
            return;
        }
        let Ok(full) = std::path::absolute(path) else { return };
        let Some(root) = full.ancestors().last().map(|r| r.to_string_lossy().into_owned()).filter(|r| !r.is_empty()) else { return };
        let mut map = self.settings.values.get("LastFoldersByDrive").and_then(|v| v.as_object()).cloned().unwrap_or_default();
        map.retain(|key, _| !key.eq_ignore_ascii_case(&root));
        map.insert(root, serde_json::Value::String(full.to_string_lossy().into_owned()));
        self.settings.values.insert("LastFoldersByDrive".into(), serde_json::Value::Object(map));
    }

    /// 드라이브 단추·Shift+글자: 마지막 폴더를 되살려 연다(NavigateLocalAsync restoreDriveFolder).
    pub(super) fn navigate_drive(&mut self, index: usize, root: PathBuf) {
        self.active = index;
        let target = self.restore_drive_folder(root);
        self.navigate(index, target, true);
    }

    /// 원본 ArrowKey의 `[`·`]`(Ctrl·Alt·Shift 없이): 목록 끝 드라이브 항목 사이를 옮긴다(`list_keys::bracket_target`).
    fn bracket_keys(&mut self, ctx: &egui::Context) {
        let (open, close) = ctx.input(|i| {
            let plain = i.modifiers.is_none();
            (plain && i.key_pressed(egui::Key::OpenBracket), plain && i.key_pressed(egui::Key::CloseBracket))
        });
        if !open && !close {
            return;
        }
        let pane = &mut self.panes[self.active];
        if pane.loading || pane.entries.is_empty() {
            return;
        }
        let count = pane.entries.len();
        let cursor = pane.cursor.min(count - 1);
        let on_drive = pane.entries[cursor].drive;
        if let Some(target) = list_keys::bracket_target(cursor, count, pane.drive_count(), on_drive, open) {
            if target != cursor {
                pane.cursor = target;
                pane.reveal = true;
                pane.shift_anchor = None;
            }
        }
    }

    /// 목록 커서 이동(egui에는 목록 상자가 없으므로 직접 한다). 원본 TNXCustomFileListView::ArrowKey(0x7d69dc):
    /// 위·아래·왼쪽·오른쪽·PageUp·PageDown·Home·End를 보기 형식의 쪽 정보로 옮기고(`list_keys::arrow_target`), 옮겼으면 커서가
    /// 보이게 넘긴다(SetItemIndex). Shift를 누르고 옮기면 원본처럼 선택을 지우고 Shift를 누른 자리(SetShiftDownIndex)부터 커서까지 고른다.
    fn list_navigation(&mut self, ctx: &egui::Context) {
        use list_keys::ListKey;
        ctx.input(|input| {
            let m = input.modifiers;
            if input.key_pressed(egui::Key::Tab) && !m.ctrl && !m.alt {
                self.active = 1 - self.active;
            }
            let pane = &mut self.panes[self.active];
            let length = pane.entries.len();
            if !m.shift {
                pane.shift_anchor = None;
            }
            if length == 0 || m.alt {
                return;
            }
            let old = pane.cursor.min(length - 1);
            if m.ctrl {
                // 원본 ArrowKey의 Ctrl(Shift 없이) 글쇠: 첫 항목·마지막 폴더·첫 파일·마지막 파일(`list_keys::ctrl_target`).
                // 마지막 파일은 목록 끝 드라이브 항목(종류 4) 앞이다.
                if m.shift {
                    return;
                }
                let drives = pane.drive_count();
                let folders = pane.entries.iter().filter(|e| e.is_dir && !e.drive).count();
                let files = length - folders - drives;
                let keys = [
                    (egui::Key::PageUp, ListKey::PageUp),
                    (egui::Key::PageDown, ListKey::PageDown),
                    (egui::Key::Home, ListKey::Home),
                    (egui::Key::End, ListKey::End),
                ];
                let mut cursor = old;
                for (key, list_key) in keys {
                    if input.key_pressed(key) {
                        if let Some(target) = list_keys::ctrl_target(folders, files, drives, length, list_key) {
                            cursor = target;
                        }
                    }
                }
                if cursor != old {
                    pane.cursor = cursor;
                    pane.reveal = true;
                }
                return;
            }
            let keys = [
                (egui::Key::ArrowUp, ListKey::Up),
                (egui::Key::ArrowDown, ListKey::Down),
                (egui::Key::ArrowLeft, ListKey::Left),
                (egui::Key::ArrowRight, ListKey::Right),
                (egui::Key::PageUp, ListKey::PageUp),
                (egui::Key::PageDown, ListKey::PageDown),
                (egui::Key::Home, ListKey::Home),
                (egui::Key::End, ListKey::End),
            ];
            let mut cursor = old;
            for (key, list_key) in keys {
                for _ in 0..input.num_presses(key) {
                    cursor = list_keys::arrow_target(&pane.page, cursor, length, list_key);
                }
            }
            if cursor == old {
                return;
            }
            pane.cursor = cursor;
            pane.reveal = true;
            if m.shift {
                let anchor = *pane.shift_anchor.get_or_insert(old);
                pane.selected.clear();
                for i in anchor.min(cursor)..=anchor.max(cursor) {
                    pane.selected.insert(i);
                }
                pane.drop_drive_marks();
            }
        });
    }

    /// 원본 miColumn의 항목들(miColAuto, 구분선, miCol1~6). 지금 값에 표시. 그 밖의 지정 값(7~19)도 C#처럼 보인다. 고른 명령.
    pub(super) fn column_items(&mut self, ui: &mut egui::Ui) -> Option<String> {
        let columns_title = self.language_pack.text("Menu.miColumn", "Columns").replace('&', "");
        let current = self.settings.values.get(if self.active == 0 { "LeftColumnCount" } else { "RightColumnCount" }).and_then(|v| v.as_i64()).unwrap_or(0);
        let mut chosen = None;
        let auto = self.raw_menu("miColAuto", "&A. Auto");
        let response = ui.selectable_label(current == 0, super::favorites_dialog::access_key(&auto).0);
        if response.clicked() || self.take_menu_letter(&auto, &response) {
            chosen = Some("column:0".into());
        }
        ui.separator();
        if (7..=19).contains(&current) && ui.selectable_label(true, format!("{current} {columns_title}")).clicked() {
            chosen = Some(format!("column:{current}"));
        }
        for count in 1..=6 {
            let raw = self.raw_menu(&format!("miCol{count}"), &format!("&{count}  Column"));
            let response = ui.selectable_label(current == count, super::favorites_dialog::access_key(&raw).0);
            if response.clicked() || self.take_menu_letter(&raw, &response) {
                chosen = Some(format!("column:{count}"));
            }
        }
        chosen
    }

    /// 원본 pmColCountPopup(0x437634): 세로 막대 열 단추(bColCount)의 pmColCount는 miColumn의 항목을 그대로 복사해(하위 메뉴 없이)
    /// 보이고, 활성 패널의 열 수(+0x388)가 0이면 첫 항목(자동), 7보다 작으면 그 수의 항목에 표시한다. 작업 폴더 메뉴는 없다.
    pub(super) fn original_column_popup(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(command) = self.column_items(ui) {
            ui.close();
            self.dispatch(&command, ctx);
        }
    }

    /// 원본 miColumn(보기 메뉴): 자동, 1~6열(지금 값에 표시).
    pub(super) fn column_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut chosen: Option<String> = None;
        let raw = self.raw_menu("miColumn", "&C. Columns");
        self.access_submenu(ui, &raw, |app, ui| {
            chosen = app.column_items(ui);
        });
        if let Some(command) = chosen {
            ui.close();
            self.dispatch(&command, ctx);
        }
    }

    /// 원본 폴더 메뉴의 miSetWorkFolder(작업 폴더 1~5 지정)와 miGoToWorkFolder(작업 폴더 1~5로 가기, C#처럼 경로를 붙여 보인다).
    pub(super) fn work_folder_menus(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let folders = self.settings.strings("WorkingFolders");
        let mut chosen: Option<String> = None;
        let set_raw = self.raw_menu("miSetWorkFolder", "&W. Set Work Folder");
        self.access_submenu(ui, &set_raw, |app, ui| {
            for index in 0..5 {
                let raw = app.raw_menu(&format!("miWorkFolder{}", index + 1), &format!("Work Folder &{}", index + 1));
                let response = ui.button(super::favorites_dialog::access_key(&raw).0);
                if response.clicked() || app.take_menu_letter(&raw, &response) {
                    chosen = Some(format!("workSet:{index}"));
                }
            }
        });
        let go_raw = self.raw_menu("miGoToWorkFolder", "&G. Go to Work Folder");
        self.access_submenu(ui, &go_raw, |app, ui| {
            for index in 0..5 {
                let path = folders.get(index).cloned().unwrap_or_default();
                let raw = app.raw_menu(&format!("miGoToWorkFolder{}", index + 1), &format!("Work Folder &{}", index + 1));
                let open = super::favorites_dialog::access_key(&raw).0;
                let label = if path.trim().is_empty() { format!("{open}{}", if ko { " (미설정)" } else { " (not set)" }) } else { format!("{open} — {path}") };
                let response = ui.button(label);
                if response.clicked() || app.take_menu_letter(&raw, &response) {
                    chosen = Some(format!("workOpen:{index}"));
                }
            }
        });
        if let Some(command) = chosen {
            ui.close();
            self.dispatch(&command, ctx);
        }
    }

    /// 드라이브 단추 목록(루트, 도움말). 5초가 지났으면 따로 다시 읽기 시작하고, 그동안은 지난 목록.
    pub(super) fn drive_button_list(&mut self) -> Vec<(PathBuf, String)> {
        let stale = self.drive_cache.as_ref().is_none_or(|(at, _)| at.elapsed() > Duration::from_secs(5));
        if stale && !self.drive_cache_pending {
            self.drive_cache_pending = true;
            let tx = self.async_tx.clone();
            thread::spawn(move || {
                let drives = native::drives()
                    .into_iter()
                    .filter(|root| root.is_dir())
                    .map(|root| {
                        let kind = native::drive_type_name(&root);
                        let tip = match native::volume_label(&root).filter(|l| !l.trim().is_empty()) {
                            Some(label) => format!("{} {label} · {kind}", root.display()),
                            None => format!("{} · {kind}", root.display()),
                        };
                        (root, tip)
                    })
                    .collect();
                let _ = tx.send(AsyncResult::Drives(drives));
            });
        }
        self.drive_cache.as_ref().map(|(_, d)| d.clone()).unwrap_or_default()
    }
}

/// 이번 프레임에 눌린 글쇠와 수식 키. C# MainWindow의 HandleKeyboardAsync는 IsRepeat을 거르지 않으므로
/// 글쇠를 누르고 있으면 반복 입력도 명령을 다시 실행한다(원격 창만 반복을 거른다).
fn pressed_key(ctx: &egui::Context) -> Option<(egui::Key, egui::Modifiers)> {
    ctx.input(|i| {
        i.events.iter().find_map(|event| match event {
            egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
            _ => None,
        })
    })
}
