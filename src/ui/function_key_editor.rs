//! C# FunctionKeyEditorWindow(기능키, 900×560: F1~F12 표 — 키/이름/종류/명령/인자, 편집·초기화, 확인·취소)와
//! FunctionKeyEditWindow("기능키 편집 — Fn", 660×430: 이름, 내부 명령/사용자 명령, 명령, 인자) + FunctionKeyBindingService.Get/InternalCommands.
//! 내부 명령 이름은 C# TranslateText처럼 언어 파일 값 역검색(`Language::translate_pair`)으로 보인다.
use super::grid::GridState;
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{MB_ICONASTERISK, MB_OK};
use serde_json::{Map, Value, json};

/// FunctionKeyBindingService.InternalCommands(Id, 이름).
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

/// C# FunctionKeyBinding(Normalize = 앞뒤 공백 뺌).
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

/// FunctionKeyBindingService.Get: FunctionKeyBindings[키](정리), 없으면 옛 FunctionKeys[키](# 시작이면 내부 명령).
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
    /// F1~F12의 지금 설정(None = 기본).
    pub bindings: Vec<(String, Option<Binding>)>,
    pub grid: GridState,
}

impl FunctionKeyEditorForm {
    pub(super) fn new(settings: &ndr::settings::Settings, ko: bool) -> Self {
        // C# F1~F12, 그 뒤에 원본 TfPreferences 목록의 ^F1~^F11(Ctrl+F1~F11, [FuncKey] "^F%d").
        let keys = (1..=12).map(|n| format!("F{n}")).chain((1..=11).map(|n| format!("^F{n}")));
        let bindings: Vec<(String, Option<Binding>)> = keys.map(|key| { let b = binding_for(settings, &key); (key, b) }).collect();
        let mut grid = GridState::new(bindings.len());
        grid.click(0, egui::Modifiers::NONE, false);
        Self { ko, bindings, grid }
    }

    /// CreateRow: (키, 이름, 종류, 명령, 인자).
    pub(super) fn row(&self, index: usize) -> [String; 5] {
        let (key, binding) = &self.bindings[index];
        match binding {
            None => [key.clone(), String::new(), if self.ko { "기본" } else { "Default" }.into(), String::new(), String::new()],
            Some(b) => [
                key.clone(),
                b.name.clone(),
                if b.internal { if self.ko { "내부 명령" } else { "Internal" } } else if self.ko { "사용자 명령" } else { "User command" }.into(),
                b.command.clone(),
                b.parameters.clone(),
            ],
        }
    }

    /// Commit: 설정한 것만 정리해 FunctionKeyBindings 값으로.
    pub(super) fn result(&self) -> Value {
        let map: Map<String, Value> = self.bindings.iter().filter_map(|(key, b)| b.as_ref().map(|b| (key.clone(), b.normalized().to_json()))).collect();
        Value::Object(map)
    }

    pub(super) fn selected(&self) -> Option<usize> {
        self.grid.selected_in_order().first().copied()
    }
}

/// FunctionKeyEditWindow.
pub(super) struct FunctionKeyEditForm {
    ko: bool,
    pub name: String,
    pub internal: bool,
    pub internal_command: usize,
    pub command: String,
    pub parameters: String,
}

impl FunctionKeyEditForm {
    pub(super) fn new(key: &str, source: &Binding, ko: bool) -> Self {
        let internal_command = INTERNAL_COMMANDS.iter().position(|(id, _)| id.eq_ignore_ascii_case(&source.command)).unwrap_or(0);
        Self {
            ko,
            name: if source.name.is_empty() { key.into() } else { source.name.clone() },
            internal: source.internal,
            internal_command,
            command: source.command.clone(),
            parameters: source.parameters.clone(),
        }
    }

    /// Commit 검사: Err(알림 글).
    pub(super) fn result(&self) -> Result<Binding, String> {
        let name = self.name.trim();
        let command = if self.internal { INTERNAL_COMMANDS[self.internal_command].0.to_owned() } else { self.command.trim().to_owned() };
        if name.is_empty() {
            return Err(if self.ko { "표시할 이름을 입력하세요." } else { "Enter a display name." }.into());
        }
        if command.is_empty() {
            return Err(if self.ko { "명령을 선택하거나 입력하세요." } else { "Select or enter a command." }.into());
        }
        Ok(Binding { name: name.into(), command, parameters: self.parameters.trim().into(), internal: self.internal })
    }
}

/// 원본 TfEditFunction PopupMenu1(btnParam "...")의 항목 글(DFM 그대로, 언어 파일에 없음).
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

/// 원본 ParameterClick(0x41fed0): 항목 글을 첫 ")"까지 자르고(Pos(")") 뒤를 Delete) eParams의 커서(글자 번호) 자리에 넣는다.
/// 돌려주는 값은 넣은 글 뒤의 새 커서(SelStart + 길이).
pub(super) fn insert_parameter(text: &mut String, caret: usize, caption: &str) -> usize {
    let token = caption.find(')').map_or(caption, |end| &caption[..=end]);
    let caret = caret.min(text.chars().count());
    let byte = text.char_indices().nth(caret).map_or(text.len(), |(index, _)| index);
    text.insert_str(byte, token);
    caret + token.chars().count()
}

