//! C# SplitFileWindow / JoinFilesWindow and the SplitFileAsync / JoinPartsAsync flows. Rules are in ndr::split.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{MB_ICONEXCLAMATION, MB_OK};
use std::fs;

/// SplitFileWindow input.
#[derive(Clone, Debug)]
pub(super) struct SplitForm {
    pub source: PathBuf,
    pub length: u64,
    pub path: String,
    pub size: String,
}

/// Original TfFileJoin input.
#[derive(Clone, Debug)]
pub(super) struct JoinForm {
    pub files: Vec<(PathBuf, Option<u64>)>,
    pub selected: Option<usize>,
    pub join_text: bool,
    pub name: String,
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
    /// AddPath: existing files only, the same path (case-insensitive) once.
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

    /// Original UpdateSizeInfo (FUN_00422dd4): the sum of GetFileSize over lbFiles, FloatToStrF(ffNumber, 12, 0) + " bytes" (the same in every language).
    /// Previously the C# "전체 크기: N바이트" text.
    fn size_info(&self) -> String {
        let total: u64 = self.files.iter().filter_map(|(_, l)| *l).sum();
        let total = ndr::parity::format_size(total, "B", true).trim_end_matches(" B").to_owned();
        format!("{total} bytes")
    }
}

/// Draws the split window. Returns Some(0) OK, Some(1) cancel.
pub(super) fn split_ui(ui: &mut egui::Ui, form: &mut SplitForm, language: &compat::Language, enter: bool) -> Option<usize> {
    let mut choice = None;
    // Original TfFileSplit (ClientWidth 469 x ClientHeight 187): lFileName (16,16), lPath (16,59) ePath (103,56 311x21) bBrowse (420,54 41x25),
    // lSplitSize (16,99) cbSplitSize (103,96 138x21, list 1440 KB...100 MB) lSplitCount (260,99), btnOk (303,151) btnCancel (384,151) 75x25.
    let (area, _) = ui.allocate_exact_size(egui::vec2(469.0, 187.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    // Original TntFormShow (0x4236e4): lFileName = "<b>name</b> (size bytes)" (size with thousands separators, FloatToStrF ffNumber 0).
    let name = form.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = ndr::parity::format_size(form.length, "B", true).trim_end_matches(" B").to_owned();
    let mut job = egui::text::LayoutJob::default();
    let color = ui.visuals().text_color();
    job.append(&name, 0.0, egui::TextFormat { font_id: egui::FontId::new(13.0, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())), color, ..Default::default() });
    job.append(&format!(" ({bytes} bytes)"), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(13.0), color, ..Default::default() });
    super::original_dialogs::put_left(ui, at(16.0, 16.0, 440.0, 15.0), egui::Label::new(job).truncate()).on_hover_text(form.source.to_string_lossy());
    let raw = |key: &str, fallback: &str| language.raw(&format!("formFileSplit.{key}")).unwrap_or_else(|| fallback.to_owned());
    let (path_id, size_id) = (egui::Id::new("split-path"), egui::Id::new("split-size"));
    super::original_dialogs::access_label(ui, at(16.0, 59.0, 85.0, 15.0), &raw("Caption.lPath", "&Path to Save:"), path_id);
    ui.put(at(103.0, 56.0, 311.0, 21.0), egui::TextEdit::singleline(&mut form.path).id(path_id));
    if ui.put(at(420.0, 54.0, 41.0, 25.0), egui::Button::new("...")).clicked() {
        // Original (0x423b8e): FILESPLIT + SELECTDEST.
        let title = format!("{}{}", language.text("RES.FILESPLIT", "Split File"), language.text("RES.SELECTDEST", " - Select a destination"));
        let initial = Path::new(&form.path).is_dir().then(|| PathBuf::from(&form.path)).or_else(|| form.source.parent().map(Path::to_path_buf));
        if let Some(folder) = ndr::file_dialogs::browse_folder(&title, initial.as_deref()) {
            form.path = folder.to_string_lossy().into_owned();
        }
    }
    super::original_dialogs::access_label(ui, at(16.0, 99.0, 85.0, 15.0), &raw("Caption.lSplitSize", "&Split Size:"), size_id);
    ui.put(at(103.0, 96.0, 116.0, 21.0), egui::TextEdit::singleline(&mut form.size).id(size_id));
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

/// Draws the join window. Returns Some(0) OK, Some(1) cancel.
pub(super) fn join_ui(ui: &mut egui::Ui, form: &mut JoinForm, language: &compat::Language, first_frame: bool, enter: bool) -> Option<usize> {
    let mut choice = None;
    // Original TfFileJoin (ClientWidth 501 x ClientHeight 346): lFileName (16,25) eFileName (127,22 323x21) btnBrowse (456,20 33x25),
    // lFiles(16,65) lbFiles(16,84 473×169), btnAdd(16,262) btnDelete(86,262) btnUp(172,262) btnDown(242,262) 64×25,
    // lSizeInfo (right edge 486,267), chkIsText (16,317), btnOk (330,313) btnCancel (411,313) 75x25.
    let (area, _) = ui.allocate_exact_size(egui::vec2(501.0, 346.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let name_id = egui::Id::new("join-file-name");
    let caption = language.raw("formFileJoin.Caption.lFileName").unwrap_or_else(|| "&File name:".into());
    super::original_dialogs::access_label(ui, at(16.0, 25.0, 108.0, 15.0), &caption, name_id);
    let name_response = ui.put(at(127.0, 22.0, 323.0, 21.0), egui::TextEdit::singleline(&mut form.name).id(name_id));
    // eFileName is the first control in the tab order: it has the focus with its text selected when the window opens (seen in the running original).
    if first_frame {
        name_response.request_focus();
        super::original_dialogs::select_all(ui, name_id, form.name.chars().count());
    }
    if ui.put(at(456.0, 20.0, 33.0, 25.0), egui::Button::new("...")).clicked() {
        // Original btnBrowseClick (0x423444): the window's OpenDialog (no Title, Filter or InitialDir) with ofAllowMultiSelect and ofFileMustExist turned off
        // and FileName cleared, then Execute (an open box). The choice goes into eFileName. Being an open box, there is no overwrite prompt (asked separately when joining).
        if let Some(chosen) = ndr::file_dialogs::open_any_file("", None, "", &[]) {
            form.name = chosen.to_string_lossy().into_owned();
        }
    }
    super::original_dialogs::put_left(ui, at(16.0, 65.0, 300.0, 15.0), egui::Label::new(join_text(language, "Caption.lFiles", "Files to be joined:")).truncate());
    let list = at(16.0, 84.0, 473.0, 169.0);
    ui.painter().rect(list, 0.0, ui.visuals().extreme_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink(2.0)), |ui| {
        egui::ScrollArea::vertical().id_salt("join-files").auto_shrink([false, false]).show(ui, |ui| {
            // Original lbFiles holds the full paths (Items.Assign of the data item paths).
            for (index, (path, _)) in form.files.iter().enumerate() {
                if ui.selectable_label(form.selected == Some(index), path.to_string_lossy()).clicked() {
                    form.selected = Some(index);
                }
            }
        });
    });
    let raw = |key: &str, fallback: &str| language.raw(&format!("formFileJoin.{key}")).unwrap_or_else(|| fallback.to_owned());
    let button = |ui: &mut egui::Ui, x: f32, enabled: bool, label: String| super::original_dialogs::vcl_button(ui, at(x, 262.0, 64.0, 25.0), enabled, &label).1;
    if button(ui, 16.0, true, raw("Caption.btnAdd", "&Add")) {
        let title = join_text(language, "Form.Caption", "Join Files");
        let initial = form.files.first().and_then(|(p, _)| p.parent().map(Path::to_path_buf));
        for path in ndr::file_dialogs::open_files(&title, initial.as_deref()) {
            form.add(&path);
        }
        if form.selected.is_none() && !form.files.is_empty() {
            form.selected = Some(0);
        }
    }
    // Original btnDelete/btnUp/btnDown are always enabled; their handlers (0x42332c, 0x423374, 0x4233c8) do nothing without a usable ItemIndex.
    // Previously they were greyed out.
    if button(ui, 86.0, true, raw("Caption.btnDelete", "&Delete")) {
        form.remove_selected();
    }
    if button(ui, 172.0, true, raw("Caption.btnUp", "&Up")) {
        form.move_selected(-1);
    }
    if button(ui, 242.0, true, raw("Caption.btnDown", "Do&wn")) {
        form.move_selected(1);
    }
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(312.0, 267.0, 174.0, 15.0)).layout(egui::Layout::right_to_left(egui::Align::Min)), |ui| {
        ui.add(egui::Label::new(form.size_info()).truncate());
    });
    super::original_dialogs::access_checkbox(ui, at(16.0, 317.0, 300.0, 17.0), &mut form.join_text, &raw("Caption.chkIsText", "Join &text files"));
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

/// Characters of Path.GetInvalidFileNameChars that may appear in a name (including control characters).
fn invalid_file_name(name: &str) -> bool {
    name.chars().any(|c| c < ' ' || "\"<>|:*?\\/".contains(c))
}

impl App {
    /// SplitFileWindow.Confirm / JoinFilesWindow.Commit: closes with the result when valid, otherwise shows a warning box with the window title and stays.
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
                    Ok(DialogResult::Join(Some(JoinOptions { files: form.files.iter().map(|(p, _)| p.clone()).collect(), join_text: form.join_text, name: form.name.clone(), overwrite_confirmed: false })))
                }
            }
            _ => return,
        };
        match outcome {
            Ok(result) => self.answer_dialog(result, ctx),
            Err(message) => self.system_box(&title, message, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        }
    }

    /// Original miSplitFileClick (0x43b8b8): opens TfFileSplit only when the active pane is in local mode and the cursor item is a file (kind 3) (otherwise nothing
    /// happens). On OK, splits with a progress window and writes a .crc next to the first piece.
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
            // C# Run(sourcePath: source folder, destinationPath: output folder, totalFiles: piece count, totalBytes: source size).
            let parts = if size == 0 { None } else { Some(length.div_ceil(size).max(1)) };
            let progress = super::file_operation::ProgressInfo {
                from: source.parent().map(|p| p.to_string_lossy().into_owned()),
                to: Some(destination.to_string_lossy().into_owned()),
                total_files: parts,
                total_bytes: Some(length),
                prefer_bytes: true,
                caption: Some(app.language_pack.text("RES.SPLITTING", "Splitting a file...")),
            };
            let started = app.start_progress_task(title.clone(), progress, move |events, cancel| {
                let outcome = ndr::split::split_file(&source, size, &destination, &cancel, &mut |files, bytes, path| {
                    let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes, files });
                });
                let _ = tx.send(AsyncResult::SplitDone { source, outcome, cancelled: cancel.load(Ordering::Relaxed) });
                Ok(())
            });
            // split_done writes the status text. Even if the end notice (Finished) arrives after SplitDone, it does not overwrite it with the generic "title: N files" text.
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

    /// C# JoinPartsAsync: the chosen files (with one, all numbered pieces). A relative name is joined to the current folder. When the name exists
    /// (and overwriting was not agreed), it reports instead of joining.
    /// Original miJoinFileClick (0x43bec8): opens TfFileJoin only when the active pane is in local mode and there are data items (marks, otherwise the cursor item, FUN_00485bbc)
    /// (otherwise nothing happens). lbFiles holds those items as is; on OK the marks are saved as the last selection and cleared.
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
        let mut form = JoinForm { files: vec![], selected: None, join_text: false, name: ndr::split::suggest_joined_file_name(&inputs[0]) };
        for input in &inputs {
            form.add(input);
        }
        let title = self.language_pack.text("formFileJoin.Form.Caption", "Join Files");
        self.push_original_dialog(title.clone(), DialogKind::Join(form), move |app, result, _| {
            let DialogResult::Join(Some(options)) = result else { return };
            // Original: on OK, CopySelection (last selection) + ClearSelection.
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
                caption: Some(app.language_pack.text("RES.JOINING", "Joining files...")),
            };
            let started = app.start_progress_task(task_title, progress, move |events, cancel| {
                let count = options.files.len();
                let outcome = ndr::split::join_ordered_files(&options.files, &destination, options.join_text, options.overwrite_confirmed, &cancel, &mut |files, bytes, path| {
                    let _ = events.send(Event::Progress { path: path.to_path_buf(), bytes, files });
                });
                let _ = tx.send(AsyncResult::JoinDone { count, outcome: outcome.map(|_| ()), cancelled: cancel.load(Ordering::Relaxed) });
                Ok(())
            });
            // join_done writes the status text (same reason as the split above).
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
