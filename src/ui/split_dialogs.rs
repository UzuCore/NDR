//! C# SplitFileWindow / JoinFilesWindow와 SplitFileAsync / JoinPartsAsync 흐름. 규칙은 ndr::split.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{MB_ICONEXCLAMATION, MB_OK};
use std::fs;

/// SplitFileWindow 입력.
#[derive(Clone, Debug)]
pub(super) struct SplitForm {
    pub source: PathBuf,
    pub length: u64,
    pub path: String,
    pub size: String,
}

/// JoinFilesWindow 입력. overwrite_confirmed: 찾아보기 대화 상자에서 덮어쓰기를 받아들인 경로.
#[derive(Clone, Debug)]
pub(super) struct JoinForm {
    pub files: Vec<(PathBuf, Option<u64>)>,
    pub selected: Option<usize>,
    pub join_text: bool,
    pub name: String,
    pub base: PathBuf,
    pub overwrite_confirmed: Option<PathBuf>,
}

/// C# JoinFilesOptions.
#[derive(Clone, Debug)]
pub(super) struct JoinOptions {
    pub files: Vec<PathBuf>,
    pub join_text: bool,
    pub name: String,
    pub overwrite_confirmed: bool,
}

fn split_text(language: &compat::Language, key: &str, fallback: &str) -> String {
    language.text(&format!("formFileSplit.{key}"), fallback)
}

fn join_text(language: &compat::Language, key: &str, fallback: &str) -> String {
    language.text(&format!("formFileJoin.{key}"), fallback)
}

impl JoinForm {
    /// AddPath: 있는 파일만, 같은 경로(대소문자 무시)는 한 번.
    pub(super) fn add(&mut self, path: &Path) {
        if !path.is_file() {
            return;
        }
        let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        if self.files.iter().any(|(known, _)| ndr::names::equals_ignore_case(&known.to_string_lossy(), &full.to_string_lossy())) {
            return;
        }
        let length = fs::metadata(&full).ok().map(|m| m.len());
        self.files.push((full, length));
    }

    /// MoveFile(±1).
    pub(super) fn move_selected(&mut self, offset: isize) {
        let Some(index) = self.selected else { return };
        let target = index as isize + offset;
        if target < 0 || target as usize >= self.files.len() {
            return;
        }
        self.files.swap(index, target as usize);
        self.selected = Some(target as usize);
    }

    /// RemoveFile.
    pub(super) fn remove_selected(&mut self) {
        let Some(index) = self.selected else { return };
        self.files.remove(index);
        self.selected = (!self.files.is_empty()).then(|| index.min(self.files.len() - 1));
    }

    /// UpdateSizeInfo.
    fn size_info(&self, ko: bool) -> String {
        let total: u64 = self.files.iter().filter_map(|(_, l)| *l).sum();
        let unknown = self.files.iter().filter(|(_, l)| l.is_none()).count();
        let total = ndr::parity::format_size(total, "B", true).trim_end_matches(" B").to_owned();
        match (unknown, ko) {
            (0, true) => format!("전체 크기: {total}바이트"),
            (0, false) => format!("Total size: {total} bytes"),
            (n, true) => format!("전체 크기: {total}바이트 (확인 불가 {n}개)"),
            (n, false) => format!("Total size: {total} bytes ({n} unknown)"),
        }
    }
}

