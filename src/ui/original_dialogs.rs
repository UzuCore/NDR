//! 원본 대화 상자: TfGetText(입력), TfRename(이름 바꾸기), TfDelWarning(삭제 경고), TfDelConfirm(보호 항목 삭제 확인).
//! C# NDR의 InputDialog, RenameWindow, DeleteWarningWindow, DeleteConfirmationWindow와 같은 규칙.
//! egui 모달로 그리고 결과는 콜백으로 돌려준다. 한 번에 가장 나중에 연(맨 위) 대화 상자 하나만 보인다(창 위의 입력 상자처럼 겹쳐 열 수 있다).
use super::*;
use eframe::egui::{FontId, text::LayoutJob};

pub(super) const INVALID_NAME_CHARACTERS: &str = "\\/:*?\"<>|";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeleteWarningMode {
    Recycle,
    Delete,
    Shred,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeleteWarningChoice {
    Delete,
    Yes,
    No,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DeleteConfirmChoice {
    All,
    Yes,
    No,
}

pub(super) enum DialogKind {
    Prompt { message: String, value: String, history: Option<Vec<String>>, password: bool },
    Rename { message: String, name: String, extension: String },
    DeleteWarning { message: String, show_delete: bool, default_button: usize },
    DeleteConfirm { message: String, attributes: String },
    Attributes(AttributeForm),
    Split(super::split_dialogs::SplitForm),
    Join(super::split_dialogs::JoinForm),
    ListExport(Box<super::list_export_dialog::ListExportForm>),
    EditShortcut(super::shortcut_window::EditShortcutForm),
    AdvancedRename(Box<super::rename_dialog::RenameForm>),
    Search(Box<super::search_dialog::SearchForm>),
    Choice(ChoiceForm),
    /// 원본 TfCompareFolder(640×505) 한 창: 비교·중단·결과 저장·닫기.
    FolderCompare(Box<super::compare_dialog::FolderCompareForm>),
    Comparison(Box<super::compare_dialog::ComparisonForm>),
    CleanupOptions(Box<super::cleanup_dialog::CleanupOptionsForm>),
    Favorites(Box<super::favorites_dialog::FavoritesForm>),
    /// C# CredentialsWindow.
    Credentials { user: String, password: String },
    Destination(Box<super::destination_dialog::DestinationForm>),
    ArchiveCreate(Box<super::archive_dialog::ArchiveCreateForm>),
    /// C# DiskCopyWindow.
    DiskCopy(Box<super::disk_copy_dialog::DiskCopyForm>),
    /// C# RemoteWindow.
    Remote(Box<super::remote_window::RemoteForm>),
    /// C# RemoteTransferProgressWindow.
    RemoteTransfer(Box<super::remote_window::TransferForm>),
    /// C# RemotePermissionsWindow.
    RemotePermissions(Box<super::remote_window::PermissionForm>),
    /// C# RemoteSitesWindow.
    RemoteSites(Box<super::remote_sites::SitesForm>),
    /// C# RemoteSiteEditorWindow.
    SiteEditor(Box<super::remote_sites::SiteEditorForm>),
    /// 원본 TfFTPQuickConnect(패널 FTP 모드 빠른 연결).
    FtpQuickConnect(Box<super::ftp_pane::FtpQuickForm>),
    /// C# SettingsWindow.
    Settings(Box<super::settings_window::SettingsForm>),
    /// C# FunctionKeyEditWindow.
    FunctionKeyEdit(Box<super::function_key_editor::FunctionKeyEditForm>),

    /// C# FileConflictWindow.
    FileConflict(Box<super::file_operation::ConflictForm>),
    /// 원본 TfChecksum(제목 CRC/MD5/SHA1): 결과 글(처음 CALCULATING, 실패면 ERROR)과, 결과가 나오면 켜지는 복사 단추.
    Checksum { value: String, ready: bool },
}

/// C# ChoiceDialog: 안내 글, 고르기 상자(처음 선택은 selected), 확인/취소.
#[derive(Clone, Debug)]
pub(super) struct ChoiceForm {
    pub message: String,
    pub options: Vec<String>,
    pub selected: usize,
}

pub(super) enum DialogResult {
    Text(Option<String>),
    Rename(Option<(String, String)>),
    DeleteWarning(DeleteWarningChoice),
    DeleteConfirm(DeleteConfirmChoice),
    Attributes(AttributeAction, Option<ndr::tools::AttributeChanges>),
    Split(Option<(PathBuf, u64, u64)>),
    Join(Option<super::split_dialogs::JoinOptions>),
    /// 파일 목록 저장 창: Some(()) 확인(btnSave, ModalResult 1), None 취소.
    ListExport(Option<()>),
    EditShortcut(Option<String>),
    AdvancedRename(bool),
    Search(Option<ndr::tools::SearchCriteria>),
    /// 고른 항목의 번호(취소면 None).
    Choice(Option<usize>),
    /// (하위 폴더 포함, 파일 내용 비교).
    FolderCompare,
    /// 동기화 미리보기에서 "복사"를 켠 차이(원래 순서). 비교 결과 창을 닫으면 None.
    Comparison(Option<Vec<ndr::folder_compare::Difference>>),
    /// 而댄벂??泥?냼 李쎌쓣 ?レ쓬: Some((?ъ슜??吏???꾩튂, ?뺤옣??) = btnClose(?④릿??, None = Esc쨌?リ린 ?⑥텛.
    CleanupOptions(Option<(Vec<String>, String)>),
    /// (지울 파일, 휴지통도 비울지). 취소면 None.
    /// 저장한 (이름, 경로) 칸들. 취소면 None.
    Favorites(Option<Vec<(String, String)>>),
    /// (사용자 이름(앞뒤 공백 뺌), 비밀번호). 취소면 None.
    Credentials(Option<(String, String)>),
    /// 고른 대상 폴더(전체 경로). 취소면 None.
    Destination(Option<PathBuf>),
    ArchiveCreate(Option<super::archive_dialog::ArchiveChoice>),
    /// 디스크 복사 창이 닫힘(C# MainWindow는 ShowDialog 결과를 쓰지 않는다).
    DiskCopy,
    Remote,
    RemoteTransfer,
    /// (동작, 권한 비트). 취소면 None.
    RemotePermissions(Option<(super::remote_window::PermissionAction, u32)>),
    /// 저장한 사이트 목록과 연결할 사이트 번호. 취소면 None.
    RemoteSites(Option<(Vec<super::remote_window::SiteProfile>, Option<usize>)>),
    SiteEditor(Option<super::remote_window::SiteProfile>),
    /// 연결할 입력값. 취소면 None.
    FtpQuickConnect(Option<super::ftp_pane::FtpQuickForm>),
    /// 확인한 설정 사본. 취소면 None.
    Settings(Option<ndr::settings::Settings>),
    FunctionKeyEdit(Option<super::function_key_editor::Binding>),

    /// (고른 답, 모두 적용). 창을 닫으면 None(= 취소).
    FileConflict(Option<(ndr::fsops::ConflictChoice, bool)>),
    /// 체크섬 창이 닫힘.
    Checksum,
}

/// C# FileAttributeDialogAction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AttributeAction {
    Apply,
    ApplyToAll,
    Skip,
    Cancel,
}

/// C# AttributeTimeWindow의 입력 상태. 속성 칸은 세 상태(None = 그대로, 처음 상태).
#[derive(Clone, Debug)]
pub(super) struct AttributeForm {
    pub path: String,
    pub archive: Option<bool>,
    pub read_only: Option<bool>,
    pub hidden: Option<bool>,
    pub system: Option<bool>,
    pub recursive: bool,
    pub apply_to_folders: bool,
    pub change_time: bool,
    pub date: String,
    pub time: String,
}

impl AttributeForm {
    pub(super) fn new(path: &str) -> Self {
        let now = chrono::Local::now();
        Self {
            path: path.into(),
            archive: None,
            read_only: None,
            hidden: None,
            system: None,
            recursive: false,
            apply_to_folders: true,
            change_time: false,
            date: now.format("%Y-%m-%d").to_string(),
            time: now.format("%H:%M:%S").to_string(),
        }
    }

    /// C# Complete: 시각을 바꿀 때만 날짜와 시간을 읽는다(시간은 0 이상 하루 미만). 틀리면 Err.
    pub(super) fn changes(&self) -> Result<ndr::tools::AttributeChanges, ()> {
        let modified = if self.change_time {
            let date = ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"].iter().find_map(|f| chrono::NaiveDate::parse_from_str(self.date.trim(), f).ok()).ok_or(())?;
            let time = ["%H:%M:%S%.f", "%H:%M:%S", "%H:%M"].iter().find_map(|f| chrono::NaiveTime::parse_from_str(self.time.trim(), f).ok()).ok_or(())?;
            let local = date.and_time(time).and_local_timezone(chrono::Local).earliest().ok_or(())?;
            Some(std::time::SystemTime::from(local))
        } else {
            None
        };
        Ok(ndr::tools::AttributeChanges {
            archive: self.archive,
            hidden: self.hidden,
            system: self.system,
            read_only: self.read_only,
            modified,
            recursive: self.recursive,
            apply_to_folders: self.apply_to_folders,
        })
    }
}

/// WPF 세 상태 확인란의 순환: 그대로(None) → 끔 → 켬 → 그대로.
fn cycle(state: &mut Option<bool>) {
    *state = match *state {
        None => Some(false),
        Some(false) => Some(true),
        Some(true) => None,
    };
}

pub(super) type DialogCallback = Box<dyn FnOnce(&mut App, DialogResult, &egui::Context)>;

pub(super) struct OriginalDialog {
    pub title: String,
    pub kind: DialogKind,
    pub serial: u64,
    pub focused: bool,
    on_result: Option<DialogCallback>,
}

/// Delphi TOpenDialog.Filter("이름|무늬|이름|무늬")를 (이름, 무늬) 쌍으로. 짝이 없는 끝 조각은 버린다.
pub(super) fn delphi_filter(text: &str) -> Vec<(String, String)> {
    let parts: Vec<&str> = text.split('|').collect();
    parts.chunks_exact(2).map(|pair| (pair[0].to_owned(), pair[1].to_owned())).collect()
}

/// 원본 OD_PROGRAMFILTER(프로그램 고르기 열기 상자 FUN_00417258·FUN_0041fca4·FUN_004506c8의 Filter).
pub(super) fn program_filter(language: &compat::Language) -> Vec<(String, String)> {
    delphi_filter(&language.text("RES.OD_PROGRAMFILTER", "Program Files(*.exe)|*.exe|All Files(*.*)|*.*"))
}

/// NXLabel 마크업을 egui 글 배치로.
pub(super) fn markup_job(text: &str, size: f32, color: Color32, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    for span in ndr::markup::render(text) {
        let mut format = egui::TextFormat { font_id: FontId::proportional(size), color: span.color.map_or(color, |(r, g, b)| Color32::from_rgb(r, g, b)), ..Default::default() };
        if span.bold {
            // 굵은 꼴(Segoe UI Bold·맑은 고딕 Bold, set_app_fonts의 "bold" 묶음). 색 태그가 없으면 C# 판처럼 강조색.
            format.color = if span.color.is_some() { format.color } else { Color32::WHITE };
            format.font_id = FontId::new(size, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()));
        }
        job.append(&span.text, 0.0, format);
    }
    job
}

