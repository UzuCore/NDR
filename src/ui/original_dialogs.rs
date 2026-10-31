//! Original dialog boxes: TfGetText (input), TfRename (rename), TfDelWarning (delete warning), TfDelConfirm (confirm deleting protected items).
//! Same rules as C# NDR's InputDialog, RenameWindow, DeleteWarningWindow, DeleteConfirmationWindow.
//! Drawn as egui modals; results are returned through a callback. Only the most recently opened (topmost) dialog is visible at a time (they can stack like an input box on top of a window).
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
    /// The window was closed without a button (original OnClose 0x41eea4 → 0x10): the whole delete stops.
    Cancel,
}

pub(super) enum DialogKind {
    /// `file_name`: bit 0 (+0x400) of the flags in the original TfGetText display FUN_0040bea0. If true, btnOKClick (0x428c64) blocks forbidden characters.
    Prompt { message: String, value: String, history: Option<Vec<String>>, password: bool, file_name: bool },
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
    /// Original TfCompareFolder (640×505), single window: compare, abort, save results, close.
    FolderCompare(Box<super::compare_dialog::FolderCompareForm>),
    Comparison(Box<super::compare_dialog::ComparisonForm>),
    Favorites(Box<super::favorites_dialog::FavoritesForm>),
    /// Original TfFolderTree (folder selection, F10).
    FolderTree(Box<super::folder_tree_dialog::FolderTreeForm>),
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
    /// Original TfFTPQuickConnect (quick connect in panel FTP mode).
    FtpQuickConnect(Box<super::ftp_pane::FtpQuickForm>),
    /// C# SettingsWindow.
    Settings(Box<super::settings_window::SettingsForm>),
    /// C# FunctionKeyEditWindow.
    FunctionKeyEdit(Box<super::function_key_editor::FunctionKeyEditForm>),

    /// C# FileConflictWindow.
    FileConflict(Box<super::file_operation::ConflictForm>),
    /// Original TfChecksum (title CRC/MD5/SHA1): result text (initially CALCULATING, ERROR on failure) and a Copy button that becomes enabled once the result is ready.
    Checksum { value: String, ready: bool },
}

/// C# ChoiceDialog: instruction text, a selection box (initial choice is `selected`), OK/Cancel.
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
    /// File list save window: Some(()) OK (btnSave, ModalResult 1), None Cancel.
    ListExport(Option<()>),
    EditShortcut(Option<String>),
    AdvancedRename(bool),
    Search(Option<ndr::tools::SearchCriteria>),
    /// Index of the chosen item (None on cancel).
    Choice(Option<usize>),
    /// (include subfolders, compare file contents).
    FolderCompare,
    /// Differences whose "Copy" was enabled in the sync preview (original order). None if the comparison result window is closed.
    Comparison(Option<Vec<ndr::folder_compare::Difference>>),
    /// (files to delete, whether to also empty the recycle bin). None on cancel.
    /// Saved (name, path) fields. None on cancel.
    Favorites(Option<Vec<(String, String)>>),
    /// Chosen folder. None on cancel.
    FolderTree(Option<PathBuf>),
    /// (user name (leading/trailing whitespace trimmed), password). None on cancel.
    Credentials(Option<(String, String)>),
    /// Chosen destination folder (full path). None on cancel.
    Destination(Option<PathBuf>),
    ArchiveCreate(Option<super::archive_dialog::ArchiveChoice>),
    /// Disk copy window was closed (C# MainWindow does not use the ShowDialog result).
    DiskCopy,
    Remote,
    RemoteTransfer,
    /// (action, permission bits). None on cancel.
    RemotePermissions(Option<(super::remote_window::PermissionAction, u32)>),
    /// Saved site list and the index of the site to connect to. None on cancel.
    RemoteSites(Option<(Vec<super::remote_window::SiteProfile>, Option<usize>)>),
    SiteEditor(Option<super::remote_window::SiteProfile>),
    /// Connection input values. None on cancel.
    FtpQuickConnect(Option<super::ftp_pane::FtpQuickForm>),
    /// Confirmed copy of the settings. None on cancel.
    Settings(Option<ndr::settings::Settings>),
    FunctionKeyEdit(Option<super::function_key_editor::Binding>),

    /// (chosen answer, apply to all). None (= cancel) if the window is closed.
    FileConflict(Option<(ndr::fsops::ConflictChoice, bool)>),
    /// Checksum window was closed.
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

