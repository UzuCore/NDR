//! C# FileConflictWindow (name conflict prompt), FileOperationProgressWindow (file job progress), FileOperationErrorDialog (continue after each error),
//! ReportFileOperationResult (copy, move, paste results), delete and shred results, TryExternalCopyAsync (external copy program).
//! Difference (HANDOFF): an English UI also shows in English the texts C# fixed to Korean (progress window buttons and states, result notices).
//! Progress windows of archive, split, join and folder size jobs have no pause button (those jobs have no pause points).
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{IDYES, MB_ICONEXCLAMATION, MB_ICONHAND, MB_ICONQUESTION, MB_OK, MB_YESNO};
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::time::SystemTime;

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// C# FileConflictInfo + the apply-to-all checkbox.
pub(super) struct ConflictForm {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub source_dir: bool,
    pub destination_dir: bool,
    pub source_size: Option<u64>,
    pub destination_size: Option<u64>,
    pub source_modified: Option<SystemTime>,
    pub destination_modified: Option<SystemTime>,
    pub allow_resume: bool,
    /// One side is a remote (FTP/SFTP) item: original FUN_00467764 shows btnOverNewer and hides btnReplace.
    pub remote: bool,
    pub apply_all: bool,
    /// Original imgOrg / imgNew (32x32 shell icons): RGBA and drawn textures of [existing, new].
    pub icons: [Option<(usize, usize, Vec<u8>)>; 2],
    pub textures: [Option<egui::TextureHandle>; 2],
}

/// Info about a conflict target that is not a real file (archive or remote item): (shown path, is folder, size, modified time, remote).
/// The job thread sets it before asking and removes it after the answer.
type VirtualEntry = (PathBuf, bool, Option<u64>, Option<SystemTime>, bool);
static VIRTUAL_ENTRIES: std::sync::Mutex<Vec<VirtualEntry>> = std::sync::Mutex::new(Vec::new());

fn put_virtual_entry(path: &Path, info: Option<(bool, Option<u64>, Option<SystemTime>)>, remote: bool) {
    let mut entries = VIRTUAL_ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
    entries.retain(|(p, ..)| p != path);
    if let Some((directory, size, modified)) = info {
        entries.push((path.to_path_buf(), directory, size, modified, remote));
    }
}

/// Archive item shown in the conflict window.
pub(super) fn set_virtual_entry(path: &Path, info: Option<(bool, Option<u64>, Option<SystemTime>)>) {
    put_virtual_entry(path, info, false);
}

/// Remote (FTP/SFTP) item shown in the conflict window; the window then uses the original FTP button set (FUN_00467764).
pub(super) fn set_remote_entry(path: &Path, info: Option<(bool, Option<u64>, Option<SystemTime>)>) {
    put_virtual_entry(path, info, true);
}

fn is_remote_entry(path: &Path) -> bool {
    VIRTUAL_ENTRIES.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(p, .., remote)| *remote && p == path)
}

fn describe(path: &Path) -> (bool, Option<u64>, Option<SystemTime>) {
    // For archive and remote items, the info set by the job thread wins (so "/dir/a" is not resolved to a real file on the current drive).
    if let Some((_, directory, size, modified, _)) = VIRTUAL_ENTRIES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(p, ..)| p == path) {
        return (*directory, *size, *modified);
    }
    match fs::symlink_metadata(path) {
        Ok(m) => {
            let attributes = m.file_attributes();
            let directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
            let reparse = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
            (directory, (!directory && !reparse).then(|| m.len()), m.modified().ok())
        }
        Err(_) => (false, None, None),
    }
}

impl ConflictForm {
    pub(super) fn read(source: &Path, destination: &Path, allow_resume: bool) -> Self {
        let (source_dir, source_size, source_modified) = describe(source);
        let (destination_dir, destination_size, destination_modified) = describe(destination);
        let remote = is_remote_entry(source) || is_remote_entry(destination);
        Self { source: source.into(), destination: destination.into(), source_dir, destination_dir, source_size, destination_size, source_modified, destination_modified, allow_resume, remote, apply_all: false, icons: [None, None], textures: [None, None] }
    }
    /// C# CanResume.
    pub(super) fn can_resume(&self) -> bool {
        self.allow_resume && !self.source_dir && !self.destination_dir && matches!((self.source_size, self.destination_size), (Some(s), Some(d)) if d < s)
    }
    /// C# ConflictPolicy.IsCompatible: whether an apply-to-all answer also fits this conflict.
    pub(super) fn compatible(&self, choice: ConflictChoice) -> bool {
        match choice {
            ConflictChoice::Resume => self.can_resume(),
            ConflictChoice::ReplaceFolder => self.source_dir || self.destination_dir,
            _ => true,
        }
    }
}

/// Original FUN_00422068: file info text "<b>%s</b> bytes ( long date, time )" (size via FloatToStrF ffNumber = thousands separators).
/// With highlight, <c0000FF>...</c> like FUN_00422314 (red, the newer side).
pub(super) fn conflict_file_info(size: Option<u64>, modified: Option<SystemTime>, highlight: bool, ko: bool) -> String {
    let bytes = size.map(|s| format!("<b>{}</b> bytes", n0(s as usize))).unwrap_or_else(|| (if ko { "크기 확인 불가" } else { "Size unavailable" }).to_owned());
    let info = match modified.map(ndr::listing::long_date_and_time) {
        Some((date, time)) => format!("{bytes} ( {date}, {time} )"),
        None => bytes,
    };
    if highlight { format!("<c0000FF>{info}</c>") } else { info }
}