impl App {
    fn push_dialog(&mut self, title: String, kind: DialogKind, on_result: DialogCallback) {
        self.msgbox_serial += 1;
        self.original_dialogs.push(OriginalDialog { title, kind, serial: self.msgbox_serial, focused: false, on_result: Some(on_result) });
    }

    /// 다른 모듈의 창(분할·합치기 등)을 대화 상자 줄에 넣는다.
    pub(super) fn push_original_dialog(&mut self, title: String, kind: DialogKind, on_result: impl FnOnce(&mut App, DialogResult, &egui::Context) + 'static) {
        self.push_dialog(title, kind, Box::new(on_result));
    }

    /// TfGetText(title, message, initial): OK면 입력값, 취소면 None. history가 있으면 기록 목록을 고를 수 있다.
    pub(super) fn prompt(
        &mut self,
        title: &str,
        message: &str,
        initial: &str,
        history: Option<Vec<String>>,
        password: bool,
        on_result: impl FnOnce(&mut App, Option<String>, &egui::Context) + 'static,
    ) {
        self.push_dialog(
            title.into(),
            DialogKind::Prompt { message: message.into(), value: initial.into(), history, password },
            Box::new(move |app, result, ctx| {
                if let DialogResult::Text(value) = result {
                    on_result(app, value, ctx)
                }
            }),
        );
    }

    /// TfRename: (새 이름, 새 확장자) 또는 None.
    pub(super) fn rename_dialog(
        &mut self,
        file_name: &str,
        name: &str,
        extension: &str,
        index: usize,
        count: usize,
        on_result: impl FnOnce(&mut App, Option<(String, String)>, &egui::Context) + 'static,
    ) {
        let template = self.language_pack.text(
            "RES.MSG_RENAME",
            "Type a new file name.(\\/:*?\"<>| characters cannot be used.)\n\n<b><c0000FF>%s</c></b> <c666666>(%d / %d)</c>",
        );
        let message = ndr::markup::substitute(&template, &[file_name, &index.to_string(), &count.to_string()]);
        let title = self.language_pack.text("RES.RENAME", "Rename");
        self.push_dialog(
            title,
            DialogKind::Rename { message, name: name.into(), extension: extension.into() },
            Box::new(move |app, result, ctx| {
                if let DialogResult::Rename(value) = result {
                    on_result(app, value, ctx)
                }
            }),
        );
    }

    /// C# DeleteWarningWindow.BuildMessage: 하나면 MSG_DELWARNING1(방식 문구, 파일 이름), 여럿이면 MSG_DELWARNING2(방식 문구, 개수).
    pub(super) fn delete_warning_message(&self, mode: DeleteWarningMode, count: usize, first_name: &str) -> String {
        let mode_text = match mode {
            DeleteWarningMode::Delete => self.language_pack.text("RES.MSG_DELWARN_DELETE", "<b>Delete</b>?\n\n"),
            DeleteWarningMode::Shred => self.language_pack.text("RES.MSG_DELWARN_SHRED", "<b>Shred</b>?\n(Contents are wiped and <cFF0000>cannot be recovered</c>.)\n"),
            DeleteWarningMode::Recycle => self.language_pack.text("RES.MSG_DELWARN_RECYCLEBIN", "Move to the <b>Recycle Bin</b>?\n\n"),
        };
        if count == 1 {
            let template = self.language_pack.text("RES.MSG_DELWARNING1", "Do you really want to %s<b><c0000FF>%s</c></b>");
            ndr::markup::substitute(&template, &[&mode_text, first_name])
        } else {
            let template = self.language_pack.text("RES.MSG_DELWARNING2", "Do you really want to %s<b><c0000FF>%d selected files/folders</c></b>");
            ndr::markup::substitute(&template, &[&mode_text, &count.to_string()])
        }
    }

