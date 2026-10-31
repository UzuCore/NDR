//! C# FunctionKeyEditorWindow (function keys, 900x560: F1-F12 table with key/name/kind/command/arguments, edit and reset, OK and cancel) and
//! FunctionKeyEditWindow ("edit function key - Fn", 660x430: name, internal/user command, command, arguments) + FunctionKeyBindingService.Get/InternalCommands.
//! Internal command names are shown by reverse lookup of language file values (`Language::translate_pair`) like C# TranslateText.
use super::grid::GridState;
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{MB_ICONASTERISK, MB_OK};
use serde_json::{Map, Value, json};

/// FunctionKeyBindingService.InternalCommands (Id, name).
pub(super) const INTERNAL_COMMANDS: &[(&str, &str)] = &[
    ("#None", "None"), ("#Open", "Open"), ("#OpenWithParam", "Open with parameters"), ("#CopyTo", "Copy To"), ("#MoveTo", "Move To"),
    ("#CopyToLeft", "Copy to left"), ("#CopyToRight", "Copy to right"), ("#MoveToLeft", "Move to left"), ("#MoveToRight", "Move to right"),
    ("#Delete", "Delete"), ("#RecycleBin", "Send to RecycleBin"), ("#Shred", "Shred"), ("#Rename", "Rename"), ("#AdvRename", "Advanced Rename"),
    ("#ChangeAttr", "Change Attributes"), ("#Properties", "Properties"), ("#EditDescription", "Add/Edit Comment"), ("#Favorites", "Favorites"),
    ("#NewFolder", "New Folder"), ("#Refresh", "Refresh"), ("#GoToDesktop", "Go to Desktop"), ("#GoToDocuments", "Go to Documents"),
    ("#GoToMusic", "Go to Music"), ("#GoToPictures", "Go to Pictures"), ("#GoToVideos", "Go to Videos"), ("#GoToDownloads", "Go to Downloads"),
    ("#View", "View"), ("#Edit", "Edit"), ("#SelectAll", "Select All"), ("#AddToArchive", "Add to Archive"), ("#Extract", "Extract"),
    ("#AutoExtract", "Auto Extract"), ("#CloseTab", "Close Tab"), ("#SelectAllWithFolder", "Select All(with Folders)"), ("#FindNext", "Find Next"),
    ("#ExchangePath", "Exchange Path"), ("#ShowPathHistory", "Path History"), ("#FocusAddressBar", "Focus Address Bar"), ("#NetworkDialog", "Network Dialog"),
    ("#SelectSameName", "Select Same Name"), ("#SelectSameExt", "Select Same Ext"), ("#SelectCurrAndMove", "Select and Move Cursor"), ("#Select", "Select"),
    ("#DeselectAll", "Deselect All"), ("#FTPSiteManager", "FTP Site Manager"), ("#Back", "Back"), ("#Forward", "Forward"),
    ("#OpenAllSelected", "Open All Selected Files"), ("#FolderTree", "Folder Tree"), ("#GoToRoot", "Go to Root"), ("#GoToParent", "Go to Parent"),
    ("#GoToLastDest", "Go to last destination"), ("#EditCopy", "Copy"), ("#EditCut", "Cut"), ("#EditPaste", "Paste"), ("#InvertSelection", "Invert Selection"),
    ("#NewTab", "New Tab"), ("#CommandLine", "Command Line"), ("#SplitFile", "Split File"), ("#JoinFiles", "Join Files"), ("#OpenRecycleBin", "Open Recycle Bin"),
];

/// C# FunctionKeyBinding (Normalize = trim leading and trailing spaces).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Binding {
    pub name: String,
    pub command: String,
    pub parameters: String,
    pub internal: bool,
}

impl Binding {
    fn normalized(&self) -> Self {
        Self { name: self.name.trim().into(), command: self.command.trim().into(), parameters: self.parameters.trim().into(), internal: self.internal }
    }
    fn to_json(&self) -> Value {
        json!({"Name": self.name, "Command": self.command, "Parameters": self.parameters, "IsInternalCommand": self.internal})
    }
}

/// FunctionKeyBindingService.Get: FunctionKeyBindings[key] (normalized), otherwise the old FunctionKeys[key] (an internal command when it starts with #).
pub(super) fn binding_for(settings: &ndr::settings::Settings, key: &str) -> Option<Binding> {
    let find = |name: &str| settings.values.get(name).and_then(Value::as_object).and_then(|m| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone()));
    if let Some(value) = find("FunctionKeyBindings") {
        let text = |field: &str| value.get(field).and_then(Value::as_str).unwrap_or("").to_owned();
        return Some(Binding { name: text("Name"), command: text("Command"), parameters: text("Parameters"), internal: value.get("IsInternalCommand").and_then(Value::as_bool).unwrap_or(false) }.normalized());
    }
    let legacy = find("FunctionKeys").and_then(|v| v.as_str().map(str::to_owned)).filter(|v| !v.trim().is_empty())?;
    let command = legacy.trim().to_owned();
    Some(Binding { name: key.into(), internal: command.starts_with('#'), command, parameters: String::new() })
}

