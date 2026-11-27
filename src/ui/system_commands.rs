//! Small system commands of C# MainWindow.CoreFeatures / ParityFeatures:
//! OpenAllSelectedFilesAsync (original FUN_004459d0), opening archive items in OpenItemAsync, CreateOriginalShortcuts (original FUN_00441af4),
//! PasteShortcutAsync (original 0x4340d4), GoToSystemFolderAsync/GoToDownloadsAsync, ShowProperties, ShowFileInformation
//! (FileInformationService), SelectThemeAsync/ApplyTheme, SelectLanguageAsync, ShowAbout, OpenNexusFileHelp, OpenProgramHomepage.
use super::*;
use ndr::msgbox::{MB_ICONEXCLAMATION, MB_OK};

/// Original skins (skin\ folder names): black, classic.
use ndr::styles::THEMES;

/// NDR base palette used where the skin does not paint. (app background, pane background, raised cell, border, text, faint text, accent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ThemePalette {
    pub app: [u8; 3],
    pub panel: [u8; 3],
    pub raised: [u8; 3],
    pub border: [u8; 3],
    pub text: [u8; 3],
    pub muted: [u8; 3],
    pub accent: [u8; 3],
}

impl ThemePalette {
    /// A dark pane background uses the egui dark visuals.
    pub fn dark(&self) -> bool {
        let [r, g, b] = self.panel;
        (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000 < 128
    }

    pub fn egui_theme(&self) -> egui::Theme {
        if self.dark() { egui::Theme::Dark } else { egui::Theme::Light }
    }

    pub fn apply_window_caption(&self, ctx: &egui::Context, hwnd: isize) {
        if hwnd != 0 && !native::set_window_caption_colors(hwnd, self.app, self.text) {
            // Older Windows builds reject explicit DWM colors. Use the
            // toolkit's best-effort native light/dark theme, not a custom frame.
            let theme = if self.dark() { egui::SystemTheme::Dark } else { egui::SystemTheme::Light };
            ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(theme));
        }
    }
}

/// egui base colors per skin (places the skin does not paint, such as dialogs). Unknown names are black.
pub(super) fn theme_palette(theme: &str) -> ThemePalette {
    match ndr::styles::canonical_theme_name(theme) {
        "Classic" => ThemePalette { app: [0xEB, 0xEB, 0xED], panel: [0xEF, 0xEF, 0xF0], raised: [0xFB, 0xFB, 0xFC], border: [0xAD, 0xAF, 0xB3], text: [0x19, 0x1A, 0x1B], muted: [0x56, 0x59, 0x5E], accent: [0x24, 0x5F, 0xAD] },
        "GitHub" => ThemePalette { app: [0x21, 0x26, 0x2D], panel: [0x16, 0x1B, 0x22], raised: [0x30, 0x36, 0x3D], border: [0x3D, 0x44, 0x4D], text: [0xE6, 0xED, 0xF3], muted: [0xA0, 0xAB, 0xB7], accent: [0x58, 0xA6, 0xFF] },
        _ => ThemePalette { app: [0x2B, 0x2C, 0x2E], panel: [0x19, 0x1A, 0x1C], raised: [0x25, 0x26, 0x29], border: [0x49, 0x4A, 0x4D], text: [0xC5, 0xC6, 0xC8], muted: [0xA1, 0xA2, 0xA5], accent: [0x53, 0x8B, 0xE5] },
    }
}