    /// TfDelWarning(원본 FUN_0040c5c4). Delete 단추는 휴지통 방식이고 showdelbtn일 때만 있다. defdelbtn이 기본 단추를 고른다.
    pub(super) fn delete_warning(
        &mut self,
        mode: DeleteWarningMode,
        count: usize,
        first_name: &str,
        on_result: impl FnOnce(&mut App, DeleteWarningChoice, &egui::Context) + 'static,
    ) {
        let message = self.delete_warning_message(mode, count, first_name);
        let show_delete = mode == DeleteWarningMode::Recycle && self.settings.flag("ShowDeleteButton", true);
        let default_button = (self.settings.values.get("DefaultDeleteButton").and_then(|v| v.as_i64()).unwrap_or(2)).clamp(0, 2) as usize;
        let title = self.language_pack.text("formDelWarning.Form.Caption", "Delete");
        self.push_dialog(
            title,
            DialogKind::DeleteWarning { message, show_delete, default_button },
            Box::new(move |app, result, ctx| {
                if let DialogResult::DeleteWarning(choice) = result {
                    on_result(app, choice, ctx)
                }
            }),
        );
    }

    /// TfDelConfirm(원본 FUN_0041ea78): 보호 속성이 있는 항목 하나에 대한 Yes to All / Yes / No.
    pub(super) fn delete_confirm(
        &mut self,
        message: &str,
        attributes: &str,
        on_result: impl FnOnce(&mut App, DeleteConfirmChoice, &egui::Context) + 'static,
    ) {
        let title = self.language_pack.text("formDelConfirm.Form.Caption", "Confirm File Delete");
        self.push_dialog(
            title,
            DialogKind::DeleteConfirm { message: message.into(), attributes: attributes.into() },
            Box::new(move |app, result, ctx| {
                if let DialogResult::DeleteConfirm(choice) = result {
                    on_result(app, choice, ctx)
                }
            }),
        );
    }

    /// C# ChoiceDialog.Show(title, prompt, choices, selected): 고른 번호, 취소면 None. 고를 것이 없으면 열지 않는다(C#은 null).
    pub(super) fn choice_dialog(
        &mut self,
        title: &str,
        message: &str,
        options: Vec<String>,
        selected: usize,
        on_result: impl FnOnce(&mut App, Option<usize>, &egui::Context) + 'static,
    ) {
        if options.is_empty() {
            return;
        }
        let selected = selected.min(options.len() - 1);
        self.push_dialog(
            title.into(),
            DialogKind::Choice(ChoiceForm { message: message.into(), options, selected }),
            Box::new(move |app, result, ctx| {
                if let DialogResult::Choice(value) = result {
                    on_result(app, value, ctx)
                }
            }),
        );
    }

    /// 맨 위 대화 상자를 결과와 함께 닫는다(테스트와 버튼 공용).
    pub(super) fn answer_dialog(&mut self, result: DialogResult, ctx: &egui::Context) {
        if self.original_dialogs.is_empty() {
            return;
        }
        let mut current = self.original_dialogs.pop().expect("a dialog is open");
        if let Some(callback) = current.on_result.take() {
            callback(self, result, ctx);
        }
    }

    /// C# AttributeTimeWindow(항목 하나): 모두 적용 / 건너뛰기 / 확인 / 취소.
    pub(super) fn attribute_dialog(&mut self, path: &str, on_result: impl FnOnce(&mut App, AttributeAction, Option<ndr::tools::AttributeChanges>, &egui::Context) + 'static) {
        let title = self.language_pack.text("formChangeAttr.Form.Caption", "Change Attributes");
        self.push_dialog(
            title,
            DialogKind::Attributes(AttributeForm::new(path)),
            Box::new(move |app, result, ctx| {
                if let DialogResult::Attributes(action, changes) = result {
                    on_result(app, action, changes, ctx)
                }
            }),
        );
    }

    /// 속성 창의 단추: 확인과 모두 적용은 날짜·시간을 검사해 틀리면 경고 상자(창 제목, 경고 아이콘)를 보이고 창은 남긴다.
    pub(super) fn submit_attributes(&mut self, action: AttributeAction, ctx: &egui::Context) {
        let Some(DialogKind::Attributes(form)) = self.original_dialogs.last().map(|d| &d.kind) else { return };
        let result = match action {
            AttributeAction::Skip | AttributeAction::Cancel => DialogResult::Attributes(action, None),
            _ => match form.changes() {
                Ok(changes) => DialogResult::Attributes(action, Some(changes)),
                Err(()) => {
                    let title = self.original_dialogs[self.original_dialogs.len() - 1].title.clone();
                    let text = self.tr("날짜 또는 시간이 올바르지 않습니다.", "The date or time is invalid.").into_owned();
                    return self.system_box(&title, &text, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
                }
            },
        };
        self.answer_dialog(result, ctx);
    }

    /// 지금 열린 속성 창의 입력을 바꾼다(테스트용).
    #[cfg(test)]
    pub(super) fn edit_dialog_kind(&mut self, edit: impl FnOnce(&mut DialogKind)) {
        if let Some(dialog) = self.original_dialogs.last_mut() {
            edit(&mut dialog.kind);
        }
    }

    /// 지금 열린 속성 창의 입력을 바꾼다(테스트용).
    #[cfg(test)]
    pub(super) fn edit_attribute_form(&mut self, edit: impl FnOnce(&mut AttributeForm)) {
        if let Some(DialogKind::Attributes(form)) = self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            edit(form);
        }
    }

    /// 지금 열린 대화 상자 이름 칸의 값을 바꾼다(테스트용).
    #[cfg(test)]
    pub(super) fn set_dialog_text(&mut self, first: &str, second: &str) {
        if let Some(dialog) = self.original_dialogs.last_mut() {
            match &mut dialog.kind {
                DialogKind::Prompt { value, .. } => *value = first.into(),
                DialogKind::Rename { name, extension, .. } => {
                    *name = first.into();
                    *extension = second.into();
                }
                _ => {}
            }
        }
    }

    /// TfRename OK: 이름이 비어 있지 않고 금지 문자가 없어야 한다. 아니면 MessageBoxW(MSG_INVALID_FILENAME, NULL 캡션, MB_OK)를 보이고 창은 남는다.
    pub(super) fn submit_rename(&mut self, ctx: &egui::Context) {
        let Some(DialogKind::Rename { name, extension, .. }) = self.original_dialogs.last().map(|d| &d.kind) else { return };
        let (name, extension) = (name.clone(), extension.clone());
        let acceptable = !name.is_empty() && !name.contains(|c| INVALID_NAME_CHARACTERS.contains(c)) && !extension.contains(|c| INVALID_NAME_CHARACTERS.contains(c));
        if acceptable {
            self.answer_dialog(DialogResult::Rename(Some((name, extension))), ctx);
        } else {
            let text = self.language_pack.text("RES.MSG_INVALID_FILENAME", "A file name can't contain any of the following characters:\n  \\ / : * ? \" < > | ");
            self.system_box("", &text, ndr::msgbox::MB_OK, |_, _, _| {});
        }
    }