pub(super) struct FunctionKeyEditorForm {
    ko: bool,
    /// Current settings of the listed keys (None = default).
    pub bindings: Vec<(String, Option<Binding>)>,
    /// Bindings of keys the original list does not show (F1, F9-F12 from older NDR settings), kept unchanged when saving.
    hidden: Vec<(String, Binding)>,
    pub grid: GridState,
}

impl FunctionKeyEditorForm {
    pub(super) fn new(settings: &ndr::settings::Settings, ko: bool) -> Self {
        // Original TfPreferences lvFunctionKey: F2-F8 (the keys of the function key bar), then ^F1-^F11 (Ctrl+F1-F11, [FuncKey] "^F%d").
        // Previously the C# F1-F12.
        let keys = (2..=8).map(|n| format!("F{n}")).chain((1..=11).map(|n| format!("^F{n}")));
        let bindings: Vec<(String, Option<Binding>)> = keys.map(|key| { let b = binding_for(settings, &key); (key, b) }).collect();
        let hidden = [1, 9, 10, 11, 12].iter().filter_map(|n| { let key = format!("F{n}"); binding_for(settings, &key).map(|b| (key, b)) }).collect();
        let mut grid = GridState::new(bindings.len());
        grid.click(0, egui::Modifiers::NONE, false);
        Self { ko, bindings, hidden, grid }
    }

    /// CreateRow: (key, name, kind, command, arguments). An unassigned F2-F8 shows its original default command (default\nexusfile.ini [FuncKey],
    /// "#Rename" ... "#RecycleBin") with no name, as the original list does.
    pub(super) fn row(&self, index: usize) -> [String; 5] {
        let (key, binding) = &self.bindings[index];
        match binding {
            None => [key.clone(), String::new(), if self.ko { "기본" } else { "Default" }.into(), super::user_commands::original_default_function_key(key).to_owned(), String::new()],
            Some(b) => [
                key.clone(),
                b.name.clone(),
                if b.internal { if self.ko { "내부 명령" } else { "Internal" } } else if self.ko { "사용자 명령" } else { "User command" }.into(),
                b.command.clone(),
                b.parameters.clone(),
            ],
        }
    }

    /// Commit: only configured entries, normalized, as FunctionKeyBindings values (with the unlisted keys carried over).
    pub(super) fn result(&self) -> Value {
        let listed = self.bindings.iter().filter_map(|(key, b)| b.as_ref().map(|b| (key.clone(), b.normalized().to_json())));
        let hidden = self.hidden.iter().map(|(key, b)| (key.clone(), b.normalized().to_json()));
        let map: Map<String, Value> = listed.chain(hidden).collect();
        Value::Object(map)
    }

    pub(super) fn selected(&self) -> Option<usize> {
        self.grid.selected_in_order().first().copied()
    }
}

/// Input of the original TfEditFunction.
pub(super) struct FunctionKeyEditForm {
    pub name: String,
    pub internal: bool,
    pub internal_command: usize,
    pub command: String,
    pub parameters: String,
}

impl FunctionKeyEditForm {
    pub(super) fn new(key: &str, source: &Binding, _ko: bool) -> Self {
        let internal_command = INTERNAL_COMMANDS.iter().position(|(id, _)| id.eq_ignore_ascii_case(&source.command)).unwrap_or(0);
        Self {
            name: if source.name.is_empty() { key.into() } else { source.name.clone() },
            internal: source.internal,
            internal_command,
            command: source.command.clone(),
            parameters: source.parameters.clone(),
        }
    }

    /// Original btnOkClick (0x41f73c): for a user command with an empty name, the command file name (PathFindFileNameW without extension); for an internal command
    /// "#" + the chosen command. The original does not block empty fields (no C# "enter a name" notice). An empty command leaves that key unassigned.
    pub(super) fn result(&self) -> Result<Binding, String> {
        let command = if self.internal { INTERNAL_COMMANDS[self.internal_command].0.to_owned() } else { self.command.trim().trim_matches('"').to_owned() };
        let mut name = self.name.trim().to_owned();
        if name.is_empty() && !self.internal {
            let file = command.rsplit(['\\', '/']).next().unwrap_or("");
            name = match file.rfind('.') {
                Some(dot) if dot > 0 => file[..dot].to_owned(),
                _ => file.to_owned(),
            };
        }
        let parameters = if self.internal { String::new() } else { self.parameters.trim().to_owned() };
        Ok(Binding { name, command, parameters, internal: self.internal })
    }