/// Input state of the C# AttributeTimeWindow. Attribute fields have three states (None = unchanged, the initial state).
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
    pub date: chrono::NaiveDate,
    pub time: chrono::NaiveTime,
}

impl AttributeForm {
    /// Original FUN_0043657c → FUN_0041a0e8: for each item, set the checkboxes to a two-state value from that item's attributes
    /// (GetFileAttributesW: 0x20 A, 1 R, 2 H, 4 S), and the date/time fields to that item's modification time (files GetFileSystemTime, folders GetDirSystemTime, local time).
    /// Changed: the original used the previous item's time (1899-12-30 for the first) when it could not open the file → now uses the current time.
    pub(super) fn new(path: &str) -> Self {
        use std::os::windows::fs::MetadataExt;
        let metadata = std::fs::metadata(path).ok();
        let attributes = metadata.as_ref().map_or(0, |m| m.file_attributes());
        let modified = metadata.and_then(|m| m.modified().ok()).map_or_else(chrono::Local::now, chrono::DateTime::<chrono::Local>::from);
        let bit = |mask: u32| Some(attributes & mask != 0);
        Self {
            path: path.into(),
            archive: bit(0x20),
            read_only: bit(0x1),
            hidden: bit(0x2),
            system: bit(0x4),
            recursive: false,
            apply_to_folders: true,
            change_time: false,
            date: modified.date_naive(),
            time: chrono::Timelike::with_nanosecond(&modified.time(), 0).unwrap_or_else(|| modified.time()),
        }
    }