    pub(super) fn original_dialog_ui(&mut self, ctx: &egui::Context) {
        // 메시지 상자가 위에 있으면(예: 이름 오류) 이 창은 그 뒤에 그리기만 하고 글쇠·단추·초점은 받지 않는다(Windows 모달 상자처럼).
        let covered = !self.msgboxes.is_empty();
        if self.original_dialogs.is_empty() {
            return;
        }
        let editing_shortcut = matches!(self.original_dialogs[self.original_dialogs.len() - 1].kind, DialogKind::EditShortcut(_));
        if editing_shortcut && !covered {
            self.capture_shortcut_keys(ctx);
        }
        let shortcut_lines = if editing_shortcut { self.shortcut_lines() } else { vec![] };
        // 그리는 동안 self를 빌려 쓰도록 맨 위 대화 상자를 잠시 꺼낸다.
        let mut taken = self.original_dialogs.pop().expect("a dialog is open");
        let dialog = &mut taken;
        let serial = dialog.serial;
        let first_frame = !dialog.focused && !covered;
        if !covered {
            dialog.focused = true;
        }
        let title = dialog.title.clone();
        let text_color = if self.dark { Color32::from_rgb(0xB9, 0xC8, 0xD9) } else { Color32::from_rgb(40, 48, 60) };
        let ko = self.language == "ko";
        // 원본 TfGetText 단추(formGetText.Caption.btnOk·btnCancel).
        let get_text_buttons = (self.language_pack.text("formGetText.Caption.btnOk", if ko { "확인" } else { "OK" }), self.language_pack.text("formGetText.Caption.btnCancel", if ko { "취소" } else { "Cancel" }));
        enum Outcome {
            None,
            Ok,
            Cancel,
            Choice(usize),
        }
        let mut outcome = Outcome::None;
        let mut search_action = None;
        let mut comparison_action = None;
        let mut folder_compare_action = None;
        let mut cleanup_action = None;
        let mut favorites_action = None;
        let mut destination_action = None;
        let mut disk_action = None;
        let mut remote_action = None;
        let mut transfer_action = None;
        let mut permission_result = None;
        let mut sites_action = None;
        let mut editor_action = None;
        let mut settings_action = None;
        let mut function_key_edit = None;
        let mut ftp_quick = None;

        let mut conflict_choice = None;
        let remote_labels = if matches!(dialog.kind, DialogKind::Remote(_)) {
            let mut labels = self.remote_labels();
            labels.transfer_icons = [self.toolbar_icon(ctx, "vertbarbtns.png"), self.toolbar_icon(ctx, "vertbarbtns_over.png")];
            Some(labels)
        } else {
            None
        };
        let shift = ctx.input(|i| i.modifiers.shift);
        let screen = ctx.content_rect();
        // 모달 진행 창(예: 사용자 지정 정리 검색)이 위에 있으면 이 창은 그리기만 하고 글쇠·단추는 받지 않는다.
        let waiting = covered || self.job.as_ref().is_some_and(|j| j.modal.is_some());
        // TfRename: 입력한 금지 문자는 받지 않는다(붙여 넣기는 OK에서 검사).
        if matches!(dialog.kind, DialogKind::Rename { .. }) {
            ctx.input_mut(|input| {
                for event in &mut input.events {
                    if let egui::Event::Text(text) = event {
                        text.retain(|c| !INVALID_NAME_CHARACTERS.contains(c));
                    }
                }
            });
        }
        let warning_icon = if matches!(dialog.kind, DialogKind::DeleteWarning { .. } | DialogKind::DeleteConfirm { .. }) { self.msgbox_icon(ctx, 0x7f02) } else { None };
        let (enter, escape) = if waiting { (false, false) } else { ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape))) };
        set_dialog_char(ctx, !waiting);
        let modal_id = egui::Id::new(("ndr-original-dialog", serial));
        super::keep_opaque(ctx, modal_id);
        super::movable_modal(ctx, modal_id).show(ctx, |ui| {
            if super::modal_title_bar(ui, modal_id, &title, None) {
                outcome = Outcome::Cancel;
            }
            ui.separator();
            match &mut dialog.kind {
                DialogKind::Prompt { message, value, history, password } => {
                    // 원본 TfGetText(ClientWidth 444 × ClientHeight 127): lMessage(18,8 411×53, 아래 맞춤), cbText(16,67 413×21),
                    // btnOK(265,95 79×25, 기본), btnCancel(351,95 79×25, 취소).
                    let (area, _) = ui.allocate_exact_size(egui::vec2(444.0, 127.0), egui::Sense::hover());
                    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
                    bottom_label(ui, at(18.0, 8.0, 411.0, 53.0), markup_job(message, 13.0, text_color, 411.0));
                    let has_history = history.as_ref().is_some_and(|h| !h.is_empty());
                    let edit = egui::TextEdit::singleline(value).password(*password);
                    let response = ui.put(at(16.0, 67.0, if has_history { 389.0 } else { 413.0 }, 21.0), edit);
                    if let Some(items) = history.as_ref().filter(|h| !h.is_empty()) {
                        ui.scope_builder(egui::UiBuilder::new().max_rect(at(407.0, 67.0, 22.0, 21.0)), |ui| {
                            ui.menu_button("▼", |ui| {
                                for item in items {
                                    if ui.button(item).clicked() {
                                        *value = item.clone();
                                        ui.close();
                                    }
                                }
                            });
                        });
                    }
                    if first_frame {
                        response.request_focus();
                        select_all(ui, response.id, value.chars().count());
                    }
                    if ui.put(at(265.0, 95.0, 79.0, 25.0), egui::Button::new(get_text_buttons.0.as_str())).clicked() {
                        outcome = Outcome::Ok;
                    }
                    if ui.put(at(351.0, 95.0, 79.0, 25.0), egui::Button::new(get_text_buttons.1.as_str())).clicked() {
                        outcome = Outcome::Cancel;
                    }
                    if enter {
                        outcome = Outcome::Ok;
                    }
                }
                DialogKind::Rename { message, name, extension } => {
                    // 원본 TfRename(444 × 132): lMessage(18,8 410×53, 아래 맞춤), cbName(16,67 334×21, 260자), Label1 "."(355,70),
                    // cbExt(362,67 69×21), btnOK(270,100 75×25), btnCancel(357,99 75×25).
                    let (area, _) = ui.allocate_exact_size(egui::vec2(444.0, 132.0), egui::Sense::hover());
                    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
                    bottom_label(ui, at(18.0, 8.0, 410.0, 53.0), markup_job(message, 13.0, text_color, 410.0));
                    let response = ui.put(at(16.0, 67.0, 334.0, 21.0), egui::TextEdit::singleline(name).char_limit(260));
                    super::original_dialogs::put_left(ui, at(355.0, 70.0, 6.0, 13.0), egui::Label::new("."));
                    ui.put(at(362.0, 67.0, 69.0, 21.0), egui::TextEdit::singleline(extension));
                    if first_frame {
                        response.request_focus();
                        select_all(ui, response.id, name.chars().count());
                    }
                    let ok_label = self.language_pack.text("formRename.Caption.btnOK", "OK");
                    let cancel_label = self.language_pack.text("formRename.Caption.btnCancel", "Cancel");
                    if ui.put(at(270.0, 100.0, 75.0, 25.0), egui::Button::new(ok_label)).clicked() {
                        outcome = Outcome::Ok;
                    }
                    if ui.put(at(357.0, 99.0, 75.0, 25.0), egui::Button::new(cancel_label)).clicked() {
                        outcome = Outcome::Cancel;
                    }
                    if enter {
                        outcome = Outcome::Ok;
                    }
                }
                DialogKind::DeleteWarning { message, show_delete, default_button } => {
                    // 원본 TfDelWarning(443 × 119): Image1(13,16), lMsg(64,16 364×62), btnDelete(64,86 75×25, 숨김이 처음),
                    // btnYes(272,86), btnNo(353,86). 글이 lMsg보다 길면 그만큼 창을 늘린다(넘치는 긴 목록은 스크롤).
                    let galley = ui.painter().layout_job(markup_job(message, 13.0, text_color, 364.0));
                    let extra = (galley.size().y.min(560.0) - 62.0).max(0.0);
                    let (area, _) = ui.allocate_exact_size(egui::vec2(443.0, 119.0 + extra), egui::Sense::hover());
                    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
                    if let Some(icon) = warning_icon.clone() {
                        ui.painter().image(icon.id(), at(13.0, 16.0, 32.0, 32.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    ui.scope_builder(egui::UiBuilder::new().max_rect(at(64.0, 16.0, 364.0, 62.0 + extra)), |ui| {
                        egui::ScrollArea::vertical().max_height(62.0 + extra).show(ui, |ui| {
                            ui.label(markup_job(message, 13.0, text_color, 364.0));
                        });
                    });
                    // 탭 순서 btnYes, btnNo, btnDelete.
                    let focus_slot = if *show_delete { *default_button } else if *default_button == 2 { 2 } else { 1 };
                    let raw = |key: &str, fallback: &str| self.language_pack.raw(&format!("formDelWarning.{key}")).unwrap_or_else(|| fallback.to_owned());
                    if *show_delete {
                        let (delete, pressed) = vcl_button(ui, at(64.0, 86.0 + extra, 75.0, 25.0), true, &raw("Caption.btnDelete", "&Delete"));
                        if first_frame && focus_slot == 0 {
                            delete.request_focus();
                        }
                        if pressed {
                            outcome = Outcome::Choice(0);
                        }
                    }
                    let (yes, yes_pressed) = vcl_button(ui, at(272.0, 86.0 + extra, 75.0, 25.0), true, &raw("Caption.btnYes", "&Yes"));
                    let (no, no_pressed) = vcl_button(ui, at(353.0, 86.0 + extra, 75.0, 25.0), true, &raw("Caption.btnNo", "&No"));
                    if first_frame {
                        match focus_slot {
                            1 => yes.request_focus(),
                            2 => no.request_focus(),
                            _ => {}
                        }
                    }
                    if yes_pressed && matches!(outcome, Outcome::None) {
                        outcome = Outcome::Choice(1);
                    }
                    if no_pressed && matches!(outcome, Outcome::None) {
                        outcome = Outcome::Choice(2);
                    }
                }
                DialogKind::Split(form) => {
                    if let Some(choice) = super::split_dialogs::split_ui(ui, form, &self.language_pack, enter) {
                        outcome = Outcome::Choice(choice);
                    }
                }
                DialogKind::Search(form) => {
                    search_action = super::search_dialog::search_ui(ui, form, &self.language_pack, ko, enter);
                }
                DialogKind::Choice(form) => {
                    // ChoiceDialog: 440 너비, 글(14), 고르기 상자, 오른쪽 아래 확인·취소. Enter는 확인.
                    ui.set_width(404.0);
                    ui.add(egui::Label::new(RichText::new(form.message.as_str()).size(14.0).color(text_color)).wrap());
                    ui.add_space(12.0);
                    let current = form.options.get(form.selected).cloned().unwrap_or_default();
                    egui::ComboBox::from_id_salt(("ndr-choice", serial)).width(404.0).selected_text(current).show_opaque(ui, |ui| {
                        for (index, option) in form.options.iter().enumerate() {
                            ui.selectable_value(&mut form.selected, index, option.as_str());
                        }
                    });
                    ui.add_space(24.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if ui.add_sized([82.0, 28.0], egui::Button::new(if ko { "취소" } else { "Cancel" })).clicked() {
                            outcome = Outcome::Cancel;
                        }
                        if ui.add_sized([82.0, 28.0], egui::Button::new(if ko { "확인" } else { "OK" })).clicked() {
                            outcome = Outcome::Ok;
                        }
                    });
                    if enter {
                        outcome = Outcome::Ok;
                    }
                }
                DialogKind::FolderCompare(form) => {
                    folder_compare_action = super::compare_dialog::folder_compare_ui(ui, form, &self.language_pack, text_color, enter);
                }
                DialogKind::ArchiveCreate(form) => {
                    if let Some(choice) = super::archive_dialog::archive_create_ui(ui, form, &self.language_pack, enter) {
                        outcome = Outcome::Choice(choice);
                    }
                }
                DialogKind::Destination(form) => {
                    let icon = self.path_icon(ctx, Path::new("A"), true);
                    destination_action = super::destination_dialog::destination_ui_original(ui, form, enter, icon.as_ref(), screen.height());
                }
                DialogKind::DiskCopy(form) => {
                    disk_action = super::disk_copy_dialog::disk_copy_ui(ui, form);
                }
                DialogKind::Remote(form) => {
                    if let Some(labels) = &remote_labels {
                        remote_action = super::remote_window::remote_ui(ui, form, labels, &self.settings);
                    }
                }
                DialogKind::RemoteTransfer(form) => {
                    transfer_action = super::remote_window::transfer_ui(ui, form);
                }
                DialogKind::RemotePermissions(form) => {
                    if let Some(choice) = super::remote_window::permission_ui(ui, form, &self.language_pack, enter && !shift, enter && shift, escape) {
                        permission_result = Some(choice.map(|action| (action, form.mode())));
                    }
                }
                DialogKind::RemoteSites(form) => {
                    sites_action = super::remote_sites::sites_ui(ui, form, &self.language_pack, enter);
                }
                DialogKind::SiteEditor(form) => {
                    editor_action = super::remote_sites::site_editor_ui(ui, form, &self.language_pack, enter);
                }
                DialogKind::FtpQuickConnect(form) => {
                    ftp_quick = super::ftp_pane::ftp_quick_ui(ui, form, &self.language_pack, enter);
                }
                DialogKind::Settings(form) => {
                    // 원본 lvFunctionKey OnKeyDown: Delete = 삭제(메시지 상자·모달 작업 중에는 받지 않는다).
                    let delete = !waiting && ui.input(|i| i.key_pressed(egui::Key::Delete));
                    settings_action = super::settings_window::settings_ui(ui, form, enter, delete);
                }
                DialogKind::FunctionKeyEdit(form) => {
                    function_key_edit = super::function_key_editor::function_key_edit_ui(ui, form, &self.language_pack, enter);
                }

                DialogKind::FileConflict(form) => {
                    conflict_choice = super::file_operation::conflict_ui(ui, form, &self.language_pack, ko, enter).map(|c| (c, form.apply_all));
                }

                DialogKind::Credentials { user, password } => {
                    if let Some(ok) = super::tools_dialog::credentials_ui(ui, user, password, &self.language_pack, enter, first_frame) {
                        outcome = if ok { Outcome::Ok } else { Outcome::Cancel };
                    }
                }
                DialogKind::Checksum { value, ready } => {
                    // 원본 TfChecksum DFM(속 288×92): 가운데 맞춘 테두리 없는 읽기 전용 eChecksum(20,24, 249×17),
                    // btnCopy(48,56, 193×25)는 결과가 나오면 켜지고 초점을 받는다(Enter = 복사). Esc = 닫기(FormKeyDown).
                    let (area, _) = ui.allocate_exact_size(egui::vec2(288.0, 92.0), egui::Sense::hover());
                    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(at(20.0, 24.0, 249.0, 17.0)).layout(egui::Layout::top_down(egui::Align::Center)), |ui| {
                        ui.add(egui::Label::new(RichText::new(value.as_str()).color(text_color)).selectable(true).truncate());
                    });
                    let copy = self.language_pack.text("formChecksum.Caption.btnCopy", "Copy to Clipboard(&C)").replace('&', "");
                    let clicked = put_enabled(ui, at(48.0, 56.0, 193.0, 25.0), *ready, egui::Button::new(copy)).clicked();
                    if clicked {
                        outcome = Outcome::Choice(0);
                    }
                    if enter && *ready {
                        outcome = Outcome::Choice(0);
                    }
                }
                DialogKind::Favorites(form) => {
                    favorites_action = super::favorites_dialog::favorites_ui(ui, form, &self.language_pack);
                }
                DialogKind::CleanupOptions(form) => {
                    cleanup_action = super::cleanup_dialog::options_ui(ui, form, &self.language_pack, ko, enter);
                }
                DialogKind::Comparison(form) => {
                    // FolderComparisonWindow: 1120×700(작은 화면이면 줄인다).
                    let size = egui::vec2((screen.width() - 64.0).clamp(480.0, 1096.0), (screen.height() - 300.0).clamp(140.0, 500.0));
                    comparison_action = super::compare_dialog::comparison_ui(ui, form, &self.language_pack, ko, enter, size);
                }
                DialogKind::AdvancedRename(form) => {
                    if let Some(choice) = super::rename_dialog::rename_ui(ui, form, &self.language_pack, ko) {
                        outcome = Outcome::Choice(choice);
                    }
                    if enter {
                        outcome = Outcome::Choice(0);
                    }
                }
                DialogKind::EditShortcut(form) => {
                    if let Some(choice) = super::shortcut_window::edit_shortcut_ui(ui, form, &self.language_pack, &shortcut_lines) {
                        outcome = Outcome::Choice(choice);
                    }
                }
                DialogKind::ListExport(form) => {
                    if let Some(choice) = super::list_export_dialog::list_export_ui(ui, form, &self.language_pack) {
                        outcome = Outcome::Choice(choice);
                    }
                }
                DialogKind::Join(form) => {
                    if let Some(choice) = super::split_dialogs::join_ui(ui, form, &self.language_pack, ko, enter) {
                        outcome = Outcome::Choice(choice);
                    }
                }
                DialogKind::Attributes(form) => {
                    // 원본 TfChangeAttr(ClientWidth 437 × ClientHeight 288): lFileName(16,16), gbAttributes(16,44 177×161: chkArchive(빨강)·
                    // chkReadOnly·chkHidden·chkSystem 왼쪽 24, 위 34/59/85/111), gbTime(212,44 209×161: chkChangeTime(16,26), lDate(16,57)
                    // dtpDate(16,76 145×21), lTime(16,103) dtpTime(16,122)), chkRecursive(16,219), chkApplyToFolder(212,219),
                    // btnAll(95,255) btnSkip(176,255) btnOk(264,255) btnCancel(345,255) 75×25.
                    let text = |key: &str, fallback: &str| self.language_pack.text(&format!("formChangeAttr.{key}"), fallback).replace('_', "");
                    let raw = |key: &str, fallback: &str| self.language_pack.raw(&format!("formChangeAttr.{key}")).unwrap_or_else(|| fallback.to_owned()).replace('_', "");
                    let (area, _) = ui.allocate_exact_size(egui::vec2(437.0, 288.0), egui::Sense::hover());
                    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
                    put_left(ui, at(16.0, 16.0, 405.0, 22.0), egui::Label::new(form.path.as_str()).truncate()).on_hover_text(form.path.as_str());
                    group_box(ui, at(16.0, 44.0, 177.0, 161.0), &text("GroupBox.gbAttributes", "Attributes"));
                    let red = Color32::from_rgb(0xff, 0x40, 0x40);
                    for (top, state, key, fallback, color) in [
                        (34.0, &mut form.archive, "Caption.chkArchive", "&Archive", Some(red)),
                        (59.0, &mut form.read_only, "Caption.chkReadOnly", "&Read Only", None),
                        (85.0, &mut form.hidden, "Caption.chkHidden", "&Hidden", None),
                        (111.0, &mut form.system, "Caption.chkSystem", "&System", None),
                    ] {
                        let mut checked = *state == Some(true);
                        let (label, access) = access_job_colored(ui, &raw(key, fallback), color);
                        let clicked = put_left(ui, at(40.0, 44.0 + top, 130.0, 17.0), egui::Checkbox::new(&mut checked, label).indeterminate(state.is_none())).clicked();
                        if clicked || take_dialog_char(ui.ctx(), access) {
                            cycle(state);
                        }
                    }
                    group_box(ui, at(212.0, 44.0, 209.0, 161.0), &text("GroupBox.gbTime", "Time"));
                    access_checkbox(ui, at(228.0, 70.0, 169.0, 17.0), &mut form.change_time, &raw("Caption.chkChangeTime", "Modify Date/Time (&G)"));
                    let enabled = form.change_time;
                    put_left(ui, at(228.0, 101.0, 145.0, 15.0), egui::Label::new(text("Caption.lDate", "Date")));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(at(228.0, 120.0, 145.0, 21.0)), |ui| {
                        ui.add_enabled(enabled, egui::TextEdit::singleline(&mut form.date).desired_width(145.0).hint_text("yyyy-MM-dd"));
                    });
                    put_left(ui, at(228.0, 147.0, 145.0, 15.0), egui::Label::new(text("Caption.lTime", "Time")));
                    ui.scope_builder(egui::UiBuilder::new().max_rect(at(228.0, 166.0, 145.0, 21.0)), |ui| {
                        ui.add_enabled(enabled, egui::TextEdit::singleline(&mut form.time).desired_width(145.0));
                    });
                    access_checkbox(ui, at(16.0, 219.0, 177.0, 17.0), &mut form.recursive, &raw("Caption.chkRecursive", "Include Sub&folders"));
                    access_checkbox(ui, at(212.0, 219.0, 177.0, 17.0), &mut form.apply_to_folders, &raw("Caption.chkApplyToFolder", "Apply to &Folders"));
                    for (left, slot, key, fallback) in [(95.0, 0, "Caption.btnAll", "A&ll"), (176.0, 1, "Caption.btnSkip", "S&kip"), (264.0, 2, "Caption.btnOk", "OK"), (345.0, 3, "Caption.btnCancel", "Cancel")] {
                        if access_button(ui, at(left, 255.0, 75.0, 25.0), &raw(key, fallback)) && matches!(outcome, Outcome::None) {
                            outcome = Outcome::Choice(slot);
                        }
                    }
                    if enter {
                        outcome = Outcome::Choice(2);
                    }
                    // 원본 TntFormKeyDown(0x41a2f4, KeyPreview): End(0x23)면 btnAll(+0x40c).Click.
                    if !waiting && ctx.input(|i| i.key_pressed(egui::Key::End)) {
                        outcome = Outcome::Choice(0);
                    }
                }
                DialogKind::DeleteConfirm { message, attributes } => {
                    // 원본 TfDelConfirm(434 × 138): Image1(16,13), lMsg(80,13 337×58), lAttributes(80,80),
                    // btnAll(172,101 75×25), btnYes(256,101), btnNo(342,101).
                    let (area, _) = ui.allocate_exact_size(egui::vec2(434.0, 138.0), egui::Sense::hover());
                    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
                    if let Some(icon) = warning_icon.clone() {
                        ui.painter().image(icon.id(), at(16.0, 13.0, 32.0, 32.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    super::original_dialogs::put_left(ui, at(80.0, 13.0, 337.0, 58.0), egui::Label::new(message.as_str()).wrap());
                    super::original_dialogs::put_left(ui, at(80.0, 80.0, 337.0, 15.0), egui::Label::new(attributes.as_str()).truncate());
                    let raw = |key: &str, fallback: &str| self.language_pack.raw(&format!("formDelConfirm.{key}")).unwrap_or_else(|| fallback.to_owned());
                    for (left, slot, key, fallback) in [(172.0, 0, "Caption.btnAll", "Yes to &All"), (256.0, 1, "Caption.btnYes", "&Yes"), (342.0, 2, "Caption.btnNo", "&No")] {
                        if access_button(ui, at(left, 101.0, 75.0, 25.0), &raw(key, fallback)) && matches!(outcome, Outcome::None) {
                            outcome = Outcome::Choice(slot);
                        }
                    }
                }
            }
        });
        self.original_dialogs.push(taken);
        if waiting {
            return;
        }
        if let Some(action) = cleanup_action {
            return self.cleanup_action(action, ctx);
        }
        if let Some(action) = favorites_action {
            return self.favorites_action(action, ctx);
        }
        if let Some(action) = destination_action {
            return self.destination_action(action, ctx);
        }
        if let Some(action) = disk_action {
            return self.disk_copy_action(action, ctx);
        }
        if let Some(action) = remote_action {
            return self.remote_action(action, ctx);
        }
        if let Some(action) = transfer_action {
            return self.transfer_action(action);
        }
        if let Some(result) = permission_result {
            return self.answer_dialog(DialogResult::RemotePermissions(result), ctx);
        }
        if let Some(action) = sites_action {
            return self.sites_action(action, ctx);
        }
        if let Some(action) = editor_action {
            return self.site_editor_action(action, ctx);
        }
        if let Some(action) = settings_action {
            return self.settings_action(action, ctx);
        }
        if let Some(ok) = function_key_edit {
            return self.function_key_edit_done(ok, ctx);
        }
        if let Some(ok) = ftp_quick {
            let form = match self.original_dialogs.last().map(|d| &d.kind) {
                Some(DialogKind::FtpQuickConnect(form)) if ok => Some((**form).clone()),
                _ => None,
            };
            return self.answer_dialog(DialogResult::FtpQuickConnect(form), ctx);
        }

        if let Some(choice) = conflict_choice {
            return self.answer_dialog(DialogResult::FileConflict(Some(choice)), ctx);
        }
        if let Some(action) = search_action {
            return self.search_action(action, ctx);
        }
        if let Some(action) = comparison_action {
            return self.comparison_action(action, ctx);
        }
        if let Some(action) = folder_compare_action {
            return self.folder_compare_action(action, ctx);
        }
        if escape && matches!(outcome, Outcome::None) {
            // C# AttributeTimeWindow: Esc는 건너뛰기.
            outcome = if self.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::Attributes(_))) { Outcome::Choice(1) } else { Outcome::Cancel };
        }
        let Some(kind) = self.original_dialogs.last().map(|d| &d.kind) else { return };
        let result = match (outcome, kind) {
            (Outcome::None, _) => return,
            (Outcome::Ok, DialogKind::Rename { .. }) => return self.submit_rename(ctx),
            (Outcome::Choice(slot), DialogKind::Attributes(_)) => {
                let action = [AttributeAction::ApplyToAll, AttributeAction::Skip, AttributeAction::Apply, AttributeAction::Cancel][slot.min(3)];
                return self.submit_attributes(action, ctx);
            }
            (_, DialogKind::Attributes(_)) => return self.submit_attributes(AttributeAction::Cancel, ctx),
            (Outcome::Choice(0), DialogKind::Split(_) | DialogKind::Join(_)) => return self.submit_split_or_join(ctx),
            (_, DialogKind::Split(_)) => DialogResult::Split(None),
            (_, DialogKind::Join(_)) => DialogResult::Join(None),
            (Outcome::Choice(0), DialogKind::ListExport(_)) => return self.submit_list_export(ctx),
            (_, DialogKind::ListExport(_)) => DialogResult::ListExport(None),
            (Outcome::Choice(0), DialogKind::EditShortcut(form)) => DialogResult::EditShortcut(Some(form.text.clone())),
            (_, DialogKind::EditShortcut(_)) => DialogResult::EditShortcut(None),
            (Outcome::Choice(0), DialogKind::AdvancedRename(_)) => return self.commit_advanced_rename(),
            (Outcome::Choice(2), DialogKind::AdvancedRename(_)) => return self.add_rename_rule(),
            (Outcome::Choice(3), DialogKind::AdvancedRename(_)) => return self.save_rename_macro(),
            (Outcome::Choice(4), DialogKind::AdvancedRename(_)) => return self.delete_rename_macro(),
            (_, DialogKind::AdvancedRename(_)) => DialogResult::AdvancedRename(false),
            (_, DialogKind::Search(_)) => DialogResult::Search(None),
            (Outcome::Ok, DialogKind::Choice(form)) => DialogResult::Choice(Some(form.selected)),
            (_, DialogKind::Choice(_)) => DialogResult::Choice(None),
            // ×·Esc = 닫기 단추(원본 btnCancel Cancel=True). 비교 중이면 멈추고 닫는다.
            (_, DialogKind::FolderCompare(_)) => return self.folder_compare_action(super::compare_dialog::FolderCompareAction::Close, ctx),
            (_, DialogKind::Comparison(_)) => DialogResult::Comparison(None),
            (_, DialogKind::CleanupOptions(_)) => DialogResult::CleanupOptions(None),
            (_, DialogKind::Favorites(_)) => DialogResult::Favorites(None),
            (Outcome::Ok, DialogKind::Credentials { user, password }) => DialogResult::Credentials(Some((user.trim().to_owned(), password.clone()))),
            (_, DialogKind::Credentials { .. }) => DialogResult::Credentials(None),
            (_, DialogKind::Destination(_)) => DialogResult::Destination(None),
            (Outcome::Choice(choice), DialogKind::ArchiveCreate(_)) => return self.submit_archive_create(choice, ctx),
            (_, DialogKind::ArchiveCreate(_)) => DialogResult::ArchiveCreate(None),
            // ×·Esc = 닫기 단추(작업 중이면 닫지 않는다).
            (_, DialogKind::DiskCopy(_)) => return self.disk_copy_action(super::disk_copy_dialog::DiskAction::Close, ctx),
            // ×·Esc: 원격 창은 닫기 단추, 전송 창은 중단 요청, 권한 창 ×는 취소, 사이트 창은 취소.
            (_, DialogKind::Remote(_)) => return self.remote_action(super::remote_window::RemoteAction::Close, ctx),
            (_, DialogKind::RemoteTransfer(_)) => return self.transfer_close_requested(),
            (_, DialogKind::RemotePermissions(_)) => DialogResult::RemotePermissions(None),
            (_, DialogKind::RemoteSites(_)) => DialogResult::RemoteSites(None),
            (_, DialogKind::SiteEditor(_)) => DialogResult::SiteEditor(None),
            (_, DialogKind::FtpQuickConnect(_)) => DialogResult::FtpQuickConnect(None),
            (_, DialogKind::Settings(_)) => DialogResult::Settings(None),
            (_, DialogKind::FunctionKeyEdit(_)) => DialogResult::FunctionKeyEdit(None),

            (_, DialogKind::FileConflict(_)) => DialogResult::FileConflict(None),
            // 원본 btnCopyClick: 결과 글을 클립보드로(창은 그대로).
            (Outcome::Choice(0), DialogKind::Checksum { .. }) => return self.copy_checksum(),
            (_, DialogKind::Checksum { .. }) => DialogResult::Checksum,
            (Outcome::Ok, DialogKind::Prompt { value, .. }) => DialogResult::Text(Some(value.clone())),
            (Outcome::Cancel, DialogKind::Prompt { .. }) => DialogResult::Text(None),
            (_, DialogKind::Rename { .. }) => DialogResult::Rename(None),
            (Outcome::Choice(0), DialogKind::DeleteWarning { .. }) => DialogResult::DeleteWarning(DeleteWarningChoice::Delete),
            (Outcome::Choice(1), DialogKind::DeleteWarning { .. }) => DialogResult::DeleteWarning(DeleteWarningChoice::Yes),
            (_, DialogKind::DeleteWarning { .. }) => DialogResult::DeleteWarning(DeleteWarningChoice::No),
            (Outcome::Choice(0), DialogKind::DeleteConfirm { .. }) => DialogResult::DeleteConfirm(DeleteConfirmChoice::All),
            (Outcome::Choice(1), DialogKind::DeleteConfirm { .. }) => DialogResult::DeleteConfirm(DeleteConfirmChoice::Yes),
            (_, DialogKind::DeleteConfirm { .. }) => DialogResult::DeleteConfirm(DeleteConfirmChoice::No),
            (_, DialogKind::Prompt { .. }) => DialogResult::Text(None),
        };
        self.answer_dialog(result, ctx);
    }
}

