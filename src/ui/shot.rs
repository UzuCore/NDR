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
            if kind == "narrowpane" {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(800.0, 480.0)));
            }
        }
        // The pane folder tree reads the shell namespace separately, so the shot waits until expanded items are read.
        let tree_waiting = kind == "panetree" && self.tree_requests.iter().any(|key| !self.tree_cache.contains_key(key));
        let sent_id = egui::Id::new("ndr-shot-sent");
        if self.frames >= SHOT_FRAME && !tree_waiting && !ctx.data(|d| d.get_temp::<bool>(sent_id)).unwrap_or(false) {
            ctx.data_mut(|d| d.insert_temp(sent_id, true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        if kind == "narrowdetails" && self.frames == SHOT_FRAME - 4 {
            self.panes[0].detail_x = 40.0;
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
            self.options.shot = None;
        }
    }

    fn open_shot_sample(&mut self, kind: &str) {
        let folder = self.profile.join("shot");
        let _ = std::fs::create_dir_all(folder.join("a"));
        let _ = std::fs::create_dir_all(folder.join("b"));
        let text = |app: &App, key: &str| app.language_pack.text(key, key);
        match kind {
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
                self.push_original_dialog(text(self, "formDelConfirm.Form.Caption"), DialogKind::DeleteConfirm { message: format!("{}\n\ndocument.txt", text(self, "formDelConfirm.Caption.lMsg")), attributes: "R  A".into() }, |_, _, _| {});
            }
            "conflict" => {
                let (source, destination) = (folder.join("a").join("document.txt"), folder.join("b").join("document.txt"));
                let _ = std::fs::write(&source, b"new contents");
                let _ = std::fs::write(&destination, b"old");
                let form = super::file_operation::ConflictForm::read(&source, &destination, false);
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
                let remote = super::ftp_pane::PaneRemote { fs: std::sync::Arc::new(std::sync::Mutex::new(Box::new(ShotFtp) as Box<dyn super::ftp_pane::PaneFs>)), host: "ftp.example.com".into(), path: "/pub".into(), busy: Default::default(), modes: Default::default(), log: Some(log) };
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
                let form = super::shortcut_window::EditShortcutForm { caption: "Copy".into(), original: "Ctrl+C".into(), text: "Ctrl+C".into(), last_state: None, available: vec!["Ctrl+Q".into(), "Ctrl+W".into()], chosen: None };
                self.push_original_dialog(text(self, "formEditShortCut.Form.Caption"), DialogKind::EditShortcut(form), |_, _, _| {});
            }
            "savelist" => {
                let rows = (0..3).map(|i| ndr::list_export::ExportRow::new(&format!("file{i}.txt"), false, 1234 * (i + 1), Some(std::time::SystemTime::now()), 0x20, String::new())).collect();
                let form = super::list_export_dialog::ListExportForm::new(rows);
                self.push_original_dialog(text(self, "formSaveList.Form.Caption"), DialogKind::ListExport(Box::new(form)), |_, _, _| {});
            }
            "foldertree" => {
                let lines: Vec<ndr::folder_tree::Line> = [
                    (true, 0, "C:\\"), (true, 1, "Program Files"), (true, 2, "Common Files"), (false, 2, "Internet Explorer"), (true, 2, "Windows NT"),
                    (true, 3, "Accessories"), (true, 1, "Users"), (true, 2, "user"), (true, 3, "Documents"), (true, 3, "Downloads"), (true, 3, "Pictures"),
                    (true, 2, "Public"), (true, 1, "Windows"), (false, 2, "System32"), (false, 2, "Temp"),
                ].iter().map(|(e, l, n)| (*e, *l, n.to_string())).collect();
                let drives = vec![(PathBuf::from("C:\\"), "Local Disk".to_owned()), (PathBuf::from("D:\\"), "Data".to_owned())];
                let mut form = super::folder_tree_dialog::FolderTreeForm::new(&self.settings, &self.profile, drives, vec!["C:\\Users\\user\\Downloads".into()]);
                form.tree = ndr::folder_tree::Tree::from_lines(&lines).unwrap_or_default();
                form.drive = "C:\\".into();
                let target = form.tree.find_path("C:\\Users\\user\\Pictures");
                form.focus(target, 1);
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
            "advrename" | "advrename-delete" => {
                self.active = 0;
                // Item 0 is ".." (since r106) and cannot be renamed. The first ordinary item.
                self.panes[0].cursor = self.panes[0].entries.iter().position(|e| !e.is_parent() && !e.drive).unwrap_or(0);
                self.advanced_rename();
                if let Some(DialogKind::AdvancedRename(form)) = self.original_dialogs.last_mut().map(|d| &mut d.kind) {
                    form.editor.text = "new_".into();
                    form.rules.push(form.editor.clone());
                    form.selected_rule = Some(0);
                    if kind == "advrename-delete" {
                        form.editor.action = "Delete".into();
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
            "credentials" => self.credentials_dialog("Network drive credentials", |_, _| {}),
            // C# remote window (not connected): for checking the skin images of the upload/download buttons in the middle.
            "remote" => self.open_remote_window(None, None),
            "search" => self.search_command(),
            "advselect" => self.advanced_select(false),
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

fn save_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}
