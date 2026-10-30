//! C# OriginalShortcutKeysWindow(F12, miShortcutClick 0x43c964)와 OriginalEditShortcutDialog(fEditShortCut).
//! 차이: 원본·C#은 메인 창 오른쪽의 따로 떠 있는 도구 창이지만, Rust는 메인 창 안 오른쪽에 붙은 창이다.
//! 수식 키만 누를 때는 글을 바꾸지 않고 남은 키 목록만 새로 고친다(원본은 수식 키 자체도 키로 받는다).
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::shortcut_editor as editor;

/// F12 창 상태(하나만 열린다).
#[derive(Default)]
pub(super) struct ShortcutWindow {
    pub filter: String,
    pub selected: Option<usize>,
    pub focus_search: bool,
}

/// fEditShortCut 입력.
#[derive(Clone, Debug)]
pub(super) struct EditShortcutForm {
    pub caption: String,
    pub original: String,
    pub text: String,
    pub last_state: Option<u8>,
    pub available: Vec<String>,
    pub chosen: Option<usize>,
}

/// egui 키 → Windows 가상 키 코드.
pub(super) fn virtual_key(key: egui::Key) -> Option<u16> {
    use egui::Key::*;
    Some(match key {
        ArrowDown => 0x28,
        ArrowLeft => 0x25,
        ArrowRight => 0x27,
        ArrowUp => 0x26,
        Escape => 0x1b,
        Tab => 0x09,
        Backspace => 0x08,
        Enter => 0x0d,
        Space => 0x20,
        Insert => 0x2d,
        Delete => 0x2e,
        Home => 0x24,
        End => 0x23,
        PageUp => 0x21,
        PageDown => 0x22,
        Minus => 0xbd,
        Plus | Equals => 0xbb,
        OpenBracket => 0xdb,
        CloseBracket => 0xdd,
        Backslash => 0xdc,
        Semicolon => 0xba,
        Quote => 0xde,
        Comma => 0xbc,
        Period => 0xbe,
        Slash => 0xbf,
        Backtick => 0xc0,
        other => {
            let name = other.name();
            if name.len() == 1 {
                let c = name.chars().next()?.to_ascii_uppercase();
                if c.is_ascii_alphanumeric() {
                    return Some(c as u16);
                }
            }
            if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()).filter(|n| (1..=24).contains(n)) {
                return Some(0x6f + number);
            }
            if let Some(digit) = name.strip_prefix("Num").and_then(|n| n.parse::<u16>().ok()).filter(|n| *n <= 9) {
                return Some(0x30 + digit);
            }
            return None;
        }
    })
}

/// TShiftState 비트(1 Shift, 2 Alt, 4 Ctrl).
pub(super) fn shift_state(modifiers: egui::Modifiers) -> u8 {
    u8::from(modifiers.shift) | (u8::from(modifiers.alt) << 1) | (u8::from(modifiers.ctrl) << 2)
}

impl App {
    pub(super) fn shortcut_lines(&self) -> Vec<String> {
        compat::ShortcutMap::parse(self.settings.values.get("OriginalShortcutMapText").and_then(|v| v.as_str())).lines
    }