    /// Original FUN_0041a200 (checkboxes → attributes) and FUN_0041a264 (date/time fields → local time → FILETIME). The time is changed only when chkChangeTime is set.
    pub(super) fn changes(&self) -> Result<ndr::tools::AttributeChanges, ()> {
        let modified = if self.change_time {
            let local = self.date.and_time(self.time).and_local_timezone(chrono::Local).earliest().ok_or(())?;
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

/// Original TCheckBox (no AllowGrayed): toggles between two states.
fn cycle(state: &mut Option<bool>) {
    *state = Some(*state != Some(true));
}

pub(super) type DialogCallback = Box<dyn FnOnce(&mut App, DialogResult, &egui::Context)>;

pub(super) struct OriginalDialog {
    pub title: String,
    pub kind: DialogKind,
    pub serial: u64,
    pub focused: bool,
    on_result: Option<DialogCallback>,
}

/// Converts Delphi TOpenDialog.Filter ("name|pattern|name|pattern") into (name, pattern) pairs. An unpaired trailing fragment is discarded.
/// Reads the BMP (BI_RGB, 4/8/24-bit, bottom-up) in DFM Glyph.Data. Like VCL TSpeedButton (TButtonGlyph), treats the color of the bottom-left pixel as
/// transparent.
pub(super) fn decode_glyph(bmp: &[u8]) -> Option<egui::ColorImage> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(bmp.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(bmp.get(at..at + 4)?.try_into().ok()?));
    if bmp.get(0..2)? != b"BM" || u32_at(30)? != 0 {
        return None;
    }
    let (offset, header) = (u32_at(10)? as usize, u32_at(14)? as usize);
    let (width, height) = (u32_at(18)? as i32, u32_at(22)? as i32);
    let bits = u16_at(28)? as usize;
    if width <= 0 || height == 0 || width > 256 || height.abs() > 256 || ![4, 8, 24].contains(&bits) {
        return None;
    }
    let (width, rows) = (width as usize, height.unsigned_abs() as usize);
    let colors = match u32_at(46)? as usize {
        0 if bits <= 8 => 1 << bits,
        n => n,
    };
    let palette: Vec<Color32> = (0..if bits <= 8 { colors } else { 0 })
        .map(|i| bmp.get(14 + header + i * 4..14 + header + i * 4 + 3).map(|c| Color32::from_rgb(c[2], c[1], c[0])))
        .collect::<Option<_>>()?;
    let stride = (width * bits).div_ceil(32) * 4;
    let mut pixels = vec![Color32::TRANSPARENT; width * rows];
    for y in 0..rows {
        let source = if height > 0 { rows - 1 - y } else { y };
        let line = bmp.get(offset + source * stride..offset + source * stride + stride)?;
        for x in 0..width {
            pixels[y * width + x] = match bits {
                24 => Color32::from_rgb(line[x * 3 + 2], line[x * 3 + 1], line[x * 3]),
                8 => *palette.get(line[x] as usize)?,
                _ => *palette.get(((line[x / 2] >> if x % 2 == 0 { 4 } else { 0 }) & 0x0f) as usize)?,
            };
        }
    }
    let transparent = pixels[(rows - 1) * width];
    for pixel in &mut pixels {
        if *pixel == transparent {
            *pixel = Color32::TRANSPARENT;
        }
    }
    Some(egui::ColorImage { size: [width, rows], pixels, source_size: egui::vec2(width as f32, rows as f32) })
}

/// Texture for an original image button (created once per name).
pub(super) fn glyph_texture(ctx: &egui::Context, name: &str, bmp: &'static [u8]) -> Option<egui::TextureHandle> {
    let id = egui::Id::new(("ndr-glyph", name));
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let texture = ctx.load_texture(name, decode_glyph(bmp)?, egui::TextureOptions::NEAREST);
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
}

pub(super) fn delphi_filter(text: &str) -> Vec<(String, String)> {
    let parts: Vec<&str> = text.split('|').collect();
    parts.chunks_exact(2).map(|pair| (pair[0].to_owned(), pair[1].to_owned())).collect()
}

/// Original OD_PROGRAMFILTER (Filter of the program picker open dialog FUN_00417258, FUN_0041fca4, FUN_004506c8).
pub(super) fn program_filter(language: &compat::Language) -> Vec<(String, String)> {
    delphi_filter(&language.text("RES.OD_PROGRAMFILTER", "Program Files(*.exe)|*.exe|All Files(*.*)|*.*"))
}

/// Converts NXLabel markup into an egui text layout.
pub(super) fn markup_job(text: &str, size: f32, color: Color32, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    for span in ndr::markup::render(text) {
        let mut format = egui::TextFormat { font_id: FontId::proportional(size), color: span.color.map_or(color, |(r, g, b)| Color32::from_rgb(r, g, b)), ..Default::default() };
        if span.bold {
            // Bold font (Segoe UI Bold / Malgun Gothic Bold, the "bold" family in set_app_fonts). If there is no color tag, use the accent color like the C# version.
            format.color = if span.color.is_some() { format.color } else { Color32::WHITE };
            format.font_id = FontId::new(size, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()));
        }
        format.italics = span.italic;
        if span.underline {
            format.underline = egui::Stroke::new(1.0, format.color);
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

    /// Puts a window from another module (split, join, etc.) into the dialog queue.
    pub(super) fn push_original_dialog(&mut self, title: String, kind: DialogKind, on_result: impl FnOnce(&mut App, DialogResult, &egui::Context) + 'static) {
        self.push_dialog(title, kind, Box::new(on_result));
    }

    /// TfGetText(title, message, initial): the input value on OK, None on cancel. If `history` is present, the history list can be selected from.
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
            DialogKind::Prompt { message: message.into(), value: initial.into(), history, password, file_name: false },
            Box::new(move |app, result, ctx| {
                if let DialogResult::Text(value) = result {
                    on_result(app, value, ctx)
                }
            }),
        );
    }

    /// TfGetText with flags 1 (file name): new folder (FUN_00432378), empty file (FUN_00434630). If a forbidden character is present on confirm,
    /// shows MSG_INVALID_FILENAME and keeps the window open (btnOKClick 0x428c64, StrCSpnIW "\/:*?\"<>|").
    pub(super) fn prompt_file_name(&mut self, title: &str, message: &str, initial: &str, on_result: impl FnOnce(&mut App, Option<String>, &egui::Context) + 'static) {
        self.push_dialog(
            title.into(),
            DialogKind::Prompt { message: message.into(), value: initial.into(), history: None, password: false, file_name: true },
            Box::new(move |app, result, ctx| {
                if let DialogResult::Text(value) = result {
                    on_result(app, value, ctx)
                }
            }),
        );
    }

    /// TfRename: (new name, new extension) or None.
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

    /// C# DeleteWarningWindow.BuildMessage: for one item MSG_DELWARNING1 (mode text, file name), for several MSG_DELWARNING2 (mode text, count).
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

    /// TfDelWarning (original FUN_0040c5c4). The Delete button uses the recycle-bin mode and is present only when showdelbtn is set. defdelbtn selects the default button.
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

    /// TfDelConfirm (original FUN_0041ea78): Yes to All / Yes / No for a single item that has a protected attribute.
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

    /// C# ChoiceDialog.Show(title, prompt, choices, selected): the chosen index, None on cancel. If there is nothing to choose, it is not opened (C# returns null).
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

    /// Closes the topmost dialog with a result (shared by tests and buttons).
    pub(super) fn answer_dialog(&mut self, result: DialogResult, ctx: &egui::Context) {
        if self.original_dialogs.is_empty() {
            return;
        }
        let mut current = self.original_dialogs.pop().expect("a dialog is open");
        if let Some(callback) = current.on_result.take() {
            callback(self, result, ctx);
        }
    }

    /// C# AttributeTimeWindow (single item): Apply to All / Skip / OK / Cancel.
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

    /// Attribute window buttons: OK and Apply to All check the date/time and, if invalid, show a warning box (window title, warning icon) while keeping the window open.
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

    /// Changes the input of the currently open attribute window (for tests).
    #[cfg(test)]
    pub(super) fn edit_dialog_kind(&mut self, edit: impl FnOnce(&mut DialogKind)) {
        if let Some(dialog) = self.original_dialogs.last_mut() {
            edit(&mut dialog.kind);
        }
    }

    /// Changes the input of the currently open attribute window (for tests).
    #[cfg(test)]
    pub(super) fn edit_attribute_form(&mut self, edit: impl FnOnce(&mut AttributeForm)) {
        if let Some(DialogKind::Attributes(form)) = self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            edit(form);
        }
    }

