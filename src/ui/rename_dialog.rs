//! C# AdvancedRenameWindow (advanced rename): choose an action -> target and values -> add, the applied list (delete/up/down), macros (advrename.save),
//! preview (first 500), rename after a confirmation box. Rules are compat::rename_with_rules (C# AdvancedRenameService.Preview).
use super::original_dialogs::{DialogKind, DialogResult, glyph_texture};
use super::*;
use compat::RenameRule;
use ndr::msgbox::{IDYES, MB_ICONEXCLAMATION, MB_ICONQUESTION, MB_OK, MB_YESNO};
use ndr::rename_macros::{self as macros, RenameMacro};

/// Glyph.Data of sbtnSave and sbtnDelete in the original TfAdvRename DFM (the BMP part of a TBitmap stream, 16x16, 8-bit).
const SAVE_GLYPH: &[u8] = include_bytes!("glyphs/advrename-save.bmp");
const DELETE_GLYPH: &[u8] = include_bytes!("glyphs/advrename-delete.bmp");

#[derive(Clone, Debug)]
pub(super) struct RenameForm {
    pub paths: Vec<PathBuf>,
    pub dirs: Vec<bool>,
    pub rules: Vec<RenameRule>,
    pub selected_rule: Option<usize>,
    pub editor: RenameRule,
    /// Text of the position, count, start, step and digits fields.
    pub numbers: [String; 5],
    pub macros: Vec<RenameMacro>,
    /// None = "Custom".
    pub macro_name: Option<String>,
    /// Row chosen in lvPreview (original lvPreviewKeyDown: Ctrl+C = that row's original name to the clipboard).
    pub preview_selected: Option<usize>,
    /// Selected section on a compact viewport; stays local to this dialog.
    pub compact_page: usize,
}

fn text(language: &compat::Language, key: &str, fallback: &str) -> String {
    language.text(&format!("formAdvRename.{key}"), fallback)
}

fn choices(language: &compat::Language, key: &str, fallback: &str, values: &[&'static str]) -> Vec<(&'static str, String)> {
    let labels = text(language, key, fallback);
    let labels: Vec<&str> = labels.split(';').collect();
    values.iter().enumerate().map(|(i, v)| (*v, labels.get(i).map_or_else(|| (*v).to_owned(), |l| l.replace('&', "")))).collect()
}

fn action_choices(language: &compat::Language) -> Vec<(&'static str, String)> {
    [("Insert", "TabSheet.tsInsert", "Insert"), ("Replace", "TabSheet.tsReplace", "Find/Replace"), ("Delete", "TabSheet.tsDelete", "Delete"), ("Case", "TabSheet.tsCase", "Case"), ("Number", "TabSheet.tsNumbering", "Numbering"), ("Extension", "TabSheet.tsExtension", "Extension")]
        .iter()
        .map(|(v, k, f)| (*v, text(language, k, f)))
        .collect()
}

/// PositionChoices.
fn position_choices(language: &compat::Language, action: &str) -> Vec<(&'static str, String)> {
    let item = |value: &'static str, key: &str, fallback: &str| (value, text(language, key, fallback).replace('&', ""));
    match action {
        "Insert" => vec![item("Prefix", "Caption.rbInsertLocFirst", "Prefix"), item("Suffix", "Caption.rbInsertLocLast", "Suffix"), item("Position", "Caption.rbInsertLocPos", "Position")],
        "Number" => vec![item("Prefix", "Caption.rbNumLocFirst", "Prefix"), item("Suffix", "Caption.rbNumLocLast", "Suffix"), item("Position", "Caption.rbNumLocPos", "Position")],
        _ => vec![item("First", "Caption.rbReplaceFirst", "First occurrence"), item("Last", "Caption.rbReplaceLast", "Last occurrence"), item("All", "Caption.rbReplaceAll", "All occurrences")],
    }
}

fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&'static str, String)], width: f32) -> bool {
    let shown = options.iter().find(|(v, _)| *v == value.as_str()).map_or_else(|| value.clone(), |(_, l)| l.clone());
    let mut changed = false;
    egui::ComboBox::from_id_salt(id).width(width).selected_text(shown).show_opaque(ui, |ui| {
        for (option, label) in options {
            if ui.selectable_label(value.as_str() == *option, label).clicked() {
                *value = (*option).to_owned();
                changed = true;
            }
        }
    });
    changed
}

/// Original FUN_0041830c (rule list row): "action (description)" + " - extension"/" - name". Action names are ADVRENAME_ACTIONS split on ';',
/// descriptions use the same translated field labels as the rule editor.
/// Numbering 0x46c170, ChangeExtension 0x46c7ec. Long text is cut with lstrcpynW and gets "..." when longer than the limit.
pub(super) fn rule_display(rule: &RenameRule, language: &compat::Language) -> String {
    let actions = language.text("RES.ADVRENAME_ACTIONS", "Insert;Find/Replace;Delete;Case;Numbering;Change Extension");
    let actions: Vec<&str> = actions.split(';').collect();
    let slot = match rule.action.as_str() {
        "Insert" => 0,
        "Replace" => 1,
        "Delete" => 2,
        "Case" => 3,
        "Number" => 4,
        _ => 5,
    };
    let action = actions.get(slot).copied().unwrap_or(rule.action.as_str());
    // lstrcpynW(buffer, text, n) copies up to n-1 characters; "..." when the original text is longer than limit characters.
    let clip = |value: &str, keep: usize, limit: usize| {
        let mut shown: String = value.chars().take(keep).collect();
        if value.chars().count() > limit {
            shown.push_str("...");
        }
        shown
    };
    let label = |key: &str, fallback: &str| {
        let raw = language.raw(&format!("formAdvRename.{key}")).unwrap_or_else(|| fallback.to_owned());
        let text = raw.rsplit_once("(&").filter(|(_, tail)| tail.chars().count() == 2 && tail.ends_with(')')).map_or(raw.as_str(), |(head, _)| head);
        ndr::msgbox::display_caption(text).0.trim_end_matches([':', '：']).trim().to_owned()
    };
    let del_from = language.text("formAdvRename.ComboBox.cbDelFrom", "Position;Beginning;End;Delimiter");
    let from_labels: Vec<&str> = del_from.split(';').collect();
    let from_label = |index: usize, fallback: &'static str| from_labels.get(index).copied().unwrap_or(fallback);
    let place = || match rule.position.as_str() {
        "Prefix" => label("Caption.rbInsertLocFirst", "Prepend"),
        "Suffix" => label("Caption.rbInsertLocLast", "Append"),
        _ => format!("{}: {}", from_label(0, "Position"), rule.index),
    };
    let detail = match rule.action.as_str() {
        "Insert" => format!("{}, {}", clip(&rule.text, 239, 240), place()),
        "Replace" => {
            let occurrence = match rule.position.as_str() {
                "First" => label("Caption.rbReplaceFirst", "First occurrence"),
                "Last" => label("Caption.rbReplaceLast", "Last occurrence"),
                _ => label("Caption.rbReplaceAll", "Every occurrence"),
            };
            format!("{} -> {}, {occurrence}", clip(&rule.find, 95, 100), clip(&rule.replace, 95, 100))
        }
        "Delete" => {
            let from = match rule.delete_from_mode.as_str() {
                "FromStart" => from_label(1, "Beginning").to_owned(),
                "FromEnd" => from_label(2, "End").to_owned(),
                "Delimiter" => format!("\"{}\"", clip(&rule.find, 99, 100)),
                _ => rule.index.to_string(),
            };
            let to = if rule.delete_to_mode == "Delimiter" && !rule.delete_to_delimiter.is_empty() {
                format!(", {}: {}", label("Caption.rbToDelimiter", "Up to delimiter"), clip(&rule.delete_to_delimiter, 99, 100))
            } else {
                format!(", {}: {}", label("Caption.rbDelCount", "Fixed count"), rule.count)
            };
            let directions = language.text("formAdvRename.ComboBox.cbDelDirection", "Right to left;Left to right");
            let direction = directions.split(';').nth(usize::from(rule.delete_direction != "RightToLeft")).unwrap_or("");
            format!("{}: {from}{to}, {direction}", label("Caption.lDelFrom", "Remove from"))
        }
        "Case" => match rule.case_mode.as_str() {
            "Upper" => label("Caption.rbCaseUpper", "Uppercase"),
            "Lower" => label("Caption.rbCaseLower", "Lowercase"),
            "WordFirst" => label("Caption.rbCaseWordFirst", "Capitalize each word"),
            _ => label("Caption.rbCaseFirstLetter", "Capitalize first character"),
        }
        ,
        "Number" => format!("{}: {}, {}: {}, {}: {}, {}", label("Caption.lNumStart", "Start at"), rule.start, label("Caption.lNumStep", "Increment"), rule.step, label("Caption.lNumDigit", "Minimum digits"), rule.digits, place()),
        _ => format!("{}: {}", label("Caption.lExtText", "New extension"), rule.text),
    };
    let target = if rule.target == "Extension" { label("Caption.rbApplyToExt", "Extension") } else { label("Caption.rbApplyToName", "Base name") };
    format!("{action} ({detail}) - {target}")
}

