//! Test screenshots (`--smoke-seconds N --shot kind file.png`): opens a sample of an original window in the off-screen startup check window and saves that screen as PNG.
//! Used only to check by eye that windows moved to original DFM positions are really drawn that way. User files and settings are not touched
//! (sample files are created only in the shot folder of the isolated --profile folder).
use super::original_dialogs::DialogKind;
use super::*;

/// Frame number that opens the sample and frame number that takes the shot.
const OPEN_FRAME: u64 = 3;
const SHOT_FRAME: u64 = 20;

/// Fake FTP of the "ftplog" sample (two list rows, other actions refused).
struct ShotFtp;
impl super::ftp_pane::PaneFs for ShotFtp {
    fn pwd(&mut self) -> Result<String, String> {
        Ok("/pub".into())
    }
    fn change_directory(&mut self, _current: &str, path: &str) -> Result<(String, Option<String>), String> {
        Ok((path.to_owned(), None))
    }
    fn list(&mut self, _path: &str) -> Result<Vec<ndr::remote::RemoteEntry>, String> {
        Ok(vec![
            ndr::remote::RemoteEntry { name: "incoming".into(), is_dir: true, mode: Some(0o755), ..Default::default() },
            ndr::remote::RemoteEntry { name: "readme.txt".into(), size: 1234, mode: Some(0o644), ..Default::default() },
        ])
    }
    fn download(&mut self, _: &str, _: &Path, _: &AtomicBool, _: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, _: &mut dyn FnMut(ndr::remote::TransferProgress)) -> Result<(), String> {
        Err("sample".into())
    }
    fn upload(&mut self, _: &Path, _: &str, _: &AtomicBool, _: &mut dyn FnMut(&str, &str, &ndr::remote::ConflictInfo) -> ConflictChoice, _: &mut dyn FnMut(ndr::remote::TransferProgress)) -> Result<(), String> {
        Err("sample".into())
    }
    fn mkdir(&mut self, _: &str) -> Result<(), String> {
        Err("sample".into())
    }
    fn rename(&mut self, _: &str, _: &str) -> Result<(), String> {
        Err("sample".into())
    }
    fn delete(&mut self, _: &str, _: bool, _: &AtomicBool) -> Result<(), String> {
        Err("sample".into())
    }
    fn chmod(&mut self, _: &str, _: u32) -> Result<(), String> {
        Err("sample".into())
    }
    fn command(&mut self, _: &str) -> Result<String, String> {
        Err("sample".into())
    }
    fn close(&mut self) {}
}