/// 분할 창 그리기. 돌려주는 값: Some(0) 확인, Some(1) 취소.
pub(super) fn split_ui(ui: &mut egui::Ui, form: &mut SplitForm, language: &compat::Language, enter: bool) -> Option<usize> {
    let mut choice = None;
    // 원본 TfFileSplit(ClientWidth 469 × ClientHeight 187): lFileName(16,16), lPath(16,59) ePath(103,56 311×21) bBrowse(420,54 41×25),
    // lSplitSize(16,99) cbSplitSize(103,96 138×21, 목록 1440 KB…100 MB) lSplitCount(260,99), btnOk(303,151) btnCancel(384,151) 75×25.
    let (area, _) = ui.allocate_exact_size(egui::vec2(469.0, 187.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    // 원본 TntFormShow(0x4236e4): lFileName = "<b>이름</b> (크기 bytes)"(크기는 자리 나눔 쉼표, FloatToStrF ffNumber 0).
    let name = form.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = ndr::parity::format_size(form.length, "B", true).trim_end_matches(" B").to_owned();
    let mut job = egui::text::LayoutJob::default();
    let color = ui.visuals().text_color();
    job.append(&name, 0.0, egui::TextFormat { font_id: egui::FontId::new(13.0, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())), color, ..Default::default() });
    job.append(&format!(" ({bytes} bytes)"), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.0), color, ..Default::default() });
    super::original_dialogs::put_left(ui, at(16.0, 16.0, 440.0, 15.0), egui::Label::new(job).truncate()).on_hover_text(form.source.to_string_lossy());
    super::original_dialogs::put_left(ui, at(16.0, 59.0, 85.0, 15.0), egui::Label::new(split_text(language, "Caption.lPath", "Path to Save:")).truncate());
    ui.put(at(103.0, 56.0, 311.0, 21.0), egui::TextEdit::singleline(&mut form.path));
    if ui.put(at(420.0, 54.0, 41.0, 25.0), egui::Button::new("...")).clicked() {
        // 원본(0x423b8e): FILESPLIT + SELECTDEST.
        let title = format!("{}{}", language.text("RES.FILESPLIT", "Split File"), language.text("RES.SELECTDEST", " - Select a destination"));
        let initial = Path::new(&form.path).is_dir().then(|| PathBuf::from(&form.path)).or_else(|| form.source.parent().map(Path::to_path_buf));
        if let Some(folder) = ndr::file_dialogs::browse_folder(&title, initial.as_deref()) {
            form.path = folder.to_string_lossy().into_owned();
        }
    }
    super::original_dialogs::put_left(ui, at(16.0, 99.0, 85.0, 15.0), egui::Label::new(split_text(language, "Caption.lSplitSize", "Split Size:")).truncate());
    ui.put(at(103.0, 96.0, 116.0, 21.0), egui::TextEdit::singleline(&mut form.size));
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(219.0, 96.0, 22.0, 21.0)), |ui| {
        ui.menu_button("▼", |ui| {
            for preset in ndr::split::SPLIT_SIZE_PRESETS {
                if ui.button(preset).clicked() {
                    form.size = preset.into();
                    ui.close();
                }
            }
        });
    });
    if let Some(size) = ndr::split::parse_split_size(&form.size) {
        let template = language.text("RES.SPLITTED_INFO", "Number of splitted files: %d");
        super::original_dialogs::put_left(ui, at(260.0, 99.0, 200.0, 15.0), egui::Label::new(template.replace("%d", &ndr::split::part_count(form.length, size).to_string())).truncate());
    }
    if ui.put(at(303.0, 151.0, 75.0, 25.0), egui::Button::new(split_text(language, "Caption.btnOk", "OK"))).clicked() {
        choice = Some(0);
    }
    if ui.put(at(384.0, 151.0, 75.0, 25.0), egui::Button::new(split_text(language, "Caption.btnCancel", "Cancel"))).clicked() {
        choice = Some(1);
    }
    if enter {
        choice = Some(0);
    }
    choice
}