    /// Original rbCommandTypeClick (0x41f5bc): changing the kind empties the name, command and argument fields, and selects the first command for internal commands.
    pub(super) fn set_internal(&mut self, internal: bool) {
        if self.internal == internal {
            return;
        }
        self.internal = internal;
        self.name.clear();
        self.command.clear();
        self.parameters.clear();
        self.internal_command = 0;
    }
}

/// Item texts of the original TfEditFunction PopupMenu1 (btnParam "...") (as in the DFM, not in the language file).
pub(super) const PARAMETER_MENU: [&str; 18] = [
    "$(SELECTED)    ex) \"c:\\folder\\file1.ext\" \"c:\\folder\\file2.exe\"",
    "$(SELECTED1)",
    "$(SELECTED2)",
    "$(PATH)    ex) c:\\folder\\",
    "$(PATH1)",
    "$(PATH2)",
    "$(DRIVE)    ex) c:\\",
    "$(DRIVE1)",
    "$(DRIVE2)",
    "$(FILENAME)    ex) file1.ext",
    "$(FILENAME1)",
    "$(FILENAME2)",
    "$(NAME)    ex) file1",
    "$(NAME1)",
    "$(NAME2)",
    "$(EXTENSION)    ex) ext",
    "$(EXTENSION1)",
    "$(EXTENSION2)",
];

/// Original ParameterClick (0x41fed0): cuts the item text at the first ")" (Delete after Pos(")")) and inserts it at the eParams cursor (character index).
/// Returns the new cursor after the inserted text (SelStart + length).
pub(super) fn insert_parameter(text: &mut String, caret: usize, caption: &str) -> usize {
    let token = caption.find(')').map_or(caption, |end| &caption[..=end]);
    let caret = caret.min(text.chars().count());
    let byte = text.char_indices().nth(caret).map_or(text.len(), |(index, _)| index);
    text.insert_str(byte, token);
    caret + token.chars().count()
}