    /// Changes the value of the name field of the currently open dialog (for tests).
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

    /// TfRename OK: the name must be non-empty and contain no forbidden characters. Otherwise shows MessageBoxW(MSG_INVALID_FILENAME, NULL caption, MB_OK) and keeps the window open.
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
        // If a message box is on top (e.g. a name error), this window is only drawn behind it and does not receive keys, buttons, or focus (like a Windows modal box).
        let covered = !self.msgboxes.is_empty();
        if self.original_dialogs.is_empty() {
            return;
        }
        let editing_shortcut = matches!(self.original_dialogs[self.original_dialogs.len() - 1].kind, DialogKind::EditShortcut(_));
        if editing_shortcut && !covered {
            self.capture_shortcut_keys(ctx);
        }
        let shortcut_lines = if editing_shortcut { self.shortcut_lines() } else { vec![] };
        // Temporarily pop the topmost dialog so self can be borrowed while drawing.
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
        // Original TfGetText buttons (formGetText.Caption.btnOk, btnCancel).
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
        let mut favorites_action = None;
        let mut folder_tree_action = None;
        let tree_colors = matches!(dialog.kind, DialogKind::FolderTree(_)).then(|| super::folder_tree_dialog::TreeColors::read(&self.settings));
        if let DialogKind::FolderTree(form) = &mut dialog.kind {
            if form.drive_icons.len() != form.drives.len() {
                let roots: Vec<PathBuf> = form.drives.iter().map(|(r, _)| r.clone()).collect();
                form.drive_icons = roots.iter().map(|r| self.drive_icon(ctx, r)).collect();
                form.folder_icon = self.path_icon(ctx, Path::new("folder"), true);
            }
        }
        let tree_icons: Vec<Option<egui::TextureHandle>> = if tree_colors.is_some() {
            ["tb_desktop.png", "tb_documents.png", "tb_pictures.png", "tb_music.png", "tb_videos.png"].iter().map(|name| self.toolbar_icon(ctx, name)).collect()
        } else {
            vec![]
        };
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
        // If a modal progress window (e.g. a custom cleanup search) is on top, this window is only drawn and does not receive keys or buttons.
        let waiting = covered || self.job.as_ref().is_some_and(|j| j.modal.is_some());
        // TfRename: forbidden characters typed in are not accepted (paste is checked at OK).
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
                DialogKind::Prompt { message, value, history, password, .. } => {
                    // Original TfGetText (ClientWidth 444 × ClientHeight 127): lMessage (18,8 411×53, bottom aligned), cbText (16,67 413×21),
                    // btnOK (265,95 79×25, default), btnCancel (351,95 79×25, cancel).
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
                    // Original TfRename (444 × 132): lMessage (18,8 410×53, bottom aligned), cbName (16,67 334×21, 260 chars), Label1 "." (355,70),
                    // cbExt (362,67 69×21), btnOK (270,100 75×25), btnCancel (357,99 75×25).
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
                    // Original TfDelWarning (443 × 119): Image1 (13,16), lMsg (64,16 364×62), btnDelete (64,86 75×25, hidden initially),
                    // btnYes (272,86), btnNo (353,86). If the text is longer than lMsg, the window grows by that much (a long overflowing list scrolls).
                    // Original TntFormShow (0x41f018): window width = lMsg text width (CaptionWidth) + lMsg.Left (64) + 20, clamped to screen − 50 if wider than the screen,
                    // and to 350 if narrower than 350 (outer width including the border). lMsg anchors left/top/right; Yes/No anchor top/right. Height is unchanged (NDR
                    // grows when there are many lines and scrolls on overflow). Changed: previously the width was fixed at 443, so a long name wrapped.
                    let natural = ui.painter().layout_job(markup_job(message, 13.0, text_color, f32::INFINITY)).size().x;
                    // Subtracting the bsDialog border (2 × SM_CXDLGFRAME 3) from the outer width gives the inner width. (+0x50 is the Left seen via SetBounds 0x662c80.)
                    let width = (natural + 64.0 + 20.0).min(screen.width() - 50.0).max(350.0) - 6.0;
                    let message_width = width - 79.0;
                    let galley = ui.painter().layout_job(markup_job(message, 13.0, text_color, message_width));
                    let extra = (galley.size().y.min(560.0) - 62.0).max(0.0);
                    let (area, _) = ui.allocate_exact_size(egui::vec2(width, 119.0 + extra), egui::Sense::hover());
                    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
                    if let Some(icon) = warning_icon.clone() {
                        ui.painter().image(icon.id(), at(13.0, 16.0, 32.0, 32.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    ui.scope_builder(egui::UiBuilder::new().max_rect(at(64.0, 16.0, message_width, 62.0 + extra)), |ui| {
                        egui::ScrollArea::vertical().max_height(62.0 + extra).show(ui, |ui| {
                            ui.label(markup_job(message, 13.0, text_color, message_width));
                        });
                    });
                    // Tab order btnYes, btnNo, btnDelete.
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
                    let (yes, yes_pressed) = vcl_button(ui, at(width - 171.0, 86.0 + extra, 75.0, 25.0), true, &raw("Caption.btnYes", "&Yes"));
                    let (no, no_pressed) = vcl_button(ui, at(width - 90.0, 86.0 + extra, 75.0, 25.0), true, &raw("Caption.btnNo", "&No"));
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
                    // ChoiceDialog: 440 wide, message (14), selection box, OK/Cancel at the bottom right. Enter is OK.
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
                    let drive_icons: Vec<Option<egui::TextureHandle>> = form.drives.iter().map(|d| self.drive_icon(ctx, Path::new(&d.root))).collect();
                    destination_action = super::destination_dialog::destination_ui_original(ui, form, enter, icon.as_ref(), &drive_icons, screen.height());
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
                    // Original lvFunctionKey OnKeyDown: Delete = delete (not received during a message box or modal operation).
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
                    // Original TfChecksum DFM (client 288×92): centered borderless read-only eChecksum (20,24, 249×17),
                    // btnCopy (48,56, 193×25) becomes enabled and takes focus once the result is ready (Enter = copy). Esc = close (FormKeyDown).
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
                DialogKind::FolderTree(form) => {
                    // FUN_00423e00: if tmainwndsize, use the file list window size; otherwise if tmaximized, maximize; otherwise twidth×theight (outer size).
                    let full = egui::vec2((screen.width() - 24.0).max(657.0), (screen.height() - 64.0).max(400.0));
                    let size = if self.settings.flag("TreeWindowMatchesMainSize", true) || self.settings.flag("TreeWindowMaximized", false) {
                        full
                    } else {
                        let number = |key: &str, default: f64| self.settings.values.get(key).and_then(|v| v.as_f64()).unwrap_or(default) as f32;
                        egui::vec2((number("TreeWindowWidth", 800.0) - 16.0).clamp(400.0, full.x), (number("TreeWindowHeight", 600.0) - 39.0 - 24.0).clamp(300.0, full.y))
                    };
                    let colors = tree_colors.unwrap_or_else(|| super::folder_tree_dialog::TreeColors::read(&self.settings));
                    folder_tree_action = super::folder_tree_dialog::folder_tree_ui(ui, form, &self.language_pack, &colors, size, !waiting, &tree_icons);
                }
                DialogKind::Comparison(form) => {
                    // FolderComparisonWindow: 1120×700 (shrunk on a small screen).
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
                    // Original TfChangeAttr (ClientWidth 437 × ClientHeight 288): lFileName (16,16), gbAttributes (16,44 177×161: chkArchive (red),
                    // chkReadOnly, chkHidden, chkSystem at left 24, top 34/59/85/111), gbTime (212,44 209×161: chkChangeTime (16,26), lDate (16,57)
                    // dtpDate (16,76 145×21), lTime (16,103) dtpTime (16,122)), chkRecursive (16,219), chkApplyToFolder (212,219),
                    // btnAll (95,255) btnSkip (176,255) btnOk (264,255) btnCancel (345,255) 75×25.
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
                    // Original dtpDate (TDateTimePicker dtkDate) and dtpTime (dtkTime) 16,76 / 16,122 145×21, enabled by chkChangeTimeClick (0x41a1a4).
                    super::date_time_picker::date_picker(ui, at(228.0, 120.0, 145.0, 21.0), enabled, "change-attr-date", &mut form.date);
                    put_left(ui, at(228.0, 147.0, 145.0, 15.0), egui::Label::new(text("Caption.lTime", "Time")));
                    super::date_time_picker::time_picker(ui, at(228.0, 166.0, 145.0, 21.0), enabled, "change-attr-time", &mut form.time);
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
                    // Original TntFormKeyDown (0x41a2f4, KeyPreview): End (0x23) = btnAll (+0x40c).Click.
                    if !waiting && ctx.input(|i| i.key_pressed(egui::Key::End)) {
                        outcome = Outcome::Choice(0);
                    }
                }
                DialogKind::DeleteConfirm { message, attributes } => {
                    // Original TfDelConfirm (434 × 138): Image1 (16,13), lMsg (80,13 337×58), lAttributes (80,80),
                    // btnAll (172,101 75×25), btnYes (256,101), btnNo (342,101).
                    let (area, _) = ui.allocate_exact_size(egui::vec2(434.0, 138.0), egui::Sense::hover());
                    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
                    if let Some(icon) = warning_icon.clone() {
                        ui.painter().image(icon.id(), at(16.0, 13.0, 32.0, 32.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    super::original_dialogs::put_left(ui, at(80.0, 13.0, 337.0, 58.0), egui::Label::new(message.as_str()).wrap());
                    super::original_dialogs::put_left(ui, at(80.0, 80.0, 337.0, 15.0), egui::Label::new(attributes.as_str()).truncate());
                    let raw = |key: &str, fallback: &str| self.language_pack.raw(&format!("formDelConfirm.{key}")).unwrap_or_else(|| fallback.to_owned());
                    // Original: btnAll with tab order 0 takes focus first (TntFormShow 0x41ea78 does not call SetFocus), so Enter = Yes to All.
                    // Changed: previously there was no focus, so Enter did nothing.
                    for (left, slot, key, fallback) in [(172.0, 0, "Caption.btnAll", "Yes to &All"), (256.0, 1, "Caption.btnYes", "&Yes"), (342.0, 2, "Caption.btnNo", "&No")] {
                        let (button, pressed) = vcl_button(ui, at(left, 101.0, 75.0, 25.0), true, &raw(key, fallback));
                        if first_frame && slot == 0 {
                            button.request_focus();
                        }
                        if pressed && matches!(outcome, Outcome::None) {
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
        if let Some(action) = favorites_action {
            return self.favorites_action(action, ctx);
        }
        if let Some(action) = folder_tree_action {
            return self.folder_tree_action(action, ctx);
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
        // The folder tree window handles Esc itself (cancels scanning if a scan is in progress).
        // Original TfDelConfirm has no button with Cancel = True (DFM btnAll/btnYes/btnNo), so Esc does nothing there.
        let escape = escape && !self.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::FolderTree(_) | DialogKind::DeleteConfirm { .. }));
        if escape && matches!(outcome, Outcome::None) {
            // C# AttributeTimeWindow: Esc is Skip.
            outcome = if self.original_dialogs.last().is_some_and(|d| matches!(d.kind, DialogKind::Attributes(_))) { Outcome::Choice(1) } else { Outcome::Cancel };
        }
        let Some(kind) = self.original_dialogs.last().map(|d| &d.kind) else { return };
        let result = match (outcome, kind) {
            (Outcome::None, _) => return,
            (Outcome::Ok, DialogKind::Rename { .. }) => return self.submit_rename(ctx),
            (Outcome::Ok, DialogKind::Prompt { value, file_name: true, .. }) if value.contains(|c| INVALID_NAME_CHARACTERS.contains(c)) => {
                let text = self.language_pack.text("RES.MSG_INVALID_FILENAME", "A file name can't contain any of the following characters:\n  \\ / : * ? \" < > | ");
                self.system_box("", &text, ndr::msgbox::MB_OK, |_, _, _| {});
                return;
            }
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
            // ×/Esc = close button (original btnCancel Cancel=True). If comparing, stop and close.
            (_, DialogKind::FolderCompare(_)) => return self.folder_compare_action(super::compare_dialog::FolderCompareAction::Close, ctx),
            (_, DialogKind::Comparison(_)) => DialogResult::Comparison(None),
            (_, DialogKind::Favorites(_)) => DialogResult::Favorites(None),
            (_, DialogKind::FolderTree(_)) => return self.folder_tree_action(super::folder_tree_dialog::FolderTreeAction::Cancel, ctx),
            (Outcome::Ok, DialogKind::Credentials { user, password }) => DialogResult::Credentials(Some((user.trim().to_owned(), password.clone()))),
            (_, DialogKind::Credentials { .. }) => DialogResult::Credentials(None),
            (_, DialogKind::Destination(_)) => DialogResult::Destination(None),
            (Outcome::Choice(choice), DialogKind::ArchiveCreate(_)) => return self.submit_archive_create(choice, ctx),
            (_, DialogKind::ArchiveCreate(_)) => DialogResult::ArchiveCreate(None),
            // ×/Esc = close button (does not close while an operation is running).
            (_, DialogKind::DiskCopy(_)) => return self.disk_copy_action(super::disk_copy_dialog::DiskAction::Close, ctx),
            // ×/Esc: for the remote window the close button, for the transfer window an abort request, × on the permissions window is cancel, and the sites window cancels.
            (_, DialogKind::Remote(_)) => return self.remote_action(super::remote_window::RemoteAction::Close, ctx),
            (_, DialogKind::RemoteTransfer(_)) => return self.transfer_close_requested(),
            (_, DialogKind::RemotePermissions(_)) => DialogResult::RemotePermissions(None),
            (_, DialogKind::RemoteSites(_)) => DialogResult::RemoteSites(None),
            (_, DialogKind::SiteEditor(_)) => DialogResult::SiteEditor(None),
            (_, DialogKind::FtpQuickConnect(_)) => DialogResult::FtpQuickConnect(None),
            (_, DialogKind::Settings(_)) => DialogResult::Settings(None),
            (_, DialogKind::FunctionKeyEdit(_)) => DialogResult::FunctionKeyEdit(None),

            (_, DialogKind::FileConflict(_)) => DialogResult::FileConflict(None),
            // Original btnCopyClick: copy the result text to the clipboard (window stays open).
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
            // Original TfDelConfirm: btnAll 8, btnYes 4, btnNo 2 (0x41ee68/0x41ee7c/0x41ee90); closing with × leaves 0 and OnClose (0x41eea4) makes it 0x10.
            (Outcome::Cancel, DialogKind::DeleteConfirm { .. }) => DialogResult::DeleteConfirm(DeleteConfirmChoice::Cancel),
            (_, DialogKind::DeleteConfirm { .. }) => DialogResult::DeleteConfirm(DeleteConfirmChoice::No),
            (_, DialogKind::Prompt { .. }) => DialogResult::Text(None),
        };
        self.answer_dialog(result, ctx);
    }
}

/// Select all when an input field is opened (C# SelectAll).
/// Places a widget filling and centered in the rect like ui.put, but allows it to be disabled (original Enabled = False button).
pub(super) fn put_enabled(ui: &mut egui::Ui, rect: egui::Rect, enabled: bool, widget: impl egui::Widget) -> egui::Response {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)), |ui| ui.add_enabled(enabled, widget)).inner
}

/// VCL CM_DIALOGCHAR: the accelerator character typed this frame. Cleared once a button accepts it.
const DIALOG_CHAR: &str = "ndr-dialog-char";

/// Determines this frame's accelerator character: Alt+letter always, a bare letter only when no text input field has focus (like a VCL window where a button or checkbox has focus).
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
    // Changed: egui's wants_keyboard_input is also true for a focused button, so a bare-letter accelerator (Delete (&D), etc.) did not work.
    // Like VCL, pass an Alt-less letter through only when a text input field (focus with TextEdit state) is focused.
    let text_focus = ctx.memory(|m| m.focused()).is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some());
    let typed = typed.filter(|(_, alt)| *alt || !text_focus).map(|(c, _)| c);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DIALOG_CHAR), typed));
}