/// 합치기 창 그리기. 돌려주는 값: Some(0) 확인, Some(1) 취소.
pub(super) fn join_ui(ui: &mut egui::Ui, form: &mut JoinForm, language: &compat::Language, ko: bool, enter: bool) -> Option<usize> {
    let mut choice = None;
    // 원본 TfFileJoin(ClientWidth 501 × ClientHeight 346): lFileName(16,25) eFileName(127,22 323×21) btnBrowse(456,20 33×25),
    // lFiles(16,65) lbFiles(16,84 473×169), btnAdd(16,262) btnDelete(86,262) btnUp(172,262) btnDown(242,262) 64×25,
    // lSizeInfo(오른쪽 끝 486,267), chkIsText(16,317), btnOk(330,313) btnCancel(411,313) 75×25.
    let (area, _) = ui.allocate_exact_size(egui::vec2(501.0, 346.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    super::original_dialogs::put_left(ui, at(16.0, 25.0, 108.0, 15.0), egui::Label::new(join_text(language, "Caption.lFileName", "File name:")).truncate());
    ui.put(at(127.0, 22.0, 323.0, 21.0), egui::TextEdit::singleline(&mut form.name));
    if ui.put(at(456.0, 20.0, 33.0, 25.0), egui::Button::new("...")).clicked() {
        let title = join_text(language, "Form.Caption", "Join Files");
        let typed = Path::new(&form.name);
        let full = if typed.is_absolute() { typed.to_path_buf() } else { form.base.join(typed) };
        let directory = full.parent().map(Path::to_path_buf).unwrap_or_else(|| form.base.clone());
        let file_name = typed.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // C# Filter "All files (*.*)|*.*", AddExtension false.
        if let Some(chosen) = ndr::file_dialogs::save_file(&title, Some(&directory), &file_name, &[("All files (*.*)", "*.*")], "", true) {
            form.overwrite_confirmed = chosen.is_file().then(|| chosen.clone());
            form.name = chosen.to_string_lossy().into_owned();
        }
    }
    super::original_dialogs::put_left(ui, at(16.0, 65.0, 300.0, 15.0), egui::Label::new(join_text(language, "Caption.lFiles", "Files to be joined:")).truncate());
    let list = at(16.0, 84.0, 473.0, 169.0);
    ui.painter().rect(list, 0.0, ui.visuals().extreme_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink(2.0)), |ui| {
        egui::ScrollArea::vertical().id_salt("join-files").auto_shrink([false, false]).show(ui, |ui| {
            for (index, (path, _)) in form.files.iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if ui.selectable_label(form.selected == Some(index), name).on_hover_text(path.to_string_lossy()).clicked() {
                    form.selected = Some(index);
                }
            }
        });
    });
    let selected = form.selected;
    let button = |ui: &mut egui::Ui, x: f32, enabled: bool, label: String| super::original_dialogs::put_enabled(ui, at(x, 262.0, 64.0, 25.0), enabled, egui::Button::new(label)).clicked();
    if button(ui, 16.0, true, join_text(language, "Caption.btnAdd", "Add")) {
        let title = join_text(language, "Form.Caption", "Join Files");
        let initial = form.files.first().and_then(|(p, _)| p.parent().map(Path::to_path_buf));
        for path in ndr::file_dialogs::open_files(&title, initial.as_deref()) {
            form.add(&path);
        }
        if form.selected.is_none() && !form.files.is_empty() {
            form.selected = Some(0);
        }
    }
    if button(ui, 86.0, selected.is_some(), join_text(language, "Caption.btnDelete", "Delete")) {
        form.remove_selected();
    }
    if button(ui, 172.0, selected.is_some_and(|i| i > 0), join_text(language, "Caption.btnUp", "Up")) {
        form.move_selected(-1);
    }
    if button(ui, 242.0, selected.is_some_and(|i| i + 1 < form.files.len()), join_text(language, "Caption.btnDown", "Down")) {
        form.move_selected(1);
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(312.0, 267.0, 174.0, 15.0)).layout(egui::Layout::right_to_left(egui::Align::Min)), |ui| {
        ui.add(egui::Label::new(form.size_info(ko)).truncate());
    });
    super::original_dialogs::put_left(ui, at(16.0, 317.0, 300.0, 17.0), egui::Checkbox::new(&mut form.join_text, join_text(language, "Caption.chkIsText", "Join text files")))
        .on_hover_text("Insert CRLF only between text files that meet without an existing line break. Text encodings must match.");
    if ui.put(at(330.0, 313.0, 75.0, 25.0), egui::Button::new(join_text(language, "Caption.btnOk", "OK"))).clicked() {
        choice = Some(0);
    }
    if ui.put(at(411.0, 313.0, 75.0, 25.0), egui::Button::new(join_text(language, "Caption.btnCancel", "Cancel"))).clicked() {
        choice = Some(1);
    }
    if enter {
        choice = Some(0);
    }
    choice
}

/// Path.GetInvalidFileNameChars 중 이름에 들어갈 수 있는 글자(제어 문자 포함).
fn invalid_file_name(name: &str) -> bool {
    name.chars().any(|c| c < ' ' || "\"<>|:*?\\/".contains(c))
}