/// Preview: shows the original name even when a rule fails.
pub(super) fn preview_name(form: &RenameForm, index: usize) -> String {
    let name = form.paths[index].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    compat::rename_with_rules(&name, &form.rules, index, form.dirs[index]).unwrap_or(name)
}

/// Original FUN_00469b64: the new name must be shorter than 0x105 (261) characters with no forbidden characters. A name whose rules fail counts as invalid.
pub(super) fn valid_new_name(form: &RenameForm, index: usize) -> bool {
    let name = form.paths[index].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match compat::rename_with_rules(&name, &form.rules, index, form.dirs[index]) {
        Ok(new_name) => new_name.encode_utf16().count() < 0x105 && !new_name.contains(|c| super::original_dialogs::INVALID_NAME_CHARACTERS.contains(c)),
        Err(_) => false,
    }
}

/// Compact alternative to the original three-column dialog. The existing
/// rule model, commit/confirmation handlers, macro format and preview
/// validation are shared; only the presentation changes.
fn compact_rename_ui(ui: &mut egui::Ui, form: &mut RenameForm, language: &compat::Language) -> Option<usize> {
    use super::original_dialogs::{access_job_colored, compact_button_height, take_dialog_char, vcl_wrapped_button};
    let screen = ui.ctx().content_rect();
    let width = (screen.width() - 40.0).clamp(180.0, 650.0);
    let height = (screen.height() - 54.0).clamp(180.0, 555.0);
    let mut selected_page = form.compact_page.min(2);
    let t = |key: &str, fallback: &str| text(language, key, fallback).replace('&', "");
    let raw = |key: &str, fallback: &str| language.raw(&format!("formAdvRename.{key}")).unwrap_or_else(|| fallback.to_owned());
    let titles = [t("Caption.lAction", "1. Choose Action"), t("Caption.lApplyDest", "2. Apply Action"), t("Caption.lPreview", "3. Preview")];
    let footer = [raw("Caption.btnRun", "&Rename"), t("Caption.btnCancel", "Cancel")];
    let tab_width = (width - 14.0) / 3.0;
    let tab_height = compact_button_height(ui, &titles, tab_width);
    let button_width = (width - 14.0) / 2.0;
    let button_height = compact_button_height(ui, &footer, button_width);
    let (area, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let at = |x, y, w, h| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    for (i, title) in titles.iter().enumerate() {
        let rect = at(3.0 + i as f32 * (tab_width + 4.0), 0.0, tab_width, tab_height);
        if ui.put(rect, egui::Button::new(title).selected(selected_page == i).wrap()).clicked() {
            selected_page = i;
        }
    }
    let previous = form.editor.action.clone();
    let digit_keys = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6];
    if let Some(i) = ui.input(|i| if i.modifiers.alt { digit_keys.iter().position(|k| i.key_pressed(*k)) } else { None }) {
        if let Some((value, _)) = action_choices(language).get(i) {
            form.editor.action = (*value).to_owned();
            selected_page = 0;
        }
    }
    let body = at(3.0, tab_height + 5.0, width - 6.0, (height - tab_height - button_height - 15.0).max(40.0));
    let mut choice = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(body));
        ui.push_id(selected_page, |ui| {
            if selected_page == 2 {
                let headers = t("ListViewCols.lvPreview", "Num;Original Name;New Name");
                let headers: Vec<_> = headers.split(';').collect();
                let preview_width = (body.width() - 13.0).max(100.0);
                let font = egui::FontId::proportional(13.0);
                let shown = form.paths.len().min(500);
                let more = form.paths.len() - shown;
                ui.add(egui::Label::new(format!("{} → {}", headers.get(1).copied().unwrap_or("Original Name"), headers.get(2).copied().unwrap_or("New Name"))).wrap());
                if let Some(row) = form.preview_selected.filter(|r| *r < shown && ui.ctx().memory(|m| m.focused().is_none())) {
                    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::C)) || ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
                        if let Some(name) = form.paths[row].file_name() { ui.ctx().copy_text(name.to_string_lossy().into_owned()); }
                    }
                }
                let remaining = ui.available_height().max(24.0);
                egui::ScrollArea::vertical().id_salt("compact-rename-preview").max_height(remaining).min_scrolled_height(0.0).auto_shrink([false, false]).show_rows(ui, 40.0, shown + usize::from(more > 0), |ui, range| {
                    for row in range {
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(preview_width, 40.0), egui::Sense::click());
                        if row < shown {
                            let old = form.paths[row].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            let new = preview_name(form, row);
                            if response.clicked() { form.preview_selected = Some(row); }
                            if form.preview_selected == Some(row) { ui.painter().rect_filled(rect, 2.0, ui.visuals().selection.bg_fill); }
                            let fg = ui.visuals().text_color();
                            let color = if valid_new_name(form, row) { fg } else { Color32::RED };
                            for (line, value, c) in [(0.0, format!("{}. {}", row + 1, old), fg), (19.0, format!("→ {new}"), color)] {
                                let value = super::original_dialogs::path_ellipsis(ui, &value, rect.width() - 8.0, &font);
                                ui.painter().with_clip_rect(rect).text(rect.min + egui::vec2(4.0, line + 2.0), egui::Align2::LEFT_TOP, value, font.clone(), c);
                            }
                            response.on_hover_text(format!("{old}\n→ {new}"));
                        } else {
                            let notice = super::file_commands::operation_format(language, "NDR.OP_RENAME_MORE_ITEMS", "{0} more items", &[&more]);
                            ui.painter().with_clip_rect(rect).text(rect.min + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, notice, font.clone(), ui.visuals().text_color());
                        }
                    }
                });
                return;
            }
            egui::ScrollArea::vertical().id_salt("compact-rename-body").max_height(body.height()).min_scrolled_height(0.0).max_width(body.width()).auto_shrink([false, false]).show(ui, |ui| {
                let field = |ui: &mut egui::Ui, label: &str, value: &mut String, enabled: bool| {
                    ui.add(egui::Label::new(label).wrap());
                    ui.add_enabled_ui(enabled, |ui| ui.add_sized([ui.available_width(), 23.0], egui::TextEdit::singleline(value)));
                };
                let radio = |ui: &mut egui::Ui, value: &mut String, option: &str, caption: &str| {
                    let (mut job, access) = access_job_colored(ui, caption, None);
                    job.wrap.max_width = (ui.available_width() - 22.0).max(1.0);
                    job.wrap.break_anywhere = true;
                    if ui.add(egui::RadioButton::new(value.as_str() == option, job)).clicked() || (ui.is_enabled() && take_dialog_char(ui.ctx(), access)) {
                        *value = option.to_owned();
                    }
                };
                if selected_page == 0 {
                    let options = action_choices(language);
                    combo(ui, "compact-rename-action", &mut form.editor.action, &options, (ui.available_width() - 8.0).max(80.0));
                    if previous != form.editor.action && !position_choices(language, &form.editor.action).iter().any(|(v, _)| *v == form.editor.position) {
                        form.editor.position = if form.editor.action == "Replace" { "All" } else { "Prefix" }.into();
                    }
                    let action = form.editor.action.clone();
                    let editor = &mut form.editor;
                    let numbers = &mut form.numbers;
                    match action.as_str() {
                        "Insert" | "Number" => {
                            if action == "Insert" {
                                field(ui, &t("Caption.lInsText", "A. Text:"), &mut editor.text, true);
                                ui.label(t("Caption.lInsLoc", "B. Insert Location:"));
                            } else {
                                ui.label(t("Caption.lNumFormat", "A. Format"));
                                field(ui, &t("Caption.lNumStart", "Start:"), &mut numbers[2], true);
                                field(ui, &t("Caption.lNumStep", "Step:"), &mut numbers[3], true);
                                field(ui, &t("Caption.lNumDigit", "Digits:"), &mut numbers[4], true);
                                ui.label(t("Caption.lNumLoc", "B. Location"));
                            }
                            let keys = if action == "Insert" {
                                ["Caption.rbInsertLocPos", "Caption.rbInsertLocFirst", "Caption.rbInsertLocLast"]
                            } else { ["Caption.rbNumLocPos", "Caption.rbNumLocFirst", "Caption.rbNumLocLast"] };
                            for (i, option) in ["Position", "Prefix", "Suffix"].into_iter().enumerate() {
                                radio(ui, &mut editor.position, option, &raw(keys[i], option));
                                if i == 0 && editor.position == "Position" {
                                    ui.add_sized([ui.available_width(), 23.0], egui::TextEdit::singleline(&mut numbers[0]));
                                }
                            }
                        }
                        "Replace" => {
                            field(ui, &t("Caption.lRepFind", "Find:"), &mut editor.find, true);
                            field(ui, &t("Caption.lRepReplace", "Replace with:"), &mut editor.replace, true);
                            for (value, key, label) in [("All", "Caption.rbReplaceAll", "All occurrences"), ("First", "Caption.rbReplaceFirst", "First occurrence"), ("Last", "Caption.rbReplaceLast", "Last occurrence")] {
                                radio(ui, &mut editor.position, value, &raw(key, label));
                            }
                        }
                        "Delete" => {
                            ui.label(t("Caption.lDelFrom", "A. From"));
                            let w = (ui.available_width() - 8.0).max(80.0);
                            if combo(ui, "compact-delete-from", &mut editor.delete_from_mode, &choices(language, "ComboBox.cbDelFrom", "Position;From Start;From End;Delimiter", &["Position", "FromStart", "FromEnd", "Delimiter"]), w) {
                                match editor.delete_from_mode.as_str() { "FromStart" => editor.delete_direction = "LeftToRight".into(), "FromEnd" => editor.delete_direction = "RightToLeft".into(), _ => {} }
                            }
                            match editor.delete_from_mode.as_str() {
                                "Position" => { ui.add_sized([ui.available_width(), 23.0], egui::TextEdit::singleline(&mut numbers[0])); }
                                "Delimiter" => { ui.add_sized([ui.available_width(), 23.0], egui::TextEdit::singleline(&mut editor.find)); }
                                _ => {}
                            }
                            ui.label(t("Caption.lDelDirection", "B. Direction"));
                            combo(ui, "compact-delete-direction", &mut editor.delete_direction, &choices(language, "ComboBox.cbDelDirection", "right-to-left;left-to-right", &["RightToLeft", "LeftToRight"]), w);
                            ui.label(t("Caption.lDelTo", "C. To"));
                            radio(ui, &mut editor.delete_to_mode, "Count", &raw("Caption.rbDelCount", "Count:"));
                            if editor.delete_to_mode == "Count" { field(ui, &t("Caption.lUnit", "character(s)"), &mut numbers[1], true); }
                            radio(ui, &mut editor.delete_to_mode, "Delimiter", &raw("Caption.rbToDelimiter", "Delimiter:"));
                            if editor.delete_to_mode == "Delimiter" { ui.add_sized([ui.available_width(), 23.0], egui::TextEdit::singleline(&mut editor.delete_to_delimiter)); }
                        }
                        "Case" => for (value, key, label) in [("Upper", "Caption.rbCaseUpper", "Upper case"), ("Lower", "Caption.rbCaseLower", "Lower case"), ("WordFirst", "Caption.rbCaseWordFirst", "First of each word uppercase"), ("FirstLetter", "Caption.rbCaseFirstLetter", "First letter upper case")] {
                            radio(ui, &mut editor.case_mode, value, &raw(key, label));
                        },
                        _ => field(ui, &t("Caption.lExtText", "New Extension:"), &mut editor.text, true),
                    }
                    ui.separator();
                    ui.add_enabled_ui(action != "Extension", |ui| {
                        radio(ui, &mut editor.target, "Name", &raw("Caption.rbApplyToName", "Name"));
                        radio(ui, &mut editor.target, "Extension", &raw("Caption.rbApplyToExt", "Extension"));
                    });
                    let caption = raw("Caption.btnAddAction", "&Add >");
                    let bw = ui.available_width();
                    let bh = compact_button_height(ui, std::slice::from_ref(&caption), bw);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(bw, bh), egui::Sense::hover());
                    if vcl_wrapped_button(ui, rect, true, &caption).1 { choice = Some(2); }
                } else {
                    let selected = form.selected_rule.filter(|i| *i < form.rules.len());
                    let captions = [raw("Caption.btnDeleteAction", "< &Delete"), raw("Caption.btnUp", "Up"), raw("Caption.btnDown", "Down")];
                    let bw = (ui.available_width() - 8.0) / 3.0;
                    let bh = compact_button_height(ui, &captions, bw);
                    let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), bh), egui::Sense::hover());
                    for (i, caption) in captions.iter().enumerate() {
                        let enabled = selected.is_some_and(|n| match i { 0 => true, 1 => n > 0, _ => n + 1 < form.rules.len() });
                        let rect = egui::Rect::from_min_size(row.min + egui::vec2(i as f32 * (bw + 4.0), 0.0), egui::vec2(bw, bh));
                        if vcl_wrapped_button(ui, rect, enabled, caption).1 {
                            if let Some(index) = selected {
                                if i == 0 { form.rules.remove(index); form.selected_rule = selection_after_remove(index, form.rules.len()); }
                                else { let next = if i == 1 { index - 1 } else { index + 1 }; form.rules.swap(index, next); form.selected_rule = Some(next); }
                            }
                        }
                    }
                    let custom = text(language, "ComboBox.cbMacroList", "Custom");
                    let shown = form.macro_name.clone().unwrap_or_else(|| custom.clone());
                    let mut picked = None;
                    egui::ComboBox::from_id_salt("compact-rename-macros").width((ui.available_width() - 8.0).max(80.0)).selected_text(shown).show_opaque(ui, |ui| {
                        if ui.selectable_label(form.macro_name.is_none(), &custom).clicked() { picked = Some(None); }
                        for item in &form.macros {
                            if ui.selectable_label(form.macro_name.as_deref() == Some(item.name.as_str()), &item.name).clicked() { picked = Some(Some(item.name.clone())); }
                        }
                    });
                    if let Some(name) = picked {
                        if let Some(found) = name.as_ref().and_then(|n| form.macros.iter().find(|m| m.name.eq_ignore_ascii_case(n))) { form.rules = found.rules.clone(); form.selected_rule = None; }
                        form.macro_name = name;
                    }
                    // The tiny glyphs were ambiguous on a narrow display.
                    // Reuse the localized macro-dialog titles and keep
                    // the established save/overwrite/delete handlers.
                    let captions = [t("Button.btnSaveMacro", "Save Macro"), t("Button.btnDeleteMacro", "Delete Macro")];
                    let hints = [t("Caption.macroSaveTitle", "Save Rename Macro"), t("Caption.macroDeleteTitle", "Delete Rename Macro")];
                    let bw = (ui.available_width() - 4.0) / 2.0;
                    let bh = compact_button_height(ui, &captions, bw);
                    let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), bh), egui::Sense::hover());
                    for (i, caption) in captions.iter().enumerate() {
                        let rect = egui::Rect::from_min_size(row.min + egui::vec2(i as f32 * (bw + 4.0), 0.0), egui::vec2(bw, bh));
                        let enabled = if i == 0 { !form.rules.is_empty() } else { form.macro_name.is_some() };
                        let (response, pressed) = vcl_wrapped_button(ui, rect, enabled, caption);
                        response.on_hover_text(&hints[i]);
                        if pressed { choice = Some(3 + i); }
                    }
                    ui.separator();
                    ui.label(t("Caption.lAppliedList", "Applied Actions"));
                    for (index, rule) in form.rules.iter().enumerate() {
                        let description = rule_display(rule, language);
                        if ui.add(egui::Button::selectable(form.selected_rule == Some(index), &description).wrap()).on_hover_text(&description).clicked() { form.selected_rule = Some(index); }
                    }
                }
            });
        });
    });
    form.compact_page = selected_page;
    if vcl_wrapped_button(ui, at(3.0, height - button_height - 3.0, button_width, button_height), true, &footer[0]).1 { choice = Some(0); }
    if ui.put(at(width - button_width - 3.0, height - button_height - 3.0, button_width, button_height), egui::Button::new(&footer[1]).wrap()).clicked() { choice = Some(1); }
    choice
}