/// 입력 칸을 열 때 전체 선택(C# SelectAll).
/// ui.put처럼 칸에 꽉 채워 가운데 맞춤으로 놓되, 끌 수 있게(원본 Enabled = False 단추).
pub(super) fn put_enabled(ui: &mut egui::Ui, rect: egui::Rect, enabled: bool, widget: impl egui::Widget) -> egui::Response {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)), |ui| ui.add_enabled(enabled, widget)).inner
}

/// VCL CM_DIALOGCHAR: 이 프레임에 친 단축 문자. 단추 하나가 받으면 지운다.
const DIALOG_CHAR: &str = "ndr-dialog-char";

/// 이 프레임의 단축 문자를 정한다: Alt+글자는 늘, 글자만은 글 입력 칸에 초점이 없을 때(단추·확인란에 초점이 있는 VCL 창처럼).
pub(super) fn set_dialog_char(ctx: &egui::Context, enabled: bool) {
    let typed = if enabled {
        ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } if !modifiers.ctrl && !modifiers.command => {
                    let name = key.name();
                    let mut chars = name.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) if c.is_ascii_alphanumeric() => Some((c.to_ascii_uppercase(), modifiers.alt)),
                        _ => None,
                    }
                }
                _ => None,
            })
        })
    } else {
        None
    };
    let typed = typed.filter(|(_, alt)| *alt || !ctx.wants_keyboard_input()).map(|(c, _)| c);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DIALOG_CHAR), typed));
}