/// If the accelerator character is `access`, take it (so two buttons with the same letter are not both triggered).
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

/// VCL Caption text: underline the letter after '&'. (text, accelerator character). If `color` is None, the widget text color.
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

/// VCL TButton ('&' accelerator character in Caption, Enabled). (response, whether pressed: clicked or accelerator character).
pub(super) fn vcl_button(ui: &mut egui::Ui, rect: egui::Rect, enabled: bool, caption: &str) -> (egui::Response, bool) {
    let (job, access) = access_job(ui, caption);
    let response = put_enabled(ui, rect, enabled, egui::Button::new(job));
    let pressed = response.clicked() || (enabled && ui.is_enabled() && take_dialog_char(ui.ctx(), access));
    (response, pressed)
}

/// VCL TButton ('&' accelerator character in Caption). true if clicked or the accelerator character is typed.
pub(super) fn access_button(ui: &mut egui::Ui, rect: egui::Rect, caption: &str) -> bool {
    vcl_button(ui, rect, true, caption).1
}

/// VCL TRadioButton ('&' accelerator character in Caption). true if clicked or the accelerator character is typed.
pub(super) fn access_radio(ui: &mut egui::Ui, rect: egui::Rect, selected: bool, caption: &str) -> bool {
    let (job, access) = access_job(ui, caption);
    let clicked = put_left(ui, rect, egui::RadioButton::new(selected, job)).clicked();
    clicked || (ui.is_enabled() && take_dialog_char(ui.ctx(), access))
}

