//! C# OriginalShortcutKeysWindow (F12, miShortcutClick 0x43c964) and OriginalEditShortcutDialog (fEditShortCut).
//! Difference: in the original and C# it is a separate floating tool window right of the main window; in Rust it is a window docked inside the main window on the right.
//! Pressing only modifiers does not change the text and only refreshes the remaining key list (the original also takes modifiers themselves as keys).
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::shortcut_editor as editor;

/// F12 window state (only one opens).
#[derive(Default)]
pub(super) struct ShortcutWindow {
    pub filter: String,
    pub selected: Option<usize>,
    pub focus_search: bool,
    /// Whether this window is active (the last click was inside it). The original TfShortcut is a separate form, so FormShortCut takes Esc only while it is active.
    pub active: bool,
}

/// fEditShortCut input.
#[derive(Clone, Debug)]
pub(super) struct EditShortcutForm {
    pub caption: String,
    pub original: String,
    pub text: String,
    pub last_state: Option<u8>,
    pub available: Vec<String>,
    pub chosen: Option<usize>,
}

/// egui key -> Windows virtual key code.
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

/// TShiftState bits (1 Shift, 2 Alt, 4 Ctrl).
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

    /// miShortcutClick: does nothing when already open.
    pub(super) fn show_shortcut_window(&mut self) {
        if self.shortcut_window.is_none() {
            self.shortcut_window = Some(ShortcutWindow { focus_search: true, ..Default::default() });
        }
    }

    /// List rows (filter applied).
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
        // The edit dialog captures key presses (including Esc). The F12
        // list underneath must not consume them or steal search focus.
        let covered = !self.original_dialogs.is_empty() || !self.msgboxes.is_empty() || self.job.as_ref().is_some_and(|job| job.modal.is_some());
        let screen = ctx.content_rect();
        let font = egui::TextStyle::Body.resolve(&ctx.style());
        let bold_font = egui::FontId::new(font.size, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()));
        let measure = |s: &str| ctx.fonts_mut(|fonts| fonts.layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE).size().x);
        let measure_header = |s: &str| ctx.fonts_mut(|fonts| fonts.layout_no_wrap(s.to_owned(), bold_font.clone(), Color32::WHITE).size().x);
        let command_width = rows.iter().map(|r| measure(&r.caption) + 16.0).fold((measure_header(&command_header) + 20.0).max(140.0), f32::max);
        let key_width = rows.iter().map(|r| measure(&r.key_text) + 12.0).fold((measure_header(&key_header) + 20.0).max(115.0), f32::max);
        let width = (command_width + key_width + 30.0).max(300.0).min((screen.width() - 40.0).max(120.0));
        let key_width = key_width.min((width * 0.42).max(46.0));
        let command_width = (width - key_width - 30.0).max(44.0);
        let height = (screen.height() - 80.0).max(80.0);
        let compact_rows = screen.width() < 420.0;
        // Use the actual bold font and explicit column widths. In a narrow
        // window, wrap headers and up to two lines per row while preserving
        // show_rows virtualization and the full hover caption.
        let galley = |text: &str, font: &egui::FontId, width: f32, max_rows: usize| {
            let mut job = egui::text::LayoutJob::simple(text.to_owned(), font.clone(), Color32::PLACEHOLDER, width.max(1.0));
            job.wrap.break_anywhere = true;
            job.wrap.max_rows = max_rows;
            ctx.fonts_mut(|fonts| fonts.layout_job(job))
        };
        let command_heading = galley(&command_header, &bold_font, command_width - 4.0, usize::MAX);
        let key_heading = galley(&key_header, &bold_font, key_width - 4.0, usize::MAX);
        let heading_height = command_heading.size().y.max(key_heading.size().y).max(18.0);
        egui::Window::new(title)
            .id(egui::Id::new("ndr-shortcut-window"))
            .open(&mut open)
            .enabled(!covered)
            .collapsible(false)
            .resizable([false, true])
            .default_pos(egui::pos2(screen.right() - width - 10.0, screen.top() + 40.0))
            .min_width(width)
            .max_width(width)
            .max_height(height)
            .default_size(egui::vec2(width, height))
            .show(ctx, |ui| {
                ui.set_width(width);
                let search = ui.add(egui::TextEdit::singleline(&mut state.filter).desired_width(181.0_f32.min(width - 14.0)));
                if state.focus_search && !covered {
                    search.request_focus();
                    state.focus_search = false;
                    state.active = true;
                }
                // Original TntFormShortCut (0x455a54): Esc (0x1b) closes regardless of focus (Handled). NDR docks the window inside the main window, so it takes Esc
                // only when the last click was inside this window (= when this form is active in the original). Fixed: previously the search field needed focus.
                if !covered && ui.input(|i| i.pointer.any_pressed()) {
                    state.active = ui.ctx().pointer_interact_pos().is_some_and(|p| ui.ctx().layer_id_at(p) == Some(ui.layer_id()));
                }
                if !covered && (state.active || search.has_focus() || search.lost_focus()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                    close = true;
                }
                ui.add(egui::Label::new(tip).wrap());
                ui.horizontal(|ui| {
                    ui.add_sized([command_width, heading_height], egui::Label::new(command_heading)).on_hover_text(&command_header);
                    ui.add_sized([key_width, heading_height], egui::Label::new(key_heading)).on_hover_text(&key_header);
                });
                let pad = ui.spacing().button_padding;
                let row_height = if compact_rows { (ctx.fonts_mut(|f| f.row_height(&font)) * 2.0 + 2.0 * pad.y + 2.0).ceil().max(36.0) } else { 20.0 };
                egui::ScrollArea::vertical().id_salt("shortcut-rows").max_height((ui.available_height() - 2.0).max(24.0)).show_rows(ui, row_height, rows.len(), |ui, range| {
                    for row in &rows[range] {
                        let response = ui
                            .horizontal(|ui| {
                                let selected = state.selected == Some(row.index);
                                let response = if compact_rows {
                                    let label = galley(&row.caption, &font, command_width - 2.0 * pad.x - 2.0, 2);
                                    ui.add_sized([command_width, row_height], egui::Button::selectable(selected, label).frame(selected))
                                } else {
                                    ui.add_sized([command_width, row_height], egui::Button::selectable(selected, row.caption.as_str()).frame(selected).truncate())
                                }.on_hover_text(&row.caption);
                                if compact_rows {
                                    ui.add_sized([key_width, row_height], egui::Label::new(galley(&row.key_text, &font, key_width - 4.0, 2))).on_hover_text(&row.key_text);
                                } else {
                                    ui.add_sized([key_width, row_height], egui::Label::new(row.key_text.as_str()).truncate()).on_hover_text(&row.key_text);
                                }
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

    /// lvShortCutDblClick: shows fEditShortCut and on OK changes and saves that row.
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

    /// FormShortCut: refreshes the remaining keys when the modifier state changes, and puts the text of a pressed key in the field.
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
        // Remove the pressed key so it does not go to other fields or buttons of the dialog (original: keys do not enter the field).
        ctx.input_mut(|input| input.events.retain(|event| !matches!(event, egui::Event::Key { .. } | egui::Event::Text(_))));
    }
}

/// Draws the assignment window. Some(0) OK, Some(1) cancel.
pub(super) fn edit_shortcut_ui(ui: &mut egui::Ui, form: &mut EditShortcutForm, language: &compat::Language, lines: &[String]) -> Option<usize> {
    let raw = |key: &str| language.raw(&format!("formMain.{key}"));
    let resource = |key: &str, fallback: &str| language.text(&format!("RES.{key}"), fallback);
    let validation = editor::validate(&form.text, &form.original, lines, &editor::Texts { main_form_raw: &raw, resource: &resource });
    let text = |key: &str, fallback: &str| language.text(&format!("formEditShortCut.{key}"), fallback);
    let mut choice = None;
    let screen = ui.ctx().content_rect();
    // Keep the capture field, validation, list and all three actions usable
    // when the original 290x383 dialog cannot fit the viewport. The full-size
    // DFM geometry below is unchanged.
    if screen.width() < 335.0 || screen.height() < 440.0 {
        let width = (screen.width() - 40.0).clamp(120.0, 290.0);
        ui.set_width(width);
        ui.set_max_width(width);
        let ok_label = text("Caption.btnOk", "OK");
        let cancel_label = text("Caption.btnCancel", "Cancel");
        let remove_label = text("Caption.btnRemove", "Remove");
        let button_width = ((width - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(32.0);
        let font = egui::TextStyle::Button.resolve(ui.style());
        let button_height = [&ok_label, &cancel_label, &remove_label].into_iter().map(|label| {
            ui.fonts_mut(|fonts| fonts.layout(label.to_string(), font.clone(), Color32::PLACEHOLDER, (button_width - 2.0 * ui.spacing().button_padding.x).max(1.0)).size().y)
        }).fold(25.0_f32, |h, text_height| h.max(text_height + 8.0));
        let body_height = (screen.height() - button_height - 72.0).max(48.0);
        egui::ScrollArea::vertical().id_salt("compact-shortcut-editor").max_width(width).max_height(body_height).auto_shrink([false, false]).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(&form.caption).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()))).wrap());
            ui.add(egui::TextEdit::singleline(&mut form.text.clone()).desired_width(ui.available_width()).interactive(false));
            if !validation.tip.is_empty() { ui.add(egui::Label::new(RichText::new(&validation.tip).color(Color32::RED)).wrap()); }
            if !validation.tip2.is_empty() { ui.add(egui::Label::new(&validation.tip2).wrap()); }
            ui.add(egui::Label::new(text("Caption.lAvailableKeys", "Available Keys")).wrap());
            for (index, item) in form.available.iter().enumerate() {
                if ui.selectable_label(form.chosen == Some(index), item).clicked() {
                    form.chosen = Some(index);
                    form.text = editor::replace_key_part(&form.text, item);
                }
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            let size = [button_width, button_height];
            if ui.add_enabled_ui(validation.ok_enabled, |ui| ui.add_sized(size, egui::Button::new(&ok_label).wrap())).inner.clicked() { choice = Some(0); }
            if ui.add_sized(size, egui::Button::new(&cancel_label).wrap()).clicked() { choice = Some(1); }
            if ui.add_sized(size, egui::Button::new(&remove_label).wrap()).clicked() { form.text.clear(); }
        });
        return choice;
    }
    // Original TfEditShortCut (ClientWidth 290 x ClientHeight 383): lCommandName (8,8 bold), eShortCut (8,27 193x21), lTip (8,54 red),
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