/// 단축 문자가 `access`이면 그것을 가져간다(같은 글자 단추 둘이 함께 눌리지 않게).
pub(super) fn take_dialog_char(ctx: &egui::Context, access: Option<char>) -> bool {
    let Some(access) = access else { return false };
    ctx.data_mut(|d| {
        let slot = d.get_temp_mut_or_default::<Option<char>>(egui::Id::new(DIALOG_CHAR));
        let hit = slot.is_some_and(|c| c.eq_ignore_ascii_case(&access));
        if hit {
            *slot = None;
        }
        hit
    })
}

/// VCL Caption 글: '&' 다음 글자에 밑줄. (글, 단축 문자). `color`가 없으면 위젯 글 색.
pub(super) fn access_job_colored(ui: &egui::Ui, caption: &str, color: Option<Color32>) -> (LayoutJob, Option<char>) {
    let (display, underline) = ndr::msgbox::display_caption(caption);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let line = color.unwrap_or(ui.visuals().text_color());
    let mut job = LayoutJob::default();
    let mut access = None;
    for (index, character) in display.chars().enumerate() {
        let mut format = egui::TextFormat { font_id: font.clone(), color: color.unwrap_or(Color32::PLACEHOLDER), ..Default::default() };
        if underline == Some(index) {
            format.underline = egui::Stroke::new(1.0, line);
            access = Some(character);
        }
        job.append(&character.to_string(), 0.0, format);
    }
    (job, access)
}

