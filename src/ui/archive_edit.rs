//! User request (not in the original): inside an archive, delete, rename, new folder, empty file, copy or move into the archive (add),
//! move out of the archive (extract then delete), copy or move between archives. Only zip, 7z, tar and wim, which the bundled 7-Zip can modify, are supported
//! (ndr::archive::can_modify). The original NexusFile could only extract archive items.
use super::original_dialogs::{DeleteWarningChoice, DeleteWarningMode};
use super::*;
use ndr::names;

/// Archive to modify: (first volume file, inner path "folder/" or "", known password).
pub(super) type EditableArchive = (PathBuf, String, Option<String>);

/// A skipped child means the selected parent folder must remain in the source.
/// Conversely, skipping a destination folder keeps all selected items below it.
fn skipped_path_intersects(skipped: &[PathBuf], target: &Path) -> bool {
    skipped.iter().any(|path| ndr::fsops::same_or_inside(path, target) || ndr::fsops::same_or_inside(target, path))
}

/// Only record OverwriteNewer as skipped when the archive extractor actually
/// skips it. The extracted entry's timestamp is preserved in its staging file.
fn extraction_choice_skips(choice: ConflictChoice, from: &Path, to: &Path) -> bool {
    match choice {
        ConflictChoice::Skip => true,
        ConflictChoice::OverwriteNewer => match (
            std::fs::metadata(from).and_then(|m| m.modified()),
            std::fs::metadata(to).and_then(|m| m.modified()),
        ) {
            (Ok(source), Ok(destination)) => source <= destination,
            _ => true,
        },
        _ => false,
    }
}

impl App {
    /// That archive, when the pane is inside a modifiable archive.
    pub(super) fn editable_archive(&self, pane: usize) -> Option<EditableArchive> {
        let location = self.panes[pane].archive.as_ref()?;
        let file = ndr::archive::first_volume_path(&location.file);
        ndr::archive::can_modify(&file).then(|| (file, location.prefix.clone(), (!location.password.is_empty()).then(|| location.password.clone())))
    }

    pub(super) fn archive_read_only_notice(&mut self) {
        self.notice(
            "이 압축 형식은 고칠 수 없습니다(zip·7z·tar·wim 압축만 항목을 더하고 지우고 이름을 바꿀 수 있습니다).",
            "This archive format cannot be changed (only zip, 7z, tar and wim archives can have items added, deleted or renamed).",
        );
    }

    /// Archive keys ("folder/file") of the selected items (or the cursor item when none).
    fn archive_target_keys(&self, pane: usize) -> Vec<String> {
        self.panes[pane].targets().into_iter().map(|k| k.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_owned()).filter(|k| !k.is_empty()).collect()
    }

    /// Job that modifies the archive (progress window, cancellable). Both panes are reread when done (report_job).
    fn archive_edit_task(&mut self, title: String, archive: &Path, task: impl FnOnce(&Sender<Event>, &AtomicBool) -> Result<(), String> + Send + 'static) {
        let progress = super::file_operation::ProgressInfo { from: Some(archive.to_string_lossy().into_owned()), caption: Some(title.clone()), ..Default::default() };
        self.start_progress_task(title, progress, move |events, cancel| task(&events, &cancel));
    }

    /// Deletes archive items (immediately, no Recycle Bin; asks with the original TfDelWarning "delete" text).
    pub(super) fn archive_delete(&mut self, pane: usize) {
        let Some((file, _, password)) = self.editable_archive(pane) else { return self.archive_read_only_notice() };
        let keys = self.archive_target_keys(pane);
        let Some(first) = keys.first().map(|k| k.rsplit('/').next().unwrap_or(k).to_owned()) else { return };
        self.delete_warning(DeleteWarningMode::Delete, keys.len(), &first, move |app, choice, _| {
            if choice == DeleteWarningChoice::No {
                return;
            }
            let title = app.tr("압축 안 항목 삭제", "Delete from archive").into_owned();
            app.archive_edit_task(title, &file.clone(), move |_, cancel| ndr::archive::delete_entries(&file, &keys, password.as_deref(), cancel));
        });
    }

    /// Renames archive items (original TfRename, asks per item and renames all at once after every name is decided).
    pub(super) fn archive_rename(&mut self, pane: usize) {
        let Some(archive) = self.editable_archive(pane) else { return self.archive_read_only_notice() };
        let items: Vec<(String, bool)> = self.target_entries(pane).into_iter().map(|e| (e.path.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_owned(), e.is_dir)).filter(|(k, _)| !k.is_empty()).collect();
        self.archive_rename_step(pane, archive, items, 0, Vec::new());
    }