/// Integer with thousands separators (C# N0).
pub(super) fn n0(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Buttons of the original TfFileExists (right column in the DFM, left 425, width 112, height 25).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConflictButton {
    Resume,
    Overwrite,
    Skip,
    OverwriteNewer,
    ReplaceFolder,
    Rename,
    Cancel,
}

impl ConflictButton {
    /// DFM Top (original btnResume 25, btnOverwrite 56, btnSkip 87, btnOverNewer/btnReplace 118, btnRename 149, btnAbort 220).
    pub(super) fn top(self) -> f32 {
        match self {
            Self::Resume => 25.0,
            Self::Overwrite => 56.0,
            Self::Skip => 87.0,
            Self::OverwriteNewer | Self::ReplaceFolder => 118.0,
            Self::Rename => 149.0,
            Self::Cancel => 220.0,
        }
    }
    pub(super) fn choice(self) -> ConflictChoice {
        match self {
            Self::Resume => ConflictChoice::Resume,
            Self::Overwrite => ConflictChoice::Overwrite,
            Self::Skip => ConflictChoice::Skip,
            Self::OverwriteNewer => ConflictChoice::OverwriteNewer,
            Self::ReplaceFolder => ConflictChoice::ReplaceFolder,
            Self::Rename => ConflictChoice::Rename,
            Self::Cancel => ConflictChoice::Cancel,
        }
    }
}

/// xilesShell::IsSubFolder(parent, child) (0x7b543c): true when child is inside parent (not the same path). Case-insensitive.
pub(super) fn is_sub_folder(parent: &Path, child: &Path) -> bool {
    let normal = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let (parent, child) = (normal(parent), normal(child));
    child.len() > parent.len() && child.starts_with(&parent) && child.as_bytes().get(parent.len()) == Some(&b'\\')
}

/// Visible buttons and their enabled state. Original FUN_004116dc (local copy): btnReplace when the new item is a folder, otherwise btnOverNewer (same position, Visible).
/// FUN_00422870 only disables (Enabled = False, virtual SetEnabled +0x94): when existing = new path, overwrite, overwrite newer and replace folder are disabled;
/// when replace folder is visible, it is disabled if the new item is inside the existing one, and replace folder and overwrite are disabled if the existing one is inside the new item. Resume (btnResume,
/// DFM Visible = False) shows for resumable jobs like the C# version and is enabled only when the existing file is smaller than the new one, like the original.
/// Fixed (user report): unusable buttons used to be hidden, which made the window look empty.
pub(super) fn conflict_buttons(form: &ConflictForm) -> Vec<(ConflictButton, bool)> {
    let same = form.source.to_string_lossy().eq_ignore_ascii_case(&form.destination.to_string_lossy());
    let mut overwrite = !same;
    let mut replace = !same;
    if form.source_dir {
        if is_sub_folder(&form.destination, &form.source) {
            replace = false;
        }
        if is_sub_folder(&form.source, &form.destination) {
            replace = false;
            overwrite = false;
        }
    }
    let mut buttons = Vec::new();
    if form.allow_resume {
        buttons.push((ConflictButton::Resume, form.can_resume()));
    }
    buttons.push((ConflictButton::Overwrite, overwrite));
    buttons.push((ConflictButton::Skip, true));
    // Original FTP queue (FUN_00467764): btnOverNewer Visible = 1 and btnReplace Visible = 0, whatever the item kinds.
    // Fixed (r169): the FTP window used to show "replace folder" for a remote file against a local folder.
    if form.source_dir && !form.remote {
        buttons.push((ConflictButton::ReplaceFolder, replace));
    } else {
        buttons.push((ConflictButton::OverwriteNewer, !same));
    }
    buttons.push((ConflictButton::Rename, true));
    buttons.push((ConflictButton::Cancel, true));
    buttons
}

/// Original TfFileExists (ClientWidth 545 x ClientHeight 253) layout: lMsg (20,13), lFileName (20,34 bold),
/// Panel1(17,56 396×155: lOrgLabel·imgOrg·lOrgDir·lOrgFile, Bevel1, lNewLabel·imgNew·lNewDir·lNewFile), chkApplyAll(20,221),
/// right button column (conflict_buttons). Enter = skip (C# version), Esc = cancel (close the window).
pub(super) fn conflict_ui(ui: &mut egui::Ui, form: &mut ConflictForm, language: &compat::Language, ko: bool, enter: bool) -> Option<ConflictChoice> {
    let text = |key: &str, fallback: &str| language.text(&format!("formFileExists.{key}"), fallback);
    let (area, _) = ui.allocate_exact_size(egui::vec2(545.0, 253.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let text_color = ui.visuals().text_color();
    super::original_dialogs::put_left(ui, at(20.0, 13.0, 390.0, 15.0), egui::Label::new(text("Caption.lMsg", "A destination folder already contains a file/folder named")).truncate());
    let name = form.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    super::original_dialogs::put_left(ui, at(20.0, 34.0, 390.0, 17.0), egui::Label::new(RichText::new(name).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()))).truncate());
    // Panel1 (TNXPaintPanel, CustomDraw): bordered box.
    let panel = at(17.0, 56.0, 396.0, 155.0);
    ui.painter().rect(panel, 4.0, ui.visuals().faint_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    ui.painter().hline(panel.min.x + 11.0..=panel.min.x + 382.0, panel.min.y + 72.0, ui.visuals().widgets.noninteractive.bg_stroke);
    for (slot, icon) in form.icons.iter().enumerate() {
        if form.textures[slot].is_none() {
            if let Some((width, height, rgba)) = icon.as_ref().filter(|(w, h, rgba)| rgba.len() == w * h * 4) {
                let image = egui::ColorImage::from_rgba_unmultiplied([*width, *height], rgba);
                form.textures[slot] = Some(ui.ctx().load_texture(format!("conflict-icon-{slot}"), image, Default::default()));
            }
        }
    }
    // Original FUN_00422314: CompareFileTime(existing, new) wraps the newer side's info text in <c0000FF> (red).
    let newer = match (form.destination_modified, form.source_modified) {
        (Some(existing), Some(new)) if existing < new => Some(false),
        (Some(existing), Some(new)) if existing > new => Some(true),
        _ => None,
    };
    let rows = [
        (0.0, text("Caption.lOrgLabel", "Existing File:"), form.destination.clone(), form.destination_dir, form.destination_size, form.destination_modified, newer == Some(true)),
        (77.0, text("Caption.lNewLabel", "Copying File:"), form.source.clone(), form.source_dir, form.source_size, form.source_modified, newer == Some(false)),
    ];
    for (slot, (dy, label, path, directory, size, modified, highlight)) in rows.into_iter().enumerate() {
        let local = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(panel.min + egui::vec2(x, y + dy), egui::vec2(w, h));
        super::original_dialogs::put_left(ui, local(14.0, 9.0, 370.0, 15.0), egui::Label::new(RichText::new(label).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()))).truncate());
        if let Some(texture) = &form.textures[slot] {
            ui.painter().image(texture.id(), local(14.0, 31.0 - if slot == 1 { 1.0 } else { 0.0 }, 32.0, 32.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        // Original lOrgDir/lNewDir: the folder without the file name (with the trailing "\", FUN_00421f40), epPathEllipsis.
        let folder = path.parent().map(|p| { let mut s = p.to_string_lossy().into_owned(); if !s.ends_with('\\') { s.push('\\'); } s }).unwrap_or_default();
        let folder_font = egui::FontId::proportional(13.0);
        let folder = super::original_dialogs::path_ellipsis(ui, &folder, 324.0, &folder_font);
        super::original_dialogs::put_left(ui, local(62.0, 30.0, 324.0, 18.0), egui::Label::new(RichText::new(folder).font(folder_font)));
        // Original FUN_00422314: for a folder conflict (+0x460) the info text is the full path, otherwise the FUN_00422068 text. The newer side is red.
        let info = if directory {
            let full = path.to_string_lossy().into_owned();
            if highlight { format!("<c0000FF>{full}</c>") } else { full }
        } else {
            conflict_file_info(size, modified, highlight, ko)
        };
        super::original_dialogs::put_left(ui, local(62.0, if slot == 0 { 50.0 } else { 49.0 }, 324.0, 18.0), egui::Label::new(super::original_dialogs::markup_job(&info, 13.0, text_color, 324.0)).truncate());
    }
    // The original TfFileExists has only buttons and a checkbox, so typing a letter triggers the access key (VCL CM_DIALOGCHAR).
    let raw = |key: &str, fallback: &str| language.raw(&format!("formFileExists.{key}")).unwrap_or_else(|| fallback.to_owned());
    super::original_dialogs::access_checkbox(ui, at(20.0, 221.0, 390.0, 17.0), &mut form.apply_all, &raw("Caption.chkApplyAll", "&Apply to all"));
    let mut chosen = None;
    for (button, enabled) in conflict_buttons(form) {
        // Original FUN_00422314: for a folder conflict the overwrite button text is MERGE_FOLDER ("폴더 합치기(&G)").
        let label = match button {
            ConflictButton::Overwrite if form.source_dir && form.destination_dir => language.raw("RES.MERGE_FOLDER").unwrap_or_else(|| "Mer&ge".into()),
            ConflictButton::Overwrite => raw("Caption.btnOverwrite", "Over&write"),
            ConflictButton::Resume => raw("Caption.btnResume", "Res&ume"),
            ConflictButton::Skip => raw("Caption.btnSkip", "&Skip"),
            ConflictButton::OverwriteNewer => raw("Caption.btnOverNewer", "Overwrite N&ewer"),
            ConflictButton::ReplaceFolder => raw("Caption.btnReplace", "Replac&e Folder"),
            ConflictButton::Rename => raw("Caption.btnRename", "&Rename"),
            ConflictButton::Cancel => raw("Caption.btnAbort", "&Cancel"),
        };
        if super::original_dialogs::vcl_button(ui, at(425.0, button.top(), 112.0, 25.0), enabled, &label).1 && chosen.is_none() {
            chosen = Some(button.choice());
        }
    }
    if enter && chosen.is_none() {
        chosen = Some(ConflictChoice::Skip);
    }
    chosen
}

/// C# result notice style used when a job ends.
#[derive(Clone, Debug)]
pub(super) enum JobReport {
    /// Status line summary, plus an error text window when there are errors (archive, split, join, folder size and so on).
    Generic,
    /// C# ReportFileOperationResult(verb). requested = number of chosen items.
    Transfer { verb: (&'static str, &'static str), requested: usize, clear_clipboard: bool },
    /// End of C# DeleteSelectedAsync: skipped = protected items chosen to keep.
    Delete { recycle: bool, skipped: usize },
    /// End of C# ShredAsync.
    Shred,
}

/// C# FileOperationProgressWindow arguments.
#[derive(Clone, Debug, Default)]
pub(super) struct ProgressInfo {
    pub from: Option<String>,
    pub to: Option<String>,
    pub total_files: Option<u64>,
    pub total_bytes: Option<u64>,
    pub prefer_bytes: bool,
    /// Original progress window caption (COMPRESSING, EXTRACTING, SPLITTING, JOINING). Otherwise chosen by the job kind, or the job title.
    pub caption: Option<String>,
}

/// Original 0x412308 lTo: the folder of the current destination file. If the job reported the destination of the current source file (`current`) (Event::Target), that folder;
/// otherwise the same relative path below the starting source folder (`from`) appended to the starting destination folder (`to`) (`to` when it cannot be derived).
pub(super) fn progress_target_line(from: Option<&str>, to: Option<&str>, current: &str, target: Option<&(PathBuf, PathBuf)>) -> Option<String> {
    if let Some((_, destination)) = target.filter(|(source, _)| source.as_os_str() == std::ffi::OsStr::new(current)) {
        if let Some(folder) = destination.parent() {
            return Some(folder.to_string_lossy().into_owned());
        }
    }
    let folder = Some(Path::new(current)).filter(|p| p.is_absolute()).and_then(Path::parent);
    match (from, to, folder) {
        (Some(base), Some(target), Some(folder)) => match folder.strip_prefix(base) {
            Ok(relative) if relative.as_os_str().is_empty() => Some(target.to_owned()),
            Ok(relative) => Some(Path::new(target).join(relative).to_string_lossy().into_owned()),
            Err(_) => Some(target.to_owned()),
        },
        _ => to.map(str::to_owned),
    }
}



/// C# GetFileOperationSourcePath: a folder itself, or the folder containing a file.
pub(super) fn source_folder(path: &Path) -> Option<String> {
    let full = std::path::absolute(path).ok()?;
    let metadata = fs::metadata(&full).ok()?;
    let folder = if metadata.is_dir() { full } else { full.parent()?.to_path_buf() };
    Some(folder.to_string_lossy().into_owned())
}

/// C# TryGetFlatFileProgressTotals: (count, size) only when all are ordinary files.
pub(super) fn flat_totals(paths: &[PathBuf]) -> Option<(u64, u64)> {
    let mut bytes = 0u64;
    for path in paths {
        let m = fs::symlink_metadata(path).ok()?;
        if m.file_attributes() & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return None;
        }
        bytes = bytes.checked_add(m.len())?;
    }
    (!paths.is_empty()).then_some((paths.len() as u64, bytes))
}

/// Where copy/move was called from (the C# status texts differ).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransferKind {
    OtherPane,
    Folder,
    Paste,
    Drag,
}

impl App {
    /// Original FUN_0042bdc4 (copy/move) and FUN_0042c734 (delete): when a job is made from the marks (not drag-and-drop or paste), the pane's marks are
    /// saved to the "last selection" (+0xd08, CopySelection) and cleared (ClearSelection). Without marks the last selection becomes empty. Skin theme.
    pub(super) fn original_take_marks(&mut self, sources: &[PathBuf]) {
        let Some(parent) = sources.first().and_then(|p| p.parent()) else { return };
        let order = [self.active, 1 - self.active];
        let Some(i) = order.into_iter().find(|&i| {
            let pane = &self.panes[i];
            pane.archive.is_none() && pane.remote.is_none() && ndr::names::equals_ignore_case(&pane.path.to_string_lossy(), &parent.to_string_lossy())
        }) else {
            return;
        };
        self.original_take_pane_marks(i);
    }

    /// Variant of original_take_marks that knows the pane (drag-and-drop: original FUN_004856d8 does CopySelection + ClearSelection on the dragging list after the drop). Skin theme.
    pub(super) fn original_take_pane_marks(&mut self, i: usize) {
        if i > 1 {
            return;
        }
        let pane = &mut self.panes[i];
        self.last_selection[i] = pane.selected.iter().filter_map(|&row| pane.entries.get(row)).filter(|e| !e.drive && !e.is_parent()).map(|e| e.path.clone()).collect();
        pane.selected.clear();
    }

    /// C# RunLocalFileOperationWithProgress(title, paths, destination, move): progress window and result notice. If an external copy program
    /// is configured, hands over to it (C# TryExternalCopyAsync).
    pub(super) fn local_transfer(&mut self, sources: Vec<PathBuf>, destination: PathBuf, moving: bool, kind: TransferKind) {
        if matches!(kind, TransferKind::OtherPane | TransferKind::Folder) {
            self.original_take_marks(&sources);
        }
        if self.try_external_copy(&sources, &destination, moving, kind) {
            return;
        }
        self.remember_destination(&destination);
        let ko = self.language == "ko";
        let (title, verb) = match (moving, kind) {
            (true, _) => (if ko { "항목 이동" } else { "Move items" }, ("이동", "move")),
            (false, TransferKind::Paste) => (if ko { "항목 붙여넣기" } else { "Paste items" }, ("붙여넣기", "paste")),
            (false, _) => (if ko { "항목 복사" } else { "Copy items" }, ("복사", "copy")),
        };
        let totals = flat_totals(&sources);
        let progress = ProgressInfo {
            from: if sources.len() == 1 { source_folder(&sources[0]) } else { None },
            to: Some(destination.to_string_lossy().into_owned()),
            total_files: totals.map(|t| t.0),
            total_bytes: totals.map(|t| t.1),
            prefer_bytes: !moving,
            caption: None,
        };
        let report = JobReport::Transfer { verb, requested: sources.len(), clear_clipboard: moving && kind == TransferKind::Paste };
        self.start_job(title.into(), Operation::Copy { sources, destination, move_files: moving }, report, Some(progress));
    }

    /// Conflict question: answers at once when an apply-to-all answer fits, otherwise FileConflictWindow.
    pub(super) fn ask_file_conflict(&mut self, source: PathBuf, destination: PathBuf, allow_resume: bool, reply: Sender<ConflictChoice>) {
        let mut form = ConflictForm::read(&source, &destination, allow_resume);
        if let Some(choice) = self.conflict_all.filter(|c| form.compatible(*c)) {
            let _ = reply.send(choice);
            return;
        }
        // Original FUN_00422314: imgOrg = the existing item, imgNew = the shell icon of the item being copied (SHGetFileInfoW SHGFI_ICON).
        form.icons = [(self.system.large_icon)(&destination), (self.system.large_icon)(&source)];
        let title = self.language_pack.text("formFileExists.Form.Caption", "Confirm File/Folder Replace");
        self.push_original_dialog(title, DialogKind::FileConflict(Box::new(form)), move |app, result, _| {
            let choice = match result {
                DialogResult::FileConflict(Some((choice, all))) => {
                    if all && choice != ConflictChoice::Cancel {
                        app.conflict_all = Some(choice);
                    }
                    choice
                }
                _ => ConflictChoice::Cancel,
            };
            let _ = reply.send(choice);
        });
    }

    /// Original FUN_0041190c: a TfGetText-style window (TITLE_RENAME_AND_COPY, MSG_RENAME_AND_COPY with %s = original name, initial value = original name).
    pub(super) fn ask_rename_target(&mut self, name: String, reply: Sender<Option<String>>) {
        let title = self.language_pack.text("RES.TITLE_RENAME_AND_COPY", "Rename and Copy");
        let message = self
            .language_pack
            .text("RES.MSG_RENAME_AND_COPY", "Type a new name.\nThis file will be copied with a new name.\nOriginal filename:%s")
            .replace("%s", &name);
        self.prompt(&title, &message, &name, None, false, move |_, value, _| {
            let _ = reply.send(value);
        });
    }

    /// C# FileOperationErrorDialog: error box (TITLE_FILEOPERROR, error icon), with MSG_WANTTOCONTINUE yes/no when it can continue.
    pub(super) fn ask_file_failure(&mut self, path: PathBuf, message: String, can_continue: bool, reply: Sender<bool>) {
        let title = self.language_pack.text("RES.TITLE_FILEOPERROR", "File Operation Error");
        let detail = if message.trim().is_empty() { self.language_pack.text("RES.MSG_UNKNOWN_ERROR", "Unknown error occurred. Error number: %d").replace("%d", "0") } else { message };
        self.system_box(&title.clone(), &detail, MB_OK | MB_ICONHAND, move |app, _, _| {
            if !can_continue {
                let _ = reply.send(true);
                return;
            }
            let prompt = app
                .language_pack
                .text("RES.MSG_WANTTOCONTINUE", "%s\nError occurred while doing operation for this file. Do you want to keep working on the rest of the file/folder(s)?")
                .replace("%s", &path.to_string_lossy());
            app.system_box(&title, &prompt, MB_YESNO | MB_ICONQUESTION, move |_, answer, _| {
                let _ = reply.send(answer == IDYES);
            });
        });
    }

    /// Original TfProgress (ClientWidth 441 x ClientHeight 143): lFrom (20,23 353x16) and lTo (20,42 353x15) path rows (epPathEllipsis),
    /// lFileName (20,71 317x17, end ellipsis), ProgressBar1 (18,94 327x16), btnCancel (355,90 78x25), lRemaining (18,116), lCount (right edge 345, 116).
    /// Original FUN_00412308 (per file): lFrom = the source file's folder (PathRemoveFileSpecW), lTo = destination folder, lFileName = file name,
    /// lCount = "(%d)" remaining items (total - done). FUN_00412558 (bytes): bar = percentage; for copy/move lRemaining =
    /// StrFormatByteSizeW(remaining bytes). FUN_00411cbc (preparing): lFileName = MSG_*_PREPARING, the other texts empty.
    /// Only in the C# version: the pause button (355,60 above btnCancel), remaining time for jobs without a byte total (in lRemaining's place), the done count when the total is unknown.
    /// Closing the window (x, Esc) requests cancel (C# OnClosing). With another window on top it only draws.
    pub(super) fn file_progress_ui(&mut self, ctx: &egui::Context) {
        use super::original_dialogs::{path_ellipsis, put_enabled, put_left};
        let ko = self.language == "ko";
        let covered = !self.msgboxes.is_empty() || !self.original_dialogs.is_empty();

        let cancel_label = super::favorites_dialog::access_key(&self.language_pack.text("formProgress.Caption.btnCancel", "&Cancel")).0;
        let Some(job) = self.job.as_mut().filter(|j| !j.finished) else { return };
        let Some(info) = job.progress.clone() else { return };
        // Original rows: the first given path, otherwise the folder of the first reported path.
        if info.from.is_none() && job.from_seen.is_none() && !job.path.is_empty() {
            job.from_seen = Path::new(&job.path).parent().map(|p| p.to_string_lossy().into_owned());
        }
        let from = info.from.clone().or_else(|| job.from_seen.clone());
        // Original 0x412308: per file lFrom = folder of the current source file, lTo = folder of the current destination file (PathRemoveFileSpecW).
        // File jobs (fsops) report the destination file (Event::Target, including names changed by conflicts). For jobs that do not,
        // it is derived from the same relative path below the starting folder (the starting destination folder when it cannot be derived).
        let current_folder = Some(Path::new(&job.path)).filter(|p| p.is_absolute()).and_then(Path::parent).map(Path::to_path_buf);
        let to_line = progress_target_line(info.from.as_deref(), info.to.as_deref(), &job.path, job.target.as_ref());
        let from = current_folder.map(|f| f.to_string_lossy().into_owned()).or(from);
        let fraction = if info.prefer_bytes && info.total_bytes.is_some_and(|t| t > 0) {
            Some((job.bytes as f64 / info.total_bytes.unwrap_or(1) as f64).clamp(0.0, 1.0))
        } else if info.total_files.is_some_and(|t| t > 0) {
            Some((job.files as f64 / info.total_files.unwrap_or(1) as f64).clamp(0.0, 1.0))
        } else {
            None
        };
        // lRemaining: original FUN_00412558 = remaining bytes (StrFormatByteSizeW), only when the byte total is known. Initially (FUN_00411cbc) empty.
        let remaining = match info.total_bytes.filter(|t| *t > 0) {
            Some(total) if info.prefer_bytes => ndr::native::format_byte_size(total.saturating_sub(job.bytes)),
            _ => String::new(),
        };
        // lCount: original "(%d)" = remaining items. When the total is unknown, the done count like the C# version.
        let count = match info.total_files {
            Some(total) => format!("({})", n0(total.saturating_sub(job.files) as usize)),
            None => format!("({})", n0(job.files as usize)),
        };
        // lFileName: file name. While preparing, the original MSG_*_PREPARING.
        let file_line = if job.path.is_empty() {
            // Original FUN_00411cbc: the preparing text = MSG_COPY/MOVE/DEL_PREPARING by kind. Other jobs use the C# text.
            match &job.report {
                JobReport::Transfer { verb: (_, "move"), .. } => self.language_pack.text("RES.MSG_MOVE_PREPARING", "Preparing for moving file/folder(s)"),
                JobReport::Transfer { .. } => self.language_pack.text("RES.MSG_COPY_PREPARING", "Preparing for copying file/folder(s)"),
                JobReport::Delete { .. } | JobReport::Shred => self.language_pack.text("RES.MSG_DEL_PREPARING", "Preparing for deleting file/folder(s)"),
                _ => (if ko { "작업을 준비하고 있습니다." } else { "Preparing the operation." }).to_owned(),
            }
        } else {
            Path::new(&job.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| job.path.clone())
        };
        let mut cancel = !covered && ctx.input(|i| i.key_pressed(egui::Key::Escape));
        // Original progress window caption (FUN_00411cbc by kind): COPYING / MOVING / DELETING / RECYCLING. Other jobs use the C# title.
        let caption = match &job.report {
            _ if info.caption.is_some() => info.caption.clone().unwrap_or_default(),
            JobReport::Transfer { verb: (_, "move"), .. } => self.language_pack.text("RES.MOVING", "Moving..."),
            JobReport::Transfer { .. } => self.language_pack.text("RES.COPYING", "Copying..."),
            JobReport::Delete { recycle: true, .. } => self.language_pack.text("RES.RECYCLING", "Sending to the RecycleBin..."),
            JobReport::Delete { .. } | JobReport::Shred => self.language_pack.text("RES.DELETING", "Deleting..."),
            _ => job.title.clone(),
        };
        super::keep_opaque(ctx, egui::Id::new("ndr-file-progress"));
        let progress_id = egui::Id::new("ndr-file-progress");
        super::movable_modal(ctx, progress_id).show(ctx, |ui| {
            ui.set_width(441.0);
            if super::modal_title_bar(ui, progress_id, &caption, None) {
                cancel = true;
            }
            ui.separator();
            let (area, _) = ui.allocate_exact_size(egui::vec2(441.0, 143.0), egui::Sense::hover());
            let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
            let font = egui::FontId::proportional(13.0);
            for (top, height, value) in [(23.0, 16.0, from.as_deref()), (42.0, 15.0, to_line.as_deref())] {
                if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
                    let shown = path_ellipsis(ui, value, 353.0, &font);
                    put_left(ui, at(20.0, top, 353.0, height), egui::Label::new(RichText::new(shown).font(font.clone())));
                }
            }
            put_left(ui, at(20.0, 71.0, 317.0, 17.0), egui::Label::new(RichText::new(&file_line).font(font.clone())).truncate());
            let bar = match fraction {
                Some(value) => egui::ProgressBar::new(value as f32),
                None => egui::ProgressBar::new(0.0).animate(true),
            };
            ui.put(at(18.0, 94.0, 327.0, 16.0), bar.desired_height(16.0));
            // Original btnCancelClick (0x451528): only sets the cancel flag (buttons and text stay; the job stops at its next check point).
            if put_enabled(ui, at(355.0, 90.0, 78.0, 25.0), true, egui::Button::new(&cancel_label)).clicked() {
                cancel = true;
            }
            put_left(ui, at(18.0, 116.0, 268.0, 14.0), egui::Label::new(RichText::new(&remaining).size(12.0)).truncate());
            ui.painter().text(area.min + egui::vec2(345.0, 116.0), egui::Align2::RIGHT_TOP, &count, egui::FontId::proportional(12.0), ui.visuals().text_color());
        });
        if covered {
            return;
        }
        if cancel {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Result notice when a job ends (Event::Finished). The job remains in its finished state (it does not block the next job).
    pub(super) fn report_job(&mut self, cancelled: bool) {
        let ko = self.language == "ko";
        let Some(job) = self.job.as_ref() else { return };
        let report = job.report.clone();
        let errors = job.errors.clone();
        let fatal = job.fatal.clone();
        let (files, interactive, skipped, title) = (job.files, job.interactive, job.skipped, job.title.clone());
        let (chime, custom_status, bytes) = (job.chime, job.custom_status, job.bytes);
        let unreported: Vec<String> = errors.iter().skip(interactive.min(errors.len())).cloned().collect();
        let sound = |app: &App| {
            // Original 0x43d9fc (completesound): PlaySoundW("<program folder>\complete.wav", 0x22003). If it cannot play, MessageBeep like C#.
            // Original default completesound=1 (constructor +0xdc = 1).
            if app.settings.flag("PlayCompletionSound", true) && !(app.system.play_wave)(Some(&app.resources.join("complete.wav"))) {
                native::completion_sound();
            }
        };
        // After creating an archive, put the cursor on it in the new listing of the pane that shows it.
        if let Some((pane, output)) = self.archive_focus.take() {
            let in_pane = output.parent().is_some_and(|folder| ndr::names::equals_ignore_case(&folder.to_string_lossy(), &self.panes[pane].path.to_string_lossy()));
            if !cancelled && fatal.is_none() && output.exists() && in_pane && self.panes[pane].archive.is_none() {
                self.pending_focus[pane] = output.file_name().map(|n| n.to_string_lossy().into_owned());
            }
        }
        self.refresh(0);
        self.refresh(1);
        self.ftp_queue_done();
        match report {
            JobReport::Generic => {
                if chime && !cancelled && errors.is_empty() && fatal.is_none() {
                    sound(self);
                }
                if let Some(message) = &fatal {
                    let shown = self.archive_error_text(message);
                    self.system_box("NDR", &shown, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                }
                if custom_status {
                    return;
                }
                self.status = format!(
                    "{title}: {} files, {}{}{}",
                    files,
                    format_size(bytes),
                    if cancelled { " (cancelled)" } else { "" },
                    if errors.is_empty() { String::new() } else { format!(" · {} errors", errors.len()) }
                );
                if !errors.is_empty() {
                    self.show_error_text(&title, &errors);
                }
            }
            JobReport::Transfer { verb, requested, clear_clipboard } => {
                let v = if ko { verb.0 } else { verb.1 };
                if let Some(message) = fatal {
                    let caption = if ko { format!("{v} 오류") } else { format!("{} error", capitalize(v)) };
                    self.system_box(&caption, &message, MB_OK | MB_ICONHAND, |_, _, _| {});
                    self.status = if ko { format!("{v} 작업을 마치지 못했습니다.") } else { format!("The {v} operation did not finish.") };
                    return;
                }
                if cancelled {
                    self.status = if ko { format!("{v} 작업을 취소했습니다. 완료된 항목은 유지됩니다.") } else { format!("The {v} operation was cancelled. Completed items were kept.") };
                    return;
                }
                if !errors.is_empty() {
                    let mut details = unreported.iter().take(30).cloned().collect::<Vec<_>>().join("\r\n");
                    if unreported.len() > 30 {
                        details += &if ko { format!("\r\n외 {}건", n0(unreported.len() - 30)) } else { format!("\r\nand {} more", n0(unreported.len() - 30)) };
                    }
                    if skipped > 0 {
                        details += &if ko { format!("\r\n\r\n사용자 선택으로 건너뛴 항목: {}개", n0(skipped)) } else { format!("\r\n\r\nItems skipped by choice: {}", n0(skipped)) };
                    }
                    if !unreported.is_empty() || skipped > 0 {
                        let caption = if ko { format!("일부 {v} 실패 ({}건)", n0(errors.len())) } else { format!("Some items failed to {v} ({})", n0(errors.len())) };
                        self.system_box(&caption, &details, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                    }
                    self.status = if ko {
                        format!("{v} 완료: 요청 {}개, 실패 {}건, 건너뜀 {}개", n0(requested), n0(errors.len()), n0(skipped))
                    } else {
                        format!("{} finished: {} requested, {} failed, {} skipped", capitalize(v), n0(requested), n0(errors.len()), n0(skipped))
                    };
                    return;
                }
                if skipped > 0 {
                    self.status = if ko { format!("{v} 완료: {}개 항목을 건너뛰었습니다.", n0(skipped)) } else { format!("{} finished: skipped {} item(s).", capitalize(v), n0(skipped)) };
                    return;
                }
                if clear_clipboard {
                    self.clipboard = None;
                }
                self.status = if ko { format!("요청한 {}개 항목을 {v}했습니다.", n0(requested)) } else { format!("Finished: {v} {} requested item(s).", n0(requested)) };
                sound(self);
            }
            JobReport::Delete { recycle, skipped } => {
                let failed = errors.len() + usize::from(fatal.is_some());
                let summary = if skipped > 0 { if ko { format!(" 건너뜀 {}개", n0(skipped)) } else { format!(" Skipped {}.", n0(skipped)) } } else { String::new() };
                let completed = n0(files as usize);
                self.status = match (cancelled, recycle, ko) {
                    (true, _, true) => format!("삭제 작업을 중단했습니다. 완료 {completed}개, 실패 {}개", n0(failed)),
                    (true, _, false) => format!("Deletion stopped. Completed {completed}, failed {}.", n0(failed)),
                    (false, true, true) => format!("{completed}개 항목을 휴지통으로 이동했습니다.{summary} 실패 {}개", n0(failed)),
                    (false, true, false) => format!("Moved {completed} item(s) to the Recycle Bin.{summary} Failed {}.", n0(failed)),
                    (false, false, true) => format!("{completed}개 항목을 삭제했습니다.{summary} 실패 {}개", n0(failed)),
                    (false, false, false) => format!("Deleted {completed} item(s).{summary} Failed {}.", n0(failed)),
                };
                if !cancelled && failed == 0 && skipped == 0 && files > 0 {
                    sound(self);
                }
                let mut rest = unreported;
                rest.extend(fatal);
                if !rest.is_empty() {
                    let title = self.tr("삭제하지 못한 항목", "Items that could not be deleted").into_owned();
                    self.show_error_text(&title, &rest);
                }
            }
            JobReport::Shred => {
                let mut all = errors;
                all.extend(fatal);
                if all.is_empty() {
                    self.status = self.tr("완전 삭제 작업이 끝났습니다.", "Shredding finished.").into_owned();
                    sound(self);
                } else {
                    let title = self.tr("일부 항목을 완전히 삭제하지 못했습니다.", "Some items could not be shredded.").into_owned();
                    self.text = Some(TextDocument { title, text: all.join("\r\n") });
                }
            }
        }
    }

    /// C# TextWindow (title, 500 error rows + "... N more errors").
    fn show_error_text(&mut self, title: &str, errors: &[String]) {
        let mut text = errors.iter().take(500).map(|e| self.archive_error_text(e)).collect::<Vec<_>>().join("\r\n");
        if errors.len() > 500 {
            text += &if self.language == "ko" { format!("\r\n… 나머지 {}개 오류", n0(errors.len() - 500)) } else { format!("\r\n… {} more errors", n0(errors.len() - 500)) };
        }
        self.text = Some(TextDocument { title: title.into(), text });
    }

    /// C# TryExternalCopyAsync: with UseCopyHandler and an existing CopyHandlerPath file (environment variables expanded), hands over to that program and returns true.
    /// KillCopy (GUI) and kc (console) take an ANSI list file, TeraCopy 4+ uses SendTo for multi-item copies, the rest get "Copy|Move source|*list destination /Close".
    /// When the program ends both panes refresh and a status text is written like C#. Failures show a warning box (C# RunSafely).
    fn try_external_copy(&mut self, sources: &[PathBuf], destination: &Path, moving: bool, kind: TransferKind) -> bool {
        if !self.settings.flag("UseCopyHandler", false) {
            return false;
        }
        let program = compat::expand_setting(&self.settings.text("CopyHandlerPath", ""));
        if program.trim().is_empty() || !Path::new(&program).is_file() {
            return false;
        }
        let mut paths: Vec<String> = Vec::new();
        for source in sources.iter().filter(|p| p.exists()) {
            let full = std::path::absolute(source).unwrap_or_else(|_| source.clone()).to_string_lossy().into_owned();
            if !paths.iter().any(|p| p.eq_ignore_ascii_case(&full)) {
                paths.push(full);
            }
        }
        if paths.is_empty() {
            return false;
        }
        let target = std::path::absolute(destination).unwrap_or_else(|_| destination.to_path_buf());
        self.remember_destination(&target);
        let target = target.to_string_lossy().into_owned();
        let ko = self.language == "ko";
        let name = Path::new(&program).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let kill_gui = name.contains("killcopy");
        let kill_console = name == "kc";
        let count = paths.len();
        self.status = if kill_gui || kill_console || !ko {
            format!("External copy handler is processing {} item(s).", n0(count))
        } else {
            format!("외부 복사 프로그램에서 {}개 항목을 처리하고 있습니다.", n0(count))
        };
        let done = match (kind, moving, ko) {
            (TransferKind::OtherPane, false, true) => Some(format!("{}개 항목을 복사했습니다.", n0(count))),
            (TransferKind::OtherPane, true, true) => Some(format!("{}개 항목을 이동했습니다.", n0(count))),
            (TransferKind::OtherPane, false, false) => Some(format!("Copied {} item(s).", n0(count))),
            (TransferKind::OtherPane, true, false) => Some(format!("Moved {} item(s).", n0(count))),
            (TransferKind::Drag, false, true) => Some(format!("드래그한 {}개 항목을 복사했습니다.", n0(count))),
            (TransferKind::Drag, true, true) => Some(format!("드래그한 {}개 항목을 이동했습니다.", n0(count))),
            (TransferKind::Drag, false, false) => Some(format!("Copied {} dragged item(s).", n0(count))),
            (TransferKind::Drag, true, false) => Some(format!("Moved {} dragged item(s).", n0(count))),
            _ => None,
        };
        let clear_clipboard = moving && kind == TransferKind::Paste;
        let runner = self.program_runner;
        let tx = self.async_tx.clone();
        let teracopy = !moving && paths.len() >= 2 && name.contains("teracopy") && native::file_major_version(Path::new(&program)).is_some_and(|v| v >= 4) && {
            // C#: estimated command line length (with quotes) below 30,000.
            let estimate = program.len() * 2 + target.len() * 2 + 256 + paths.iter().map(|p| p.len() * 2 + 8).sum::<usize>();
            estimate < 30_000
        };
        thread::spawn(move || {
            let result = run_external_copy(runner, &program, &paths, &target, moving, kill_gui, kill_console, teracopy);
            let _ = tx.send(AsyncResult::ExternalCopy { result, status: done, clear_clipboard });
        });
        true
    }

    pub(super) fn external_copy_done(&mut self, result: Result<(), String>, status: Option<String>, clear_clipboard: bool) {
        self.refresh(0);
        self.refresh(1);
        match result {
            Ok(()) => {
                if clear_clipboard {
                    self.clipboard = None;
                }
                if let Some(status) = status {
                    self.status = status;
                }
            }
            Err(message) => self.system_box("NDR", &message, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// Temporary list file name (C# ndr-copy-{Guid:N}).
fn list_path(extension: &str) -> PathBuf {
    let mut bytes = [0u8; 16];
    let _ = native::random_bytes(&mut bytes);
    std::env::temp_dir().join(format!("ndr-copy-{}.{extension}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

#[allow(clippy::too_many_arguments)]
fn run_external_copy(runner: fn(&Path, &[String], &Path) -> Result<i32, String>, program: &str, paths: &[String], target: &str, moving: bool, kill_gui: bool, kill_console: bool, teracopy: bool) -> Result<(), String> {
    let directory = Path::new(program).parent().map(Path::to_path_buf).unwrap_or_default();
    let mut list = None;
    let result = (|| {
        let mut args = Vec::new();
        if kill_gui || kill_console {
            // KillCopy reads the list in the current ANSI code page.
            let mut bytes = Vec::new();
            for path in paths {
                bytes.extend(ndr::split::ansi_bytes(path).ok_or("KillCopy reads file lists using the current Windows ANSI code page, which cannot represent one or more selected paths. Use a Unicode-capable handler such as TeraCopy for these paths.")?);
                bytes.extend(b"\r\n");
            }
            let file = list_path("lst");
            fs::write(&file, bytes).map_err(|e| e.to_string())?;
            let shown = file.to_string_lossy().into_owned();
            list = Some(file);
            if kill_gui {
                args.push(format!("/{shown}/{target}/{}", if moving { " -m" } else { "" }));
            } else {
                args.push(shown);
                args.push(target.to_owned());
                if moving {
                    args.push("-m".into());
                }
            }
            let code = runner(Path::new(program), &args, &directory).map_err(|e| format!("KillCopy could not be started. {e}"))?;
            if code != 0 {
                return Err(format!("KillCopy exited with error code {code}."));
            }
            return Ok(());
        }
        if teracopy {
            args.extend(["SendTo".to_owned(), target.to_owned(), "/Close".to_owned()]);
            args.extend(paths.iter().cloned());
        } else {
            args.push(if moving { "Move" } else { "Copy" }.into());
            if paths.len() == 1 {
                args.push(paths[0].clone());
            } else {
                let file = list_path("txt");
                let mut text = String::from("\u{feff}");
                for path in paths {
                    text += path;
                    text += "\r\n";
                }
                fs::write(&file, text).map_err(|e| e.to_string())?;
                args.push(format!("*{}", file.display()));
                list = Some(file);
            }
            args.push(target.to_owned());
            args.push("/Close".into());
        }
        let code = runner(Path::new(program), &args, &directory)?;
        if code != 0 {
            return Err(format!("외부 복사 프로그램이 오류 코드 {code}로 종료되었습니다."));
        }
        Ok(())
    })();
    if let Some(file) = list {
        let _ = fs::remove_file(file);
    }
    result
}

/// Runs the program directly and waits until it ends (C# UseShellExecute = false, WaitForExitAsync).
pub(super) fn run_program(program: &Path, args: &[String], directory: &Path) -> Result<i32, String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    if directory.is_dir() {
        command.current_dir(directory);
    }
    command.status().map(|s| s.code().unwrap_or(-1)).map_err(|e| e.to_string())
}
