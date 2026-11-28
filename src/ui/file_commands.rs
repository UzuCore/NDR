//! Ports the C# NDR name, description, open and delete flows. Each function notes the original address and the C# name.
//! Dialogs return results through callbacks, so loops that show a dialog per item pass their state on to the next step.
use super::original_dialogs::{DeleteConfirmChoice, DeleteWarningChoice, DeleteWarningMode};
use super::*;
use super::file_operation::JobReport;
use ndr::msgbox::{IDYES, MB_ICONHAND, MB_OK, MB_YESNO};
use ndr::names;

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

/// RenameAsync progress state.
struct RenameRun {
    pane: usize,
    items: Vec<Entry>,
    index: usize,
    focused: Option<String>,
    focus_after: Option<String>,
}

/// Progress state of the DeleteAsync protected item checks.
pub(super) struct DeleteRun {
    pub paths: Vec<PathBuf>,
    pub items: Vec<(PathBuf, u32)>,
    pub index: usize,
    pub approve_all: bool,
    pub skipped: Vec<PathBuf>,
}

/// C# DeleteWarningWindow.DisplayName: the last name without the trailing separator, otherwise (drive root) the path as is.
pub(super) fn display_name(path: &Path) -> String {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['\\', '/']);
    let name = Path::new(trimmed).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() { text.into_owned() } else { name }
}