    fn archive_rename_step(&mut self, pane: usize, archive: EditableArchive, items: Vec<(String, bool)>, index: usize, mut renames: Vec<(String, String)>) {
        let Some((key, is_dir)) = items.get(index).cloned() else {
            if renames.is_empty() {
                return;
            }
            let (file, _, password) = archive;
            let focus = renames.last().map(|(_, to)| to.rsplit('/').next().unwrap_or(to).to_owned());
            self.pending_focus[pane] = focus;
            let title = self.tr("압축 안 이름 바꾸기", "Rename in archive").into_owned();
            return self.archive_edit_task(title, &file.clone(), move |_, cancel| {
                for (from, to) in &renames {
                    ndr::archive::rename_entry(&file, from, to, password.as_deref(), cancel)?;
                }
                Ok(())
            });
        };
        let file_name = key.rsplit('/').next().unwrap_or(&key).to_owned();
        let (name, extension) = names::split_for_rename(&file_name, is_dir);
        let count = items.len();
        let shown = file_name.clone();
        self.rename_dialog(&shown, &name, &extension, index + 1, count, move |app, value, _| {
            let Some((new_name, new_extension)) = value else {
                return app.archive_rename_step(pane, archive, items, usize::MAX, renames);
            };
            let target = if new_extension.is_empty() { new_name } else { format!("{new_name}.{new_extension}") };
            if !target.is_empty() && target != file_name {
                let parent = key.rsplit_once('/').map(|(p, _)| format!("{p}/")).unwrap_or_default();
                renames.push((key.clone(), format!("{parent}{target}")));
            }
            app.archive_rename_step(pane, archive, items, index + 1, renames);
        });
    }

    /// New folder in the current archive folder (same text as the original new folder box).
    pub(super) fn archive_new_folder(&mut self, pane: usize) {
        let Some((file, prefix, password)) = self.editable_archive(pane) else { return self.archive_read_only_notice() };
        let title = self.root_text("TITLE_NEWFOLDER", "New Folder");
        let message = self.root_text("MSG_NEWFOLDER", "Type a folder name to be created.");
        self.prompt_file_name(&title, &message, &names::new_folder_default_name(), move |app, value, _| {
            let Some(name) = value.filter(|v| !v.trim().is_empty()) else { return };
            app.pending_focus[pane] = Some(name.clone());
            let title = app.tr("압축 안 새 폴더", "New folder in archive").into_owned();
            app.archive_edit_task(title, &file.clone(), move |_, cancel| ndr::archive::add_folder(&file, &prefix, &name, password.as_deref(), cancel));
        });
    }