/// Moves the C# ApplyTheme color table onto the egui base (skins and color tables are applied over it).
pub(super) fn palette_visuals(palette: &ThemePalette) -> egui::Visuals {
    let c = |[r, g, b]: [u8; 3]| Color32::from_rgb(r, g, b);
    let mut visuals = if palette.dark() { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = c(palette.panel);
    visuals.window_fill = c(palette.raised);
    visuals.extreme_bg_color = c(palette.panel);
    visuals.faint_bg_color = c(palette.raised);
    visuals.override_text_color = Some(c(palette.text));
    visuals.selection.bg_fill = c(palette.accent);
    visuals.hyperlink_color = c(palette.accent);
    visuals.widgets.noninteractive.bg_stroke.color = c(palette.border);
    visuals.widgets.noninteractive.fg_stroke.color = c(palette.muted);
    visuals.window_stroke.color = c(palette.border);
    visuals
}

/// Item states when a menu opens: C# UpdateArchiveMenuItems (archive menu) + original pmEditPopup (0x43727c) and pmFolderPopup (0x437210).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) struct ArchiveMenuState {
    pub selected: bool,
    pub current: bool,
    /// Original pmEditPopup: miEditPaste.Enabled = IsClipboardFormatAvailable(CF_HDROP).
    pub paste: bool,
    /// Original pmEditPopup: miRestoreLastSel.Enabled = saved selection count > 0.
    pub restore: bool,
    /// Original pmFolderPopup: miToLastDest.Enabled = there is a last destination.
    pub last_destination: bool,
    /// Original pmFolderPopup (0x437210): miHistoryBack (+0x4ac) / miHistoryForward (+0x4a8).Enabled = history has back/forward entries.
    pub history_back: bool,
    pub history_forward: bool,
    /// Original pmEditPopup (0x43727c): miFindNext (+0x4cc).Enabled = last find text (+0xd0c) length > 0.
    pub find_next: bool,
    /// Original miViewFileInfoBarClick (0x434bc4): miViewExtensionInfo (+0x7e0).Enabled = the file info bar is visible (+0x58).
    pub file_info_bar: bool,
    /// Original pmConnectPopup (0x43ee78): the current pane is in FTP mode (connected).
    pub ftp_active: bool,
    /// One of the two panes is in FTP mode.
    pub ftp_any: bool,
    /// The current pane is inside an archive (original pane mode 1).
    pub in_archive: bool,
    /// Skin theme (original look): uses the original pmArchivePopup rules.
    pub original: bool,
}

impl ArchiveMenuState {
    /// (visible, enabled). extractCurrent/extractAuto are enabled only with a chosen archive (C# version). The skin theme, like the original pmArchivePopup (0x43a4e4),
    /// enables miArcExtractToCurr (+0x5c0), miArcExtractToFolder (+0x7bc) and miArcExtractAuto (+0x72c) unless the pane mode is 1 (inside an archive).
    /// archiveInstall only with a chosen archive,
    /// archiveProperties visible and enabled only with a chosen or current archive. Paste, restore last selection and last destination are
    /// enabled only when there is something to use, like the original. The rest stay as is.
    pub fn item(&self, command: &str) -> (bool, bool) {
        match command {
            "extractCurrent" | "extractAuto" | "extractNewFolder" if self.original => (true, !self.in_archive),
            "extractCurrent" | "extractAuto" => (true, self.selected),
            "archiveInstall" => (self.selected, self.selected),
            "archiveProperties" => {
                let available = self.selected || self.current;
                (available, available)
            }
            "paste" => (true, self.paste),
            "restoreSelection" => (true, self.restore),
            "goLastDestination" => (true, self.last_destination),
            "historyBack" => (true, self.history_back),
            "historyForward" => (true, self.history_forward),
            "findNext" => (true, self.find_next),
            "toggleExtensionInfoBar" => (true, self.file_info_bar),
            "ftpDisconnect" | "ftpDownload" => (true, self.ftp_active),
            "ftpUpload" => (true, self.ftp_any),
            _ => (true, true),
        }
    }
}

impl App {
    /// C# UpdateArchiveMenuItems: hasSelectedArchive = an archive among the files chosen on a local pane,
    /// hasCurrentArchive = inside an archive that exists.
    pub(super) fn archive_menu_state(&self) -> ArchiveMenuState {
        let i = self.active;
        let selected = self.panes[i].archive.is_none() && self.target_entries(i).iter().any(|e| !e.is_dir && ndr::archive::archive_format(&e.path).is_some());
        let current = self.panes[i].archive.as_ref().is_some_and(|l| l.file.is_file());
        // Rust also pastes archive items through the NDR internal clipboard, so those count as pasteable too.
        let paste = self.vault_clipboard.is_some() || self.clipboard.is_some() || native::clipboard_has_files();
        let restore = !self.last_selection[i].is_empty();
        let last_destination = !self.settings.text("LastDestination", "").trim().is_empty();
        let ftp_active = self.panes[i].remote.is_some();
        let ftp_any = self.panes.iter().any(|p| p.remote.is_some());
        let (history_back, history_forward) = (!self.panes[i].back.is_empty(), !self.panes[i].forward.is_empty());
        let find_next = !self.last_find.is_empty();
        let file_info_bar = self.settings.flag("ShowFileInfoBar", true);
        ArchiveMenuState { selected, current, paste, restore, last_destination, history_back, history_forward, find_next, file_info_bar, ftp_active, ftp_any, in_archive: self.panes[i].archive.is_some(), original: false }
    }