impl App {
    /// SplitFileWindow.Confirm / JoinFilesWindow.Commit: 맞으면 결과로 닫고, 틀리면 창 제목의 경고 상자를 띄우고 남긴다.
    pub(super) fn submit_split_or_join(&mut self, ctx: &egui::Context) {
        let ko = self.language == "ko";
        let Some(dialog) = self.original_dialogs.last() else { return };
        let title = dialog.title.clone();
        let outcome = match &dialog.kind {
            DialogKind::Split(form) => match ndr::split::parse_split_size(&form.size) {
                None => Err(if ko { "분할 크기를 입력하세요. 예: 1440 KB 또는 100 MB." } else { "Enter a positive split size, such as 1440 KB or 100 MB." }),
                Some(_) if form.path.trim().is_empty() => Err(if ko { "저장 경로를 입력하세요." } else { "Enter an output folder." }),
                Some(size) => match std::path::absolute(form.path.trim()) {
                    Err(_) => Err(if ko { "올바른 저장 경로를 입력하세요." } else { "Enter a valid output folder." }),
                    Ok(folder) if !folder.is_dir() => Err(if ko { "저장할 폴더가 없습니다." } else { "The output folder does not exist." }),
                    Ok(folder) => Ok(DialogResult::Split(Some((folder, size, ndr::split::part_count(form.length, size))))),
                },
            },
            DialogKind::Join(form) => {
                let file_name = Path::new(&form.name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if form.files.is_empty() {
                    Err(if ko { "합칠 파일을 하나 이상 추가하세요." } else { "Add at least one file to join." })
                } else if form.name.trim().is_empty() || form.name.chars().any(|c| c < ' ' || c == '|' || c == '"' || c == '<' || c == '>') || file_name.is_empty() || invalid_file_name(&file_name) {
                    Err(if ko { "올바른 결과 파일 이름을 입력하세요." } else { "Enter a valid output file name." })
                } else {
                    let confirmed = form.overwrite_confirmed.as_ref().is_some_and(|p| ndr::names::equals_ignore_case(&p.to_string_lossy(), &form.name));
                    Ok(DialogResult::Join(Some(JoinOptions { files: form.files.iter().map(|(p, _)| p.clone()).collect(), join_text: form.join_text, name: form.name.clone(), overwrite_confirmed: confirmed })))
                }
            }
            _ => return,
        };
        match outcome {
            Ok(result) => self.answer_dialog(result, ctx),
            Err(message) => self.system_box(&title, message, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        }
    }

    /// 원본 miSplitFileClick(0x43b8b8): 활성 패널이 로컬 모드이고 커서 항목이 파일(종류 3)일 때만 TfFileSplit을 연다(아니면 아무것도
    /// 하지 않는다). 확인하면 진행 창으로 나누고 첫 조각 옆에 .crc를 쓴다.
    pub(super) fn split_command(&mut self) {
        let i = self.active;
        let title = self.language_pack.text("formFileSplit.Form.Caption", "Split File");
        let pane = &self.panes[i];
        if pane.archive.is_some() || pane.remote.is_some() {
            return;
        }
        let Some(file) = pane.entries.get(pane.cursor).filter(|e| !e.is_dir && !e.drive && !e.is_parent()).map(|e| e.path.clone()) else { return };
        let length = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        let form = SplitForm {
            path: file.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            size: ndr::split::default_split_size(length).into(),
            source: file.clone(),
            length,
        };
        self.push_original_dialog(title.clone(), DialogKind::Split(form), move |app, result, _| {
            let DialogResult::Split(Some((destination, size, _))) = result else { return };
            let tx = app.async_tx.clone();
            let source = file.clone();
            // C# Run(sourcePath: 원본 폴더, destinationPath: 출력 폴더, totalFiles: 조각 수, totalBytes: 원본 크기).
            let parts = if size == 0 { None } else { Some(length.div_ceil(size).max(1)) };
            let progress = super::file_operation::ProgressInfo {
                from: source.parent().map(|p| p.to_string_lossy().into_owned()),
                to: Some(destination.to_string_lossy().into_owned()),
                total_files: parts,
                total_bytes: Some(length),
                prefer_bytes: true,
                allow_pause: true,
                caption: Some(app.language_pack.text("RES.SPLITTING", "Splitting a file...")),
            };
            let started = app.start_pausable_task(title.clone(), progress, move |events, cancel, pause| {
                let outcome = ndr::split::split_file(&source, size, &destination, &cancel, &mut |files, bytes, path| {
                    let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes, files });
                    super::commands::wait_while_paused(&pause, &cancel);
                });
                let _ = tx.send(AsyncResult::SplitDone { source, outcome, cancelled: cancel.load(Ordering::Relaxed) });
                Ok(())
            });
            // split_done이 상태 글을 쓴다. 끝 알림(Finished)이 SplitDone보다 늦게 와도 일반 "제목: N files" 글로 덮지 않는다.
            if let Some(job) = app.job.as_mut().filter(|_| started) {
                job.custom_status = true;
            }
        });
    }

