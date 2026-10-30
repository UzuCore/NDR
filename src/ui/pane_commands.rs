//! C# NDR 패널 명령(선택, 이동, 보기, 숨김, 폴더 크기)을 옮긴 것.
use super::*;
use ndr::msgbox::{MB_ICONASTERISK, MB_OK};

/// C# ChangeAttributesAsync 진행 상태.
pub(super) struct AttributeRun {
    pane: usize,
    paths: Vec<PathBuf>,
    index: usize,
    completed: usize,
    skipped: usize,
    errors: Vec<String>,
    /// 원본 FUN_0043657c: 적용했거나 건너뛴 항목(실패하지 않은 것)은 목록 표시를 푼다(Select(번호, false)). 취소한 항목부터는 남는다.
    processed: Vec<PathBuf>,
}

impl App {
    fn side_name(&self, pane: usize) -> std::borrow::Cow<'static, str> {
        if pane == 0 { self.tr("왼쪽", "Left") } else { self.tr("오른쪽", "Right") }
    }

    /// 원본 bSelectAllClick(0x431450, 수직 막대 "모두 선택" 단추, 스킨 테마). 폴더도 고를지 = 설정(selectfolder) XOR Shift.
    /// 표시가 없으면 SelectAll(폴더 여부). 있으면: 폴더·파일을 모두 골랐으면 지우고 파일만, 파일만 모두 골랐으면 폴더까지, 그 밖에는 파일을 더한다.
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

    /// 원본 miInvertSelectionClick(0x43163c, 스킨 테마): 파일은 늘, 폴더는 설정(selectfolder)이 켜졌을 때만 표시를 뒤집는다.
    /// 드라이브 항목은 고르지 않는다(Select가 종류 4를 거른다). "선택 복원"용 기억은 바꾸지 않는다.
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

    /// 원본 miSelectSameExtClick(0x431720)·miSelectSameNameClick(0x431a04)(스킨 테마): 커서가 파일이면 확장자(또는 확장자를 뺀 이름)가
    /// 같은 파일 가운데 하나라도 표시가 없으면 모두 고르고, 다 골라져 있으면 모두 푼다(다른 표시는 그대로).
    /// 같은 확장자에서 커서가 폴더면 폴더 전체를 같은 방식으로 뒤집는다. 같은 이름은 파일일 때만.
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

    /// 원본 CopySelection(목록 → +0xd08): 파일 작업을 시작할 때 지금 표시를 "선택 복원"(miRestoreLastSel)용으로 남긴다.
    pub(super) fn remember_marks(&mut self) {
        let pane = &self.panes[self.active];
        let mut rows: Vec<usize> = pane.selected.iter().copied().collect();
        rows.sort_unstable();
        self.last_selection[self.active] = rows.into_iter().filter_map(|i| pane.entries.get(i).map(|e| e.path.clone())).collect();
    }

    /// C# GoToRootAsync: 압축 파일 안이면 압축 파일의 맨 위, 아니면 드라이브 루트(이미 루트면 그대로). 원격은 "/".
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

    /// C# GoToLastDestinationAsync: 마지막 대상 폴더가 없거나 사라졌으면 알린다.
    pub(super) fn go_last_destination(&mut self) {
        let destination = self.settings.text("LastDestination", "");
        if destination.trim().is_empty() || !Path::new(&destination).is_dir() {
            let text = self.tr("아직 사용할 수 있는 마지막 대상 폴더가 없습니다.", "There is no last destination folder to use yet.").into_owned();
            return self.system_box("NDR", &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
        }
        self.navigate(self.active, destination.into(), true);
    }

    /// C# ToggleHiddenAsync: 활성 패널의 숨김 표시를 바꾸고 설정(Left/RightShowHiddenFiles, ShowHiddenFiles)에 남긴 뒤 새로 고친다.
    pub(super) fn toggle_hidden(&mut self) {
        let i = self.active;
        let visible = !self.panes[i].show_hidden;
        self.panes[i].show_hidden = visible;
        self.settings.set_flag(if i == 0 { "LeftShowHiddenFiles" } else { "RightShowHiddenFiles" }, visible);
        let any = self.panes[0].show_hidden || self.panes[1].show_hidden;
        self.settings.set_flag("ShowHiddenFiles", any);
        // 원본 miShowHiddenClick(0x4348e0): 새로 고침(FUN_00487ce8)을 (커서 지킴 1, 표시 지킴 0)으로 부른다.
        self.panes[i].selected.clear();
        self.refresh(i);
        let side = self.side_name(i);
        self.status = if self.language == "ko" {
            format!("{side} 패널 숨김 항목 {}", if visible { "표시" } else { "숨김" })
        } else {
            format!("{side} pane hidden items {}", if visible { "shown" } else { "hidden" })
        };
    }

    /// C# SetViewStyle: Left/RightViewStyle에 "Auto"/"Details"/"Names"/"Thumbnails"를 남기고 상태 줄에 알린다.
    pub(super) fn set_view_style(&mut self, style: &str) {
        let i = self.active;
        self.settings.set_text(if i == 0 { "LeftViewStyle" } else { "RightViewStyle" }, style);
        let side = self.side_name(i);
        self.status = if self.language == "ko" { format!("{side} 패널 보기: {style}") } else { format!("{side} pane view: {style}") };
    }

    /// C# CalculateSelectedFolderSizeAsync: 같은 경로는 한 번, 고른 폴더 안에 든 항목은 빼고, 진행 창에서 센 뒤 요약 상자를 띄운다.
    /// 스킨 테마는 원본 miCalculateSelectedFolderSizeClick(0x4457d4): 대상(표시, 없으면 커서) 가운데 폴더(종류 1)만 모아,
    /// 있으면 표시를 지우고 패널의 폴더 크기 세기(FUN_0048efa8)에 넘긴다. 크기는 목록의 크기 칸에 나온다(요약 상자 없음).
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

    /// C# ChangeAttributesAsync: 항목마다 속성 창. 취소는 멈추고, 건너뛰기는 다음 항목, 모두 적용은 남은 항목 전부(지금 항목 포함)에
    /// 적용하고 끝낸다. 적용은 따로 돌고 끝나면 다음 항목으로 넘어간다. 끝나면 새로 고치고 오류 창이나 상태 줄로 알린다.
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