/// Original TfAdvRename (ClientWidth 650 x ClientHeight 555): lAction "1. choose action" (16,17), pcAction (12,45 353x220: tsInsert, tsReplace,
/// tsDelete·tsCase·tsNumbering·tsExtension), lApplyDest "2. 동작 적용"(371,17) rbApplyToName(371,69)·rbApplyToExt(371,97),
/// btnAddAction(371,129 67×25) btnDeleteAction(371,168), lAppliedList(448,45) lbActions(448,64 194×145) btnUp(448,215)·btnDown(549,215) 93×25,
/// cbMacroList(448,248 141×21) sbtnSave(593,248)·sbtnDelete(619,248) 23×22, lPreview "3. 미리보기"(12,277) lvPreview(12,300 630×213:
/// number, original name 250, new name 250), btnRun (409,522), btnCancel (529,522) 114x25. Each page's image (Image1-6) is not drawn.
/// Returns 0 rename, 1 cancel, 2 add action, 3 save macro, 4 delete macro.
pub(super) fn rename_ui(ui: &mut egui::Ui, form: &mut RenameForm, language: &compat::Language, _ko: bool) -> Option<usize> {
    let screen = ui.ctx().content_rect();
    if screen.width() < 690.0 || screen.height() < 610.0 { return compact_rename_ui(ui, form, language); }
    use super::original_dialogs::{put_enabled, put_left};
    let mut choice = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(650.0, 555.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let blue = Color32::from_rgb(0x3a, 0x8a, 0xde);
    let t = |key: &str, fallback: &str| text(language, key, fallback).replace('&', "");
    // VCL Caption as is (with the '&' access key).
    let r = |key: &str, fallback: &str| language.raw(&format!("formAdvRename.{key}")).unwrap_or_else(|| fallback.to_owned());
    for (x, y, key, fallback) in [(16.0, 17.0, "Caption.lAction", "1. Choose Action"), (371.0, 17.0, "Caption.lApplyDest", "2. Apply Action"), (12.0, 277.0, "Caption.lPreview", "3. Preview")] {
        put_left(ui, at(x, y, 260.0, 16.0), egui::Label::new(RichText::new(t(key, fallback)).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())).color(blue)));
    }
    // pcAction tab row and pages.
    let before = form.editor.action.clone();
    // Original TntFormKeyDown (0x418bd0): Alt + '1'-'6' goes to that page (0-5) of pcAction.
    let digit_keys = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6];
    if let Some(page) = ui.input(|i| if i.modifiers.alt { digit_keys.iter().position(|k| i.key_pressed(*k)) } else { None }) {
        if let Some((value, _)) = action_choices(language).get(page) {
            form.editor.action = (*value).to_owned();
        }
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(14.0, 45.0, 351.0, 22.0)).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (value, label) in action_choices(language) {
            if ui.selectable_label(form.editor.action == value, label).clicked() {
                form.editor.action = value.to_owned();
            }
        }
    });
    if before != form.editor.action {
        // UpdateEditor: the position choice stays when the new action has that value, otherwise the default (All for replace, Prefix for the rest).
        let positions = position_choices(language, &form.editor.action);
        if !positions.iter().any(|(v, _)| *v == form.editor.position) {
            form.editor.position = if form.editor.action == "Replace" { "All" } else { "Prefix" }.into();
        }
    }
    ui.painter().rect_stroke(at(12.0, 67.0, 353.0, 198.0), 2.0, stroke, egui::StrokeKind::Inside);
    let page = |x: f32, y: f32, w: f32, h: f32| at(16.0 + x, 69.0 + y, w, h);
    ui.painter().hline(area.min.x + 30.0..=area.min.x + 351.0, area.min.y + 69.0 + 38.0, stroke);
    let action = form.editor.action.clone();
    let editor = &mut form.editor;
    let numbers = &mut form.numbers;
    let radio = |ui: &mut egui::Ui, rect: egui::Rect, value: &mut String, option: &str, label: String| {
        if super::original_dialogs::access_radio(ui, rect, value.as_str() == option, &label) {
            *value = option.to_owned();
        }
    };
    match action.as_str() {
        "Insert" => {
            put_left(ui, page(16.0, 51.0, 300.0, 15.0), egui::Label::new(t("Caption.lInsText", "A. Text:")));
            ui.put(page(48.0, 70.0, 241.0, 21.0), egui::TextEdit::singleline(&mut editor.text));
            put_left(ui, page(18.0, 105.0, 300.0, 15.0), egui::Label::new(t("Caption.lInsLoc", "B. Insert Location:")));
            radio(ui, page(48.0, 126.0, 137.0, 17.0), &mut editor.position, "Position", r("Caption.rbInsertLocPos", "Position"));
            // Original 0x41888c: seInsertPos is enabled only when rbInsertLocPos is chosen.
            put_enabled(ui, page(191.0, 124.0, 64.0, 22.0), editor.position == "Position", egui::TextEdit::singleline(&mut numbers[0]));
            radio(ui, page(48.0, 147.0, 137.0, 17.0), &mut editor.position, "Prefix", r("Caption.rbInsertLocFirst", "Prefix"));
            radio(ui, page(48.0, 168.0, 137.0, 17.0), &mut editor.position, "Suffix", r("Caption.rbInsertLocLast", "Suffix"));
        }
        "Replace" => {
            put_left(ui, page(16.0, 73.0, 118.0, 15.0), egui::Label::new(t("Caption.lRepFind", "Find:")));
            ui.put(page(136.0, 70.0, 183.0, 21.0), egui::TextEdit::singleline(&mut editor.find));
            put_left(ui, page(16.0, 104.0, 118.0, 15.0), egui::Label::new(t("Caption.lRepReplace", "Replace with:")));
            ui.put(page(136.0, 101.0, 183.0, 21.0), egui::TextEdit::singleline(&mut editor.replace));
            radio(ui, page(23.0, 149.0, 85.0, 17.0), &mut editor.position, "All", r("Caption.rbReplaceAll", "All occurrences"));
            radio(ui, page(110.0, 149.0, 111.0, 17.0), &mut editor.position, "First", r("Caption.rbReplaceFirst", "First occurrence"));
            radio(ui, page(223.0, 149.0, 111.0, 17.0), &mut editor.position, "Last", r("Caption.rbReplaceLast", "Last occurrence"));
        }
        "Delete" => {
            put_left(ui, page(16.0, 55.0, 190.0, 15.0), egui::Label::new(t("Caption.lDelFrom", "A. From")));
            let from_before = editor.delete_from_mode.clone();
            ui.scope_builder(egui::UiBuilder::new().max_rect(page(28.0, 74.0, 106.0, 21.0)), |ui| {
                combo(ui, "rename-delete-from", &mut editor.delete_from_mode, &choices(language, "ComboBox.cbDelFrom", "Position;From Start;From End;Delimiter", &["Position", "FromStart", "FromEnd", "Delimiter"]), 98.0);
            });
            // HandleDeleteFromModeChanged: from the start is left -> right, from the end right -> left.
            if from_before != editor.delete_from_mode {
                match editor.delete_from_mode.as_str() {
                    "FromStart" => editor.delete_direction = "LeftToRight".into(),
                    "FromEnd" => editor.delete_direction = "RightToLeft".into(),
                    _ => {}
                }
            }
            match editor.delete_from_mode.as_str() {
                "Position" => {
                    ui.put(page(140.0, 74.0, 61.0, 21.0), egui::TextEdit::singleline(&mut numbers[0]));
                }
                "Delimiter" => {
                    ui.put(page(140.0, 74.0, 61.0, 21.0), egui::TextEdit::singleline(&mut editor.find));
                }
                _ => {}
            }
            put_left(ui, page(214.0, 55.0, 120.0, 15.0), egui::Label::new(t("Caption.lDelDirection", "B. Direction")));
            ui.scope_builder(egui::UiBuilder::new().max_rect(page(224.0, 74.0, 105.0, 21.0)), |ui| {
                combo(ui, "rename-delete-direction", &mut editor.delete_direction, &choices(language, "ComboBox.cbDelDirection", "right-to-left;left-to-right", &["RightToLeft", "LeftToRight"]), 97.0);
            });
            put_left(ui, page(16.0, 110.0, 190.0, 15.0), egui::Label::new(t("Caption.lDelTo", "C. To")));
            radio(ui, page(28.0, 133.0, 120.0, 17.0), &mut editor.delete_to_mode, "Count", r("Caption.rbDelCount", "Count:"));
            // Original 0x418954: seDelCount is enabled only with rbDelCount, eDelToDelimiter only with rbToDelimiter.
            let by_count = editor.delete_to_mode == "Count";
            put_enabled(ui, page(151.0, 130.0, 57.0, 21.0), by_count, egui::TextEdit::singleline(&mut numbers[1]));
            put_left(ui, page(214.0, 133.0, 120.0, 15.0), egui::Label::new(t("Caption.lUnit", "character(s)")));
            radio(ui, page(28.0, 160.0, 120.0, 17.0), &mut editor.delete_to_mode, "Delimiter", r("Caption.rbToDelimiter", "Delimiter:"));
            put_enabled(ui, page(151.0, 158.0, 90.0, 21.0), !by_count, egui::TextEdit::singleline(&mut editor.delete_to_delimiter));
        }
        "Case" => {
            for (top, value, key, fallback) in [
                (71.0, "Upper", "Caption.rbCaseUpper", "Upper case"),
                (95.0, "Lower", "Caption.rbCaseLower", "Lower case"),
                (119.0, "WordFirst", "Caption.rbCaseWordFirst", "First of each word uppercase"),
                (144.0, "FirstLetter", "Caption.rbCaseFirstLetter", "First letter upper case"),
            ] {
                radio(ui, page(16.0, top, 320.0, 17.0), &mut editor.case_mode, value, t(key, fallback));
            }
        }
        "Number" => {
            put_left(ui, page(12.0, 56.0, 76.0, 15.0), egui::Label::new(t("Caption.lNumFormat", "A. Format")));
            put_left(ui, page(90.0, 56.0, 74.0, 15.0), egui::Label::new(t("Caption.lNumStart", "Start:")).truncate());
            ui.put(page(166.0, 53.0, 51.0, 21.0), egui::TextEdit::singleline(&mut numbers[2]));
            put_left(ui, page(223.0, 56.0, 52.0, 15.0), egui::Label::new(t("Caption.lNumStep", "Step:")).truncate());
            ui.put(page(276.0, 53.0, 52.0, 21.0), egui::TextEdit::singleline(&mut numbers[3]));
            put_left(ui, page(90.0, 88.0, 74.0, 15.0), egui::Label::new(t("Caption.lNumDigit", "Digits:")).truncate());
            ui.put(page(166.0, 85.0, 51.0, 21.0), egui::TextEdit::singleline(&mut numbers[4]));
            put_left(ui, page(12.0, 124.0, 90.0, 15.0), egui::Label::new(t("Caption.lNumLoc", "B. Location")));
            radio(ui, page(104.0, 123.0, 149.0, 17.0), &mut editor.position, "Position", r("Caption.rbNumLocPos", "Position"));
            // Original 0x4189a8: seNumLocPos is enabled only when rbNumLocPos is chosen.
            put_enabled(ui, page(259.0, 121.0, 64.0, 21.0), editor.position == "Position", egui::TextEdit::singleline(&mut numbers[0]));
            radio(ui, page(104.0, 144.0, 149.0, 17.0), &mut editor.position, "Prefix", r("Caption.rbNumLocFirst", "Prefix"));
            radio(ui, page(104.0, 164.0, 149.0, 17.0), &mut editor.position, "Suffix", r("Caption.rbNumLocLast", "Suffix"));
        }
        _ => {
            put_left(ui, page(16.0, 77.0, 300.0, 15.0), egui::Label::new(t("Caption.lExtText", "New Extension:")));
            ui.put(page(56.0, 96.0, 217.0, 21.0), egui::TextEdit::singleline(&mut editor.text));
        }
    }
    // 2. Apply to (not used by change extension).
    ui.add_enabled_ui(action != "Extension", |ui| {
        radio(ui, at(371.0, 69.0, 75.0, 17.0), &mut editor.target, "Name", r("Caption.rbApplyToName", "Name"));
        radio(ui, at(371.0, 97.0, 75.0, 17.0), &mut editor.target, "Extension", r("Caption.rbApplyToExt", "Extension"));
    });
    if super::original_dialogs::access_button(ui, at(371.0, 129.0, 67.0, 25.0), &r("Caption.btnAddAction", "&Add >")) {
        choice = Some(2);
    }
    let selected = form.selected_rule.filter(|i| *i < form.rules.len());
    if super::original_dialogs::vcl_button(ui, at(371.0, 168.0, 67.0, 25.0), selected.is_some(), &r("Caption.btnDeleteAction", "< &Delete")).1 {
        if let Some(index) = selected {
            form.rules.remove(index);
            // Original btnDeleteActionClick (0x4187bc): after removing, select the item now at that index, or the new last one when the last was removed.
            // Fixed (match): NDR used to clear the selection, so the next Delete or Up/Down needed another click.
            form.selected_rule = selection_after_remove(index, form.rules.len());
        }
    }
    put_left(ui, at(448.0, 45.0, 194.0, 16.0), egui::Label::new(t("Caption.lAppliedList", "Applied Actions")));
    let list = at(448.0, 64.0, 194.0, 145.0);
    ui.painter().rect(list, 0.0, ui.visuals().extreme_bg_color, stroke, egui::StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink(2.0)), |ui| {
        egui::ScrollArea::vertical().id_salt("rename-rules").auto_shrink([false, false]).show(ui, |ui| {
            for (index, rule) in form.rules.iter().enumerate() {
                let description = rule_display(rule, language);
                if ui.add(egui::Button::selectable(form.selected_rule == Some(index), &description).wrap()).on_hover_text(&description).clicked() {
                    form.selected_rule = Some(index);
                }
            }
        });
    });
    for (left, offset, key, fallback) in [(448.0, -1isize, "Caption.btnUp", "Up"), (549.0, 1, "Caption.btnDown", "Down")] {
        let movable = selected.is_some_and(|i| { let next = i as isize + offset; next >= 0 && (next as usize) < form.rules.len() });
        if super::original_dialogs::vcl_button(ui, at(left, 215.0, 93.0, 25.0), movable, &r(key, fallback)).1 {
            if let Some(index) = selected {
                let next = (index as isize + offset) as usize;
                form.rules.swap(index, next);
                form.selected_rule = Some(next);
            }
        }
    }
    let custom = text(language, "ComboBox.cbMacroList", "Custom");
    let shown = form.macro_name.clone().unwrap_or_else(|| custom.clone());
    let mut picked: Option<Option<String>> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(448.0, 248.0, 141.0, 21.0)), |ui| {
        egui::ComboBox::from_id_salt("rename-macros").width(133.0).selected_text(shown).show_opaque(ui, |ui| {
            if ui.selectable_label(form.macro_name.is_none(), &custom).clicked() {
                picked = Some(None);
            }
            for item in &form.macros {
                if ui.selectable_label(form.macro_name.as_deref() == Some(item.name.as_str()), &item.name).clicked() {
                    picked = Some(Some(item.name.clone()));
                }
            }
        });
    });
    if let Some(name) = picked {
        // HandleMacroSelection: choosing a macro replaces the actions with its actions.
        // Original cbMacroListChange (0x419644): with ItemIndex >= 1 the rules are cleared (FUN_0046cd94), the section's lines read (FUN_00417718)
        // and each line of 4+ characters goes to the reader named by its first three characters (lstrcmpW "ins"/"rep"/"del"/"cas"/"num"/"ext");
        // the first item changes nothing.
        if let Some(found) = name.as_ref().and_then(|n| form.macros.iter().find(|m| m.name.eq_ignore_ascii_case(n))) {
            form.rules = found.rules.clone();
            form.selected_rule = None;
        }
        form.macro_name = name;
    }
    // Original sbtnSave and sbtnDelete (TSpeedButton 23x22, Glyph.Data 16x16 bitmaps, no Hint). sbtnDeleteClick (0x419420) is always enabled and
    // does nothing without a chosen macro (cbMacroList.ItemIndex < 1).
    let ctx = ui.ctx().clone();
    let glyph = |name: &str, bmp: &'static [u8]| glyph_texture(&ctx, name, bmp).map(|t| egui::Image::new(&t).fit_to_exact_size(egui::vec2(16.0, 16.0)));
    let save = glyph("advrename-save", SAVE_GLYPH).map_or_else(|| egui::Button::new("S"), egui::Button::image);
    if ui.put(at(593.0, 248.0, 23.0, 22.0), save).on_hover_text(t("Caption.macroSaveTitle", "Save Rename Macro")).clicked() {
        choice = Some(3);
    }
    let delete = glyph("advrename-delete", DELETE_GLYPH).map_or_else(|| egui::Button::new("X"), egui::Button::image);
    if ui.put(at(619.0, 248.0, 23.0, 22.0), delete).on_hover_text(t("Caption.macroDeleteTitle", "Delete Rename Macro")).clicked() {
        choice = Some(4);
    }
    // lvPreview (number, original name, new name, row lines).
    let headers = text(language, "ListViewCols.lvPreview", "Num;Original Name;New Name");
    let headers: Vec<&str> = headers.split(';').collect();
    let preview = at(12.0, 300.0, 630.0, 213.0);
    ui.painter().rect(preview, 0.0, ui.visuals().extreme_bg_color, stroke, egui::StrokeKind::Inside);
    let widths = [50.0, 250.0, 250.0];
    let font = egui::FontId::proportional(13.0);
    let header = egui::Rect::from_min_size(preview.min + egui::vec2(2.0, 2.0), egui::vec2(626.0, 20.0));
    let mut x = header.left();
    for (index, fallback) in ["Num", "Original Name", "New Name"].iter().enumerate() {
        let cell = egui::Rect::from_min_size(egui::pos2(x, header.top()), egui::vec2(widths[index], 20.0));
        ui.painter().rect_filled(cell.shrink2(egui::vec2(0.5, 0.0)), 0.0, ui.visuals().widgets.inactive.weak_bg_fill);
        ui.painter().with_clip_rect(cell.shrink2(egui::vec2(4.0, 0.0))).text(cell.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, headers.get(index).unwrap_or(fallback), font.clone(), ui.visuals().strong_text_color());
        x += widths[index];
    }
    let shown = form.paths.len().min(500);
    let more = form.paths.len() - shown;
    let rows = egui::Rect::from_min_max(egui::pos2(preview.left() + 2.0, header.bottom()), preview.max - egui::vec2(2.0, 2.0));
    let text_color = ui.visuals().text_color();
    // Original lvPreviewKeyDown (0x419a4c): Ctrl + C copies the chosen row's SubItems[0] (original name) to the clipboard (a focused text field copies its own text).
    if let Some(row) = form.preview_selected.filter(|r| *r < shown && ui.ctx().memory(|m| m.focused().is_none())) {
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::C)) || ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
            if let Some(name) = form.paths[row].file_name() {
                ui.ctx().copy_text(name.to_string_lossy().into_owned());
            }
        }
    }
    let selection = ui.visuals().selection.bg_fill;
    let mut clicked_row = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rows), |ui| {
        egui::ScrollArea::vertical().id_salt("rename-preview").auto_shrink([false, false]).show_rows(ui, 18.0, shown + usize::from(more > 0), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in range {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(626.0, 18.0), egui::Sense::click());
                if response.clicked() && row < shown {
                    clicked_row = Some(row);
                }
                if form.preview_selected == Some(row) {
                    ui.painter().rect_filled(rect, 0.0, selection);
                }
                let cells = if row < shown {
                    [(row + 1).to_string(), form.paths[row].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), preview_name(form, row)]
                } else {
                    [String::new(), super::file_commands::operation_format(language, "NDR.OP_RENAME_MORE_ITEMS", "{0} more items", &[&more]), "…".into()]
                };
                let mut x = rect.left();
                // Original lvPreviewAdvancedCustomDrawSubItem (FUN_00418b04): the new name cell (SubItem 2) turns red (clRed) when it fails FUN_00469b64 (length < 0x105 and no forbidden characters
                // "\/:*?\"<>|"). Fixed: previously all rows had the same color, so no row matched the REN_INVALIDNAMES notice.
                let invalid = row < shown && !valid_new_name(form, row);
                for (index, cell_text) in cells.iter().enumerate() {
                    let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(widths[index], 18.0));
                    let text_color = if index == 2 && invalid { Color32::from_rgb(0xFF, 0, 0) } else { text_color };
                    ui.painter().with_clip_rect(cell.shrink2(egui::vec2(4.0, 0.0))).text(cell.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, cell_text, font.clone(), text_color);
                    ui.painter().vline(cell.right(), cell.y_range(), stroke);
                    x += widths[index];
                }
                ui.painter().hline(rect.x_range(), rect.bottom(), stroke);
            }
        });
    });
    if clicked_row.is_some() {
        form.preview_selected = clicked_row;
    }
    if super::original_dialogs::access_button(ui, at(409.0, 522.0, 114.0, 25.0), &r("Caption.btnRun", "&Rename")) {
        choice = Some(0);
    }
    if ui.put(at(529.0, 522.0, 114.0, 25.0), egui::Button::new(t("Caption.btnCancel", "Cancel"))).clicked() {
        choice = Some(1);
    }
    choice
}
/// Original btnDeleteActionClick (0x4187bc) selection after removing the action at `index` (`remaining` actions left):
/// the item now at that index, the new last item when the last was removed, nothing when the list is empty.
pub(super) fn selection_after_remove(index: usize, remaining: usize) -> Option<usize> {
    (remaining > 0).then(|| index.min(remaining - 1))
}
/// Checks of C# BuildPlan: overlapping results, unusable names, existing destinations. On success the list of names to change.
pub(super) fn build_plan(form: &RenameForm, language: &compat::Language) -> Result<Vec<String>, String> {
    let mut raw: Vec<String> = Vec::new();
    for (i, path) in form.paths.iter().enumerate() {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        raw.push(match compat::rename_with_rules(&name, &form.rules, i, form.dirs[i]) {
            Ok(value) => value,
            Err(error) => return Err(super::file_commands::operation_format(language, "NDR.OP_RENAME_INVALID_RESULT", "The resulting name cannot be used: {0} ({1})", &[&name, &error])),
        });
    }
    let changed: Vec<usize> = (0..raw.len()).filter(|i| form.paths[*i].file_name().map(|n| n.to_string_lossy() != raw[*i].as_str()).unwrap_or(true)).collect();
    let key = |i: usize| form.paths[i].parent().map(|p| p.join(&raw[i])).unwrap_or_default().to_string_lossy().to_lowercase();
    let mut duplicates = Vec::new();
    for (position, a) in changed.iter().enumerate() {
        if changed[position + 1..].iter().any(|b| key(*a) == key(*b)) && !duplicates.contains(&raw[*a]) {
            duplicates.push(raw[*a].clone());
        }
    }
    if !duplicates.is_empty() {
        let list = duplicates.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
        return Err(super::file_commands::operation_format(language, "NDR.OP_RENAME_DUPLICATE_RESULT", "Two or more items would get the same name: {0}", &[&list]));
    }
    let sources: Vec<String> = changed.iter().map(|i| form.paths[*i].to_string_lossy().to_lowercase()).collect();
    for i in &changed {
        let target = form.paths[*i].parent().map(|p| p.join(&raw[*i])).unwrap_or_default();
        if target.exists() && !sources.contains(&target.to_string_lossy().to_lowercase()) {
            return Err(super::file_commands::operation_format(language, "NDR.OP_RENAME_EXISTS", "The target name is already in use: {0}", &[&target.display()]));
        }
    }
    Ok(raw)
}