    /// Check marks of the View menu (Checked of original pmViewPopup 0x436fb8, C# AddToggleMenuItem): None is an ordinary item.
    pub(super) fn menu_checked(&self, command: &str) -> Option<bool> {
        let view = self.settings.text(if self.active == 0 { "LeftViewStyle" } else { "RightViewStyle" }, "Auto");
        Some(match command {
            "toggleHidden" => self.panes[self.active].show_hidden,
            "toggleDual" => self.dual,
            "viewAuto" | "viewDetails" | "viewNames" | "viewThumbnails" => view.eq_ignore_ascii_case(&command[4..]),
            "toggleVerticalBar" => self.settings.flag("ShowVerticalBar", true),
            "toggleToolBar" => self.settings.flag("ShowToolBar", true),
            "toggleResults" => self.show_results,
            "toggleFolders" => self.settings.flag(if self.active == 0 { "ShowLeftFolderTree" } else { "ShowRightFolderTree" }, false),
            "toggleFunctionBar" => self.settings.flag("ShowFunctionBar", true),
            "toggleDriveButtons" => self.settings.flag("ShowDriveButtons", false),
            "toggleFolderInfoBar" => self.settings.flag("ShowFolderDiskInfoBar", true),
            "toggleFileInfoBar" => self.settings.flag("ShowFileInfoBar", true),
            "toggleExtensionInfoBar" => self.settings.flag("ShowExtensionInfoBar", false),
            _ => return None,
        })
    }

    /// C# ApplySingleInstanceModeAsync(enabled): success when already in that state. Turning on takes the user/profile mutex and starts receiving navigation requests
    /// (fails when another NDR already holds it, sending it nothing). Turning off releases it.
    pub(super) fn apply_single_instance(&mut self, enabled: bool) -> bool {
        if enabled == self.instance_guard.is_some() {
            return true;
        }
        if !enabled {
            self.instance_guard = None;
            self.navigation = None;
            return true;
        }
        match ndr::lifecycle::InstanceGuard::try_acquire(&self.profile) {
            Ok(Some((guard, requests))) => {
                self.instance_guard = Some(guard);
                self.navigation = Some(requests);
                true
            }
            _ => false,
        }
    }

    /// C# OpenAllSelectedFilesAsync (original FUN_004459d0): for each collected item (FUN_00485bbc), records .exe/.lnk in the run history and
    /// ShellExecute (working folder = current folder). The shell reports failures itself. Inside an archive (approved fix) only files are extracted temporarily and opened.
    pub(super) fn open_all_command(&mut self) {
        let i = self.active;
        let items = self.target_entries(i);
        if items.is_empty() {
            return;
        }
        if let Some(location) = self.panes[i].archive.clone() {
            let keys = items.iter().filter(|e| !e.is_dir).map(|e| (e.name.clone(), e.path.to_string_lossy().into_owned())).collect();
            return self.spawn_archive_open(i, location, keys, false);
        }
        let directory = self.command_directory(i);
        let mut rows = self.settings.strings("ExecutableHistory");
        for item in &items {
            let actual = if self.panes[i].vault.is_some() {
                directory.join(&item.name)
            } else { item.path.clone() };
            ndr::command_line::remember_opened_executable(&mut rows, &actual.to_string_lossy());
            let _ = (self.shell_runner)(&actual, "", &directory);
        }
        self.settings.set_strings("ExecutableHistory", rows);
    }

    /// Archive pane branch of C# OpenItemAsync: extracts the item into %TEMP%\NDR\...; if the result is a readable archive, goes inside it,
    /// otherwise opens it with the shell.
    pub(super) fn open_archive_entry(&mut self, pane: usize, entry: &Entry) {
        let Some(location) = self.panes[pane].archive.clone() else { return };
        let key = entry.path.to_string_lossy().into_owned();
        self.spawn_archive_open(pane, location, vec![(entry.name.clone(), key)], true);
    }