fn access_job(ui: &egui::Ui, caption: &str) -> (LayoutJob, Option<char>) {
    access_job_colored(ui, caption, None)
}

/// VCL TButton(Caption의 '&' 단축 문자, Enabled). (응답, 눌렸는가: 누르거나 단축 문자).
pub(super) fn vcl_button(ui: &mut egui::Ui, rect: egui::Rect, enabled: bool, caption: &str) -> (egui::Response, bool) {
    let (job, access) = access_job(ui, caption);
    let response = put_enabled(ui, rect, enabled, egui::Button::new(job));
    let pressed = response.clicked() || (enabled && ui.is_enabled() && take_dialog_char(ui.ctx(), access));
    (response, pressed)
}

/// VCL TButton(Caption의 '&' 단축 문자). 누르거나 단축 문자를 치면 true.
pub(super) fn access_button(ui: &mut egui::Ui, rect: egui::Rect, caption: &str) -> bool {
    vcl_button(ui, rect, true, caption).1
}

/// VCL TRadioButton(Caption의 '&' 단축 문자). 누르거나 단축 문자를 치면 true.
pub(super) fn access_radio(ui: &mut egui::Ui, rect: egui::Rect, selected: bool, caption: &str) -> bool {
    let (job, access) = access_job(ui, caption);
    let clicked = put_left(ui, rect, egui::RadioButton::new(selected, job)).clicked();
    clicked || (ui.is_enabled() && take_dialog_char(ui.ctx(), access))
}