impl App {
    fn rename_title(&self) -> String {
        self.language_pack.text("formAdvRename.Form.Caption", "Advanced Rename")
    }

    fn rename_form(&mut self) -> Option<&mut RenameForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::AdvancedRename(form)) => Some(form),
            _ => None,
        }
    }

    /// C# ShowAdvancedRenameAsync: opens the window with the chosen items (otherwise the cursor item). Macros are read from advrename.save.
    pub(super) fn advanced_rename(&mut self) {
        let i = self.active;
        if self.panes[i].archive.is_some() {
            return self.warn_operation(&self.language_pack.text("NDR.OP_RENAME_ADVANCED_RENAME_IS_NOT_AVAILABLE_INSIDE_AN_ARCHIVE_OR", "Advanced rename is not available inside an archive or a remote folder."));
        }
        let paths = self.command_targets(i);
        if paths.is_empty() {
            return;
        }
        let title = self.rename_title();
        let configured: Vec<RenameMacro> = self.settings.values.get("AdvancedRenameMacros").and_then(|v| serde_json::from_value::<Vec<compat::RenamePreset>>(v.clone()).ok()).unwrap_or_default().into_iter().map(|p| RenameMacro { name: p.name, rules: p.rules, legacy_rows: vec![] }).collect();
        let executable = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
        let loaded = match macros::load(&self.profile, &executable, configured.clone()) {
            Ok(list) => list,
            Err(error) => {
                self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                configured
            }
        };
        let dirs = paths.iter().map(|p| p.is_dir()).collect();
        // Original DFM: pcAction.ActivePage = tsInsert, rbInsertLocPos.Checked (going to the replace page, UpdateEditor resets to the rbReplaceAll default).
        let editor = RenameRule { action: "Insert".into(), position: "Position".into(), ..RenameRule::default() };
        let form = RenameForm { paths, dirs, rules: vec![], selected_rule: None, editor, numbers: ["1".into(), "0".into(), "1".into(), "1".into(), "1".into()], macros: loaded, macro_name: None, preview_selected: None, compact_page: 0 };
        self.push_original_dialog(title, DialogKind::AdvancedRename(Box::new(form)), move |app, result, _| {
            if let DialogResult::AdvancedRename(true) = result {
                app.refresh(i);
            }
        });
    }

    /// AddRule: checks integers and ranges, then builds the action from the current input.
    pub(super) fn add_rename_rule(&mut self) {
        let title = self.rename_title();
        let range = self.language_pack.text("formAdvRename.Caption.numericRangeError", "Position: 1-255, count: up to 255, start: up to 999999, step: -999999 to 999999, digits: 1-250.");
        let Some(form) = self.rename_form() else { return };
        let parsed: Vec<Option<i64>> = form.numbers.iter().map(|v| v.trim().parse::<i32>().ok().map(i64::from)).collect();
        let [Some(index), Some(count), Some(start), Some(step), Some(digits)] = parsed[..] else {
            let message = self.language_pack.text("NDR.OP_RENAME_WHOLE_NUMBERS", "Enter whole numbers for positions and numbering.");
            return self.system_box(&title, &message, MB_OK | ndr::msgbox::MB_ICONASTERISK, |_, _, _| {});
        };
        if !(1..=255).contains(&index) || !(0..=255).contains(&count) || start > 999_999 || !(-999_999..=999_999).contains(&step) || !(1..=250).contains(&digits) {
            return self.system_box(&title, &range, MB_OK | ndr::msgbox::MB_ICONASTERISK, |_, _, _| {});
        }
        let mut rule = form.editor.clone();
        (rule.index, rule.count, rule.start, rule.step, rule.digits) = (index, count, start, step, digits as usize);
        form.rules.push(rule);
        form.compact_page = 1;
    }

    /// Original sbtnSaveClick (0x418c7c): TfGetText(window title, RES NAME "Name", empty text) first; then, with no rules, nothing happens; a
    /// section "[name]" already in advrename.save (TStringList.IndexOf, case-insensitive) asks TfMsgBox(window title, MSG_SAMENAME, yes/no)
    /// and replaces its lines, otherwise the section is appended; the file is saved and the macro combo selects the name.
    /// Fixed: an empty name stored a nameless "[]" section; it is ignored. (The 64-character limit and the notices of the C# port are gone.)
    pub(super) fn save_rename_macro(&mut self) {
        let title = self.rename_title();
        if self.rename_form().is_none() {
            return;
        }
        let prompt = self.language_pack.text("RES.NAME", "Name");
        self.prompt(&title, &prompt, "", None, false, move |app, value, _| {
            let Some(name) = value.map(|v| v.trim().to_owned()) else { return };
            if name.is_empty() || app.rename_form().is_none_or(|f| f.rules.is_empty()) {
                return;
            }
            let lower = name.to_lowercase();
            let exists = app.rename_form().is_some_and(|f| f.macros.iter().any(|m| m.name.to_lowercase() == lower));
            if exists {
                let title = app.rename_title();
                let text = app.language_pack.text("RES.MSG_SAMENAME", "Same name already exists. Do you want to replace it?");
                app.message_box(&title, "", &text, MB_YESNO | MB_ICONQUESTION, move |app, result, _| {
                    if result == IDYES {
                        app.store_rename_macro(name);
                    }
                });
            } else {
                app.store_rename_macro(name);
            }
        });
    }

    fn store_rename_macro(&mut self, name: String) {
        let path = self.profile.join(macros::FILE_NAME);
        let title = self.language_pack.text("formAdvRename.Caption.macroSaveTitle", "Save Rename Macro");
        let Some(form) = self.rename_form() else { return };
        let mut updated = form.macros.clone();
        let rules = form.rules.clone();
        match updated.iter_mut().find(|m| m.name.to_lowercase() == name.to_lowercase()) {
            Some(existing) => {
                existing.rules = rules;
                existing.legacy_rows.clear();
            }
            None => updated.push(RenameMacro { name: name.clone(), rules, legacy_rows: vec![] }),
        }
        // C# SaveSelectedMacro: warns macroFormatUnsupported(value) when not CanSave, and macroSettingsSyncFailed when the settings save fails after the file save.
        if let Err(value) = macros::can_save(&updated) {
            let text = self.language_pack.text("formAdvRename.Caption.macroFormatUnsupported", "NexusFile macros cannot store semicolons or line breaks in action values: {0}").replace("{0}", &value);
            return self.system_box(&title, &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
        match macros::save(&path, &updated) {
            Ok(()) => {
                self.sync_rename_macros(&updated, &title);
                if let Some(form) = self.rename_form() {
                    form.macros = updated;
                    form.macro_name = Some(name);
                }
            }
            Err(error) => self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        }
    }

    /// DeleteSelectedMacro: deletes after TfMsgBox(window title, MSG_DELETEITEM, yes/no, question).
    pub(super) fn delete_rename_macro(&mut self) {
        let title = self.rename_title();
        let Some(name) = self.rename_form().and_then(|f| f.macro_name.clone()) else { return };
        let text = self.language_pack.text("RES.MSG_DELETEITEM", "Are you sure you want to delete a selected item?");
        self.message_box(&title, "", &text, MB_YESNO | MB_ICONQUESTION, move |app, result, _| {
            if result != IDYES {
                return;
            }
            let path = app.profile.join(macros::FILE_NAME);
            let Some(form) = app.rename_form() else { return };
            let updated: Vec<RenameMacro> = form.macros.iter().filter(|m| !m.name.eq_ignore_ascii_case(&name)).cloned().collect();
            let title = app.language_pack.text("formAdvRename.Caption.macroDeleteTitle", "Delete Rename Macro");
            // C# DeleteSelectedMacro: warns macroFormatUnsupported when the remaining macros cannot be written in the original format.
            if let Err(value) = macros::can_save(&updated) {
                let text = app.language_pack.text("formAdvRename.Caption.macroFormatUnsupported", "NexusFile macros cannot store semicolons or line breaks in action values: {0}").replace("{0}", &value);
                return app.system_box(&title, &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
            match macros::save(&path, &updated) {
                Ok(()) => {
                    app.sync_rename_macros(&updated, &title);
                    if let Some(form) = app.rename_form() {
                        form.macros = updated;
                        form.macro_name = None;
                    }
                }
                Err(error) => {
                    app.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                }
            }
        });
    }

    /// Keeps the settings copy (AdvancedRenameMacros) in sync and saves. A failed save warns macroSettingsSyncFailed(error) like the C# version (title = the save/delete window title).
    fn sync_rename_macros(&mut self, list: &[RenameMacro], title: &str) {
        let presets: Vec<compat::RenamePreset> = list.iter().map(|m| compat::RenamePreset { name: m.name.clone(), rules: m.rules.clone() }).collect();
        self.settings.values.insert("AdvancedRenameMacros".into(), serde_json::to_value(presets).unwrap_or_default());
        if let Err(error) = self.try_save() {
            let text = self.language_pack.text("formAdvRename.Caption.macroSettingsSyncFailed", "The macro file was updated, but the settings copy could not be updated: {0}").replace("{0}", &error);
            self.system_box(title, &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
    }

    /// Commit: a notice without actions, plan checks, a yes/no box showing the first 12, and on yes renames and closes.
    pub(super) fn commit_advanced_rename(&mut self) {
        let language = self.language_pack.clone();
        let title = self.rename_title();
        let Some(form) = self.rename_form() else { return };
        if form.rules.is_empty() {
            let message = language.text("NDR.OP_RENAME_NO_ACTIONS", "Add at least one rename action.");
            return self.system_box(&title, &message, MB_OK, |_, _, _| {});
        }
        // Original FUN_0046d000: if any new name is unusable (FUN_00469b64), only MessageBoxW(REN_INVALIDNAMES, REN_TITLE,
        // MB_ICONASTERISK) shows and nothing is renamed.
        let invalid = form.paths.iter().enumerate().any(|(i, path)| {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            compat::rename_with_rules(&name, &form.rules, i, form.dirs[i]).is_err()
        });
        if invalid {
            let caption = self.language_pack.text("RES.REN_TITLE", "NexusFile Rename");
            let text = self.language_pack.text("RES.REN_INVALIDNAMES", "There are invalid file names.\nFile names with red color are invalid names. Please, correct names then try again.");
            return self.system_box(&caption, &text, MB_OK | ndr::msgbox::MB_ICONASTERISK, |_, _, _| {});
        }
        let plan = match build_plan(form, &language) {
            Ok(plan) => plan,
            Err(error) => return self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        };
        let paths = form.paths.clone();
        let mut display: Vec<String> = paths.iter().zip(&plan).take(12).map(|(p, n)| format!("{}  →  {n}", p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())).collect();
        if plan.len() > 12 {
            display.push(super::file_commands::operation_format(&language, "NDR.OP_RENAME_MORE_ITEMS", "{0} more items", &[&(plan.len() - 12)]));
        }
        let text = super::file_commands::operation_format(&language, "NDR.OP_RENAME_CONFIRM", "{0}\n\nRename these items?", &[&display.join("\r\n")]);
        self.system_box(&title.clone(), &text, MB_YESNO | MB_ICONQUESTION, move |app, result, ctx| {
            if result != IDYES {
                return;
            }
            match compat::apply_renames(&paths, &plan) {
                Ok(()) => app.answer_dialog(DialogResult::AdvancedRename(true), ctx),
                Err(error) => app.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
            }
        });
    }
}


#[cfg(test)]
mod glyph_tests {
    use super::*;

    /// Original btnDeleteActionClick (0x4187bc): the item now at the removed index, the new last item, or nothing when the list is empty.
    #[test]
    fn deleting_an_action_selects_the_next_one() {
        assert_eq!(selection_after_remove(1, 3), Some(1), "the next item moves into the removed slot");
        assert_eq!(selection_after_remove(3, 3), Some(2), "the last one was removed");
        assert_eq!(selection_after_remove(0, 0), None, "nothing left");
    }

    /// NDR action glyphs retain the 16px sizing and bitmap transparency convention.
    #[test]
    fn macro_buttons_use_distinct_ndr_glyphs() {
        assert_ne!(SAVE_GLYPH, DELETE_GLYPH);
        for bmp in [SAVE_GLYPH, DELETE_GLYPH] {
            let image = super::super::original_dialogs::decode_glyph(bmp).expect("decodable BMP");
            assert_eq!(image.size, [16, 16]);
            assert_eq!(image.pixels[15 * 16], Color32::TRANSPARENT, "bottom-left is the transparent colour");
            let drawn = image.pixels.iter().filter(|p| **p != Color32::TRANSPARENT).count();
            assert!(drawn > 30 && drawn < 256, "{drawn} drawn pixels");
        }
        let delete = super::super::original_dialogs::decode_glyph(DELETE_GLYPH).unwrap();
        assert!(delete.pixels.iter().any(|p| p.r() > 0xC0 && p.g() < 0x60 && p.b() < 0x80), "the red delete icon");
    }
}