/// 원본 TfEditFunction(ClientWidth 462 × ClientHeight 223): rbUserCommand(20,13) rbInternalCommand(229,13), Bevel(20,36 421),
/// lName(20,60) eName(113,57 141×21), lCommand(20,88) cbCommand(113,85 294×22) btnBrowse(412,85 29×21),
/// lParams(20,115) eParams(113,112 294×21) btnParam(412,112 28×23, PopupMenu1), lParamTip(오른쪽 끝 407,143),
/// btnOk(285,191) btnCancel(366,191) 75×25. C# 판의 매크로 안내 글은 lParamTip 아래에 작게 둔다. 확인이면 Some(true), 취소면 Some(false).
pub(super) fn function_key_edit_ui(ui: &mut egui::Ui, form: &mut FunctionKeyEditForm, language: &compat::Language, enter: bool) -> Option<bool> {
    use super::original_dialogs::put_left;
    let ko = form.ko;
    let text = |key: &str, fallback: &str| language.text(&format!("formEditFunction.{key}"), fallback);
    let mut result = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(462.0, 223.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    if put_left(ui, at(20.0, 13.0, 189.0, 17.0), egui::RadioButton::new(!form.internal, text("Caption.rbUserCommand", "User Defined Command"))).clicked() {
        form.internal = false;
    }
    if put_left(ui, at(229.0, 13.0, 188.0, 17.0), egui::RadioButton::new(form.internal, text("Caption.rbInternalCommand", "Internal Command"))).clicked() {
        form.internal = true;
    }
    ui.painter().hline(area.min.x + 20.0..=area.min.x + 441.0, area.min.y + 36.0, ui.visuals().widgets.noninteractive.bg_stroke);
    put_left(ui, at(20.0, 60.0, 92.0, 15.0), egui::Label::new(text("Caption.lName", "Display Name:")).truncate());
    ui.put(at(113.0, 57.0, 141.0, 21.0), egui::TextEdit::singleline(&mut form.name));
    put_left(ui, at(20.0, 88.0, 92.0, 15.0), egui::Label::new(text("Caption.lCommand", "Command:")).truncate());
    if form.internal {
        // C# LocalizeCommand(name) = TranslateText(name): 언어 파일 값 역검색으로 지금 언어의 글.
        let names: Vec<String> = INTERNAL_COMMANDS.iter().map(|(_, name)| language.translate_pair(name, name)).collect();
        ui.scope_builder(egui::UiBuilder::new().max_rect(at(113.0, 85.0, 294.0, 22.0)), |ui| {
            egui::ComboBox::from_id_salt("function-key-command").width(286.0).selected_text(names[form.internal_command].as_str()).show_opaque(ui, |ui| {
                for (index, name) in names.iter().enumerate() {
                    ui.selectable_value(&mut form.internal_command, index, name.as_str());
                }
            });
        });
    } else {
        ui.put(at(113.0, 85.0, 294.0, 21.0), egui::TextEdit::singleline(&mut form.command));
    }
    // 원본 btnBrowse(OpenDialog): 사용자 명령이면 실행할 파일을 고른다.
    let browse = super::original_dialogs::put_enabled(ui, at(412.0, 85.0, 29.0, 21.0), !form.internal, egui::Button::new("..."));
    if browse.clicked() {
        let title = text("Form.Caption", "Edit Function Key");
        let initial = Path::new(form.command.trim()).parent().filter(|p| p.is_dir()).map(Path::to_path_buf);
        if let Some(path) = ndr::file_dialogs::open_file(&title, initial.as_deref(), "", &[("*.*", "*.*")]) {
            form.command = path.to_string_lossy().into_owned();
        }
    }
    put_left(ui, at(20.0, 115.0, 92.0, 15.0), egui::Label::new(text("Caption.lParams", "Parameter(s):")).truncate());
    // 원본 eParams + btnParam("...", PopupMenu1 → ParameterClick).
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
    // 원본 lParamTip(오른쪽 맞춤, 오른쪽 끝 407).
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(20.0, 143.0, 387.0, 15.0)).layout(egui::Layout::right_to_left(egui::Align::Min)), |ui| {
        ui.add(egui::Label::new(text("Caption.lParamTip", "* $(???) : Active Pane, $(???1) : Left Pane, $(???2) : Right Pane")).truncate());
    });
    let note = if ko {
        "선택 파일: !\\!.!   인자 실행 전 편집: 끝에 %P   활성 패널 매크로: $(PATH), $(SELECTED), $(FILENAME), $(NAME), $(EXTENSION), $(DRIVE)   왼쪽/오른쪽: 숫자 1/2 추가"
    } else {
        "Selected files: !\\!.!   Edit parameters before launch: append %P   Active pane macros: $(PATH), $(SELECTED), $(FILENAME), $(NAME), $(EXTENSION), $(DRIVE); add 1 or 2 for left/right."
    };
    put_left(ui, at(20.0, 160.0, 421.0, 28.0), egui::Label::new(RichText::new(note).small().weak()).wrap());
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
