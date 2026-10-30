//! C# NDR의 이름·설명·열기·삭제 흐름을 옮긴 것. 각 함수는 원본 주소와 C# 이름을 적는다.
//! 대화 상자는 콜백으로 결과를 돌려주므로, 항목마다 대화 상자를 띄우는 반복은 상태를 다음 단계로 넘기는 식으로 만든다.
use super::original_dialogs::{DeleteConfirmChoice, DeleteWarningChoice, DeleteWarningMode};
use super::*;
use super::file_operation::JobReport;
use ndr::msgbox::{IDYES, MB_ICONHAND, MB_OK, MB_YESNO};
use ndr::names;

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

/// RenameAsync 진행 상태.
struct RenameRun {
    pane: usize,
    items: Vec<Entry>,
    index: usize,
    focused: Option<String>,
    focus_after: Option<String>,
}

/// DeleteAsync의 보호 항목 확인 진행 상태.
pub(super) struct DeleteRun {
    pub paths: Vec<PathBuf>,
    pub items: Vec<(PathBuf, u32)>,
    pub index: usize,
    pub approve_all: bool,
    pub skipped: Vec<PathBuf>,
}

/// C# DeleteWarningWindow.DisplayName: 끝 구분자를 뗀 마지막 이름, 없으면(드라이브 루트) 경로 그대로.
pub(super) fn display_name(path: &Path) -> String {
    let text = path.to_string_lossy();
    let trimmed = text.trim_end_matches(['\\', '/']);
    let name = Path::new(trimmed).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() { text.into_owned() } else { name }
}

impl App {
    /// [RES]/[Root] 문구(언어 파일 값, 없으면 원본 영어 리터럴).
    fn root_text(&self, key: &str, fallback: &str) -> String {
        self.language_pack.text(&format!("RES.{key}"), fallback)
    }

    /// C#이 MessageBox.Show(this, text)로 알리는 NDR 자체 안내(원본에 없는 경우).
    pub(super) fn notice(&mut self, korean: &str, english: &str) {
        let text = self.tr(korean, english).into_owned();
        self.system_box("", &text, MB_OK, |_, _, _| {});
    }