/// Format numbered localization placeholders without rescanning inserted values.
/// A path or an error containing "{1}" must stay literal.
pub(super) fn operation_format(language: &compat::Language, key: &str, fallback: &str, args: &[&dyn std::fmt::Display]) -> String {
    use std::fmt::Write as _;
    let template = language.text(key, fallback);
    let mut rest = template.as_str();
    let mut out = String::with_capacity(rest.len());
    while let Some(open) = rest.find('{') {
        let (before, tail) = rest.split_at(open);
        out.push_str(before);
        if let Some(close) = tail.find('}') {
            if let Ok(index) = tail[1..close].parse::<usize>() {
                if let Some(value) = args.get(index) {
                    let _ = write!(&mut out, "{value}");
                    rest = &tail[close + 1..];
                    continue;
                }
            }
        }
        out.push('{');
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

impl App {
    pub(super) fn notice_operation(&mut self, text: &str) {
        self.system_box("", text, MB_OK, |_, _, _| {});
    }

    pub(super) fn warn_operation(&mut self, text: &str) {
        self.system_box("NDR", text, MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
    }

    /// [RES]/[Root] text (language file value, otherwise the original English literal).
    pub(super) fn root_text(&self, key: &str, fallback: &str) -> String {
        self.language_pack.text(&format!("RES.{key}"), fallback)
    }

    /// Target items in display order (original FUN_00485bbc: marked items, otherwise the cursor item).
    pub(super) fn target_entries(&self, pane: usize) -> Vec<Entry> {
        let p = &self.panes[pane];
        if p.loading {
            return vec![];
        }
        self.displayed_target_entries(pane)
    }

    /// Only for callers that have independently proved the displayed rows
    /// belong to the current location. Never use during actual navigation.
    pub(super) fn displayed_target_entries(&self,pane:usize)->Vec<Entry>{
        let p=&self.panes[pane];
        ndr::parity::collect_items(p.entries.len(), &p.selected, Some(p.cursor)).into_iter().filter_map(|i| p.entries.get(i).filter(|e| !e.drive && !e.is_parent()).cloned()).collect()
    }

    fn focused_name(&self, pane: usize) -> Option<String> {
        let p = &self.panes[pane];
        (!p.loading).then(|| p.entries.get(p.cursor).map(|e| e.name.clone())).flatten()
    }

    /// C# FocusPaneItemByName after RefreshPaneAsync: when the new list arrives, put the cursor on that name (case-insensitive).
    pub(super) fn refresh_and_focus(&mut self, pane: usize, name: Option<String>) {
        self.pending_focus[pane] = name;
        self.refresh(pane);
    }

    /// Original FUN_0040c31c (C# ShowNameOperationError): system error text (else MSG_UNKNOWN_ERROR), TITLE_ERROR caption, error icon.
    /// With ask_continue, "\n" + MSG_WANTTOCONTINUE2 and yes/no. The callback receives whether to continue (true).
    pub(super) fn show_name_operation_error(
        &mut self,
        error: u32,
        ask_continue: bool,
        then: impl FnOnce(&mut App, bool, &egui::Context) + 'static,
    ) {
        let mut text = names::system_message(error)
            .unwrap_or_else(|| self.root_text("MSG_UNKNOWN_ERROR", "Unknown error occurred. Error number: %d").replace("%d", &error.to_string()));
        let title = self.root_text("TITLE_ERROR", "NexusFile Error");
        let flags = if ask_continue {
            text.push('\n');
            text.push_str(&self.root_text("MSG_WANTTOCONTINUE2", "Do you want to keep working on the rest of the files?"));
            MB_YESNO | MB_ICONHAND
        } else {
            MB_OK | MB_ICONHAND
        };
        self.system_box(&title, &text, flags, move |app, result, ctx| then(app, !ask_continue || result == IDYES, ctx));
    }

    /// Original FUN_00432378 (C# NewFolderAsync): default is the short date ('\' and '/' -> '-'), no trimming or character filtering, empty input does nothing,
    /// CreateDirectoryW (no intermediate folders, an existing name fails). Failure shows only an error box; success refreshes and puts the cursor on the new folder.
    pub(super) fn new_folder(&mut self) {
        if self.panes[self.active].vault.is_some() { return self.vault_new_item(true); }
        let pane = self.active;
        if self.panes[pane].remote.is_some() {
            return self.ftp_new_folder(pane);
        }
        if self.panes[pane].archive.is_some() {
            return self.archive_new_folder(pane);
        }
        let directory = self.panes[pane].path.to_string_lossy().into_owned();
        let title = self.root_text("TITLE_NEWFOLDER", "New Folder");
        let message = self.root_text("MSG_NEWFOLDER", "Type a folder name to be created.");
        self.prompt_file_name(&title, &message, &names::new_folder_default_name(), move |app, value, _| {
            let Some(name) = value.filter(|v| !v.is_empty()) else { return };
            let path = names::combine_typed_name(&directory, &name);
            let error = names::create_directory(&path);
            if error != 0 {
                return app.show_name_operation_error(error, false, |_, _, _| {});
            }
            app.refresh_and_focus(pane, Some(display_name(Path::new(&path))));
            app.completion_notice_operation(app.language_pack.text("NDR.OP_FILE_FOLDER_CREATED", "Folder created"), name);
        });
    }

    /// Original FUN_00434630 (C# CreateEmptyFileAsync): empty default; CreateFileW(CREATE_NEW) never empties an existing item.
    /// Approved fix: the original silently ignores failure, NDR reports it. Then refresh and put the cursor on the typed name.
    pub(super) fn create_empty_file(&mut self) {
        if self.panes[self.active].vault.is_some() { return self.vault_new_item(false); }
        let pane = self.active;
        if self.panes[pane].archive.is_some() {
            return self.archive_empty_file(pane);
        }
        let directory = self.panes[pane].path.to_string_lossy().into_owned();
        let title = self.root_text("TITLE_CREATEEMPTYFILE", "Create an empty file");
        let message = self.root_text("MSG_CREATEEMPTYFILE", "0 byte file will be created.\nType a new file name to be created.");
        self.prompt_file_name(&title, &message, "", move |app, value, _| {
            let Some(name) = value.filter(|v| !v.is_empty()) else { return };
            let path = names::combine_typed_name(&directory, &name);
            let error = names::create_empty_file(&path);
            if error != 0 {
                app.show_name_operation_error(error, false, |_, _, _| {});
            }
            app.refresh_and_focus(pane, Some(names::find_file_name(&path)));
            if error == 0 {
                app.completion_notice_operation(app.language_pack.text("NDR.OP_FILE_FILE_CREATED", "File created"), name);
            }
        });
    }

    /// Original FUN_004351cc (C# RenameAsync): TfRename for each item in display order. Cancel stops.
    /// New name = the name when the extension is empty, else name + "." + extension. MoveFileW within the same folder.
    pub(super) fn rename_items(&mut self) {
        if self.panes[self.active].vault.is_some() { return self.vault_rename_item(); }
        let pane = self.active;
        if self.panes[pane].remote.is_some() {
            return self.ftp_rename(pane);
        }
        if self.panes[pane].archive.is_some() {
            return self.archive_rename(pane);
        }
        let on_drive_item = self.panes[pane].entries.get(self.panes[pane].cursor).is_some_and(|e| e.drive);
        if super::panes::is_drives(&self.panes[pane].path) || on_drive_item {
            return self.rename_drive_label(pane);
        }
        let items = self.target_entries(pane);
        if items.is_empty() {
            return;
        }
        let focused = self.focused_name(pane);
        self.rename_step(RenameRun { pane, items, index: 0, focused, focus_after: None });
    }

    fn rename_step(&mut self, mut run: RenameRun) {
        let Some(entry) = run.items.get(run.index).cloned() else { return self.finish_rename(run) };
        let encrypted_folder = entry.path.is_file() && entry.path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ndr"));
        let (name, extension) = names::split_for_rename(&entry.name, entry.is_dir);
        let count = run.items.len();
        let file_name = entry.name.clone();
        self.rename_dialog(&file_name, &name, &extension, run.index + 1, count, move |app, value, _| {
            let Some((new_name, new_extension)) = value else { return app.finish_rename(run) };
            let directory = entry.path.parent().map(Path::to_path_buf).unwrap_or_else(|| app.panes[run.pane].path.clone());
            let mut target_name = if new_extension.is_empty() { new_name } else { format!("{new_name}.{new_extension}") };
            if encrypted_folder && !target_name.to_ascii_lowercase().ends_with(".ndr") {
                target_name.push_str(".ndr");
            }
            let error = names::move_file(&entry.path.to_string_lossy(), &directory.join(&target_name).to_string_lossy());
            if error == 0 {
                app.completion_notice_operation(app.language_pack.text("NDR.OP_FILE_RENAME_COMPLETED", "Rename completed"), target_name.clone());
                // Original FUN_004361f8 -> FUN_0040f7f0: the description follows the new name.
                let include_legacy = app.settings.flag("UseDescriptionFile", true);
                if let Err(message) = compat::rename_descriptions(&directory, &[(entry.name.clone(), target_name.clone())], include_legacy) {
                    app.status = message;
                }
                if run.focused.as_deref().is_some_and(|f| names::equals_ignore_case(f, &entry.name)) {
                    run.focus_after = Some(if encrypted_folder {
                        target_name.strip_suffix(".ndr").unwrap_or(&target_name).to_owned()
                    } else { target_name });
                }
                run.index += 1;
                return app.rename_step(run);
            }
            // The original asks whether to continue only when items remain after the failed one.
            let ask = count - 1 > run.index;
            app.show_name_operation_error(error, ask, move |app, go_on, _| {
                if go_on {
                    run.index += 1;
                    app.rename_step(run);
                } else {
                    app.finish_rename(run);
                }
            });
        });
    }

    fn finish_rename(&mut self, run: RenameRun) {
        self.refresh_and_focus(run.pane, run.focus_after);
    }

    /// Drive item branch of original FUN_004351cc: if the drive is ready, TfGetText(window title, MSG_DRIVELABEL, current volume name).
    /// On OK with non-empty text, SetVolumeLabelW(root, name). Success rereads; ERROR_ACCESS_DENIED (5) shows ACCESS_DENIED;
    /// other failures are ignored. ACCESS_DENIED is the brief notice over that list like the original (FUN_0040e138, TfMessage).
    fn rename_drive_label(&mut self, pane: usize) {
        let Some(entry) = self.panes[pane].entries.get(self.panes[pane].cursor).cloned() else { return };
        let Some(label) = (self.system.volume_label)(&entry.path) else { return };
        let message = self.language_pack.text("RES.MSG_DRIVELABEL", "Rename Drive. Type a new drive name.");
        let root = entry.path;
        self.prompt("NDR", &message, &label, None, false, move |app, value, _| {
            let Some(value) = value.filter(|v| !v.is_empty()) else { return };
            match (app.system.set_volume_label)(&root, &value) {
                Ok(()) => {
                    super::panes::DriveListCache::invalidate(&app.drive_list);
                    app.refresh(pane)
                }
                Err(5) => {
                    let text = app.language_pack.text("RES.ACCESS_DENIED", "Access Denied");
                    app.pane_message(pane, text);
                }
                Err(_) => {}
            }
        });
    }

    /// Original FUN_00432958 (C# EditDescriptionAsync): TfGetText(TITLE_DESCRIPT, MSG_DESCRIPT with %s = name) for each item in display order.
    /// Cancel stops. OK saves the description (empty removes it) and unmarks the item. At the end descript.ion is moved and the pane refreshed.
    pub(super) fn edit_descriptions(&mut self) {
        let pane = self.active;
        if self.panes[pane].archive.is_some() {
            return self.warn_operation(&self.language_pack.text("NDR.OP_FILE_COMMENTS_CANNOT_BE_CHANGED_INSIDE_AN_ARCHIVE_OR_A", "Comments cannot be changed inside an archive or a remote folder."));
        }
        let items: Vec<String> = self.target_entries(pane).into_iter().map(|e| e.name).collect();
        if items.is_empty() {
            return;
        }
        let directory = self.command_directory(pane);
        self.description_step(pane, directory, items, 0);
    }

    fn description_step(&mut self, pane: usize, directory: PathBuf, items: Vec<String>, index: usize) {
        let include_legacy = self.settings.flag("UseDescriptionFile", true);
        let Some(name) = items.get(index).cloned() else { return self.finish_descriptions(pane, &directory, include_legacy) };
        let current = compat::read_descriptions(&directory, include_legacy)
            .ok()
            .and_then(|map| map.into_iter().find(|(key, _)| names::equals_ignore_case(key, &name)).map(|(_, value)| value))
            .unwrap_or_default();
        let title = self.root_text("TITLE_DESCRIPT", "File Comment");
        let message = self
            .root_text("MSG_DESCRIPT", "Add/Edit comment for a file.\nComment will be shown on file information status bar.\n<b>%s</b>")
            .replace("%s", &name);
        self.prompt(&title, &message, &current, None, false, move |app, value, _| {
            let Some(value) = value else { return app.finish_descriptions(pane, &directory, include_legacy) };
            if let Err(message) = compat::set_description(&directory, &name, &value, include_legacy) {
                // C# ends the flow with an exception (no refresh).
                app.status = message;
                return;
            }
            let p = &mut app.panes[pane];
            if let Some(row) = p.entries.iter().position(|e| e.name == name) {
                p.selected.remove(&row);
            }
            app.description_step(pane, directory, items, index + 1);
        });
    }

    fn finish_descriptions(&mut self, pane: usize, directory: &Path, include_legacy: bool) {
        let listed: Vec<String> = self.panes[pane].entries.iter().map(|e| e.name.clone()).collect();
        if let Err(message) = compat::migrate_legacy_descriptions(directory, &listed, include_legacy) {
            self.status = message;
        }
        self.refresh(pane);
    }

    /// Both pane states for macro expansion (current folder, marked names, cursor item).
    pub(super) fn macro_panes(&self) -> [compat::MacroPane; 2] {
        std::array::from_fn(|i| {
            let p=&self.panes[i];
            compat::MacroPane {
                path: self.command_directory(i),
                focused: p.entries.get(p.cursor).map(|e| e.name.clone()),
                selected: p.selected.iter().filter_map(|row| p.entries.get(*row).map(|e| e.name.clone())).collect(),
            }
        })
    }

    /// Enters a folder item in the other pane (same rule as the folder branch of open_current).
    fn enter_folder(&mut self, pane: usize, entry: Entry) {
        if let Some(location) = self.panes[pane].archive.as_mut() {
            location.prefix = format!("{}/", entry.path.to_string_lossy().replace('\\', "/"));
            self.refresh(pane);
        } else {
            self.navigate(pane, entry.path, true);
        }
    }

    /// Original FUN_00431dd4 (C# OpenWithParametersAsync, Ctrl+Enter): acts only on the cursor item.
    /// Folder: enter it, and if the other pane has a folder with the same name enter it there too (approved fix: a file with the same name is ignored).
    /// Files in archive or remote panes: ordinary open. Otherwise TfGetText(OPEN_WITH_PARAM, MSG_OPEN_WITH_PARAM with %s = name, empty default),
    /// expand $(...) macros and ShellExecuteExW(file, arguments, current folder). The shell reports failures; NDR shows nothing extra.
    pub(super) fn open_with_parameters(&mut self) {
        let pane = self.active;
        if self.panes[pane].loading {
            return;
        }
        let Some(focused) = self.panes[pane].entries.get(self.panes[pane].cursor).cloned() else { return };
        if focused.is_dir {
            let other = 1 - pane;
            let twin = self.panes[other].entries.iter().find(|e| e.is_dir && names::equals_ignore_case(&e.name, &focused.name)).cloned();
            self.open_current();
            if let Some(twin) = twin {
                self.enter_folder(other, twin);
            }
            return;
        }
        if self.panes[pane].archive.is_some() {
            return self.open_current();
        }
        let title = self.root_text("OPEN_WITH_PARAM", "Open with parameters");
        let message = self.root_text("MSG_OPEN_WITH_PARAM", "Type parameters to be used with this file execution\n<b>%s</b>").replace("%s", &focused.name);
        let directory = self.command_directory(pane);
        self.prompt(&title, &message, "", None, false, move |app, value, _| {
            let Some(parameters) = value else { return };
            let panes = app.macro_panes();
            let expanded = compat::expand_command(&parameters, &panes[pane], &panes[0], &panes[1]);
            let path = names::combine_typed_name(&directory.to_string_lossy(), &focused.name);
            let _ = (app.shell_runner)(Path::new(&path), &expanded, &directory);
        });
    }

    /// Original FUN_0042c734 (C# DeleteAsync): TfDelWarning when delconfirm is on or the mode is not the Recycle Bin.
    /// No stops; the Delete button switches to permanent delete. Permanent delete asks TfDelConfirm for each item with protected attributes.
    pub(super) fn delete_items(&mut self, recycle: bool) {
        if self.panes[self.active].vault.is_some() { return self.vault_remove_items(); }
        let pane = self.active;
        if self.panes[pane].remote.is_some() {
            return self.ftp_delete(pane);
        }
        if self.panes[pane].archive.is_some() {
            return self.archive_delete(pane);
        }
        let paths = self.panes[pane].targets();
        if paths.is_empty() {
            return;
        }
        if self.settings.flag("ConfirmDelete", true) || !recycle {
            let mode = if recycle { DeleteWarningMode::Recycle } else { DeleteWarningMode::Delete };
            let first = display_name(&paths[0]);
            self.delete_warning(mode, paths.len(), &first, move |app, choice, _| match choice {
                DeleteWarningChoice::No => {}
                DeleteWarningChoice::Delete => app.delete_confirmed(paths, false),
                DeleteWarningChoice::Yes => app.delete_confirmed(paths, recycle),
            });
            return;
        }
        self.delete_confirmed(paths, recycle);
    }

    fn delete_confirmed(&mut self, paths: Vec<PathBuf>, recycle: bool) {
        self.original_take_marks(&paths);
        if recycle {
            let progress = super::file_operation::delete_progress(&paths, false);
            return self.start_job("Delete".into(), Operation::Delete { sources: paths, recycle: true }, JobReport::Delete { recycle: true, skipped: 0 }, Some(progress));
        }
        // C# FindProtectedDeleteItems runs in Task.Run. Search separately so large trees do not freeze the screen, and pick up the result in poll.
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let items = fsops::find_protected_delete_items(&paths);
            let _ = tx.send(AsyncResult::ProtectedItems(DeleteRun { paths, items, index: 0, approve_all: false, skipped: vec![] }));
        });
    }

    /// TfDelConfirm for each protected item (QUE_DELCONFIRM with %s = path, HAS_ATTRIBUTES with %s = attribute names).
    /// Yes to all approves the rest, No keeps that item (for a folder, everything inside). At the end the delete job starts.
    pub(super) fn protected_step(&mut self, mut run: DeleteRun) {
        while let Some((path, attributes)) = run.items.get(run.index).cloned() {
            if run.approve_all || run.skipped.iter().any(|root| fsops::same_or_inside(&path, root)) {
                run.index += 1;
                continue;
            }
            let mut labels = Vec::new();
            if attributes & FILE_ATTRIBUTE_READONLY != 0 {
                labels.push(self.root_text("READONLY", "Read only"));
            }
            if attributes & FILE_ATTRIBUTE_SYSTEM != 0 {
                labels.push(self.root_text("SYSTEM", "System"));
            }
            if attributes & FILE_ATTRIBUTE_HIDDEN != 0 {
                labels.push(self.root_text("HIDDEN", "Hidden"));
            }
            let attribute_text = self.root_text("HAS_ATTRIBUTES", "File/Folder Attributes: %s").replace("%s", &labels.join(", "));
            let warning = self
                .root_text("QUE_DELCONFIRM", "%s\nThis file has important attributes\nAre you sure you want to delete this file?")
                .replace("%s", &path.to_string_lossy());
            self.delete_confirm(&warning, &attribute_text, move |app, choice, _| {
                match choice {
                    DeleteConfirmChoice::All => run.approve_all = true,
                    DeleteConfirmChoice::No => run.skipped.push(path),
                    DeleteConfirmChoice::Yes => {}
                    // Original FUN_00414884: answer 0x10 aborts the delete (FUN_0040ac38, ERROR_CANCELLED). NDR asks before deleting
                    // anything, so nothing is deleted (the original keeps what it deleted before reaching this item).
                    DeleteConfirmChoice::Cancel => return,
                }
                run.index += 1;
                app.protected_step(run);
            });
            return;
        }
        let skipped = run.skipped.len();
        let progress = super::file_operation::delete_progress(&run.paths, false);
        let operation = if run.skipped.is_empty() {
            Operation::Delete { sources: run.paths, recycle: false }
        } else {
            Operation::DeleteSkipping { sources: run.paths, skipped: run.skipped }
        };
        self.start_job("Delete".into(), operation, JobReport::Delete { recycle: false, skipped }, Some(progress));
    }

    /// C# ShredAsync (original FUN_0042c734 mode 2): the MSG_DELWARN_SHRED warning always shows and has no Delete button. Only Yes proceeds.
    pub(super) fn shred_items(&mut self) {
        let pane = self.active;
        // Original miShredClick (0x431c60) also goes to FUN_0042c734 (mode 2), whose FTP mode branch deletes remotely regardless of the mode.
        if self.panes[pane].remote.is_some() {
            return self.ftp_delete(pane);
        }
        if self.panes[pane].archive.is_some() {
            // User request: archive items have no real file to overwrite, so they are deleted from the archive instead.
            return self.archive_delete(pane);
        }
        let paths = self.panes[pane].targets();
        if paths.is_empty() {
            return self.warn_operation(&self.language_pack.text("NDR.OP_FILE_SELECT_THE_ITEMS_TO_SHRED", "Select the items to shred."));
        }
        let first = display_name(&paths[0]);
        self.delete_warning(DeleteWarningMode::Shred, paths.len(), &first, move |app, choice, _| {
            if choice == DeleteWarningChoice::Yes {
                app.original_take_marks(&paths);
                let progress = super::file_operation::delete_progress(&paths, true);
                app.start_job("Shred".into(), Operation::Shred { sources: paths }, JobReport::Shred, Some(progress));
            }
        });
    }
}

#[cfg(test)]
mod operation_format_tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn bundled_zip_labels_and_archive_preview_keep_runtime_format() {
        let catalogs = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Resources/Languages"));
        let locales = compat::available_codes(catalogs);
        assert_eq!(locales.len(), 40);
        for code in locales {
            let pack = compat::Language::load(catalogs, &code);
            let primary = |key| pack.primary_raw(key).unwrap_or_else(|| panic!("{code}: {key} is missing"));
            let title = primary("NDR.OP_ARCHIVE_CREATE_ZIP_ARCHIVE");
            assert!(title.contains("ZIP"), "{code}: {title:?}");
            let filter = primary("NDR.OP_ARCHIVE_ZIP_ARCHIVES_ZIP");
            assert!(filter.ends_with("(*.zip)"), "{code}: {filter:?}");
            let default_name = primary("NDR.OP_ARCHIVE_NEW_ARCHIVE_ZIP");
            assert!(default_name.ends_with(".zip"), "{code}: {default_name:?}");
            assert!(!default_name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']), "{code}: {default_name:?}");
            let key = "NDR.OP_ARCHIVE_CREATED_THE_ZIP_FILE_0";
            assert_eq!(primary(key).matches("{0}").count(), 1, "{code}");
            let file = "archive-{0}.zip";
            let done = operation_format(&pack, key, "missing", &[&file]);
            assert_eq!(done.matches(file).count(), 1, "{code}: {done:?}");
            let key = "NDR.OP_ARCHIVE_MORE";
            assert!(primary(key).starts_with('\n'), "{code}");
            let more = operation_format(&pack, key, "missing", &[&12345]);
            assert!(more.starts_with('\n') && !more.contains('\r'), "{code}: {more:?}");
            assert_eq!(more.matches('\n').count(), 1, "{code}: {more:?}");
            assert_eq!(more.matches("12345").count(), 1, "{code}: {more:?}");
            let key = "NDR.OP_ARCHIVE_EXTERNAL_VOLUME";
            let warning = primary(key);
            assert_eq!(warning.matches(".{0}").count(), 1, "{code}: {warning:?}");
            assert!(!warning.contains("{archive}") && !warning.contains("{sources}"), "{code}: {warning:?}");
            let displayed = operation_format(&pack, key, "missing", &[&"rar"]);
            assert!(displayed.contains(".rar") && displayed.contains("ZIP") && displayed.contains("7z"), "{code}: {displayed:?}");
        }
    }

    #[test]
    fn operation_catalog_reorders_arguments_and_does_not_interpolate_error_contents() {
        let dir = std::env::temp_dir().join(format!("ndr-op-loc-{}-{}", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("en.lang"), "[NDR]\nOP_TEST=From {0} to {1}\n").unwrap();
        std::fs::write(dir.join("fr.lang"), "[NDR]\nOP_TEST=Vers {1} depuis {0}\n").unwrap();
        let language = compat::Language::load(&dir, "fr");
        let from = "source-{1}.txt";
        let to = "target-{0}.txt";
        assert_eq!(operation_format(&language, "NDR.OP_TEST", "unused", &[&from, &to]), "Vers target-{0}.txt depuis source-{1}.txt");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