impl App {
    pub(super) fn shot_step(&mut self, ctx: &egui::Context) {
        let Some((kind, path)) = self.options.shot.clone() else { return };
        ctx.request_repaint();
        if self.frames == OPEN_FRAME {
            self.open_shot_sample(&kind);
            if matches!(kind.as_str(), "narrowpane" | "error-long") {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(800.0, 480.0)));
            }
        }
        // The pane folder tree reads the shell namespace separately, so the shot waits until expanded items are read.
        let tree_waiting = kind == "panetree" && self.tree_requests.iter().any(|key| !self.tree_cache.contains_key(key));
        let sent_id = egui::Id::new("ndr-shot-sent");
        if self.frames >= SHOT_FRAME && !tree_waiting && !ctx.data(|d| d.get_temp::<bool>(sent_id)).unwrap_or(false) {
            // Record actual rendered text, glyph coverage and clipping, not only successful startup.
            audit_shot_text(ctx, &path);
            ctx.data_mut(|d| d.insert_temp(sent_id, true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        if kind == "narrowdetails" && self.frames == SHOT_FRAME - 4 {
            self.panes[0].detail_x = 40.0;
        }
        // Deterministic real-rendering QA of the transient long-name tooltip.
        // It is drawn into the main viewport for the screenshot, even while
        // smoke mode deliberately keeps the window unfocused and offscreen.
        if kind == "longnametip" && self.frames >= OPEN_FRAME {
            let screen = ctx.content_rect();
            let filename = format!("Long_Filename_Layout_Problem_{}_한글테스트_abcdefghijklmnopqrstuvwxyz0123456789.txt", "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_".repeat(4));
            self.smoke_tip_window = super::panes::draw_bounded_name_tip(
                ctx, 0, &filename, egui::pos2(screen.right() - 65.0, screen.bottom() - 65.0),
                screen.width() * 1.8, 0.0, 16.0, 0,
            ).map(|r| [r.left(), r.top(), r.width(), r.height()]);
        }
        // Check of the separate tooltip window (panes::show_tip_window): draws a rectangle past the main window's right edge and records the created window's screen position in the report.
        if kind == "tipwindow" && self.frames >= OPEN_FRAME {
            let screen = ctx.content_rect();
            let rect = egui::Rect::from_min_size(egui::pos2(screen.right() - 60.0, screen.top() + 80.0), egui::vec2(180.0, 22.0));
            let galley = ctx.fonts_mut(|f| f.layout_no_wrap("a-very-long-file-name-for-the-tooltip.txt".into(), egui::FontId::proportional(12.0), egui::Color32::BLACK));
            let paint = |painter: &egui::Painter, rect: egui::Rect| {
                painter.rect(rect, 0.0, egui::Color32::from_rgb(255, 255, 225), egui::Stroke::new(1.0_f32, egui::Color32::BLACK), egui::StrokeKind::Inside);
                painter.galley((rect.center() - galley.size() / 2.0).round(), galley.clone(), egui::Color32::BLACK);
            };
            if super::panes::show_tip_window(ctx, 0, rect, &paint) {
                let id = egui::ViewportId::from_hash_of(("ndr-name-tip-window", 0usize));
                if let Some(r) = ctx.input_for(id, |i| i.viewport().inner_rect) {
                    self.smoke_tip_window = Some([r.left(), r.top(), r.width(), r.height()]);
                }
            }
        }
        let image = ctx.input(|i| i.raw.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
        if let Some(image) = image {
            let _ = save_png(&path, image.width() as u32, image.height() as u32, image.as_raw());
            // The progress samples own a presentation-only job with no worker.
            // Release it after capture so normal close guards do not keep the
            // isolated smoke-test instance alive indefinitely.
            if kind.starts_with("progress-") {
                self.job = None;
            }
            self.options.shot = None;
        }
    }

    fn open_shot_sample(&mut self, kind: &str) {
        let folder = self.profile.join("shot");
        let _ = std::fs::create_dir_all(folder.join("a"));
        let _ = std::fs::create_dir_all(folder.join("b"));
        let text = |app: &App, key: &str| app.language_pack.text(key, key);
        match kind {
            "progress-copy" | "progress-move" | "progress-delete" | "progress-recycle" | "progress-active" | "progress-archive" => {
                // Presentation-only fixtures. Never start an actual filesystem job.
                let (_, receiver) = std::sync::mpsc::channel();
                let mut job = Job::new(receiver, std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), text(self, "RES.COPYING"));
                use super::file_operation::{JobReport, ProgressInfo, TransferReservation};
                let source = folder.join("a").join("document.bin");
                let destination = folder.join("b");
                job.progress = Some(ProgressInfo {
                    from: Some(folder.join("a").to_string_lossy().into_owned()),
                    to: (!matches!(kind, "progress-delete" | "progress-recycle")).then(|| destination.to_string_lossy().into_owned()),
                    total_files: Some(57), total_bytes: Some(12345678), prefer_bytes: true,
                    caption: None,
                });
                job.report = match kind {
                    "progress-delete" | "progress-recycle" => JobReport::Delete { recycle: kind == "progress-recycle", skipped: 0 },
                    "progress-archive" => JobReport::Generic,
                    _ => JobReport::Transfer { verb: if kind == "progress-move" { ("이동", "move") } else { ("복사", "copy") }, requested: 57, clear_clipboard: false },
                };
                if matches!(kind, "progress-copy" | "progress-move" | "progress-active") {
                    job.reservation = TransferReservation::from_operation(&Operation::Copy { sources: vec![source.clone()], destination, move_files: kind == "progress-move" });
                }
                if kind == "progress-active" {
                    job.path = source.to_string_lossy().into_owned();
                    job.bytes = 6543210;
                    job.files = 23;
                }
                if kind == "progress-archive" {
                    job.title = text(self, "RES.EXTRACTING");
                    job.archive_clipboard_started = Some(Instant::now() - Duration::from_secs(1));
                }
                self.job = Some(job);
            }
            "update-download" | "update-verify" | "update-prepare" | "update-install" | "update-failed" => {
                use ndr::update::UpdatePhase;
                let phase = match kind {
                    "update-download" => UpdatePhase::Downloading,
                    "update-verify" => UpdatePhase::Verifying,
                    "update-prepare" => UpdatePhase::Preparing,
                    "update-install" => UpdatePhase::Installing,
                    _ => UpdatePhase::Failed,
                };
                self.update_progress = Some(super::main_window::UpdateProgress {
                    tag: "build-9999".into(), phase, downloaded: 7340032, total: 14680064,
                    error: (phase == UpdatePhase::Failed).then(|| "Update SHA-256 verification failed".into()),
                    failed_phase: (phase == UpdatePhase::Failed).then_some(UpdatePhase::Verifying),
                });
            }
            "error-transfer" | "error-archive" | "error-update" | "error-retry" | "error-continue" | "error-long" => {
                let (title, message) = match kind {
                    "error-transfer" => (text(self, "NDR.OP_TRANSFER_OPERATION_FAILED"), text(self, "NDR.OP_TRANSFER_PARTIAL_DETAIL").replace("{0}", "2").replace("{1}", "1").replace("{2}", "3")),
                    "error-archive" => ("NDR".into(), text(self, "RES.RAR_INVALIDFILE_OR_CORRUPTED")),
                    _ => (text(self, "NDR.UPD_TITLE"), text(self, "NDR.UPD_RELEASE_FAILURE")),
                };
                let flags = match kind {
                    "error-retry" => ndr::msgbox::MB_ABORTRETRYIGNORE,
                    "error-continue" => ndr::msgbox::MB_CANCELTRYCONTINUE,
                    _ => ndr::msgbox::MB_OK,
                };
                let message = if kind == "error-long" {
                    (0..30).map(|i| format!("{message}\nD:\\test\\{}\\document-{i}.bin", "long-folder-".repeat(18))).collect::<Vec<_>>().join("\n")
                } else { message };
                self.system_box(&title, &message, flags | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
            }
            "split" => {
                let source = folder.join("a").join("document.bin");
                let form = super::split_dialogs::SplitForm { source, length: 12345678, path: folder.join("b").to_string_lossy().into_owned(), size: "1 MB".into() };
                self.push_original_dialog(text(self, "formFileSplit.Form.Caption"), DialogKind::Split(form), |_, _, _| {});
            }
            "join" => {
                let form = super::split_dialogs::JoinForm {
                    files: vec![(folder.join("a").join("document.001"), Some(1234567)), (folder.join("a").join("document.002"), Some(7654321))],
                    selected: Some(0), join_text: false, name: "document.bin".into(),
                };
                self.push_original_dialog(text(self, "formFileJoin.Form.Caption"), DialogKind::Join(form), |_, _, _| {});
            }
            "shortcuts" => self.show_shortcut_window(),
            "dropmenu" => {
                let source = folder.join("a").join("document.txt");
                let _ = std::fs::write(&source, b"isolated drop menu fixture");
                self.show_drop_menu(1, vec![source], folder.join("b"), egui::pos2(120.0, 100.0));
            }
            "prompt" => {
                let (title, message) = (text(self, "RES.TITLE_FIND"), text(self, "RES.MSG_FIND"));
                self.prompt(&title, &message, "report", Some(vec!["one".into(), "two".into()]), false, |_, _, _| {});
            }
            "rename" => {
                let message = text(self, "RES.MSG_RENAME").replacen("%s", "document.txt", 1).replacen("%d", "1", 1).replacen("%d", "3", 1);
                self.push_original_dialog(text(self, "formRename.Form.Caption"), DialogKind::Rename { message, name: "document".into(), extension: "txt".into() }, |_, _, _| {});
            }
            "delwarning" => {
                let message = text(self, "RES.MSG_DELETEITEM");
                self.push_original_dialog(text(self, "formDelWarning.Form.Caption"), DialogKind::DeleteWarning { message, show_delete: true, default_button: 1 }, |_, _, _| {});
            }
            "delconfirm" => {
                self.push_original_dialog(text(self, "formDelConfirm.Form.Caption"), DialogKind::DeleteConfirm { message: ndr::markup::substitute(&text(self, "RES.QUE_DELCONFIRM"), &["document.txt"]), attributes: "R  A".into() }, |_, _, _| {});
            }
            "conflict" | "conflict-resume" | "conflict-folder" => {
                let (source, destination) = (folder.join("a").join("document.txt"), folder.join("b").join("document.txt"));
                let _ = std::fs::write(&source, b"new contents");
                let _ = std::fs::write(&destination, b"old");
                let mut form = super::file_operation::ConflictForm::read(&source, &destination, kind == "conflict-resume");
                if kind == "conflict-folder" { form.source_dir = true; form.destination_dir = true; }
                self.push_original_dialog(text(self, "formFileExists.Form.Caption"), DialogKind::FileConflict(Box::new(form)), |_, _, _| {});
            }
            "ftpquick" => self.ftp_quick_connect("ftp://ftp.example.com".into()),
            // For checking the original TfSelectDest layout by eye: one recent folder, one work folder.
            "destination" => {
                self.remember_destination(&folder.join("a"));
                let mut work = vec![folder.join("b").to_string_lossy().into_owned()];
                work.resize(5, String::new());
                self.settings.set_strings("WorkingFolders", work);
                let (initial, current) = (self.panes[1].path.clone(), self.panes[0].path.clone());
                self.select_destination(super::destination_dialog::DestinationPurpose::Copy, initial, Some(current), |_, _| {});
            }
            // For checking by eye the quick search text at the right end of the original menu bar (tbMainMenuAdvancedCustomDraw).
            "quicksearch" => {
                let active = self.active;
                self.typed_search[active].query = "read".into();
                self.typed_search[active].cursor = Some(self.panes[active].cursor);
            }
            // For checking by eye the log box of the pane FTP mode (original FUN_0048d7ac): the right pane uses a fake FTP connection.
            "ftplog" => {
                let log = std::sync::Arc::new(std::sync::Mutex::new(ndr::remote::ProtocolLog::default()));
                if let Ok(mut log) = log.lock() {
                    log.received("220 ProFTPD Server (example) ready.");
                    log.sent("USER anonymous");
                    log.received("331 Anonymous login ok, send your complete email address as your password");
                    log.sent("PASS guest@example.com");
                    log.received("230 Anonymous access granted, restrictions apply");
                    log.prefix = "/pub".into();
                    log.sent("PASV");
                    log.received("227 Entering Passive Mode (127,0,0,1,195,80).");
                    log.sent("LIST");
                }
                let remote = super::ftp_pane::PaneRemote { fs: std::sync::Arc::new(std::sync::Mutex::new(Box::new(ShotFtp) as Box<dyn super::ftp_pane::PaneFs>)), host: "ftp.example.com".into(), path: "/pub".into(), busy: Default::default(), modes: Default::default(), permissions: Default::default(), log: Some(log) };
                self.ftp_connected(1, remote);
            }
            // Opening the menu bar (same path as Alt+F): for checking skin menu colors and button cells by eye.
            "mainmenu" => self.menu_access = Some('f'),
            "editmenu" => self.menu_access = Some('e'),
            "foldermenu" => self.menu_access = Some('o'),
            "viewmenu" => self.menu_access = Some('w'),
            "popupmenu" => {
                // For checking skin menu colors (skin_menu_style) by eye: opens the favorites menu (with header, separator and disabled rows) at the top left.
                self.show_favorites_menu();
                if let Some(popup) = self.popup.as_mut() {
                    popup.position = Some(egui::pos2(40.0, 60.0));
                    popup.highlight = popup.items.iter().position(|i| matches!(i, super::favorites_dialog::PopupItem::Entry { .. }));
                }
            }
            "ftpattr" => {
                let form = super::remote_window::PermissionForm::new(0o644, "document.txt (1 / 3)", "/pub/document.txt");
                self.push_original_dialog(text(self, "formFTPAttributes.Form.Caption"), DialogKind::RemotePermissions(Box::new(form)), |_, _, _| {});
            }
            "checksum" => {
                self.push_original_dialog("MD5".into(), DialogKind::Checksum { value: "d41d8cd98f00b204e9800998ecf8427e".into(), ready: true }, |_, _, _| {});
            }
            "attributes" => {
                let form = super::original_dialogs::AttributeForm::new("D:\\data\\document.txt");
                self.push_original_dialog(text(self, "formChangeAttr.Form.Caption"), DialogKind::Attributes(form), |_, _, _| {});
            }
            "editshortcut" => {
                let form = super::shortcut_window::EditShortcutForm { caption: text(self, "RES.COPY"), original: "Ctrl+C".into(), text: "Ctrl+C".into(), last_state: None, available: vec!["Ctrl+Q".into(), "Ctrl+W".into()], chosen: None };
                self.push_original_dialog(text(self, "formEditShortCut.Form.Caption"), DialogKind::EditShortcut(form), |_, _, _| {});
            }
            "savelist" => {
                let rows = (0..3).map(|i| ndr::list_export::ExportRow::new(&format!("file{i}.txt"), false, 1234 * (i + 1), Some(std::time::SystemTime::now()), 0x20, String::new())).collect();
                let form = super::list_export_dialog::ListExportForm::new(rows);
                self.push_original_dialog(text(self, "formSaveList.Form.Caption"), DialogKind::ListExport(Box::new(form)), |_, _, _| {});
            }
            "foldertree" | "foldertree-search" | "foldertree-search-many" | "foldertree-favorites" => {
                let mut lines: Vec<ndr::folder_tree::Line> = [
                    (true, 0, "C:\\"), (true, 1, "Program Files"), (true, 2, "Common Files"), (false, 2, "Internet Explorer"), (true, 2, "Windows NT"),
                    (true, 3, "Accessories"), (true, 1, "Users"), (true, 2, "user"), (true, 3, "Documents"), (true, 3, "Downloads"), (true, 3, "Pictures"),
                    (true, 2, "Public"), (true, 1, "Windows"), (false, 2, "System32"), (false, 2, "Temp"),
                ].iter().map(|(e, l, n)| (*e, *l, n.to_string())).collect();
                if kind == "foldertree-search-many" {
                    lines.extend((0..24).map(|n| (false, 1, format!("needle_{n:02}"))));
                }
                let drives = vec![(PathBuf::from("C:\\"), "Local Disk".to_owned()), (PathBuf::from("D:\\"), "Data".to_owned())];
                let mut form = super::folder_tree_dialog::FolderTreeForm::new(&self.settings, &self.profile, drives, vec!["C:\\Users\\user\\Downloads".into()]);
                form.tree = ndr::folder_tree::Tree::from_lines(&lines).unwrap_or_default();
                form.drive = "C:\\".into();
                let target = form.tree.find_path("C:\\Users\\user\\Pictures");
                form.focus(target, 1);
                if kind == "foldertree-search" || kind == "foldertree-search-many" {
                    let mut search = super::folder_tree_dialog::SearchFolder { query: if kind == "foldertree-search-many" { "needle".into() } else { "user".into() }, ..Default::default() };
                    search.run(&form.tree);
                    if kind == "foldertree-search-many" { search.top_result = 16; }
                    form.search = Some(search);
                }
                if kind == "foldertree-favorites" {
                    form.favorites = (0..20).map(|n| (n, format!("QA favorite {n:02}"), "C:\\Users\\user\\Pictures".into())).collect();
                    form.favorites_open = true;
                }
                self.push_original_dialog(text(self, "formFolderTree.Form.Caption"), DialogKind::FolderTree(Box::new(form)), |_, _, _| {});
            }
            "tabs" => {
                let current = self.panes[0].path.clone();
                self.panes[0].tabs = vec![current, folder.join("a"), folder.join("b")];
                self.panes[0].tab = 0;
            }
            "columns2" => {
                self.settings.values.insert("LeftColumnCount".into(), serde_json::json!(2));
            }
            "narrowpane" => {
                self.settings.values.insert("LeftPaneWidth".into(), serde_json::json!(20));
                self.settings.values.insert("RightPaneWidth".into(), serde_json::json!(80));
            }
            "crumbmenu" => {
                // Original TNXPathLabel.DropDownMenu: the subfolder list of the left pane folder.
                let folder = self.panes[0].path.clone();
                self.skin_crumb_menu = Some((0, folder, false, egui::pos2(120.0, 104.0)));
            }
            "narrowdetails" => {
                // Original UpdateScrollBar (style 2): a horizontal bar when the total cell width exceeds the list. Narrows the left pane, Details view, horizontal position 40.
                self.settings.values.insert("LeftPaneWidth".into(), serde_json::json!(25));
                self.settings.values.insert("RightPaneWidth".into(), serde_json::json!(75));
                self.settings.set_text("LeftViewStyle", "Details");
                self.refresh(0);
            }
            "favorites" => {
                let items = vec![("Data".to_owned(), "D:\\data".to_owned()), ("Missing".to_owned(), "Z:\\nowhere".to_owned())];
                let form = super::favorites_dialog::FavoritesForm::new(items);
                self.push_original_dialog(text(self, "formOrgFavorites.Form.Caption"), DialogKind::Favorites(Box::new(form)), |_, _, _| {});
            }
            "funckey" => {
                let binding = super::function_key_editor::Binding { name: "Notepad".into(), command: "notepad.exe".into(), parameters: "$(SELECTED)".into(), internal: false };
                let form = super::function_key_editor::FunctionKeyEditForm::new("F9", &binding, self.language == "ko");
                self.push_original_dialog(text(self, "formEditFunction.Form.Caption"), DialogKind::FunctionKeyEdit(Box::new(form)), |_, _, _| {});
            }
            "archive" => {
                self.active = 0;
                // The first item other than ".." (".." is not an archive target).
                self.panes[0].cursor = self.panes[0].entries.iter().position(|e| !e.is_parent() && !e.drive).unwrap_or(0);
                self.archive_command(super::archive_dialog::ArchiveKind::Create);
            }
            "progress" => {
                let (source, target) = (folder.join("a"), folder.join("b"));
                let info = super::file_operation::ProgressInfo { from: Some(source.to_string_lossy().into_owned()), to: Some(target.to_string_lossy().into_owned()), total_files: Some(12), total_bytes: Some(48 * 1024 * 1024), prefer_bytes: true, ..Default::default() };
                let file = source.join("document.txt");
                self.start_progress_task("Copy".into(), info, move |events, cancel| {
                    let _ = events.send(ndr::fsops::Event::Progress { path: file, bytes: 20 * 1024 * 1024, files: 4 });
                    let started = std::time::Instant::now();
                    while !cancel.load(std::sync::atomic::Ordering::Relaxed) && started.elapsed() < std::time::Duration::from_secs(4) {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Ok(())
                });
            }
            "advrename" | "advrename-delete" | "advrename-replace" | "advrename-case" | "advrename-number" | "advrename-extension" | "advrename-rules" | "advrename-preview" => {
                self.active = 0;
                // Item 0 is ".." (since r106) and cannot be renamed. The first ordinary item.
                self.panes[0].cursor = self.panes[0].entries.iter().position(|e| !e.is_parent() && !e.drive).unwrap_or(0);
                self.advanced_rename();
                if let Some(DialogKind::AdvancedRename(form)) = self.original_dialogs.last_mut().map(|d| &mut d.kind) {
                    form.editor.text = "new_".into();
                    form.rules.push(form.editor.clone());
                    form.selected_rule = Some(0);
                    match kind {
                        "advrename-delete" => form.editor.action = "Delete".into(),
                        "advrename-replace" => { form.editor.action = "Replace".into(); form.editor.position = "All".into(); }
                        "advrename-case" => form.editor.action = "Case".into(),
                        "advrename-number" => form.editor.action = "Number".into(),
                        "advrename-extension" => form.editor.action = "Extension".into(),
                        "advrename-rules" => form.compact_page = 1,
                        "advrename-preview" => form.compact_page = 2,
                        _ => {}
                    }
                }
            }
            "sites" => {
                let sites = vec![
                    super::remote_window::SiteProfile { name: "Example".into(), protocol: "FTP".into(), host: "ftp.example.com".into(), port: 21, user: "guest".into(), remote_directory: "/pub".into(), ..Default::default() },
                    super::remote_window::SiteProfile { name: "Backup".into(), protocol: "SFTP".into(), host: "backup.example.com".into(), port: 22, ..Default::default() },
                ];
                let mut form = super::remote_sites::SitesForm::new(sites);
                form.load(Some(0));
                self.push_original_dialog(text(self, "formFTPSiteManager.Form.Caption"), DialogKind::RemoteSites(Box::new(form)), |_, _, _| {});
            }
            "credentials" => {
                let title = self.language_pack.text("NDR.UI_TOOLS_NETWORK_CREDENTIALS", "Network drive credentials");
                self.credentials_dialog(&title, |_, _| {});
            }
            // C# remote window (not connected): for checking the skin images of the upload/download buttons in the middle.
            "remote" => self.open_remote_window(None, None),
            "search" => self.search_command(),
            // Isolated screenshot fixture: neither dialog authenticates or creates a container.
            "vaultcreate" => self.open_vault(folder.join("a"), true),
            "vaultunlock" => self.open_vault(folder.join("sample.ndr"), false),
            "advselect" => self.advanced_select(false),
            "syncpreview" => {
                use ndr::folder_compare::{Difference, DifferenceKind};
                let now = Some(std::time::SystemTime::now());
                let rows = ["alpha.txt", "folder\\a_long_filename_for_the_synchronization_preview.txt", "beta.txt", "omega.txt"].into_iter().enumerate().map(|(i, name)| Difference {
                    relative: name.into(), right_relative: name.into(),
                    kind: if i == 2 { DifferenceKind::Different } else { DifferenceKind::LeftOnly },
                    left_size: 1234, right_size: if i == 2 { 4321 } else { 0 },
                    left_modified: now, right_modified: if i == 2 { now } else { None },
                    left_dir: false, right_dir: false,
                }).collect();
                let form = super::compare_dialog::ComparisonForm::new(folder.join("a").to_string_lossy().into_owned(), folder.join("b").to_string_lossy().into_owned(), rows, true, vec![], &self.language_pack);
                self.push_original_dialog(text(self, "NDR.UI_COMPARE_SYNC_PREVIEW"), DialogKind::Comparison(Box::new(form)), |_, _, _| {});
            }
            "compareopts" | "compareresult" => {
                use ndr::folder_compare::{Difference, DifferenceKind};
                let mut form = super::compare_dialog::FolderCompareForm::new(folder.join("a"), folder.join("b"), true);
                if kind == "compareresult" {
                    let now = Some(std::time::SystemTime::now());
                    let row = |relative: &str, kind, left: bool, right: bool| Difference {
                        relative: relative.into(),
                        right_relative: relative.into(),
                        kind,
                        left_size: if left { 1234 } else { 0 },
                        right_size: if right { 4321 } else { 0 },
                        left_modified: left.then_some(now).flatten(),
                        right_modified: right.then_some(now).flatten(),
                        left_dir: false,
                        right_dir: false,
                    };
                    form.rows = vec![
                        row("same.txt", DifferenceKind::Same, true, true),
                        row("changed.txt", DifferenceKind::Different, true, true),
                        row("left-only.txt", DifferenceKind::LeftOnly, true, false),
                        row("right-only.txt", DifferenceKind::RightOnly, false, true),
                    ];
                    form.grid = super::grid::GridState::new(form.rows.len());
                    form.compared = true;
                    form.result = Some(form.result_text(&text(self, "RES.FOLDERCOMPARE_RESULT")));
                }
                self.push_original_dialog(text(self, "formCompareFolder.Form.Caption"), DialogKind::FolderCompare(Box::new(form)), |_, _, _| {});
            }
            "ftpresults" => {
                use super::ftp_pane::{FtpResultRow, RowState};
                if let Ok(mut rows) = self.ftp_results.lock() {
                    *rows = vec![
                        FtpResultRow { upload: false, local: "C:\\down\\a.zip".into(), remote: "/pub/a.zip".into(), size: Some(4000), done: 1000, state: RowState::Running, started: None },
                        FtpResultRow { upload: false, local: "C:\\down\\b.txt".into(), remote: "/pub/b.txt".into(), size: Some(10), done: 0, state: RowState::Waiting, started: None },
                    ];
                }
                self.show_results = true;
            }
            "panetree" => {
                // Original miShowFoldersClick (Alt+F3): the folder tree left of the list inside the left pane.
                self.settings.set_flag("ShowLeftFolderTree", true);
                self.refresh(0);
            }
            "gridlines" | "rowlines" => {
                // Check of original ShowGridColumn, ShowGridRow and LineSpace: Names view (column and row lines) and Details view (row lines).
                self.settings.set_flag("ShowRowLines", true);
                self.settings.set_flag("ShowColumnLines", true);
                self.settings.values.insert("ListLineSpace".into(), serde_json::json!(6));
                self.settings.set_text("LeftViewStyle", if kind == "gridlines" { "Names" } else { "Details" });
                self.refresh(0);
            }
            "names" | "auto" | "thumbs" => {
                self.settings.set_text("LeftViewStyle", match kind { "names" => "Names", "thumbs" => "Thumbnails", _ => "Auto" });
                self.refresh(0);
            }
            _ if kind.starts_with("settings") => {
                self.open_settings_window();
                let tab = kind.trim_start_matches("settings").parse::<usize>().unwrap_or(0);
                if let Some(form) = self.settings_form() {
                    form.tab = tab.min(form.tabs.len().saturating_sub(1));
                }
            }
            _ => self.status = format!("Unknown shot kind: {kind}"),
        }
    }
}

fn audit_shot_text(ctx: &egui::Context, path: &Path) {
    use egui::epaint::Shape;
    let shapes = ctx.graphics(|g| g.clone().drain(&[], &Default::default()));
    let screen = ctx.content_rect();
    fn rect_json(r: egui::Rect) -> [f32; 4] { [r.min.x, r.min.y, r.max.x, r.max.y] }
    fn visit(ctx: &egui::Context, shape: &Shape, clip: egui::Rect, rows: &mut Vec<serde_json::Value>) {
        match shape {
            Shape::Vec(shapes) => for s in shapes { visit(ctx, s, clip, rows); },
            Shape::Text(t) => {
                let bounds = t.visual_bounding_rect();
                if !bounds.is_positive() || !bounds.intersects(clip) { return; }
                let job = &t.galley.job;
                let mut missing = std::collections::BTreeSet::new();
                ctx.fonts_mut(|fonts| {
                    for section in &job.sections {
                        if let Some(value) = job.text.get(section.byte_range.clone()) {
                            for ch in value.chars().filter(|c| !c.is_control() && !c.is_whitespace()) {
                                if !fonts.has_glyph(&section.format.font_id, ch) { missing.insert(ch); }
                            }
                        }
                    }
                });
                let visible = bounds.intersect(clip);
                let clipped = (bounds.width() - visible.width() > 1.5) || (bounds.height() - visible.height() > 1.5);
                rows.push(serde_json::json!({"text":job.text,"bounds":rect_json(bounds),"clip":rect_json(clip),"elided":t.galley.elided,"clipped":clipped,"missing":missing.into_iter().collect::<String>(),"rows":t.galley.rows.len()}));
            }
            _ => {}
        }
    }
    let mut rows = Vec::new();
    for item in &shapes { visit(ctx, &item.shape, item.clip_rect.intersect(screen), &mut rows); }
    let _ = std::fs::write(path.with_extension("layout.json"), serde_json::to_vec_pretty(&rows).unwrap_or_default());
}

/// Full locale sweep of real dialog widgets without opening desktop windows.
#[cfg(all(test, debug_assertions))]
#[test]
#[ignore = "explicit localization layout QA"]
fn all_locale_dialog_layout_sweep() {
    let output = PathBuf::from(std::env::var_os("NDR_LAYOUT_QA_DIR").expect("NDR_LAYOUT_QA_DIR"));
    std::fs::create_dir_all(&output).unwrap();
    let kinds = ["prompt", "rename", "delwarning", "delconfirm", "conflict", "ftpquick", "destination", "ftpattr", "checksum", "attributes", "editshortcut", "savelist", "foldertree", "favorites", "funckey", "archive", "advrename", "advrename-delete", "sites", "credentials", "remote", "search", "vaultcreate", "vaultunlock", "advselect", "compareopts", "compareresult", "syncpreview", "split", "join", "shortcuts", "progress-copy", "progress-move", "progress-delete", "progress-recycle", "progress-active", "progress-archive", "update-download", "update-verify", "update-prepare", "update-install", "update-failed", "error-transfer", "error-archive", "error-update", "error-retry", "error-continue", "error-long"];
    let raw = |width, height| egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() };
    for &(locale, _) in ndr::compat::BUILTIN_LANGUAGES {
        let dir = output.join(locale);
        let left = dir.join("left");
        let right = dir.join("right");
        std::fs::create_dir_all(left.join("sample-folder")).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        std::fs::write(left.join("document.txt"), b"test").unwrap();
        let ctx = egui::Context::default();
        ndr::compat::begin_locale_lookup_audit();
        let mut app = App::from_context(&ctx, Options { profile: Some(dir.join("profile")), left: Some(left), right: Some(right), language: Some(locale.into()), ..Default::default() });
        set_app_fonts(&ctx, None, false, 1.0);
        let until = Instant::now() + Duration::from_secs(10);
        while app.panes.iter().any(|p| p.loading) && Instant::now() < until {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        app.open_shot_sample("settings0");
        let tabs = app.settings_form().map_or(0, |f| f.tabs.len());
        std::fs::write(dir.join("startup.lookups.json"), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        let cases: Vec<String> = kinds.iter().map(|s| s.to_string()).chain(std::iter::once("dropmenu".to_owned())).chain((0..tabs).map(|i| format!("settings{i}"))).collect();
        for kind in cases {
            ndr::compat::begin_locale_lookup_audit();
            app.original_dialogs.clear();
            app.vault_dialog = None;
            app.msgboxes.clear();
            app.shortcut_window = None;
            app.popup = None;
            app.job = None;
            app.update_progress = None;
            app.open_shot_sample(&kind);
            assert!(!app.original_dialogs.is_empty() || app.vault_dialog.is_some() || !app.msgboxes.is_empty() || app.shortcut_window.is_some() || app.job.is_some() || app.update_progress.is_some() || app.popup.is_some(), "{locale}:{kind}: no dialog");
            for i in 0..4 {
                let _ = ctx.run(raw(1400.0, 850.0), |ctx| {
                    app.vault_window_ui(ctx);
                    app.original_dialog_ui(ctx);
                    app.msgbox_ui(ctx);
                    app.shortcut_window_ui(ctx);
                    app.popup_menu_ui(ctx);
                    app.file_progress_ui(ctx);
                    app.concurrent_transfers_ui(ctx);
                    app.update_progress_ui(ctx);
                    if i == 3 { audit_shot_text(ctx, &dir.join(format!("{kind}.png"))); }
                });
            }
            std::fs::write(dir.join(format!("{kind}.lookups.json")), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        }
        // The function key bar is sensitive to available width and long
        // translated actions. Exercise it without mutating user bindings.
        app.original_dialogs.clear();
        app.vault_dialog = None;
        app.msgboxes.clear();
        app.job = None;
        app.update_progress = None;
        for &(title, ko, en, _) in super::menu_data::MENUS {
            ndr::compat::begin_locale_lookup_audit();
            egui::Popup::close_all(&ctx);
            let caption = app.language_pack.raw(title).unwrap_or_else(|| if locale == "ko" { ko.into() } else { en.into() });
            app.menu_access = super::favorites_dialog::access_key(&caption).1.map(|c| c.to_ascii_lowercase());
            for i in 0..4 {
                let _ = ctx.run(raw(1400.0, 850.0), |ctx| {
                    egui::TopBottomPanel::top("audit-menu").show(ctx, |ui| app.menu(ui));
                    if i == 3 { audit_shot_text(ctx, &dir.join(format!("menu-{title}.png"))); }
                });
            }
            std::fs::write(dir.join(format!("menu-{title}.lookups.json")), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        }
        egui::Popup::close_all(&ctx);
        app.shortcut_window = None;
        // Main window controls, including tooltips resolved while drawing
        // the vertical bar and menu labels built only for right-clicks.
        ndr::compat::begin_locale_lookup_audit();
        let _ = ctx.run(raw(1400.0, 850.0), |ctx| {
            if let Some(skin) = app.current_skin() {
                egui::TopBottomPanel::top("audit-toolbar").exact_height(51.0).frame(egui::Frame::NONE).show(ctx, |ui| app.skin_toolbar(ui, ctx, &skin));
            }
            egui::SidePanel::left("audit-centerbar").exact_width(18.0).frame(egui::Frame::NONE).show(ctx, |ui| app.vertical_bar(ui, ctx));
            egui::CentralPanel::default().show(ctx, |ui| {
                let saved = app.panes[0].cursor;
                for folder in [false, true] {
                    if let Some(row) = app.panes[0].entries.iter().position(|e| e.is_dir == folder && !e.is_parent() && !e.drive) {
                        app.panes[0].cursor = row;
                        for (label, _, _) in app.list_menu_items(0).into_iter().flatten() {
                            ui.label(ndr::msgbox::display_caption(&label).0);
                        }
                    }
                }
                app.panes[0].cursor = saved;
                let _ = app.sort_items(ui, locale == "ko");
            });
            audit_shot_text(ctx, &dir.join("main-controls.png"));
        });
        std::fs::write(dir.join("main-controls.lookups.json"), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        for width in [800.0, 1400.0] {
            ndr::compat::begin_locale_lookup_audit();
            let _ = ctx.run(raw(width, 850.0), |ctx| {
                egui::TopBottomPanel::bottom("audit-function-bar").exact_height(24.0).show(ctx, |ui| {
                    if let Some(skin) = app.current_skin() {
                        app.skin_function_key_bar(ui, ctx, &skin);
                    }
                });
                audit_shot_text(ctx, &dir.join(format!("functionbar-{width}.png")));
            });
            std::fs::write(dir.join(format!("functionbar-{width}.lookups.json")), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        }
        // Imported profiles can contain legacy menu IDs that the default
        // shortcut map no longer lists. Test captions without executing them.
        let original_map = app.settings.values.get("OriginalShortcutMapText").cloned();
        app.settings.set_text("OriginalShortcutMapText", "NEXUSFILESHORTCUT1.0\r\nmiViewWorkWnd=Alt+F9\r\nmiSynchronizeFolder=Alt+F10\r\nmiEmail=Alt+F11\r\nmiDropCopy=Ctrl+Shift+1\r\nmiDropMove=Ctrl+Shift+2\r\nmiDropShortcut=Ctrl+Shift+3\r\nmiDropCancel=Ctrl+Shift+4\r\n");
        ndr::compat::begin_locale_lookup_audit();
        let imported = app.shortcut_rows("");
        assert_eq!(imported.len(), 7, "{locale}: imported rows");
        for row in &imported {
            let key = format!("formMain.Menu.{}", row.name);
            assert!(app.language_pack.primary_raw(&key).is_some(), "{locale}: {key}");
            assert!(!row.caption.is_empty() && row.caption != row.name, "{locale}: {row:?}");
        }
        app.open_shot_sample("shortcuts");
        for i in 0..4 {
            let _ = ctx.run(raw(1400.0, 850.0), |ctx| {
                app.shortcut_window_ui(ctx);
                if i == 3 { audit_shot_text(ctx, &dir.join("legacy-shortcuts.png")); }
            });
        }
        std::fs::write(dir.join("legacy-shortcuts.lookups.json"), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        app.shortcut_window = None;
        match original_map { Some(value) => { app.settings.values.insert("OriginalShortcutMapText".into(), value); }, None => { app.settings.values.remove("OriginalShortcutMapText"); } }
        // The internal command toolbar follows the same ID-based catalog as
        // the function key editor, including rare imported commands.
        ndr::compat::begin_locale_lookup_audit();
        for &(id, fallback) in super::function_key_editor::INTERNAL_COMMANDS {
            let name = id.trim_start_matches('#');
            let item = ndr::commands::ToolbarItem { kind: ndr::commands::ToolbarKind::Command, action: format!("function:{name}"), caption: String::new(), icon: String::new(), align_right: false, width: 1.0, children: Vec::new() };
            let key = if id == "#OpenRecycleBin" { "Menu.miRecycleBin".to_owned() } else { format!("RES.FUNC_{}", name.to_ascii_uppercase()) };
            assert!(app.language_pack.primary_raw(&key).is_some(), "{locale}: {key}");
            assert_eq!(app.toolbar_label(&item), super::function_key_editor::internal_command_caption(&app.language_pack, id, fallback), "{locale}: {id}");
        }
        std::fs::write(dir.join("imported-toolbar.lookups.json"), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        // The toolbar also accepts a user-supplied caption. Its reverse
        // catalog lookup can reach compatibility keys without a literal
        // resource reference in the Rust source.
        ndr::compat::begin_locale_lookup_audit();
        for (key, source) in [("RES.FIELD_LOCATION", "Location"), ("RES.SELECT_FOLDER", "Choose a folder"), ("RES.FOLDER_EXISTS", "A folder with that name already exists."), ("RES.SRCISDEST", "The source and destination are the same."),
            ("RES.RENAME_WITHEXT", "Include the extension"), ("RES.CHARMAP", "Character Map"), ("RES.DISKDISKFREE", "Available space"), ("RES.SELDEST_DRIVE", "Drives"), ("RES.SELDEST_MOSTUSED", "Frequent destinations"), ("RES.SELDEST_RECENTS", "Recent destinations"), ("RES.SELDEST_WORKFOLDER", "Work slots"), ("formSelectDest.Form.Caption", "Choose destination"),
            ("formNDRDestination.Browse.Title", "Select destination folder"), ("formNDRDestination.Button.Browse", "Browse another folder…"), ("formNDRDestination.Title.Copy", "Choose a copy destination"), ("formNDRDestination.Title.Extract", "Choose an extraction destination"), ("formNDRDestination.Title.Move", "Choose a move destination"), ("formNDRDestination.Hint.WorkFolders", "Click a saved work folder or press 1–5. Browse below to choose another folder."), ("formSelectDest.Caption.lHelp", "Choose a drive to browse its folders."), ("formProgress.Caption.lFileName", "File:"), ("formProgress.Caption.lFrom", "From:"), ("formProgress.Caption.lTo", "To:"), ("formProgress.Caption.lRemaining", "Remaining:")] {
            let item = ndr::commands::ToolbarItem { kind: ndr::commands::ToolbarKind::Command, action: "newFolder".into(), caption: source.into(), icon: String::new(), align_right: false, width: 1.0, children: Vec::new() };
            assert!(app.language_pack.primary_raw(key).is_some(), "{locale}: {key}");
            assert_eq!(app.toolbar_label(&item), app.language_pack.text(key, source), "{locale}: imported caption {key}");
        }
        std::fs::write(dir.join("imported-captions.lookups.json"), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        // Old folder-search and theme/disk resources are retained for
        // compatibility. Validate the primary entry without reviving the
        // obsolete UI or assuming every template is a runtime reference.
        ndr::compat::begin_locale_lookup_audit();
        for key in ["formFolderTree.Caption.lQuickSearchPrompt", "formFolderTree.Caption.lQuickSearchScanning", "formFolderTree.Caption.lQuickSearchMatch", "formFolderTree.Caption.lQuickSearchNoMatch", "formFolderTree.Caption.lQuickSearchCancelled", "formFolderTree.Caption.lQuickSearchFailed", "RES.TITLE_DISKCOPY", "RES.MSG_DISKCOPY", "RES.INVALID_SKIN", "RES.MSG_SETSKIN"] {
            assert!(app.language_pack.primary_raw(key).is_some_and(|s| !s.is_empty()), "{locale}: {key}");
        }
        std::fs::write(dir.join("legacy-search-theme.lookups.json"), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        // Exercise the compact settings page at two smaller logical sizes.
        // This simulates available space, not a change to the user's OS DPI.
        for (w, h, scale) in [(768.0, 576.0, 1.25), (640.0, 480.0, 1.5), (480.0, 360.0, 2.0)] {
            let scaled = || {
                let mut input = raw(w, h);
                input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(scale);
                input
            };
            for tab in 0..tabs {
                ndr::compat::begin_locale_lookup_audit();
                app.original_dialogs.clear();
                app.open_shot_sample(&format!("settings{tab}"));
                for i in 0..4 {
                    let _ = ctx.run(scaled(), |ctx| {
                        assert!((ctx.pixels_per_point() - scale).abs() < 0.01);
                        app.original_dialog_ui(ctx);
                        if i == 3 { audit_shot_text(ctx, &dir.join(format!("compact-settings{tab}-{w}x{h}-{scale}.png"))); }
                    });
                }
                std::fs::write(dir.join(format!("compact-settings{tab}-{w}x{h}-{scale}.lookups.json")), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
            }
        }
        app.original_dialogs.clear();
        ndr::compat::begin_locale_lookup_audit();
        app.open_shot_sample("shortcuts");
        // Resize an already-open F12 window, not just a fresh one.
        for (w, h, scale) in [(1400.0, 850.0, 1.0), (480.0, 360.0, 2.0), (320.0, 250.0, 2.0)] {
            for i in 0..4 {
                let mut input = raw(w, h);
                input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(scale);
                let _ = ctx.run(input, |ctx| {
                    app.shortcut_window_ui(ctx);
                    if i == 3 { audit_shot_text(ctx, &dir.join(format!("compact-shortcuts-{w}x{h}.png"))); }
                });
            }
            let area = ctx.memory(|m| m.area_rect(egui::Id::new("ndr-shortcut-window"))).expect("shortcut area");
            assert!(area.left() >= -1.0 && area.top() >= -1.0 && area.right() <= w + 1.0 && area.bottom() <= h + 1.0, "{locale}: {w}x{h}: {area:?}");
        }
        std::fs::write(dir.join("compact-shortcuts.lookups.json"), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        app.shortcut_window = None;
        for (w, h, scale) in [(480.0, 360.0, 2.0), (320.0, 250.0, 2.0)] {
            ndr::compat::begin_locale_lookup_audit();
            app.original_dialogs.clear();
            app.open_shot_sample("editshortcut");
            for i in 0..4 {
                let mut input = raw(w, h);
                input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(scale);
                let _ = ctx.run(input, |ctx| {
                    app.original_dialog_ui(ctx);
                    if i == 3 { audit_shot_text(ctx, &dir.join(format!("compact-editshortcut-{w}x{h}.png"))); }
                });
            }
            std::fs::write(dir.join(format!("compact-editshortcut-{w}x{h}.lookups.json")), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
        }
        for (w, h, scale) in [(480.0, 360.0, 2.0), (320.0, 250.0, 2.0)] {
            for kind in ["prompt", "rename", "destination", "delwarning", "delconfirm", "conflict", "conflict-resume", "conflict-folder", "split", "join", "foldertree", "foldertree-search", "advrename", "advrename-delete", "advrename-replace", "advrename-case", "advrename-number", "advrename-extension", "advrename-rules", "advrename-preview", "compareopts", "compareresult", "syncpreview", "foldertree-search-many", "foldertree-favorites", "search", "archive"] {
                ndr::compat::begin_locale_lookup_audit();
                app.original_dialogs.clear();
                app.open_shot_sample(kind);
                for i in 0..4 {
                    let mut input = raw(w, h);
                    input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(scale);
                    let _ = ctx.run(input, |ctx| {
                        app.original_dialog_ui(ctx);
                        if i == 3 { audit_shot_text(ctx, &dir.join(format!("compact-{kind}-{w}x{h}.png"))); }
                    });
                }
                std::fs::write(dir.join(format!("compact-{kind}-{w}x{h}.lookups.json")), serde_json::to_vec(&ndr::compat::end_locale_lookup_audit()).unwrap()).unwrap();
            }
        }
        println!("layout sweep {locale}: {} dialogs plus legacy shortcuts and compact settings/inputs", kinds.len() + tabs + 1);
    }
}

fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}