    /// Empty file in the current archive folder.
    pub(super) fn archive_empty_file(&mut self, pane: usize) {
        let Some((file, prefix, password)) = self.editable_archive(pane) else { return self.archive_read_only_notice() };
        let title = self.root_text("TITLE_CREATEEMPTYFILE", "Create an empty file");
        let message = self.root_text("MSG_CREATEEMPTYFILE", "0 byte file will be created.\nType a new file name to be created.");
        self.prompt_file_name(&title, &message, "", move |app, value, _| {
            let Some(name) = value.filter(|v| !v.trim().is_empty()) else { return };
            app.pending_focus[pane] = Some(name.clone());
            let title = app.tr("압축 안 빈 파일", "Empty file in archive").into_owned();
            app.archive_edit_task(title, &file.clone(), move |_, cancel| {
                let holder = std::env::temp_dir().join(format!("ndr-empty-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())));
                std::fs::create_dir_all(&holder).map_err(|e| e.to_string())?;
                let result = std::fs::write(holder.join(&name), b"").map_err(|e| e.to_string()).and_then(|()| ndr::archive::add_entries(&file, &prefix, &[holder.join(&name)], password.as_deref(), cancel));
                let _ = std::fs::remove_dir_all(&holder);
                result
            });
        });
    }

    /// Drop location on an archive pane: an archive folder row (item path = archive key) gives that folder ("folder/"), otherwise None (current folder).
    pub(super) fn archive_drop_prefix(&self, pane: usize, destination: &Path) -> Option<String> {
        self.panes[pane].archive.as_ref()?;
        let row = self.panes[pane].entries.iter().find(|e| e.is_dir && !e.is_parent() && !e.drive && e.path == destination)?;
        let key = row.path.to_string_lossy().replace('\\', "/");
        Some(format!("{}/", key.trim_end_matches('/')))
    }

    /// Adds local files and folders to the current folder of the archive pane `target`. A move deletes the sources after everything is added.
    pub(super) fn archive_add_into(&mut self, target: usize, sources: Vec<PathBuf>, moving: bool) {
        self.archive_add_into_at(target, None, sources, moving);
    }

    /// Same as archive_add_into, but with `prefix` the items go into that archive folder ("folder/") (a dropped folder row).
    pub(super) fn archive_add_into_at(&mut self, target: usize, prefix: Option<String>, sources: Vec<PathBuf>, moving: bool) {
        let Some((file, current, password)) = self.editable_archive(target) else { return self.archive_read_only_notice() };
        let prefix = prefix.unwrap_or(current);
        if sources.is_empty() {
            return;
        }
        if sources.iter().any(|s| ndr::fsops::same_or_inside(&file, s)) {
            return self.notice("압축 파일을 그 자신 안에 넣을 수 없습니다.", "An archive cannot be added into itself.");
        }
        let title = if moving { self.tr("압축 안으로 이동", "Move into archive") } else { self.tr("압축 안으로 복사", "Copy into archive") }.into_owned();
        self.archive_edit_task(title, &file.clone(), move |events, cancel| {
            let mut conflicts = |source: &Path, entry: &ndr::archive::ArchiveEntry| ask_add_conflict(events, cancel, &file, source, entry);
            let skipped = ndr::archive::add_entries_with(&file, &prefix, &sources, password.as_deref(), cancel, &mut conflicts)?;
            if moving {
                // Sources skipped at a name conflict are kept.
                for source in sources.iter().filter(|s| !skipped_path_intersects(&skipped, s)) {
                    let removed = if source.is_dir() { std::fs::remove_dir_all(source) } else { std::fs::remove_file(source) };
                    removed.map_err(|e| format!("{}: {e}", source.display()))?;
                }
            }
            Ok(())
        });
    }

    /// Moves or copies the selected items of archive pane `source` to the other side: if the other side is a modifiable archive, extract to a temp folder and add
    /// them to it; if it is a local folder, extract into that folder. A move then deletes them from the original archive (only when it is modifiable).
    pub(super) fn archive_transfer_out(&mut self, source: usize, keys: Vec<String>, destination: PathBuf, into: Option<EditableArchive>, moving: bool) {
        let Some(location) = self.panes[source].archive.clone() else { return };
        let file = ndr::archive::first_volume_path(&location.file);
        if moving && !ndr::archive::can_modify(&file) {
            return self.archive_read_only_notice();
        }
        if into.as_ref().is_some_and(|(target, _, _)| ndr::names::equals_ignore_case(&file.to_string_lossy(), &target.to_string_lossy())) {
            return self.notice("같은 압축 파일의 두 패널 간 복사·이동은 지원하지 않습니다. 다른 압축 파일을 선택하세요.", "Copying or moving between two panes of the same archive is not supported; choose another archive.");
        }
        if keys.is_empty() {
            return;
        }
        let password = (!location.password.is_empty()).then(|| location.password.clone());
        let title = if moving { self.tr("압축에서 이동", "Move out of archive") } else { self.tr("압축에서 복사", "Copy out of archive") }.into_owned();
        self.archive_edit_task(title, &file.clone(), move |events, cancel| {
            // Destinations that may have been skipped at a conflict (skip, overwrite older). Those items stay in the archive even on a move (a folder keeps the whole folder).
            let mut held: Vec<PathBuf> = Vec::new();
            let mut conflicts = |from: &Path, to: &Path| {
                let choice = super::commands::ask_conflict(events, cancel, from, to);
                if extraction_choice_skips(choice, from, to) {
                    held.push(to.to_path_buf());
                }
                choice
            };
            let mut passwords = |archive: &Path, _: Option<&str>| super::commands::ask_password(events, cancel, archive);
            let name_of = |k: &String| k.rsplit('/').next().unwrap_or(k).to_owned();
            let kept: Vec<String> = match &into {
                Some((target_file, target_prefix, target_password)) => {
                    let stage = std::env::temp_dir().join(format!("ndr-archive-move-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())));
                    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
                    let result = (|| {
                        ndr::archive::extract_entries_with_program(&file, &stage, &keys, password.as_deref(), cancel, &mut conflicts, &mut passwords, None, &mut |_, _, _| {})?;
                        // Chosen items are extracted by name only, without their parent path.
                        let extracted: Vec<PathBuf> = keys.iter().map(|k| stage.join(name_of(k))).filter(|p| p.exists()).collect();
                        let mut add_conflicts = |source: &Path, entry: &ndr::archive::ArchiveEntry| ask_add_conflict(events, cancel, target_file, source, entry);
                        let skipped = ndr::archive::add_entries_with(target_file, target_prefix, &extracted, target_password.as_deref(), cancel, &mut add_conflicts)?;
                        Ok::<_, String>(keys.iter().filter(|k| skipped_path_intersects(&skipped, &stage.join(name_of(k)))).cloned().collect())
                    })();
                    let _ = std::fs::remove_dir_all(&stage);
                    result?
                }
                None => {
                    std::fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
                    ndr::archive::extract_entries_with_program(&file, &destination, &keys, password.as_deref(), cancel, &mut conflicts, &mut passwords, None, &mut |_, _, _| {})?;
                    keys.iter()
                        .filter(|k| {
                            let target = destination.join(name_of(k));
                            skipped_path_intersects(&held, &target)
                        })
                        .cloned()
                        .collect()
                }
            };
            if moving {
                let moved: Vec<String> = keys.iter().filter(|k| !kept.contains(k)).cloned().collect();
                ndr::archive::delete_entries(&file, &moved, password.as_deref(), cancel)?;
            }
            Ok(())
        });
    }
}

/// When the archive already has the name (on the job thread): ask with the original TfFileExists. The destination side shows "archive\inner path" with that item's size and time.
/// Overwrite = replace (folders merge), replace folder = delete then add, newer only = only when the source is newer, rename = ask for a new name.
fn ask_add_conflict(events: &Sender<Event>, cancel: &AtomicBool, archive: &Path, source: &Path, entry: &ndr::archive::ArchiveEntry) -> ndr::archive::AddConflict {
    use ndr::archive::AddConflict;
    let shown = archive.join(entry.key.replace('/', "\\"));
    super::file_operation::set_virtual_entry(&shown, Some((entry.is_dir, (!entry.is_dir).then_some(entry.size), entry.modified)));
    let choice = super::commands::ask_conflict(events, cancel, source, &shown);
    super::file_operation::set_virtual_entry(&shown, None);
    match choice {
        ConflictChoice::Overwrite | ConflictChoice::Resume => AddConflict::Merge,
        ConflictChoice::ReplaceFolder => AddConflict::Replace,
        ConflictChoice::OverwriteNewer => {
            let newer = match (std::fs::metadata(source).and_then(|m| m.modified()).ok(), entry.modified) {
                (Some(new), Some(old)) => new > old,
                _ => true,
            };
            if newer { AddConflict::Merge } else { AddConflict::Skip }
        }
        ConflictChoice::Skip => AddConflict::Skip,
        ConflictChoice::Cancel => AddConflict::Cancel,
        ConflictChoice::Rename => {
            let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (tx, rx) = std::sync::mpsc::channel();
            if events.send(Event::RenameTarget { name, reply: tx }).is_err() {
                return AddConflict::Cancel;
            }
            loop {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return AddConflict::Cancel;
                }
                match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(Some(new_name)) if !new_name.trim().is_empty() => return AddConflict::Rename(new_name),
                    Ok(_) => return AddConflict::Skip,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => return AddConflict::Cancel,
                }
            }
        }
    }
}

#[cfg(test)]
mod archive_move_safety_tests {
    use super::*;

    #[test]
    fn skipped_nested_items_keep_the_parent_source_on_a_move() {
        let root = PathBuf::from("C:\\ndr-test\\target");
        let nested = root.join("folder").join("deep").join("unmoved.txt");
        assert!(skipped_path_intersects(&[nested], &root.join("folder")));
        assert!(!skipped_path_intersects(&[root.join("other.txt")], &root.join("folder")));
        assert!(skipped_path_intersects(&[root.join("folder")], &root.join("folder").join("deep").join("unmoved.txt")));
    }

    #[test]
    fn newer_only_keeps_source_only_when_not_actually_transferred() {
        let root = std::env::temp_dir().join(format!("ndr-move-conflict-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("source.txt");
        let target = root.join("target.txt");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&target, b"target").unwrap();
        filetime::set_file_mtime(&source, filetime::FileTime::from_unix_time(200, 0)).unwrap();
        filetime::set_file_mtime(&target, filetime::FileTime::from_unix_time(100, 0)).unwrap();
        assert!(!extraction_choice_skips(ConflictChoice::OverwriteNewer, &source, &target));
        assert!(extraction_choice_skips(ConflictChoice::Skip, &source, &target));
        filetime::set_file_mtime(&target, filetime::FileTime::from_unix_time(300, 0)).unwrap();
        assert!(extraction_choice_skips(ConflictChoice::OverwriteNewer, &source, &target));
        std::fs::remove_dir_all(root).unwrap();
    }
}