/// Original TfEditFunction (ClientWidth 462 x ClientHeight 223): rbUserCommand (20,13) rbInternalCommand (229,13), Bevel (20,36 421),
/// lName(20,60) eName(113,57 141×21), lCommand(20,88) cbCommand(113,85 294×22) btnBrowse(412,85 29×21),
/// lParams (20,115) eParams (113,112 294x21) btnParam (412,112 28x23, PopupMenu1), lParamTip (right edge 407,143),
/// btnOk (285,191, default) btnCancel (366,191) 75x25. For internal commands lParams, eParams, btnBrowse, btnParam and lParamTip are disabled (rbCommandTypeClick).
/// Some(true) on OK, Some(false) on cancel.
pub(super) fn function_key_edit_ui(ui: &mut egui::Ui, form: &mut FunctionKeyEditForm, language: &compat::Language, enter: bool) -> Option<bool> {
    use super::original_dialogs::put_left;
    let text = |key: &str, fallback: &str| language.text(&format!("formEditFunction.{key}"), fallback);
    let raw = |key: &str, fallback: &str| language.raw(&format!("formEditFunction.{key}")).unwrap_or_else(|| fallback.to_owned());
    let mut result = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(462.0, 223.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    if super::original_dialogs::access_radio(ui, at(20.0, 13.0, 189.0, 17.0), !form.internal, &raw("Caption.rbUserCommand", "User Defined Command")) {
        form.set_internal(false);
    }
    if super::original_dialogs::access_radio(ui, at(229.0, 13.0, 188.0, 17.0), form.internal, &raw("Caption.rbInternalCommand", "Internal Command")) {
        form.set_internal(true);
    }
    ui.painter().hline(area.min.x + 20.0..=area.min.x + 441.0, area.min.y + 36.0, ui.visuals().widgets.noninteractive.bg_stroke);
    put_left(ui, at(20.0, 60.0, 92.0, 15.0), egui::Label::new(text("Caption.lName", "Display Name:")).truncate());
    ui.put(at(113.0, 57.0, 141.0, 21.0), egui::TextEdit::singleline(&mut form.name));
    put_left(ui, at(20.0, 88.0, 92.0, 15.0), egui::Label::new(text("Caption.lCommand", "Command:")).truncate());
    if form.internal {
        // C# LocalizeCommand(name) = TranslateText(name): text in the current language by reverse lookup of language file values.
        let names: Vec<String> = INTERNAL_COMMANDS.iter().map(|(_, name)| language.translate_pair(name, name)).collect();
        ui.scope_builder(egui::UiBuilder::new().max_rect(at(113.0, 85.0, 294.0, 22.0)), |ui| {
            egui::ComboBox::from_id_salt("function-key-command").width(286.0).selected_text(names[form.internal_command].as_str()).show_opaque(ui, |ui| {
                for (index, name) in names.iter().enumerate() {
                    ui.selectable_value(&mut form.internal_command, index, name.as_str());
                }
            });
        });
    } else {
        // Original cbCommandChange (0x41fc30): when user command (+0x3e4) is chosen, copies the cbCommand text into eName (+0x3f8) as is.
        if ui.put(at(113.0, 85.0, 294.0, 21.0), egui::TextEdit::singleline(&mut form.command).id(egui::Id::new("ndr-function-key-command"))).changed() {
            form.name = form.command.clone();
        }
    }
    // Original rbCommandTypeClick (0x41f5bc): for internal commands hides lParams (+0x3f4), eParams (+0x400), btnBrowse (+0x40c), btnParam (+0x460) and
    // lParamTip (+0x464) (Visible, not disabled).
    let user = !form.internal;
    if user {
        // Original btnBrowseClick (0x41fca4): Filter = OD_PROGRAMFILTER, initial name = command text without surrounding quotes. The chosen file goes into cbCommand
        // and its OnChange (cbCommandChange) fills the name field too.
        if ui.put(at(412.0, 85.0, 29.0, 21.0), egui::Button::new("...")).clicked() {
            let title = text("Form.Caption", "Edit Function Key");
            let current = form.command.trim().trim_matches('"');
            let initial = Path::new(current).parent().filter(|p| p.is_dir()).map(Path::to_path_buf);
            let file_name = Path::new(current).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let filters = super::original_dialogs::program_filter(language);
            let filters: Vec<(&str, &str)> = filters.iter().map(|(n, p)| (n.as_str(), p.as_str())).collect();
            if let Some(path) = ndr::file_dialogs::open_file(&title, initial.as_deref(), &file_name, &filters) {
                form.command = path.to_string_lossy().into_owned();
                form.name = form.command.clone();
            }
        }
        put_left(ui, at(20.0, 115.0, 92.0, 15.0), egui::Label::new(text("Caption.lParams", "Parameter(s):")).truncate());
        // Original eParams + btnParam ("...", PopupMenu1 -> ParameterClick).
        let id = ui.make_persistent_id("function-key-parameters");
        ui.put(at(113.0, 112.0, 294.0, 21.0), egui::TextEdit::singleline(&mut form.parameters).id(id));
        ui.scope_builder(egui::UiBuilder::new().max_rect(at(412.0, 112.0, 28.0, 23.0)), |ui| {
            ui.menu_button("...", |ui| {
                for caption in PARAMETER_MENU {
                    if ui.button(caption).clicked() {
                        let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                        let caret = state.cursor.char_range().map_or(form.parameters.chars().count(), |range| range.primary.index);
                        let after = insert_parameter(&mut form.parameters, caret, caption);
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(after))));
                        state.store(ui.ctx(), id);
                        ui.close();
                    }
                }
            });
        });
        // Original lParamTip (right aligned, right edge 407).
        ui.scope_builder(egui::UiBuilder::new().max_rect(at(20.0, 143.0, 387.0, 15.0)).layout(egui::Layout::right_to_left(egui::Align::Min)), |ui| {
            ui.add(egui::Label::new(text("Caption.lParamTip", "* $(???) : Active Pane, $(???1) : Left Pane, $(???2) : Right Pane")).truncate());
        });
    }
    if ui.put(at(285.0, 191.0, 75.0, 25.0), egui::Button::new(text("Caption.btnOk", "OK"))).clicked() {
        result = Some(true);
    }
    if ui.put(at(366.0, 191.0, 75.0, 25.0), egui::Button::new(text("Caption.btnCancel", "Cancel"))).clicked() {
        result = Some(false);
    }
    if enter && result.is_none() {
        result = Some(true);
    }
    result
}
impl App {
    pub(super) fn function_key_edit_done(&mut self, ok: bool, ctx: &egui::Context) {
        if !ok {
            return self.answer_dialog(DialogResult::FunctionKeyEdit(None), ctx);
        }
        let Some(DialogKind::FunctionKeyEdit(form)) = self.original_dialogs.last().map(|d| &d.kind) else { return };
        match form.result() {
            Ok(binding) => self.answer_dialog(DialogResult::FunctionKeyEdit(Some(binding)), ctx),
            Err(text) => {
                let title = self.original_dialogs.last().map(|d| d.title.clone()).unwrap_or_default();
                self.system_box(&title, &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
            }
        }
    }
}