/// VCL TLabel ('&' accelerator character in Caption, FocusControl): typing the accelerator character gives focus to the `target` field.
pub(super) fn access_label(ui: &mut egui::Ui, rect: egui::Rect, caption: &str, target: egui::Id) {
    let (job, access) = access_job(ui, caption);
    put_left(ui, rect, egui::Label::new(job).truncate());
    if ui.is_enabled() && take_dialog_char(ui.ctx(), access) {
        ui.memory_mut(|m| m.request_focus(target));
    }
}

/// VCL TCheckBox ('&' accelerator character in Caption): typing the accelerator character toggles the value. true if it changed.
pub(super) fn access_checkbox(ui: &mut egui::Ui, rect: egui::Rect, value: &mut bool, caption: &str) -> bool {
    let (job, access) = access_job(ui, caption);
    let mut changed = put_left(ui, rect, egui::Checkbox::new(value, job)).changed();
    if ui.is_enabled() && take_dialog_char(ui.ctx(), access) {
        *value = !*value;
        changed = true;
    }
    changed
}

/// Original TLabel/TCheckBox placement (top-left aligned). Since ui.put stretches and centers in the rect, labels and checkboxes are placed with this.
pub(super) fn put_left(ui: &mut egui::Ui, rect: egui::Rect, widget: impl egui::Widget) -> egui::Response {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Min)), |ui| ui.add(widget)).inner
}

