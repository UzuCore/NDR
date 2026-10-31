//! Ports the C# NDR pane commands (selection, navigation, view, hidden items, folder size).
use super::*;

/// C# ChangeAttributesAsync progress state.
pub(super) struct AttributeRun {
    pane: usize,
    paths: Vec<PathBuf>,
    index: usize,
    completed: usize,
    skipped: usize,
    errors: Vec<String>,
    /// Original FUN_0043657c: applied or skipped items (those that did not fail) are unmarked in the list (Select(index, false)). Items from the cancelled one on stay marked.
    processed: Vec<PathBuf>,
}

impl App {
    fn side_name(&self, pane: usize) -> std::borrow::Cow<'static, str> {
        if pane == 0 { self.tr("왼쪽", "Left") } else { self.tr("오른쪽", "Right") }
    }

    /// Original bSelectAllClick (0x431450, vertical bar "select all" button, skin theme). Whether folders are included = setting (selectfolder) XOR Shift.
    /// Without marks SelectAll(folders). With marks: if all folders and files are selected, clear and select files only; if all files only, include folders; otherwise add files.
    pub(super) fn original_button_select_all(&mut self, shift: bool) {
        let folders_too = self.settings.flag("SelectFolders", false) != shift;
        let pane = &mut self.panes[self.active];
        let files: Vec<usize> = (0..pane.entries.len()).filter(|&i| !pane.entries[i].is_dir && !pane.entries[i].drive).collect();
        let folders: Vec<usize> = (0..pane.entries.len()).filter(|&i| pane.entries[i].is_dir && !pane.entries[i].drive && !pane.entries[i].is_parent()).collect();
        let add = |pane: &mut Pane, folders_too: bool| {
            pane.selected.extend(files.iter().copied());
            if folders_too {
                pane.selected.extend(folders.iter().copied());
            }
        };
        if pane.selected.is_empty() {
            return add(pane, folders_too);
        }
        let all_folders = folders.iter().all(|i| pane.selected.contains(i));
        let all_files = files.iter().all(|i| pane.selected.contains(i));
        if all_folders && all_files {
            pane.selected.clear();
            add(pane, false);
        } else {
            add(pane, all_files);
        }
    }

    /// Original miInvertSelectionClick (0x43163c, skin theme): flips files always and folders only when the setting (selectfolder) is on.
    /// Drive items are never selected (Select skips kind 4). The memory for "Restore Selection" is not changed.
    pub(super) fn original_invert_selection(&mut self) {
        let folders = self.settings.flag("SelectFolders", false);
        let pane = &mut self.panes[self.active];
        for i in 0..pane.entries.len() {
            let entry = &pane.entries[i];
            if entry.drive || (entry.is_dir && !folders) {
                continue;
            }
            if !pane.selected.insert(i) {
                pane.selected.remove(&i);
            }
        }
    }

    /// Original miSelectSameExtClick (0x431720) / miSelectSameNameClick (0x431a04) (skin theme): when the cursor is a file, if any file with the same extension (or the name without extension)
    /// is unmarked, mark them all; if all are marked, unmark them all (other marks stay).
    /// For same extension, when the cursor is a folder, all folders flip the same way. Same name works only on files.
    pub(super) fn original_select_same(&mut self, same_extension: bool) {
        let pane = &mut self.panes[self.active];
        if pane.loading {
            return;
        }
        let Some(focused) = pane.entries.get(pane.cursor).filter(|e| !e.drive && !e.is_parent()) else { return };
        let rows: Vec<usize> = if !focused.is_dir {
            let key = |name: &str| if same_extension { ndr::parity::list_extension(name).to_owned() } else { ndr::parity::list_stem(name).to_owned() };
            let wanted = key(&focused.name);
            (0..pane.entries.len()).filter(|&i| !pane.entries[i].is_dir && !pane.entries[i].drive && ndr::names::equals_ignore_case(&key(&pane.entries[i].name), &wanted)).collect()
        } else if same_extension {
            (0..pane.entries.len()).filter(|&i| pane.entries[i].is_dir && !pane.entries[i].drive && !pane.entries[i].is_parent()).collect()
        } else {
            return;
        };
        let select = rows.iter().any(|i| !pane.selected.contains(i));
        for i in rows {
            if select {
                pane.selected.insert(i);
            } else {
                pane.selected.remove(&i);
            }
        }
    }

    /// Original CopySelection (list -> +0xd08): when a file job starts, the current marks are saved for "Restore Selection" (miRestoreLastSel).
    pub(super) fn remember_marks(&mut self) {
        let pane = &self.panes[self.active];
        let mut rows: Vec<usize> = pane.selected.iter().copied().collect();
        rows.sort_unstable();
        self.last_selection[self.active] = rows.into_iter().filter_map(|i| pane.entries.get(i).map(|e| e.path.clone())).collect();
    }

    /// C# GoToRootAsync: the archive top inside an archive, otherwise the drive root (unchanged when already at the root). Remote goes to "/".
    pub(super) fn go_root(&mut self) {
        let i = self.active;
        if self.panes[i].remote.is_some() {
            return self.ftp_root(i);
        }
        if let Some(location) = self.panes[i].archive.as_mut() {
            if !location.prefix.is_empty() {
                location.prefix.clear();
                self.refresh(i);
            }
            return;
        }
        let path = std::path::absolute(&self.panes[i].path).unwrap_or_else(|_| self.panes[i].path.clone());
        let Some(root) = path.ancestors().last().map(Path::to_path_buf) else { return };
        if !ndr::names::equals_ignore_case(&root.to_string_lossy(), &self.panes[i].path.to_string_lossy()) {
            self.navigate(i, root, true);
        }
    }

    /// Original FUN_00443f14 case 0x33 (GoToLastDest, Ctrl+Backspace): when the last destination (+0xe20) is set and PathFileExistsW, the active pane
    /// goes there (local mode, FUN_00459968); otherwise nothing happens. Previously NDR showed the C# notice "no last destination folder yet".
    pub(super) fn go_last_destination(&mut self) {
        let destination = self.settings.text("LastDestination", "");
        if destination.trim().is_empty() || !Path::new(&destination).exists() {
            return;
        }
        self.navigate(self.active, destination.into(), true);
    }

    /// C# ToggleHiddenAsync: toggles the active pane's hidden item display, stores it in the settings (Left/RightShowHiddenFiles, ShowHiddenFiles) and refreshes.
    pub(super) fn toggle_hidden(&mut self) {
        let i = self.active;
        let visible = !self.panes[i].show_hidden;
        self.panes[i].show_hidden = visible;
        self.settings.set_flag(if i == 0 { "LeftShowHiddenFiles" } else { "RightShowHiddenFiles" }, visible);
        let any = self.panes[0].show_hidden || self.panes[1].show_hidden;
        self.settings.set_flag("ShowHiddenFiles", any);
        // Original miShowHiddenClick (0x4348e0): calls refresh (FUN_00487ce8) with (keep cursor 1, keep marks 0).
        self.panes[i].selected.clear();
        self.refresh(i);
        let side = self.side_name(i);
        self.status = if self.language == "ko" {
            format!("{side} 패널 숨김 항목 {}", if visible { "표시" } else { "숨김" })
        } else {
            format!("{side} pane hidden items {}", if visible { "shown" } else { "hidden" })
        };
    }

    /// C# SetViewStyle: stores "Auto"/"Details"/"Names"/"Thumbnails" in Left/RightViewStyle and reports it in the status line.
    pub(super) fn set_view_style(&mut self, style: &str) {
        let i = self.active;
        self.settings.set_text(if i == 0 { "LeftViewStyle" } else { "RightViewStyle" }, style);
        let side = self.side_name(i);
        self.status = if self.language == "ko" { format!("{side} 패널 보기: {style}") } else { format!("{side} pane view: {style}") };
    }

    /// C# CalculateSelectedFolderSizeAsync: each path once, items inside chosen folders left out, counted in a progress window, then a summary box.
    /// The skin theme follows original miCalculateSelectedFolderSizeClick (0x4457d4): collects only folders (kind 1) among the targets (marks, otherwise the cursor),
    /// and if any, clears the marks and hands them to the pane folder size count (FUN_0048efa8). Sizes appear in the list's size cell (no summary box).
    pub(super) fn calculate_folder_size(&mut self) {
        let i = self.active;
        let pane = &self.panes[i];
        if pane.archive.is_some() || pane.remote.is_some() {
            return;
        }
        let mut rows: Vec<usize> = if pane.selected.is_empty() { vec![pane.cursor] } else { pane.selected.iter().copied().collect() };
        rows.sort_unstable();
        rows.retain(|&r| pane.entries.get(r).is_some_and(|e| e.is_dir && !e.drive && !e.is_parent()));
        if rows.is_empty() {
            return;
        }
        self.panes[i].selected.clear();
        for row in rows {
            let path = self.panes[i].entries[row].path.clone();
            self.panes[i].folder_sizes.remove(&path);
            self.enqueue_folder_size(i, row);
        }
    }

    /// C# ChangeAttributesAsync: an attributes window per item. Cancel stops, skip goes to the next item, apply to all applies to all remaining items (including the current one)
    /// and ends. Applying runs separately and moves on to the next item when done. At the end it refreshes and reports through an error window or the status line.
    pub(super) fn change_attributes(&mut self) {
        let i = self.active;
        if self.panes[i].remote.is_some() {
            return self.ftp_change_attributes(i);
        }
        if self.panes[i].archive.is_some() {
            return self.warn("압축 파일 안에서는 속성을 바꿀 수 없습니다.", "Attributes cannot be changed inside an archive.");
        }
        let paths = self.panes[i].targets();
        if paths.is_empty() {
            return self.warn("속성을 바꿀 파일이나 폴더를 선택하세요.", "Select the files or folders to change.");
        }
        self.attribute_step(AttributeRun { pane: i, paths, index: 0, completed: 0, skipped: 0, errors: vec![], processed: vec![] });
    }

    pub(super) fn attribute_step(&mut self, mut run: AttributeRun) {
        let Some(path) = run.paths.get(run.index).cloned() else { return self.finish_attributes(run) };
        self.attribute_dialog(&path.to_string_lossy(), move |app, action, changes, _| match (action, changes) {
            (super::original_dialogs::AttributeAction::Cancel, _) => app.finish_attributes(run),
            (super::original_dialogs::AttributeAction::Skip, _) | (_, None) => {
                if action == super::original_dialogs::AttributeAction::Skip {
                    run.skipped += 1;
                }
                run.processed.push(run.paths[run.index].clone());
                run.index += 1;
                app.attribute_step(run);
            }
            (action, Some(changes)) => {
                let all = action == super::original_dialogs::AttributeAction::ApplyToAll;
                let targets: Vec<PathBuf> = if all { run.paths[run.index..].to_vec() } else { vec![run.paths[run.index].clone()] };
                let tx = app.async_tx.clone();
                thread::spawn(move || {
                    let errors = ndr::tools::change_attributes_and_time(&targets, &changes);
                    let failed = |path: &PathBuf| errors.iter().any(|e| e.starts_with(&format!("{}: ", path.display())));
                    run.processed.extend(targets.iter().filter(|p| !failed(p)).cloned());
                    run.errors.extend(errors);
                    run.completed += targets.len();
                    run.index = if all { run.paths.len() } else { run.index + 1 };
                    let _ = tx.send(AsyncResult::AttributesApplied(run));
                });
            }
        });
    }

    pub(super) fn finish_attributes(&mut self, run: AttributeRun) {
        let pane = &mut self.panes[run.pane];
        let rows: Vec<usize> = pane.entries.iter().enumerate().filter(|(_, e)| run.processed.contains(&e.path)).map(|(row, _)| row).collect();
        for row in rows {
            pane.selected.remove(&row);
        }
        self.refresh(run.pane);
        let ko = self.language == "ko";
        let n = |value: usize| ndr::parity::format_size(value as u64, "B", true).trim_end_matches(" B").to_owned();
        if !run.errors.is_empty() {
            let title = self.tr("일부 항목의 속성을 변경하지 못했습니다.", "Some attributes could not be changed.").into_owned();
            let text = run.errors.iter().take(80).cloned().collect::<Vec<_>>().join("\r\n");
            self.text = Some(TextDocument { title, text });
        } else if run.completed > 0 && run.skipped > 0 {
            self.status = if ko { format!("{}개 항목의 속성을 변경하고 {}개 항목을 건너뛰었습니다.", n(run.completed), n(run.skipped)) } else { format!("Changed {} items and skipped {}.", n(run.completed), n(run.skipped)) };
        } else if run.completed > 0 {
            self.status = if ko { format!("{}개 항목의 속성을 변경했습니다.", n(run.completed)) } else { format!("Changed the attributes of {} items.", n(run.completed)) };
        } else if run.skipped > 0 {
            self.status = if ko { format!("{}개 항목을 건너뛰었습니다.", n(run.skipped)) } else { format!("Skipped {} items.", n(run.skipped)) };
        }
    }

}