    fn main_form_raw(language: &compat::Language) -> impl Fn(&str) -> Option<String> + '_ {
        move |key: &str| language.raw(&format!("formMain.{key}"))
    }

    fn resource_text(language: &compat::Language) -> impl Fn(&str, &str) -> String + '_ {
        move |key: &str, fallback: &str| language.text(&format!("RES.{key}"), fallback)
    }

    /// miShortcutClick: 이미 열려 있으면 아무것도 하지 않는다.
    pub(super) fn show_shortcut_window(&mut self) {
        if self.shortcut_window.is_none() {
            self.shortcut_window = Some(ShortcutWindow { focus_search: true, ..Default::default() });
        }
    }

    /// 목록 줄(거르기 적용).
    pub(super) fn shortcut_rows(&self, filter: &str) -> Vec<editor::Row> {
        let raw = Self::main_form_raw(&self.language_pack);
        let resource = Self::resource_text(&self.language_pack);
        editor::rows(&self.shortcut_lines(), filter, &editor::Texts { main_form_raw: &raw, resource: &resource })
    }

    pub(super) fn shortcut_window_ui(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.shortcut_window.take() else { return };
        let title = self.language_pack.text("formShortcut.Form.Caption", "Shortcut Keys");
        let tip = self.language_pack.text("formShortcut.Caption.lTip", "Double click to edit the shortcut key");
        let headers = self.language_pack.text("formShortcut.ListViewCols.lvShortCut", "Command;Shortcut");
        let mut headers = headers.split(';').map(str::to_owned);
        let (command_header, key_header) = (headers.next().unwrap_or_else(|| "Command".into()), headers.next().unwrap_or_else(|| "Shortcut".into()));
        let rows = self.shortcut_rows(&state.filter);
        let mut open = true;
        let mut edit = None;
        let mut close = false;
        let screen = ctx.content_rect();
        egui::Window::new(title)
            .id(egui::Id::new("ndr-shortcut-window"))
            .open(&mut open)
            .collapsible(false)
            .resizable([false, true])
            .default_pos(egui::pos2(screen.right() - 310.0, screen.top() + 40.0))
            .default_size(egui::vec2(300.0, screen.height() - 80.0))
            .show(ctx, |ui| {
                let search = ui.add(egui::TextEdit::singleline(&mut state.filter).desired_width(181.0));
                if state.focus_search {
                    search.request_focus();
                    state.focus_search = false;
                }
                if (search.has_focus() || search.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
                ui.label(tip);
                ui.horizontal(|ui| {
                    ui.add_sized([140.0, 18.0], egui::Label::new(RichText::new(&command_header).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()))));
                    ui.add_sized([115.0, 18.0], egui::Label::new(RichText::new(&key_header).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()))));
                });
                egui::ScrollArea::vertical().id_salt("shortcut-rows").show_rows(ui, 20.0, rows.len(), |ui, range| {
                    for row in &rows[range] {
                        let response = ui
                            .horizontal(|ui| {
                                let selected = state.selected == Some(row.index);
                                let response = ui.add_sized([140.0, 20.0], egui::Button::selectable(selected, row.caption.as_str()).frame(selected));
                                ui.add_sized([115.0, 20.0], egui::Label::new(row.key_text.as_str()));
                                response
                            })
                            .inner;
                        if response.clicked() {
                            state.selected = Some(row.index);
                        }
                        if super::panes::windows_double_clicked(&response) {
                            state.selected = Some(row.index);
                            edit = Some(row.clone());
                        }
                    }
                });
            });
        if open && !close {
            self.shortcut_window = Some(state);
        }
        if let Some(row) = edit {
            self.edit_shortcut(row);
        }
    }

    /// lvShortCutDblClick: fEditShortCut을 띄우고, 확인이면 그 줄을 바꿔 저장한다.
    pub(super) fn edit_shortcut(&mut self, row: editor::Row) {
        let title = self.language_pack.text("formEditShortCut.Form.Caption", "Edit Shortcut Key");
        let form = EditShortcutForm { caption: row.caption.clone(), original: row.key_text.clone(), text: row.key_text.clone(), last_state: None, available: vec![], chosen: None };
        self.push_original_dialog(title, DialogKind::EditShortcut(form), move |app, result, _| {
            let DialogResult::EditShortcut(Some(text)) = result else { return };
            let updated = editor::set_line(&app.shortcut_lines(), row.index, &text);
            app.settings.set_text("OriginalShortcutMapText", editor::to_file_text(&updated));
            app.save();
        });
    }

    /// FormShortCut: 수식 키 상태가 바뀌면 남은 키를 새로 고치고, 키를 누르면 그 글을 칸에 넣는다.
    pub(super) fn capture_shortcut_keys(&mut self, ctx: &egui::Context) {
        let lines = self.shortcut_lines();
        let raw = Self::main_form_raw(&self.language_pack);
        let Some(DialogKind::EditShortcut(form)) = self.original_dialogs.last_mut().map(|d| &mut d.kind) else { return };
        let (modifiers, keys) = ctx.input(|input| {
            let keys: Vec<(egui::Key, egui::Modifiers)> = input.events.iter().filter_map(|event| match event {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            }).collect();
            (input.modifiers, keys)
        });
        let state = shift_state(modifiers);
        if form.last_state != Some(state) {
            form.last_state = Some(state);
            form.available = editor::available_keys(&lines, state, &raw);
            form.chosen = None;
        }
        for (key, modifiers) in keys {
            if let Some(vk) = virtual_key(key) {
                let state = shift_state(modifiers);
                if form.last_state != Some(state) {
                    form.last_state = Some(state);
                    form.available = editor::available_keys(&lines, state, &raw);
                }
                form.text = editor::captured_text(vk, state);
            }
        }
        // 눌린 키가 대화 상자의 다른 칸이나 단추로 가지 않게 지운다(원본: 키는 칸에 들어가지 않는다).
        ctx.input_mut(|input| input.events.retain(|event| !matches!(event, egui::Event::Key { .. } | egui::Event::Text(_))));
    }
}