    fn spawn_archive_open(&mut self, pane: usize, location: ArchiveLocation, keys: Vec<(String, String)>, navigate_archives: bool) {
        if keys.is_empty() {
            return;
        }
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            let mut password = (!location.password.is_empty()).then(|| location.password.clone());
            let mut opened = Vec::new();
            let mut failures = Vec::new();
            // C# ExtractExternalWithPasswordPromptAsync: on a password error asks for the password and extracts again. Cancel skips that item.
            let ask = tx.clone();
            let mut passwords = |archive: &Path| {
                let (reply, answer) = mpsc::channel();
                ask.send(AsyncResult::ArchivePassword { archive: archive.to_path_buf(), reply }).ok()?;
                answer.recv().ok().flatten()
            };
            for (name, key) in keys {
                match ndr::archive::extract_entry_to_temp_with_passwords(&location.file, password.as_deref(), &key, &cancel, &mut passwords) {
                    Ok((path, used)) => {
                        password = used;
                        opened.push(path);
                    }
                    Err(error) if error == "Archive password request cancelled" => {}
                    Err(error) => failures.push(format!("{name}: {error}")),
                }
            }
            let _ = tx.send(AsyncResult::ArchiveEntriesOpened { pane, archive: location.file.clone(), password, opened, failures, navigate_archives });
        });
    }

    pub(super) fn archive_entries_opened(&mut self, pane: usize, archive: PathBuf, password: Option<String>, opened: Vec<PathBuf>, failures: Vec<String>, navigate_archives: bool) {
        // A password found by asking is kept in that pane's archive location (no asking again on the next open or extract).
        if let (Some(location), Some(password)) = (self.panes[pane].archive.as_mut(), password) {
            if location.file == archive {
                location.password = password;
            }
        }
        for path in opened {
            // C# OpenItemAsync: goes inside formats that CanReadArchiveInternally or the bundled 7-Zip (GetExternalArchiveListingProgram, BundledSevenZipExtractFormats)
            // can read. NDR reads with the bundled 7-Zip (UzuCore 7zip+, including ALZ and EGG), so only ACE, which 7-Zip cannot read, is excluded.
            // ACE is extracted next to a temp folder with an external archiver like C# TryGetExternalArchiveProgram, then that folder opens (the C# error text without the program).
            let is_archive = ndr::archive::archive_format(&path).is_some();
            if navigate_archives && is_archive && !ndr::archive::bundled_can_browse(&path) {
                self.open_with_external_archive_program(pane, path, false);
                continue;
            }
            let archive = is_archive;
            if navigate_archives && archive {
                self.panes[pane].archive = Some(ArchiveLocation { file: path, prefix: String::new(), password: String::new() });
                self.refresh(pane);
                continue;
            }
            let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let _ = (self.shell_runner)(&path, "", &directory);
        }
        if !failures.is_empty() {
            let mut details = failures.iter().take(10).cloned().collect::<Vec<_>>().join("\r\n");
            if failures.len() > 10 {
                let more = self.catalog("NDR.UI_OPEN_SELECTED_MORE", "…외 %d개.", "…and %d more.");
                details.push_str("\r\n");
                details.push_str(&ndr::markup::substitute(&more, &[&(failures.len() - 10).to_string()]));
            }
            let title = self.catalog("NDR.UI_OPEN_SELECTED_TITLE", "선택한 파일 열기", "Open selected files");
            self.system_box(&title, &details, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
    }

    /// Creates desktop shortcuts and reports creation errors instead of failing silently.
    pub(super) fn shortcut_command(&mut self) {
        let i = self.active;
        if self.panes[i].archive.is_some() {
            return self.warn_operation(&self.catalog("NDR.UI_PAIR_EF84C1F650A1D817", "압축 파일 안의 항목은 바로 가기로 만들 수 없습니다.", "Items inside an archive cannot be turned into shortcuts."));
        }
        let items = self.target_entries(i);
        if items.is_empty() {
            return;
        }
        let Some(folder) = (self.system.shortcut_folder)() else {
            return self.warn_operation(&self.catalog("NDR.UI_PAIR_8E4D083EA93AAFB2", "바탕화면 폴더를 찾을 수 없습니다.", "The shortcut destination folder could not be found."));
        };
        let mut succeeded = 0usize;
        let mut failures = Vec::new();
        for item in &items {
            match (self.system.create_shortcut)(&item.path, &folder) {
                Ok(()) => succeeded += 1,
                Err(error) => failures.push(format!("{}: {error}", item.name)),
            }
        }
        if !failures.is_empty() {
            self.warning_notice_operation(
                self.catalog("NDR.OP_SHORTCUTS_PARTIAL", "바탕화면 바로가기 일부 실패", "Some desktop shortcuts failed"),
                super::file_commands::operation_format(&self.language_pack, "NDR.OP_SHORTCUTS_PARTIAL_COUNTS", "{0} succeeded · {1} failed", &[&succeeded, &failures.len()]),
            );
            let detail = failures.iter().take(8).cloned().collect::<Vec<_>>().join("\n");
            let message = super::file_commands::operation_format(&self.language_pack, "NDR.OP_SHORTCUTS_PARTIAL_DETAILS", "Shortcuts: {0} succeeded, {1} failed\nDestination: {2}\n{3}", &[&succeeded, &failures.len(), &folder.display(), &detail]);
            self.system_box("NDR", &message, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
            // On failure do not lose the marked selection: users may retry.
            return;
        }
        self.panes[i].selected.clear();
        self.completion_notice_operation(
            self.catalog("NDR.OP_SHORTCUTS_DONE", "바탕화면 바로가기 생성 완료", "Desktop shortcuts created"),
            super::file_commands::operation_format(&self.language_pack, "NDR.OP_SHORTCUTS_DONE_COUNT", "{0} shortcut(s) created", &[&succeeded]),
        );
    }

    /// C# PasteShortcutAsync (original miPasteAsShortcutClick 0x4340d4): on a local pane with an existing current folder, creates "name.ext.lnk" in clipboard file list
    /// order (no target check, silently replaced), then relists. No notice.
    pub(super) fn paste_shortcut_command(&mut self) {
        let i = self.active;
        let folder = self.command_directory(i);
        if self.panes[i].archive.is_some() || !folder.is_dir() {
            return;
        }
        let targets = (self.system.clipboard_files)();
        if targets.is_empty() {
            return;
        }
        let mut created = 0usize;
        let mut failed = 0usize;
        for target in &targets {
            match (self.system.create_shortcut)(target, &folder) {
                Ok(()) => created += 1,
                Err(_) => failed += 1,
            }
        }
        self.refresh(i);
        if failed > 0 {
            self.warning_notice("바로가기 일부 생성 실패", "Some shortcuts could not be created",
                format!("{created} succeeded · {failed} failed"));
        } else if created > 0 {
            self.completion_notice("바로가기 생성 완료", "Shortcuts created",
                if self.language == "ko" { format!("{created}개 바로가기 생성") }
                else { format!("{created} shortcut(s) created") });
        }
    }

    /// C# GoToSystemFolderAsync/GoToDownloadsAsync: documents, pictures, music, videos and downloads use the SpecialFolderPaths setting (ExpandSetting)
    /// first, otherwise the Windows folder. Moves only when the folder exists. temp = Path.GetTempPath().
    pub(super) fn special_folder_command(&mut self, id: &str) {
        if let Some(path) = self.special_folder_path(id) {
            self.navigate(self.active, path, true);
        }
    }

    /// Special folder paths for special_folder_command and the folder tree window (FUN_0040db88). Only existing folders.
    pub(super) fn special_folder_path(&mut self, id: &str) -> Option<PathBuf> {
        let key = match id {
            "documents" => "Documents",
            "pictures" => "Pictures",
            "music" => "Music",
            "videos" => "Videos",
            "downloads" => "Downloads",
            _ => "",
        };
        let configured = (!key.is_empty())
            .then(|| self.settings.values.get("SpecialFolderPaths").and_then(|v| v.get(key)).and_then(|v| v.as_str()).map(str::to_owned))
            .flatten();
        let path = match (configured, id) {
            (Some(value), _) => PathBuf::from(ndr::command_line::expand_setting(&value)),
            // Original miToTempClick (0x4330b0): GetTempPathW -> GetLongPathNameW; stops if either fails.
            (None, "temp") => (self.system.temp_folder)()?,
            (None, "downloads") => PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Downloads"),
            (None, other) => native::special_folder(other).unwrap_or_default(),
        };
        (!path.as_os_str().is_empty() && path.is_dir()).then_some(path)
    }

    /// C# ShowProperties: the shell properties window of the first chosen item, otherwise of the current folder.
    pub(super) fn properties_command(&mut self) {
        let i = self.active;
        // Original miPropertiesClick (0x432208): when there is a cursor item and the location is not FTP or an archive. Without marks and with the cursor on a drive (kind 4),
        // that drive's properties; otherwise the multi-file properties window of all collected items (marks, otherwise the cursor) (skin theme).
        let pane = &self.panes[i];
        if pane.archive.is_some() || pane.remote.is_some() {
            return;
        }
        let Some(cursor) = pane.entries.get(pane.cursor).cloned() else { return };
        if pane.selected.is_empty() && cursor.drive {
            if let Err(error) = (self.system.run_verb)(&cursor.path, "properties") {
                self.status = error;
            }
            return;
        }
        let paths: Vec<PathBuf> = self.command_targets(i);
        if !paths.is_empty() {
            if let Err(error) = (self.system.properties)(&paths) {
                self.status = error;
            }
        }
    }

    /// Saves the theme and applies it at once (end of C# SelectThemeAsync, original skin menu FUN_0043e510).
    pub(super) fn apply_theme(&mut self, theme: &str) {
        let theme = ndr::styles::canonical_theme_name(theme);
        self.settings.set_text("Theme", theme);
        self.dark = theme_palette(theme).dark();
        for index in 0..self.panes.len() { self.compute_view(index); }
        self.preferences_dirty = true;
        self.save();
    }

    /// Original skin menu (FUN_0043e580): the folders under skin\ (without "." and "..") in order. Only theme names NDR knows (case-insensitive).
    pub(super) fn skin_names(&self) -> Vec<&'static str> {
        let skins = self.resources.join("Skins");
        THEMES.into_iter().filter(|theme| {
            ndr::styles::theme_skin_directory(theme).is_some_and(|folder| skins.join(folder).is_dir())
        }).collect()
    }

    /// Saves the language and applies it at once (end of C# SelectLanguageAsync, original language menu FUN_0044193c).
    pub(super) fn apply_language(&mut self, code: &str) {
        self.refresh_language(code);
        self.settings.set_text("Language", &self.language.clone());
        self.save();
    }

    /// Display cells can outlive a catalog change, even when the directory
    /// does not change. Rebuild them without navigating or dropping selection.
    pub(super) fn refresh_language(&mut self, code: &str) {
        let language = compat::Language::load(&self.resources.join("Languages"), code);
        let font_changed = super::preferred_cjk_font(&self.language) != super::preferred_cjk_font(&language.code);
        self.language = language.code.clone();
        self.language_pack = language;
        if font_changed {
            self.preferences_dirty = true;
            self.egui_ctx.request_repaint();
        }
        for index in 0..self.panes.len() {
            self.compute_view(index);
        }
    }

    /// Original miTextViewerClick/miTextEditorClick -> FUN_00430304: only when the cursor item is a file; inside an archive it is extracted to a temp folder first;
    /// passes "\"path\"" to TextViewerPath/TextEditorPath (notepad.exe when empty) via xilesShell.ExecuteFile.
    pub(super) fn text_command(&mut self, edit: bool) {
        let i = self.active;
        // Original miTextViewerClick/miTextEditorClick -> FUN_00430304: only when the cursor item is a file (kind 3), on local and archive panes.
        let pane = &self.panes[i];
        if pane.remote.is_some() {
            return;
        }
        let Some(entry) = pane.entries.get(pane.cursor).filter(|e| !e.is_dir && !e.drive).cloned() else { return };
        // Vault files have no plaintext on-disk path. Route both View and Edit
        // commands through the same WinFsp-backed external editor as a double-click.
        if self.panes[i].vault.is_some() {
            self.enter_vault_item(i, &entry);
            return;
        }
        match self.panes[i].archive.clone() {
            None => self.text_source_ready(Ok(entry.path), edit),
            Some(location) => {
                let tx = self.async_tx.clone();
                let key = entry.path.to_string_lossy().into_owned();
                thread::spawn(move || {
                    let password = (!location.password.is_empty()).then_some(location.password.as_str());
                    let path = ndr::archive::extract_entry_to_temp(&location.file, password, &key, &AtomicBool::new(false));
                    let _ = tx.send(AsyncResult::TextSource { path, edit });
                });
            }
        }
    }

    pub(super) fn text_source_ready(&mut self, path: Result<PathBuf, String>, edit: bool) {
        let path = match path {
            Ok(path) => path,
            Err(error) => return self.system_box("NDR", &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        };
        let mut program = ndr::command_line::expand_setting(&self.settings.text(if edit { "TextEditorPath" } else { "TextViewerPath" }, ""));
        // Original FUN_00430304: when the setting is empty, "\"path\"" goes to "notepad.exe".
        if program.trim().is_empty() {
            program = "notepad.exe".into();
        }
        let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if let Err(error) = (self.shell_runner)(Path::new(&program), &native::quote_arg(&path.to_string_lossy()), &directory) {
            self.status = error;
        }
    }

    /// ShowAbout keeps the localized system-message layout with the NDR artwork.
    pub(super) fn about_command(&mut self) {
        let build = ndr::update::build_label(ndr::update::current_build())
            .unwrap_or_else(|| self.catalog("NDR.UPD_LOCAL_BUILD", "로컬 빌드", "Local build"));
        let text = format!(
            "NDR {build}\n{}: {}\nWindows 10/11 · 64-bit\n{}: {}",
            self.catalog("NDR.UI_ABOUT_APP_VERSION", "앱 버전", "App version"),
            env!("CARGO_PKG_VERSION"),
            self.catalog("NDR.UI_ABOUT_SETTINGS_LOCATION", "사용자 설정", "User settings"),
            self.profile.display()
        );
        let caption = self.catalog("NDR.UI_ABOUT_TITLE", "NDR 정보", "About NDR");
        self.program_info_box(&caption, &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_menu_items_follow_csharp_update_archive_menu_items() {
        let none = ArchiveMenuState::default();
        assert_eq!(none.item("extractCurrent"), (true, false));
        assert_eq!(none.item("extractAuto"), (true, false));
        assert_eq!(none.item("archiveInstall"), (false, false));
        assert_eq!(none.item("archiveProperties"), (false, false));
        assert_eq!(none.item("extractSelected"), (true, true));
        // original pmEditPopup / pmFolderPopup
        assert_eq!(none.item("paste"), (true, false));
        assert_eq!(none.item("restoreSelection"), (true, false));
        assert_eq!(none.item("goLastDestination"), (true, false));
        let ready = ArchiveMenuState { paste: true, restore: true, last_destination: true, ..Default::default() };
        assert_eq!((ready.item("paste"), ready.item("restoreSelection"), ready.item("goLastDestination")), ((true, true), (true, true), (true, true)));
        // Original pmFolderPopup (0x437210) and pmEditPopup (0x43727c): back/forward are gray without history, find next is gray without find text.
        assert_eq!((none.item("historyBack"), none.item("historyForward"), none.item("findNext")), ((true, false), (true, false), (true, false)));
        let history = ArchiveMenuState { history_back: true, history_forward: true, find_next: true, ..Default::default() };
        assert_eq!((history.item("historyBack"), history.item("historyForward"), history.item("findNext")), ((true, true), (true, true), (true, true)));
        // Original miViewFileInfoBarClick (0x434bc4): when the file info bar is off, the extension info item is gray too.
        assert_eq!(none.item("toggleExtensionInfoBar"), (true, false));
        assert_eq!(ArchiveMenuState { file_info_bar: true, ..Default::default() }.item("toggleExtensionInfoBar"), (true, true));
        let inside = ArchiveMenuState { current: true, ..Default::default() };
        assert_eq!(inside.item("archiveProperties"), (true, true));
        assert_eq!(inside.item("archiveInstall"), (false, false));
        assert_eq!(inside.item("extractAuto"), (true, false));
        let picked = ArchiveMenuState { selected: true, ..Default::default() };
        assert_eq!(picked.item("extractCurrent"), (true, true));
        assert_eq!(picked.item("archiveInstall"), (true, true));
        assert_eq!(picked.item("archiveProperties"), (true, true));
        // Original pmArchivePopup (skin theme): enabled outside archives regardless of the selection.
        let original = ArchiveMenuState { original: true, ..Default::default() };
        assert_eq!((original.item("extractCurrent"), original.item("extractAuto")), ((true, true), (true, true)));
        let original_inside = ArchiveMenuState { original: true, in_archive: true, current: true, ..Default::default() };
        assert_eq!((original_inside.item("extractCurrent"), original_inside.item("extractAuto")), ((true, false), (true, false)));
    }

    #[test]
    fn skin_palettes() {
        assert_eq!(theme_palette("Classic").app, [0xEB, 0xEB, 0xED]);
        assert_eq!(theme_palette("Gray"), theme_palette("Black"));
        assert!(theme_palette("Gray").dark());
        assert_eq!(theme_palette("Black").text, [0xC5, 0xC6, 0xC8]);
        assert!(theme_palette("Black").dark());
        // Unknown names are black.
        assert_eq!(theme_palette("Dark"), theme_palette("Black"));
    }

}