    pub(super) fn split_done(&mut self, source: PathBuf, outcome: Result<ndr::split::SplitResult, String>, cancelled: bool) {
        let title = self.language_pack.text("formFileSplit.Form.Caption", "Split File");
        let result = match outcome {
            Err(_) if cancelled => {
                self.status = self.tr("파일 분할을 취소했습니다.", "File split canceled.").into_owned();
                return;
            }
            Err(error) => return self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
            Ok(result) => result,
        };
        if let Some(first) = result.parts.first() {
            if let Err(error) = ndr::split::write_split_metadata(first, &source, result.source_size, &result.crc32) {
                self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
        }
        let count = ndr::parity::format_size(result.parts.len() as u64, "B", true).trim_end_matches(" B").to_owned();
        self.status = if self.language == "ko" { format!("{count}개 조각 파일을 만들었습니다.") } else { format!("Created {count} parts.") };
        self.refresh(self.active);
    }

    /// C# JoinPartsAsync: 선택한 파일들(하나면 번호 조각 전체). 이름이 상대 경로면 현재 폴더에 붙인다. 같은 이름이 있으면
    /// (덮어쓰기에 동의하지 않았으면) 합치지 않고 알린다.
    /// 원본 miJoinFileClick(0x43bec8): 활성 패널이 로컬 모드이고 데이터 항목(표시, 없으면 커서 항목, FUN_00485bbc)이 있을 때만
    /// TfFileJoin을 연다(아니면 아무것도 하지 않는다). lbFiles는 그 항목 그대로, 확인하면 표시를 마지막 선택에 남기고 지운다.
    pub(super) fn join_command(&mut self) {
        let i = self.active;
        if self.panes[i].archive.is_some() || self.panes[i].remote.is_some() {
            return;
        }
        let inputs: Vec<PathBuf> = self.panes[i].targets().into_iter().filter(|p| !p.is_dir()).collect();
        if inputs.is_empty() {
            return;
        }
        let base = self.panes[i].path.clone();
        let mut form = JoinForm { files: vec![], selected: None, join_text: false, name: ndr::split::suggest_joined_file_name(&inputs[0]), base: base.clone(), overwrite_confirmed: None };
        for input in &inputs {
            form.add(input);
        }
        let title = self.language_pack.text("formFileJoin.Form.Caption", "Join Files");
        self.push_original_dialog(title.clone(), DialogKind::Join(form), move |app, result, _| {
            let DialogResult::Join(Some(options)) = result else { return };
            // 원본: 확인하면 CopySelection(마지막 선택) + ClearSelection.
            app.original_take_pane_marks(i);
            let typed = Path::new(&options.name);
            let destination = std::path::absolute(if typed.is_absolute() { typed.to_path_buf() } else { base.join(typed) }).unwrap_or_else(|_| base.join(typed));
            if (destination.is_file() && !options.overwrite_confirmed) || destination.is_dir() {
                let text = if app.language == "ko" {
                    format!("같은 이름의 파일이 이미 있어 합치지 않았습니다.\n{}", destination.display())
                } else {
                    format!("A file with this name already exists. Nothing was joined.\n{}", destination.display())
                };
                return app.system_box(&title, &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
            let tx = app.async_tx.clone();
            let task_title = app.tr("파일 합치기", "Join Files").into_owned();
            let totals = super::file_operation::flat_totals(&options.files);
            let progress = super::file_operation::ProgressInfo {
                from: options.files.first().and_then(|f| super::file_operation::source_folder(f)),
                to: destination.parent().map(|p| p.to_string_lossy().into_owned()),
                total_files: totals.map(|t| t.0),
                total_bytes: totals.map(|t| t.1),
                prefer_bytes: true,
                allow_pause: true,
                caption: Some(app.language_pack.text("RES.JOINING", "Joining files...")),
            };
            let started = app.start_pausable_task(task_title, progress, move |events, cancel, pause| {
                let count = options.files.len();
                let outcome = ndr::split::join_ordered_files(&options.files, &destination, options.join_text, options.overwrite_confirmed, &cancel, &mut |files, bytes, path| {
                    let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes, files });
                    super::commands::wait_while_paused(&pause, &cancel);
                });
                let _ = tx.send(AsyncResult::JoinDone { count, outcome: outcome.map(|_| ()), cancelled: cancel.load(Ordering::Relaxed) });
                Ok(())
            });
            // join_done이 상태 글을 쓴다(위의 분할과 같은 까닭).
            if let Some(job) = app.job.as_mut().filter(|_| started) {
                job.custom_status = true;
            }
        });
    }

    pub(super) fn join_done(&mut self, count: usize, outcome: Result<(), String>, cancelled: bool) {
        let ko = self.language == "ko";
        match outcome {
            Err(_) if cancelled => self.status = if ko { "파일 합치기를 취소했습니다." } else { "File join canceled." }.into(),
            Err(error) => {
                let title = self.tr("파일 합치기", "Join Files").into_owned();
                self.system_box(&title, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            }
            Ok(()) => {
                let count = ndr::parity::format_size(count as u64, "B", true).trim_end_matches(" B").to_owned();
                self.status = if ko { format!("{count}개 파일을 합쳤습니다.") } else { format!("Joined {count} files.") };
                self.refresh(self.active);
            }
        }
    }
}