/// VCL TLabel(Caption의 '&' 단축 문자, FocusControl): 단축 문자를 치면 `target` 칸에 초점을 준다.
pub(super) fn access_label(ui: &mut egui::Ui, rect: egui::Rect, caption: &str, target: egui::Id) {
    let (job, access) = access_job(ui, caption);
    put_left(ui, rect, egui::Label::new(job).truncate());
    if ui.is_enabled() && take_dialog_char(ui.ctx(), access) {
        ui.memory_mut(|m| m.request_focus(target));
    }
}

/// VCL TCheckBox(Caption의 '&' 단축 문자): 단축 문자를 치면 값을 뒤집는다. 바뀌었으면 true.
pub(super) fn access_checkbox(ui: &mut egui::Ui, rect: egui::Rect, value: &mut bool, caption: &str) -> bool {
    let (job, access) = access_job(ui, caption);
    let mut changed = put_left(ui, rect, egui::Checkbox::new(value, job)).changed();
    if ui.is_enabled() && take_dialog_char(ui.ctx(), access) {
        *value = !*value;
        changed = true;
    }
    changed
}

/// 원본 TLabel·TCheckBox 자리(왼쪽 위 맞춤). ui.put은 칸 가운데에 늘려 놓으므로 글과 확인란은 이것으로 놓는다.
pub(super) fn put_left(ui: &mut egui::Ui, rect: egui::Rect, widget: impl egui::Widget) -> egui::Response {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Min)), |ui| ui.add(widget)).inner
}

/// 원본 TLabel EllipsisPosition = epPathEllipsis(DT_PATH_ELLIPSIS): 넓이를 넘으면 마지막 "\이름"은 남기고 그 앞을 "..."로 줄인다.
/// 마지막 이름만으로도 넘으면 끝을 "..."로 자른다.
pub(super) fn path_ellipsis(ui: &egui::Ui, text: &str, width: f32, font: &egui::FontId) -> String {
    let fits = |s: &str| ui.fonts_mut(|f| f.layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE).size().x) <= width;
    if fits(text) {
        return text.to_owned();
    }
    let split = text.trim_end_matches('\\').rfind('\\').unwrap_or(0);
    let (head, tail): (Vec<char>, &str) = (text[..split].chars().collect(), &text[split..]);
    for keep in (0..head.len()).rev() {
        let candidate = format!("{}...{tail}", head[..keep].iter().collect::<String>());
        if fits(&candidate) {
            return candidate;
        }
    }
    let mut chars: Vec<char> = format!("...{tail}").chars().collect();
    while chars.len() > 3 && !fits(&format!("{}...", chars.iter().collect::<String>())) {
        chars.pop();
    }
    format!("{}...", chars.iter().collect::<String>())
}

/// 원본 TGroupBox: 칸 테두리(글 높이 절반 아래에서 시작)와 왼쪽 위 8에 제목 글.
pub(super) fn group_box(ui: &egui::Ui, rect: egui::Rect, caption: &str) {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let frame = egui::Rect::from_min_max(rect.min + egui::vec2(0.0, 7.0), rect.max);
    ui.painter().rect_stroke(frame, 2.0, stroke, egui::StrokeKind::Inside);
    let galley = ui.painter().layout_no_wrap(caption.to_owned(), egui::FontId::proportional(13.0), ui.visuals().text_color());
    let text_rect = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 0.0), galley.size() + egui::vec2(4.0, 0.0));
    ui.painter().rect_filled(text_rect, 0.0, ui.visuals().window_fill);
    ui.painter().galley(text_rect.min + egui::vec2(2.0, 0.0), galley, ui.visuals().text_color());
}

/// 원본 TNXLabel(AutoSize False, Layout = tlBottom): 칸 안에서 글을 아래에 맞춰 그린다(넘치면 위가 잘린다).
fn bottom_label(ui: &egui::Ui, rect: egui::Rect, mut job: LayoutJob) {
    job.wrap.max_width = rect.width();
    let galley = ui.painter().layout_job(job);
    let top = rect.bottom() - galley.size().y;
    ui.painter().with_clip_rect(rect).galley(egui::pos2(rect.left(), top), galley, ui.visuals().text_color());
}

fn select_all(ui: &egui::Ui, id: egui::Id, length: usize) {
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(length))));
        state.store(ui.ctx(), id);
    }
}
