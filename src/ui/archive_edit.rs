//! 사용자 요청(원본에는 없는 기능): 압축 파일 안에서 지우기·이름 바꾸기·새 폴더·빈 파일, 압축 안으로 복사·이동(더하기),
//! 압축 밖으로 이동(풀고 지우기), 압축에서 다른 압축으로 복사·이동. 동봉 7-Zip이 고칠 수 있는 zip·7z·tar·wim만 된다
//! (ndr::archive::can_modify). 원본 NexusFile은 압축 안 항목을 풀기만 했다.
use super::original_dialogs::{DeleteWarningChoice, DeleteWarningMode};
use super::*;
use ndr::names;

/// 고칠 압축: (첫 볼륨 파일, 안 경로 "폴더/" 또는 "", 아는 암호).
pub(super) type EditableArchive = (PathBuf, String, Option<String>);

impl App {
    /// 그 패널이 고칠 수 있는 압축 안이면 그 압축.
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

    /// 고른 항목(없으면 커서 항목)의 압축 안 키("폴더/파일").
    fn archive_target_keys(&self, pane: usize) -> Vec<String> {
        self.panes[pane].targets().into_iter().map(|k| k.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_owned()).filter(|k| !k.is_empty()).collect()
    }

    /// 압축을 고치는 작업(진행 창, 취소 가능). 끝나면 두 패널을 다시 읽는다(report_job).
    fn archive_edit_task(&mut self, title: String, archive: &Path, task: impl FnOnce(&Sender<Event>, &AtomicBool) -> Result<(), String> + Send + 'static) {
        let progress = super::file_operation::ProgressInfo { from: Some(archive.to_string_lossy().into_owned()), caption: Some(title.clone()), ..Default::default() };
        self.start_progress_task(title, progress, move |events, cancel| task(&events, &cancel));
    }

    /// 압축 안 항목 지우기(휴지통 없이 바로, 원본 TfDelWarning "삭제" 문구로 묻는다).
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

    /// 압축 안 항목 이름 바꾸기(원본 TfRename, 항목마다 묻고 모두 정한 뒤 한 번에 바꾼다).
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

    /// 압축 안 지금 폴더에 새 폴더(원본 새 폴더 상자와 같은 글).
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

    /// 압축 안 지금 폴더에 빈 파일.
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

    /// 압축 패널에 놓은 곳: 압축 안 폴더 줄(항목 경로 = 압축 안 키)이면 그 폴더("폴더/"), 아니면 None(지금 폴더).
    pub(super) fn archive_drop_prefix(&self, pane: usize, destination: &Path) -> Option<String> {
        self.panes[pane].archive.as_ref()?;
        let row = self.panes[pane].entries.iter().find(|e| e.is_dir && !e.is_parent() && !e.drive && e.path == destination)?;
        let key = row.path.to_string_lossy().replace('\\', "/");
        Some(format!("{}/", key.trim_end_matches('/')))
    }

    /// 로컬 파일·폴더를 압축 패널(`target`)의 지금 폴더에 더한다. 이동이면 다 더한 뒤 원본을 지운다.
    pub(super) fn archive_add_into(&mut self, target: usize, sources: Vec<PathBuf>, moving: bool) {
        self.archive_add_into_at(target, None, sources, moving);
    }

    /// archive_add_into와 같고 `prefix`가 있으면 그 압축 안 폴더("폴더/")에 더한다(끌어 놓은 폴더 줄).
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
                // 같은 이름에서 건너뛴 항목은 원본을 남긴다.
                for source in sources.iter().filter(|s| !skipped.contains(s)) {
                    let removed = if source.is_dir() { std::fs::remove_dir_all(source) } else { std::fs::remove_file(source) };
                    removed.map_err(|e| format!("{}: {e}", source.display()))?;
                }
            }
            Ok(())
        });
    }

    /// 압축 패널(`source`)의 고른 항목을 다른 쪽으로 옮기거나 복사한다: 다른 쪽이 고칠 수 있는 압축이면 임시 폴더에 풀어 그 압축에
    /// 더하고, 로컬 폴더면 그 폴더로 푼다. 이동이면 끝난 뒤 원래 압축에서 지운다(고칠 수 있는 압축일 때만).
    pub(super) fn archive_transfer_out(&mut self, source: usize, keys: Vec<String>, destination: PathBuf, into: Option<EditableArchive>, moving: bool) {
        let Some(location) = self.panes[source].archive.clone() else { return };
        let file = ndr::archive::first_volume_path(&location.file);
        if moving && !ndr::archive::can_modify(&file) {
            return self.archive_read_only_notice();
        }
        if keys.is_empty() {
            return;
        }
        let password = (!location.password.is_empty()).then(|| location.password.clone());
        let title = if moving { self.tr("압축에서 이동", "Move out of archive") } else { self.tr("압축에서 복사", "Copy out of archive") }.into_owned();
        self.archive_edit_task(title, &file.clone(), move |events, cancel| {
            // 같은 이름에서 건너뛰었을 수 있는 대상(건너뛰기·새것만 덮어쓰기). 그 항목은 이동이어도 압축에서 지우지 않는다(폴더면 폴더 전체를 남긴다).
            let mut held: Vec<PathBuf> = Vec::new();
            let mut conflicts = |from: &Path, to: &Path| {
                let choice = super::commands::ask_conflict(events, cancel, from, to);
                if matches!(choice, ConflictChoice::Skip | ConflictChoice::OverwriteNewer) {
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
                        // 고른 항목은 상위 경로 없이 이름만으로 풀린다.
                        let extracted: Vec<PathBuf> = keys.iter().map(|k| stage.join(name_of(k))).filter(|p| p.exists()).collect();
                        let mut add_conflicts = |source: &Path, entry: &ndr::archive::ArchiveEntry| ask_add_conflict(events, cancel, target_file, source, entry);
                        let skipped = ndr::archive::add_entries_with(target_file, target_prefix, &extracted, target_password.as_deref(), cancel, &mut add_conflicts)?;
                        Ok::<_, String>(keys.iter().filter(|k| skipped.contains(&stage.join(name_of(k)))).cloned().collect())
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
                            held.iter().any(|h| ndr::fsops::same_or_inside(h, &target))
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

/// 압축 안에 같은 이름이 있을 때(작업 스레드에서): 원본 TfFileExists로 묻는다. 대상 칸은 "압축 파일\안 경로"와 그 항목의 크기·시각.
/// 덮어쓰기 = 바꾸기(폴더는 합치기), 폴더 바꾸기 = 지우고 더하기, 새것만 = 원본이 더 새로울 때만, 이름 바꾸기 = 새 이름을 묻는다.
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
