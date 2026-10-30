//! 시험용 화면 찍기(`--smoke-seconds N --shot 종류 파일.png`): 화면 밖 시작 확인 창에 원본 창 견본을 열고 그 화면을 PNG로 저장한다.
//! 원본 DFM 자리로 옮긴 창들이 실제로 그렇게 그려지는지 사람 눈으로 확인하는 데만 쓴다. 사용자 파일·설정은 건드리지 않는다
//! (견본 파일은 --profile 격리 폴더의 shot 폴더에만 만든다).
use super::original_dialogs::DialogKind;
use super::*;

/// 견본을 여는 그림 번호와 찍는 그림 번호.
const OPEN_FRAME: u64 = 3;
const SHOT_FRAME: u64 = 20;

/// "ftplog" 견본의 가짜 FTP(목록 두 줄, 다른 동작은 거절).
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
    fn download(&mut self, _: &str, _: &Path, _: &AtomicBool, _: &mut dyn FnMut(&str, &str) -> ConflictChoice, _: &mut dyn FnMut(ndr::remote::TransferProgress)) -> Result<(), String> {
        Err("sample".into())
    }
    fn upload(&mut self, _: &Path, _: &str, _: &AtomicBool, _: &mut dyn FnMut(&str, &str) -> ConflictChoice, _: &mut dyn FnMut(ndr::remote::TransferProgress)) -> Result<(), String> {
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
        }
        if self.frames == SHOT_FRAME {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        // 따로 뜨는 툴팁 창(panes::show_tip_window) 확인: 메인 창 오른쪽 끝을 넘는 사각을 그리고, 만들어진 창의 화면 자리를 보고서에 남긴다.
        if kind == "tipwindow" && self.frames >= OPEN_FRAME {
            let screen = ctx.content_rect();
            let rect = egui::Rect::from_min_size(egui::pos2(screen.right() - 60.0, screen.top() + 80.0), egui::vec2(180.0, 22.0));
            let galley = ctx.fonts_mut(|f| f.layout_no_wrap("a-very-long-file-name-for-the-tooltip.txt".into(), egui::FontId::proportional(12.0), egui::Color32::BLACK));
            let paint = |painter: &egui::Painter, rect: egui::Rect| {
                painter.rect(rect, 0.0, egui::Color32::from_rgb(255, 255, 225), egui::Stroke::new(1.0, egui::Color32::BLACK), egui::StrokeKind::Inside);
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
            // 원본 TfSelectDest 배치 눈 확인용: 최근 폴더 하나, 작업 폴더 하나.
            "destination" => {
                self.remember_destination(&folder.join("a"));
                let mut work = vec![folder.join("b").to_string_lossy().into_owned()];
                work.resize(5, String::new());
                self.settings.set_strings("WorkingFolders", work);
                let (initial, current) = (self.panes[1].path.clone(), self.panes[0].path.clone());
                self.select_destination(super::destination_dialog::DestinationPurpose::Copy, initial, Some(current), |_, _| {});
            }
            // 원본 메뉴 줄 오른쪽 끝의 빠른 찾기 글(tbMainMenuAdvancedCustomDraw) 눈 확인용.
            "quicksearch" => {
                let active = self.active;
                self.typed_search[active].query = "read".into();
                self.typed_search[active].cursor = Some(self.panes[active].cursor);
            }
            // 패널 FTP 모드의 기록 칸(원본 FUN_0048d7ac) 눈 확인용: 오른쪽 패널을 가짜 FTP 연결로.
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
            // 메뉴 줄 펼침(Alt+F와 같은 길): 스킨 메뉴 색·단추 칸 눈 확인용.
            "mainmenu" => self.menu_access = Some('f'),
            "popupmenu" => {
                // 스킨 메뉴 색(skin_menu_style) 눈 확인용: 즐겨찾기 메뉴(머리·구분선·꺼진 줄 포함)를 왼쪽 위에 연다.
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
            "cleanup" => {
                let mut form = super::cleanup_dialog::CleanupOptionsForm::new("tmp;$$$;bak;old".into(), &[]);
                form.user_defined = true;
                self.push_original_dialog(text(self, "formCleanup.Form.Caption"), DialogKind::CleanupOptions(Box::new(form)), |_, _, _| {});
            }
            "archive" => {
                self.active = 0;
                // ".."가 아닌 첫 항목(".."는 압축 대상이 아니다).
                self.panes[0].cursor = self.panes[0].entries.iter().position(|e| !e.is_parent() && !e.drive).unwrap_or(0);
                self.archive_command(super::archive_dialog::ArchiveKind::Create);
            }
            "progress" => {
                let (source, target) = (folder.join("a"), folder.join("b"));
                let info = super::file_operation::ProgressInfo { from: Some(source.to_string_lossy().into_owned()), to: Some(target.to_string_lossy().into_owned()), total_files: Some(12), total_bytes: Some(48 * 1024 * 1024), prefer_bytes: true, ..Default::default() };
                let file = source.join("document.txt");
                self.start_pausable_task("Copy".into(), info, move |events, cancel, _| {
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
                self.panes[0].cursor = 0;
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
            // C# 원격 창(연결하지 않음): 가운데 올리기·내려받기 단추의 스킨 그림 확인용.
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
                        FtpResultRow { upload: false, local: "C:\\down\\a.zip".into(), remote: "/pub/a.zip".into(), size: Some(4000), done: 1000, state: RowState::Running },
                        FtpResultRow { upload: false, local: "C:\\down\\b.txt".into(), remote: "/pub/b.txt".into(), size: Some(10), done: 0, state: RowState::Waiting },
                    ];
                }
                self.show_results = true;
            }
            "gridlines" | "rowlines" => {
                // 원본 ShowGridColumn·ShowGridRow·LineSpace 확인: 이름 보기(열 선·줄 선)와 자세히 보기(줄 선).
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