    /// C# RunSafely가 예외를 보이는 모양: MessageBox(메시지, "NDR", OK, Warning).
    pub(super) fn warn(&mut self, korean: &str, english: &str) {
        let text = self.tr(korean, english).into_owned();
        self.system_box("NDR", &text, MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
    }

    /// 표시 순서의 대상 항목(원본 FUN_00485bbc: 표시한 항목, 없으면 커서 항목).
    pub(super) fn target_entries(&self, pane: usize) -> Vec<Entry> {
        let p = &self.panes[pane];
        if p.loading {
            return vec![];
        }
        ndr::parity::collect_items(p.entries.len(), &p.selected, Some(p.cursor)).into_iter().filter_map(|i| p.entries.get(i).filter(|e| !e.drive && !e.is_parent()).cloned()).collect()
    }

    fn focused_name(&self, pane: usize) -> Option<String> {
        let p = &self.panes[pane];
        (!p.loading).then(|| p.entries.get(p.cursor).map(|e| e.name.clone())).flatten()
    }

    /// C# RefreshPaneAsync 후 FocusPaneItemByName: 새 목록이 오면 그 이름(대소문자 무시)에 커서를 둔다.
    pub(super) fn refresh_and_focus(&mut self, pane: usize, name: Option<String>) {
        self.pending_focus[pane] = name;
        self.refresh(pane);
    }

    /// 원본 FUN_0040c31c(C# ShowNameOperationError): 시스템 오류 문구(없으면 MSG_UNKNOWN_ERROR), TITLE_ERROR 캡션, 오류 아이콘.
    /// ask_continue면 "\n" + MSG_WANTTOCONTINUE2와 예/아니요. 콜백은 계속할지(true) 받는다.
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

    /// 원본 FUN_00432378(C# NewFolderAsync): 기본값은 짧은 날짜('\','/'→'-'), 다듬기·문자 거르기 없음, 빈 입력은 아무것도 안 함,
    /// CreateDirectoryW(중간 폴더 없음, 같은 이름은 실패). 실패는 오류 상자만, 성공은 새로 고친 뒤 새 폴더에 커서.
    pub(super) fn new_folder(&mut self) {
        let pane = self.active;
        if self.panes[pane].remote.is_some() {
            return self.ftp_new_folder(pane);
        }
        if self.panes[pane].archive.is_some() {
            return self.notice("압축 파일 안에는 새 폴더를 만들 수 없습니다.", "A folder cannot be created inside an archive.");
        }
        let directory = self.panes[pane].path.to_string_lossy().into_owned();
        let title = self.root_text("TITLE_NEWFOLDER", "New Folder");
        let message = self.root_text("MSG_NEWFOLDER", "Type a folder name to be created.");
        self.prompt(&title, &message, &names::new_folder_default_name(), None, false, move |app, value, _| {
            let Some(name) = value.filter(|v| !v.is_empty()) else { return };
            let path = names::combine_typed_name(&directory, &name);
            let error = names::create_directory(&path);
            if error != 0 {
                return app.show_name_operation_error(error, false, |_, _, _| {});
            }
            app.refresh_and_focus(pane, Some(display_name(Path::new(&path))));
        });
    }

    /// 원본 FUN_00434630(C# CreateEmptyFileAsync): 빈 기본값, CreateFileW(CREATE_NEW)라 있는 항목은 비우지 않는다.
    /// 승인된 수정: 원본은 실패를 조용히 넘기지만 NDR은 알린다. 그 뒤 새로 고치고 입력한 이름에 커서.
    pub(super) fn create_empty_file(&mut self) {
        let pane = self.active;
        if self.panes[pane].archive.is_some() {
            return self.warn("압축 파일이나 원격 폴더 안에는 새 파일을 만들 수 없습니다.", "An empty file cannot be created inside an archive or a remote folder.");
        }
        let directory = self.panes[pane].path.to_string_lossy().into_owned();
        let title = self.root_text("TITLE_CREATEEMPTYFILE", "Create an empty file");
        let message = self.root_text("MSG_CREATEEMPTYFILE", "0 byte file will be created.\nType a new file name to be created.");
        self.prompt(&title, &message, "", None, false, move |app, value, _| {
            let Some(name) = value.filter(|v| !v.is_empty()) else { return };
            let path = names::combine_typed_name(&directory, &name);
            let error = names::create_empty_file(&path);
            if error != 0 {
                app.show_name_operation_error(error, false, |_, _, _| {});
            }
            app.refresh_and_focus(pane, Some(names::find_file_name(&path)));
        });
    }

    /// 원본 FUN_004351cc(C# RenameAsync): 표시 순서대로 항목마다 TfRename. 취소하면 멈춘다.
    /// 새 이름 = 확장자가 비면 이름, 아니면 이름 + "." + 확장자. 같은 폴더 안에서 MoveFileW.
    pub(super) fn rename_items(&mut self) {
        let pane = self.active;
        if self.panes[pane].remote.is_some() {
            return self.ftp_rename(pane);
        }
        if self.panes[pane].archive.is_some() {
            return self.notice("압축 파일 안의 항목은 바로 이름을 바꿀 수 없습니다.", "Items inside an archive cannot be renamed directly.");
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
        let (name, extension) = names::split_for_rename(&entry.name, entry.is_dir);
        let count = run.items.len();
        let file_name = entry.name.clone();
        self.rename_dialog(&file_name, &name, &extension, run.index + 1, count, move |app, value, _| {
            let Some((new_name, new_extension)) = value else { return app.finish_rename(run) };
            let directory = entry.path.parent().map(Path::to_path_buf).unwrap_or_else(|| app.panes[run.pane].path.clone());
            let target_name = if new_extension.is_empty() { new_name } else { format!("{new_name}.{new_extension}") };
            let error = names::move_file(&entry.path.to_string_lossy(), &directory.join(&target_name).to_string_lossy());
            if error == 0 {
                // 원본 FUN_004361f8 → FUN_0040f7f0: 설명이 바뀐 이름을 따라간다.
                let include_legacy = app.settings.flag("UseDescriptionFile", true);
                if let Err(message) = compat::rename_descriptions(&directory, &[(entry.name.clone(), target_name.clone())], include_legacy) {
                    app.status = message;
                }
                if run.focused.as_deref().is_some_and(|f| names::equals_ignore_case(f, &entry.name)) {
                    run.focus_after = Some(target_name);
                }
                run.index += 1;
                return app.rename_step(run);
            }
            // 원본은 실패한 항목 뒤에 남은 항목이 있을 때만 계속할지 묻는다.
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

    /// 원본 FUN_004351cc의 드라이브 항목 갈래: 드라이브가 준비됐으면 TfGetText(창 제목, MSG_DRIVELABEL, 지금 볼륨 이름).
    /// 확인이고 비어 있지 않으면 SetVolumeLabelW(루트, 이름). 성공하면 새로 읽고, ERROR_ACCESS_DENIED(5)면 ACCESS_DENIED를 알리며,
    /// 다른 실패는 조용히 넘긴다. ACCESS_DENIED는 원본처럼 그 목록 위에 잠깐 뜨는 알림 창(FUN_0040e138, TfMessage)이다.
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

    /// 원본 FUN_00432958(C# EditDescriptionAsync): 표시 순서대로 항목마다 TfGetText(TITLE_DESCRIPT, MSG_DESCRIPT의 %s = 이름).
    /// 취소하면 멈춘다. 확인은 설명을 저장(빈 값은 지움)하고 표시를 푼다. 끝나면 descript.ion을 옮기고 새로 고친다.
    pub(super) fn edit_descriptions(&mut self) {
        let pane = self.active;
        if self.panes[pane].archive.is_some() {
            return self.warn("압축 파일이나 원격 폴더 안에서는 설명을 바꿀 수 없습니다.", "Comments cannot be changed inside an archive or a remote folder.");
        }
        let items: Vec<String> = self.target_entries(pane).into_iter().map(|e| e.name).collect();
        if items.is_empty() {
            return;
        }
        let directory = self.panes[pane].path.clone();
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
                // C#은 예외로 흐름을 끝낸다(새로 고침 없음).
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

    /// 매크로 확장용 두 패널 상태(현재 폴더, 표시한 이름, 커서 항목).
    pub(super) fn macro_panes(&self) -> [compat::MacroPane; 2] {
        self.panes.each_ref().map(|p| compat::MacroPane {
            path: p.path.clone(),
            focused: p.entries.get(p.cursor).map(|e| e.name.clone()),
            selected: p.selected.iter().filter_map(|i| p.entries.get(*i).map(|e| e.name.clone())).collect(),
        })
    }

    /// 다른 패널에서 폴더 항목에 들어간다(open_current의 폴더 분기와 같은 규칙).
    fn enter_folder(&mut self, pane: usize, entry: Entry) {
        if let Some(location) = self.panes[pane].archive.as_mut() {
            location.prefix = format!("{}/", entry.path.to_string_lossy().replace('\\', "/"));
            self.refresh(pane);
        } else {
            self.navigate(pane, entry.path, true);
        }
    }

    /// 원본 FUN_00431dd4(C# OpenWithParametersAsync, Ctrl+Enter): 커서 항목에만 작용한다.
    /// 폴더: 들어가고, 다른 패널에 같은 이름의 폴더가 있으면 거기도 들어간다(승인된 수정: 같은 이름의 파일은 무시).
    /// 압축·원격 패널의 파일: 보통 열기. 그 밖: TfGetText(OPEN_WITH_PARAM, MSG_OPEN_WITH_PARAM의 %s = 이름, 빈 기본값),
    /// $(...) 매크로를 펼치고 ShellExecuteExW(파일, 인수, 현재 폴더). 실패는 셸이 알리고 NDR은 따로 보이지 않는다.
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
        let directory = self.panes[pane].path.clone();
        self.prompt(&title, &message, "", None, false, move |app, value, _| {
            let Some(parameters) = value else { return };
            let panes = app.macro_panes();
            let expanded = compat::expand_command(&parameters, &panes[pane], &panes[0], &panes[1]);
            let path = names::combine_typed_name(&directory.to_string_lossy(), &focused.name);
            let _ = (app.shell_runner)(Path::new(&path), &expanded, &directory);
        });
    }

    /// 원본 FUN_0042c734(C# DeleteAsync): delconfirm이 켜져 있거나 휴지통 방식이 아니면 TfDelWarning.
    /// 아니요는 멈추고, Delete 단추는 영구 삭제로 바꾼다. 영구 삭제는 보호 속성 항목마다 TfDelConfirm을 묻는다.
    pub(super) fn delete_items(&mut self, recycle: bool) {
        let pane = self.active;
        if self.panes[pane].remote.is_some() {
            return self.ftp_delete(pane);
        }
        if self.panes[pane].archive.is_some() {
            return self.notice("압축 파일 안의 항목은 삭제할 수 없습니다.", "Items inside an archive cannot be deleted.");
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
            return self.start_job("Delete".into(), Operation::Delete { sources: paths, recycle: true }, JobReport::Delete { recycle: true, skipped: 0 }, None);
        }
        // C# FindProtectedDeleteItems는 Task.Run으로 돈다. 큰 트리에서 화면이 멈추지 않도록 따로 찾고 poll에서 받는다.
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let items = fsops::find_protected_delete_items(&paths);
            let _ = tx.send(AsyncResult::ProtectedItems(DeleteRun { paths, items, index: 0, approve_all: false, skipped: vec![] }));
        });
    }

    /// 보호 항목 하나씩 TfDelConfirm(QUE_DELCONFIRM의 %s = 경로, HAS_ATTRIBUTES의 %s = 속성 이름들).
    /// 모두 예 = 나머지 승인, 아니요 = 그 항목(폴더면 안쪽 전부)을 남긴다. 끝나면 삭제 작업을 시작한다.
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
                }
                run.index += 1;
                app.protected_step(run);
            });
            return;
        }
        let skipped = run.skipped.len();
        let operation = if run.skipped.is_empty() {
            Operation::Delete { sources: run.paths, recycle: false }
        } else {
            Operation::DeleteSkipping { sources: run.paths, skipped: run.skipped }
        };
        self.start_job("Delete".into(), operation, JobReport::Delete { recycle: false, skipped }, None);
    }

    /// C# ShredAsync(원본 FUN_0042c734 방식 2): MSG_DELWARN_SHRED 경고는 늘 보이고 Delete 단추는 없다. 예만 진행한다.
    pub(super) fn shred_items(&mut self) {
        let pane = self.active;
        if self.panes[pane].archive.is_some() {
            return self.warn("압축 파일 안의 항목은 삭제할 수 없습니다.", "Items inside an archive cannot be deleted.");
        }
        let paths = self.panes[pane].targets();
        if paths.is_empty() {
            return self.warn("완전 삭제할 항목을 선택하세요.", "Select the items to shred.");
        }
        let first = display_name(&paths[0]);
        self.delete_warning(DeleteWarningMode::Shred, paths.len(), &first, move |app, choice, _| {
            if choice == DeleteWarningChoice::Yes {
                app.original_take_marks(&paths);
                app.start_job("Shred".into(), Operation::Shred { sources: paths }, JobReport::Shred, None);
            }
        });
    }
}