/// Original TLabel EllipsisPosition = epPathEllipsis (DT_PATH_ELLIPSIS): when it exceeds the width, keep the final "\name" and shorten what precedes it to "...".
/// If even the final name alone overflows, truncate the end with "...".
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

/// Original TGroupBox: a frame border (starting half a text height down) and the title text at top-left 8.
pub(super) fn group_box(ui: &egui::Ui, rect: egui::Rect, caption: &str) {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let frame = egui::Rect::from_min_max(rect.min + egui::vec2(0.0, 7.0), rect.max);
    ui.painter().rect_stroke(frame, 2.0, stroke, egui::StrokeKind::Inside);
    let galley = ui.painter().layout_no_wrap(caption.to_owned(), egui::FontId::proportional(13.0), ui.visuals().text_color());
    let text_rect = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 0.0), galley.size() + egui::vec2(4.0, 0.0));
    ui.painter().rect_filled(text_rect, 0.0, ui.visuals().window_fill);
    ui.painter().galley(text_rect.min + egui::vec2(2.0, 0.0), galley, ui.visuals().text_color());
}

/// Original TNXLabel (AutoSize False, Layout = tlBottom): draws the text bottom-aligned within the rect (the top is clipped on overflow).
/// TNXLabel.DoDrawText 0x7c608c: Layout tlCenter/tlBottom moves the first line to (height - text height) / 2 or height - text height;
/// each line (empty ones too) advances by line height + LineSpace; Alignment right/center uses Width - TextWidth(line).
/// A disabled label draws the plain text twice with DrawTextW (clBtnHighlight at +1,+1, then clBtnShadow).
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