/// 지정 창 그리기. Some(0) 확인, Some(1) 취소.
pub(super) fn edit_shortcut_ui(ui: &mut egui::Ui, form: &mut EditShortcutForm, language: &compat::Language, lines: &[String]) -> Option<usize> {
    let raw = |key: &str| language.raw(&format!("formMain.{key}"));
    let resource = |key: &str, fallback: &str| language.text(&format!("RES.{key}"), fallback);
    let validation = editor::validate(&form.text, &form.original, lines, &editor::Texts { main_form_raw: &raw, resource: &resource });
    let text = |key: &str, fallback: &str| language.text(&format!("formEditShortCut.{key}"), fallback);
    let mut choice = None;
    // 원본 TfEditShortCut(ClientWidth 290 × ClientHeight 383): lCommandName(8,8 굵게), eShortCut(8,27 193×21), lTip(8,54 빨강),
    // lTip2(24,70 174×21), lAvailableKeys(8,99), lbRemainKeys(8,118 193×251), btnOk(207,25) btnCancel(207,56) btnRemove(207,87) 75×25.
    let (area, _) = ui.allocate_exact_size(egui::vec2(290.0, 383.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    use super::original_dialogs::put_left;
    put_left(ui, at(8.0, 8.0, 274.0, 16.0), egui::Label::new(RichText::new(&form.caption).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()))).truncate());
    ui.put(at(8.0, 27.0, 193.0, 21.0), egui::TextEdit::singleline(&mut form.text.clone()).interactive(false));
    put_left(ui, at(8.0, 54.0, 274.0, 15.0), egui::Label::new(RichText::new(&validation.tip).color(Color32::RED)).truncate());
    put_left(ui, at(24.0, 70.0, 174.0, 21.0), egui::Label::new(&validation.tip2).truncate());
    put_left(ui, at(8.0, 99.0, 193.0, 15.0), egui::Label::new(text("Caption.lAvailableKeys", "Available Keys")).truncate());
    let list = at(8.0, 118.0, 193.0, 251.0);
    ui.painter().rect(list, 0.0, ui.visuals().extreme_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink(2.0)), |ui| {
        egui::ScrollArea::vertical().id_salt("remain-keys").auto_shrink([false, false]).show(ui, |ui| {
            for (index, item) in form.available.clone().iter().enumerate() {
                if ui.selectable_label(form.chosen == Some(index), item).clicked() {
                    form.chosen = Some(index);
                    form.text = editor::replace_key_part(&form.text, item);
                }
            }
        });
    });
    let ok = super::original_dialogs::put_enabled(ui, at(207.0, 25.0, 75.0, 25.0), validation.ok_enabled, egui::Button::new(text("Caption.btnOk", "OK"))).clicked();
    if ok {
        choice = Some(0);
    }
    if ui.put(at(207.0, 56.0, 75.0, 25.0), egui::Button::new(text("Caption.btnCancel", "Cancel"))).clicked() {
        choice = Some(1);
    }
    if ui.put(at(207.0, 87.0, 75.0, 25.0), egui::Button::new(text("Caption.btnRemove", "Remove"))).clicked() {
        form.text.clear();
    }
    choice
}
