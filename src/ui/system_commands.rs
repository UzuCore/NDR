//! C# MainWindow.CoreFeatures / ParityFeatures의 작은 시스템 명령들:
//! OpenAllSelectedFilesAsync(원본 FUN_004459d0), OpenItemAsync의 압축 항목 열기, CreateOriginalShortcuts(원본 FUN_00441af4),
//! PasteShortcutAsync(원본 0x4340d4), GoToSystemFolderAsync/GoToDownloadsAsync, ShowProperties, ShowFileInformation
//! (FileInformationService), SelectThemeAsync/ApplyTheme, SelectLanguageAsync, ShowAbout, OpenNexusFileHelp, OpenProgramHomepage.
use super::*;
use ndir::msgbox::{MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_OK};
use std::fs;

/// C# SelectThemeAsync의 선택지.
pub(super) const THEMES: [&str; 6] = ["Dark", "Light", "System", "Black", "Classic", "Gray"];

/// C# ApplyTheme의 색 표(스킨이 없을 때). (앱 배경, 패널 배경, 돋운 칸, 테두리, 글자, 흐린 글자, 강조).
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
    /// 패널 배경이 어두우면 egui 어두운 바탕을 쓴다.
    pub fn dark(&self) -> bool {
        let [r, g, b] = self.panel;
        (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000 < 128
    }
}

/// C# ApplyTheme: Light와 (Windows 밝은 테마일 때의) System, Classic, Gray, Black, 그 밖은 Dark.
pub(super) fn theme_palette(theme: &str, windows_light: bool) -> ThemePalette {
    let light = ThemePalette { app: [0xF3, 0xF6, 0xFB], panel: [0xFF, 0xFF, 0xFF], raised: [0xE9, 0xEE, 0xF5], border: [0xCF, 0xD8, 0xE3], text: [0x11, 0x18, 0x27], muted: [0x52, 0x62, 0x73], accent: [0x17, 0x69, 0xAA] };
    match theme {
        "Light" => light,
        "System" if windows_light => light,
        "Classic" => ThemePalette { app: [0xDE, 0xDE, 0xDE], panel: [0xF0, 0xF0, 0xF0], raised: [0xFF, 0xFF, 0xFF], border: [0xAA, 0xAA, 0xAA], text: [0, 0, 0], muted: [0x66, 0x66, 0x66], accent: [0x17, 0x69, 0xAA] },
        "Gray" => ThemePalette { app: [0x87, 0x87, 0x87], panel: [0xD1, 0xD7, 0xE2], raised: [0x96, 0x96, 0x96], border: [0x6F, 0x6F, 0x6F], text: [0, 0, 0], muted: [0x33, 0x33, 0x33], accent: [0x4C, 0x82, 0xDC] },
        "Black" => ThemePalette { app: [0, 0, 0], panel: [0, 0, 0], raised: [0x20, 0x20, 0x20], border: [0x30, 0x30, 0x30], text: [0xC0, 0xC0, 0xC0], muted: [0x85, 0x85, 0x85], accent: [0x4C, 0x82, 0xDC] },
        _ => ThemePalette { app: [0x0B, 0x10, 0x20], panel: [0x11, 0x18, 0x27], raised: [0x17, 0x22, 0x35], border: [0x26, 0x34, 0x49], text: [0xF1, 0xF5, 0xF9], muted: [0x92, 0xA3, 0xB8], accent: [0x4E, 0xA1, 0xFF] },
    }
}

/// C# ApplyTheme의 색 표를 egui 바탕에 옮긴다(스킨·색상표는 그 뒤에 덮는다).
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

/// 메뉴를 열 때의 항목 상태: C# UpdateArchiveMenuItems(압축 메뉴) + 원본 pmEditPopup(0x43727c)·pmFolderPopup(0x437210).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) struct ArchiveMenuState {
    pub selected: bool,
    pub current: bool,
    /// 원본 pmEditPopup: miEditPaste.Enabled = IsClipboardFormatAvailable(CF_HDROP).
    pub paste: bool,
    /// 원본 pmEditPopup: miRestoreLastSel.Enabled = 저장한 선택 수 > 0.
    pub restore: bool,
    /// 원본 pmFolderPopup: miToLastDest.Enabled = 마지막 대상이 있음.
    pub last_destination: bool,
    /// 원본 pmConnectPopup(0x43ee78): 지금 패널이 FTP 모드(연결됨).
    pub ftp_active: bool,
    /// 두 패널 가운데 하나가 FTP 모드.
    pub ftp_any: bool,
    /// 지금 패널이 압축 파일 안(원본 패널 모드 1).
    pub in_archive: bool,
    /// 스킨 테마(원본 모양): 원본 pmArchivePopup 규칙을 쓴다.
    pub original: bool,
}

impl ArchiveMenuState {
    /// (보임, 켜짐). extractCurrent/extractAuto는 고른 압축 파일이 있을 때만 켜짐(C# 판). 스킨 테마는 원본 pmArchivePopup(0x43a4e4)처럼
    /// miArcExtractToCurr(+0x5c0)·miArcExtractToFolder(+0x7bc)·miArcExtractAuto(+0x72c)를 패널 모드가 1(압축 안)이 아니면 켠다.
    /// archiveInstall은 고른 압축 파일이 있을 때만,
    /// archiveProperties는 고른 압축 파일이나 지금 압축 파일이 있을 때만 보이고 켜짐. 붙여넣기·마지막 선택 복구·마지막 대상 위치는
    /// 원본처럼 쓸 것이 있을 때만 켜짐. 나머지는 그대로.
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
            "ftpDisconnect" | "ftpDownload" => (true, self.ftp_active),
            "ftpUpload" => (true, self.ftp_any),
            _ => (true, true),
        }
    }
}

/// C# FileInformationService.Read의 줄(영어 이름표, 값). 이름표는 언어 파일 역검색(TranslateText)으로 바꿔 보인다.
pub(super) fn file_information_rows(entry: Option<&Entry>, name: &str, physical: Option<&Path>, display: &str, description: Option<&str>, unit: &str, thousands: bool, type_name: impl Fn(&str, bool) -> String) -> Vec<(&'static str, String)> {
    let mut is_dir = entry.is_none_or(|e| e.is_dir);
    let mut size = entry.filter(|e| !e.is_dir).map(|e| e.size);
    let mut modified = entry.and_then(|e| e.modified);
    let mut attributes = None;
    if let Some(path) = physical {
        if let Ok(meta) = fs::metadata(path) {
            is_dir = meta.is_dir();
            size = (!is_dir).then_some(meta.len());
            modified = meta.modified().ok().or(modified);
            attributes = native::attributes(path).ok().map(native::dotnet_attributes_text);
        }
    }
    let extension = ndir::parity::dotnet_extension(name).to_owned();
    let mut kind = type_name(&extension, is_dir);
    if kind.is_empty() {
        kind = if is_dir { "폴더".into() } else { "파일".into() };
    }
    let mut rows = vec![
        ("Name", name.to_owned()),
        ("File", kind),
        ("Ext", if extension.is_empty() { "—".into() } else { extension }),
        ("Location", display.to_owned()),
        ("Size", match size { Some(bytes) if !is_dir => ndir::parity::format_size(bytes, unit, thousands), _ => "—".into() }),
        ("Date modified", modified.map(ndir::listing::general_date_text).unwrap_or_else(|| "—".into())),
    ];
    if let Some(attributes) = attributes.filter(|a| !a.trim().is_empty()) {
        rows.push(("Attr", attributes));
    }
    if let Some(description) = description.filter(|d| !d.trim().is_empty()) {
        rows.push(("Description", description.to_owned()));
    }
    rows
}

impl App {
    /// C# UpdateArchiveMenuItems: hasSelectedArchive = 로컬 패널에서 고른 파일 중 압축 파일이 있음,
    /// hasCurrentArchive = 지금 압축 파일 안이고 그 파일이 있음.
    pub(super) fn archive_menu_state(&self) -> ArchiveMenuState {
        let i = self.active;
        let selected = self.panes[i].archive.is_none() && self.target_entries(i).iter().any(|e| !e.is_dir && ndir::archive::archive_format(&e.path).is_some());
        let current = self.panes[i].archive.as_ref().is_some_and(|l| l.file.is_file());
        // Rust는 압축 안 항목을 NDIR 내부 클립보드로도 붙여넣으므로 그것도 붙여넣을 것으로 센다.
        let paste = self.clipboard.is_some() || native::clipboard_has_files();
        let restore = !self.last_selection[i].is_empty();
        let last_destination = !self.settings.text("LastDestination", "").trim().is_empty();
        let ftp_active = self.panes[i].remote.is_some();
        let ftp_any = self.panes.iter().any(|p| p.remote.is_some());
        ArchiveMenuState { selected, current, paste, restore, last_destination, ftp_active, ftp_any, in_archive: self.panes[i].archive.is_some(), original: false }
    }

    /// 보기 메뉴의 체크 표시(원본 pmViewPopup 0x436fb8의 Checked, C# AddToggleMenuItem): None이면 일반 항목.
    pub(super) fn menu_checked(&self, command: &str) -> Option<bool> {
        let view = self.settings.text(if self.active == 0 { "LeftViewStyle" } else { "RightViewStyle" }, "Auto");
        Some(match command {
            "toggleHidden" => self.panes[self.active].show_hidden,
            "toggleDual" => self.dual,
            "viewAuto" | "viewDetails" | "viewNames" | "viewThumbnails" => view.eq_ignore_ascii_case(&command[4..]),
            "toggleVerticalBar" => self.settings.flag("ShowVerticalBar", true),
            "toggleToolBar" => self.settings.flag("ShowToolBar", true),
            "toggleResults" => self.show_results,
            "toggleFolders" => self.show_tree,
            "toggleFunctionBar" => self.settings.flag("ShowFunctionBar", true),
            "toggleDriveButtons" => self.settings.flag("ShowDriveButtons", false),
            "toggleFolderInfoBar" => self.settings.flag("ShowFolderDiskInfoBar", true),
            "toggleFileInfoBar" => self.settings.flag("ShowFileInfoBar", true),
            "toggleExtensionInfoBar" => self.settings.flag("ShowExtensionInfoBar", false),
            _ => return None,
        })
    }

    /// C# ApplySingleInstanceModeAsync(enabled): 이미 그 상태면 성공. 켤 때는 사용자·프로필 뮤텍스를 잡고 이동 요청을 받기 시작한다
    /// (다른 NDIR이 이미 잡았으면 실패, 그쪽에 아무것도 보내지 않음). 끌 때는 놓는다.
    pub(super) fn apply_single_instance(&mut self, enabled: bool) -> bool {
        if enabled == self.instance_guard.is_some() {
            return true;
        }
        if !enabled {
            self.instance_guard = None;
            self.navigation = None;
            return true;
        }
        match ndir::lifecycle::InstanceGuard::try_acquire(&self.profile) {
            Ok(Some((guard, requests))) => {
                self.instance_guard = Some(guard);
                self.navigation = Some(requests);
                true
            }
            _ => false,
        }
    }

    /// C# OpenAllSelectedFilesAsync(원본 FUN_004459d0): 모은 항목(FUN_00485bbc)마다 .exe/.lnk는 실행 기록에 남기고
    /// ShellExecute(작업 폴더 = 지금 폴더). 실패는 셸이 직접 알린다. 압축 안이면(승인된 수정) 파일만 임시로 풀어 연다.
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
        let directory = self.panes[i].path.clone();
        let mut rows = self.settings.strings("ExecutableHistory");
        for item in &items {
            ndir::command_line::remember_opened_executable(&mut rows, &item.path.to_string_lossy());
            let _ = (self.shell_runner)(&item.path, "", &directory);
        }
        self.settings.set_strings("ExecutableHistory", rows);
    }

    /// C# OpenItemAsync의 압축 패널 분기: 항목을 %TEMP%\NDIR\…에 풀고, 풀린 것이 읽을 수 있는 압축 파일이면 그 안으로,
    /// 아니면 셸로 연다.
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
            // C# ExtractExternalWithPasswordPromptAsync: 암호 오류면 암호를 묻고 다시 푼다. 취소면 그 항목은 건너뛴다.
            let ask = tx.clone();
            let mut passwords = |archive: &Path| {
                let (reply, answer) = mpsc::channel();
                ask.send(AsyncResult::ArchivePassword { archive: archive.to_path_buf(), reply }).ok()?;
                answer.recv().ok().flatten()
            };
            for (name, key) in keys {
                match ndir::archive::extract_entry_to_temp_with_passwords(&location.file, password.as_deref(), &key, &cancel, &mut passwords) {
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
        // 물어서 맞힌 암호는 그 패널의 압축 위치에 남긴다(다음 열기·풀기에서 다시 묻지 않는다).
        if let (Some(location), Some(password)) = (self.panes[pane].archive.as_mut(), password) {
            if location.file == archive {
                location.password = password;
            }
        }
        for path in opened {
            // C# OpenItemAsync: CanReadArchiveInternally이거나 동봉 7-Zip(GetExternalArchiveListingProgram, BundledSevenZipExtractFormats)이
            // 읽는 형식이면 그 안으로. NDIR은 동봉 7-Zip(UzuCore 7zip+α, ALZ·EGG 포함)으로 읽으므로 7-Zip이 못 읽는 ACE만 뺀다.
            // ACE는 C# TryGetExternalArchiveProgram처럼 외부 압축 프로그램으로 임시 폴더 옆에 풀고 그 폴더로(프로그램이 없으면 C# 오류 글).
            let is_archive = ndir::archive::archive_format(&path).is_some();
            if navigate_archives && is_archive && !ndir::archive::bundled_can_browse(&path) {
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
                details.push_str(&format!("\r\n…and {} more.", failures.len() - 10));
            }
            self.system_box("Open selected files", &details, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
    }

    /// C# CreateOriginalShortcuts(원본 FUN_00441af4): 압축 안은 경고(승인된 수정). 모은 항목마다 "이름.확장자.lnk"를
    /// 바탕 화면(또는 빠른 실행) 폴더에, 폴더가 없으면 아무것도 하지 않는다. 끝나면 선택 해제, 알림 없음.
    pub(super) fn shortcut_command(&mut self, quick_launch: bool) {
        let i = self.active;
        if self.panes[i].archive.is_some() {
            return self.warn("압축 파일 안의 항목은 바로 가기로 만들 수 없습니다.", "Items inside an archive cannot be turned into shortcuts.");
        }
        let items = self.target_entries(i);
        if items.is_empty() {
            return;
        }
        let Some(folder) = (self.system.shortcut_folder)(quick_launch) else { return };
        for item in &items {
            let _ = (self.system.create_shortcut)(&item.path, &folder);
        }
        self.panes[i].selected.clear();
    }

    /// C# PasteShortcutAsync(원본 miPasteAsShortcutClick 0x4340d4): 로컬 패널이고 지금 폴더가 있을 때 클립보드 파일 목록
    /// 순서대로 "이름.확장자.lnk"(대상 검사 없음, 조용히 바꿈), 그 뒤 다시 나열. 알림 없음.
    pub(super) fn paste_shortcut_command(&mut self) {
        let i = self.active;
        let folder = self.panes[i].path.clone();
        if self.panes[i].archive.is_some() || !folder.is_dir() {
            return;
        }
        let targets = (self.system.clipboard_files)();
        if targets.is_empty() {
            return;
        }
        for target in &targets {
            let _ = (self.system.create_shortcut)(target, &folder);
        }
        self.refresh(i);
    }

    /// C# GoToSystemFolderAsync/GoToDownloadsAsync: 문서·사진·음악·동영상·다운로드는 SpecialFolderPaths 설정(ExpandSetting)을
    /// 먼저, 아니면 Windows 폴더. 폴더가 있을 때만 이동한다. temp = Path.GetTempPath().
    pub(super) fn special_folder_command(&mut self, id: &str) {
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
            (Some(value), _) => PathBuf::from(ndir::command_line::expand_setting(&value)),
            // 원본 miToTempClick(0x4330b0): GetTempPathW → GetLongPathNameW, 둘 중 하나라도 실패하면 그만(스킨 테마).
            (None, "temp") if self.skin_active() => match (self.system.temp_folder)() {
                Some(path) => path,
                None => return,
            },
            (None, "temp") => std::env::temp_dir(),
            (None, "downloads") => PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Downloads"),
            (None, other) => native::special_folder(other).unwrap_or_default(),
        };
        if !path.as_os_str().is_empty() && path.is_dir() {
            self.navigate(self.active, path, true);
        }
    }

    /// C# ShowProperties: 고른 첫 항목, 없으면 지금 폴더의 셸 속성 창.
    pub(super) fn properties_command(&mut self) {
        let i = self.active;
        // 원본 miPropertiesClick(0x432208): 커서 항목이 있고 FTP·압축 위치가 아닐 때. 표시가 없고 커서가 드라이브(종류 4)면
        // 그 드라이브의 속성, 아니면 모은 항목(표시, 없으면 커서) 전부의 여러 파일 속성 창(스킨 테마).
        if self.skin_active() {
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
            let paths: Vec<PathBuf> = self.target_entries(i).into_iter().filter(|e| !e.drive).map(|e| e.path).collect();
            if !paths.is_empty() {
                if let Err(error) = (self.system.properties)(&paths) {
                    self.status = error;
                }
            }
            return;
        }
        let path = self.target_entries(i).first().map(|e| e.path.clone()).unwrap_or_else(|| self.panes[i].path.clone());
        if path.as_os_str().is_empty() {
            return;
        }
        if let Err(error) = (self.system.run_verb)(&path, "properties") {
            self.status = error;
        }
    }

    /// C# ShowFileInformation: FileInformationService 줄을 "이름표: 값"으로, 글 창 제목은 Menu("fileInfo").
    /// 압축 안이면 위치는 "압축!항목".
    pub(super) fn file_info_command(&mut self) {
        let i = self.active;
        let pane = &self.panes[i];
        let entry = self.target_entries(i).into_iter().next();
        let archive = pane.archive.clone();
        let (physical, display) = match &archive {
            Some(location) => {
                let key = entry.as_ref().map(|e| e.path.to_string_lossy().into_owned()).unwrap_or_else(|| location.prefix.clone());
                let display = if key.is_empty() { location.file.to_string_lossy().into_owned() } else { format!("{}!{key}", location.file.display()) };
                (None, display)
            }
            None => {
                let path = entry.as_ref().map(|e| e.path.clone()).unwrap_or_else(|| pane.path.clone());
                let display = path.to_string_lossy().into_owned();
                (Some(path), display)
            }
        };
        let mut name = entry.as_ref().map(|e| e.name.clone()).unwrap_or_default();
        if name.is_empty() {
            let current = match &archive {
                Some(location) => location.prefix.trim_end_matches(['/', '\\']).to_owned(),
                None => pane.path.to_string_lossy().into_owned(),
            };
            name = ndir::names::find_file_name(current.trim_end_matches(['/', '\\']));
            if name.is_empty() {
                name = match &archive {
                    Some(location) => location.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    None => current,
                };
            }
            if name.is_empty() {
                name = "—".into();
            }
        }
        let description = entry.as_ref().and_then(|e| pane.descriptions.get(&e.name.to_lowercase())).cloned();
        let unit = self.settings.text("SizeUnit", "Auto");
        let thousands = self.settings.flag("UseThousandsSeparator", true);
        let rows = file_information_rows(entry.as_ref(), &name, physical.as_deref(), &display, description.as_deref(), &unit, thousands, native::shell_type_name);
        let report = rows.iter().map(|(label, value)| format!("{}: {value}", self.language_pack.translate_pair(label, label))).collect::<Vec<_>>().join("\r\n");
        let title = menu_data::ORIGINAL_ACTIONS
            .iter()
            .find(|(_, action)| *action == "fileInfo")
            .map(|(name, _)| self.language_pack.text(&format!("Menu.{name}"), "What is this file?"))
            .unwrap_or_else(|| "What is this file?".into());
        self.text = Some(TextDocument { path: None, title, text: report, editable: false, dirty: false });
    }

    /// C# SelectThemeAsync: "테마 선택"/"NDIR 화면 테마" 고르기 → Theme 저장, 바로 적용.
    pub(super) fn theme_command(&mut self) {
        let current = self.settings.text("Theme", "Black");
        let selected = THEMES.iter().position(|t| *t == current).unwrap_or(0);
        let (title, message) = (self.tr("테마 선택", "Select theme").into_owned(), self.tr("NDIR 화면 테마", "NDIR screen theme").into_owned());
        self.choice_dialog(&title, &message, THEMES.iter().map(|t| (*t).to_owned()).collect(), selected, |app, choice, _| {
            let Some(index) = choice else { return };
            app.apply_theme(THEMES[index]);
        });
    }

    /// 테마를 저장하고 바로 적용한다(C# SelectThemeAsync 끝, 원본 스킨 메뉴 FUN_0043e510).
    pub(super) fn apply_theme(&mut self, theme: &str) {
        self.settings.set_text("Theme", theme);
        self.dark = theme_palette(theme, theme == "System" && native::windows_light_theme()).dark();
        self.preferences_dirty = true;
        self.save();
    }

    /// 원본 스킨 메뉴(FUN_0043e580): skin\ 아래 폴더(".", ".." 빼고) 차례. NDIR가 아는 테마 이름(대소문자 무시)만.
    pub(super) fn skin_names(&self) -> Vec<&'static str> {
        let mut names: Vec<String> = std::fs::read_dir(self.resources.join("Skins"))
            .map(|dir| dir.filter_map(Result::ok).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.sort_by_key(|n| n.to_lowercase());
        names.iter().filter_map(|n| THEMES.iter().find(|t| t.eq_ignore_ascii_case(n)).copied()).collect()
    }

    /// 언어를 저장하고 바로 적용한다(C# SelectLanguageAsync 끝, 원본 언어 메뉴 FUN_0044193c).
    pub(super) fn apply_language(&mut self, code: &str) {
        self.settings.set_text("Language", code);
        self.language = code.to_owned();
        self.save();
    }

    /// C# SelectLanguageAsync: Languages\*.lang 중 아는 언어 "코드 — 이름" 고르기(처음 = 지금 언어, 없으면 ko) → Language 저장, 바로 적용.
    pub(super) fn language_command(&mut self) {
        let languages = settings_window::available_languages(&self.resources);
        if languages.is_empty() {
            return;
        }
        let choices: Vec<String> = languages.iter().map(|(code, name)| format!("{code} — {name}")).collect();
        let current = self.settings.text("Language", &self.language);
        let selected = languages
            .iter()
            .position(|(code, _)| code.eq_ignore_ascii_case(&current))
            .or_else(|| languages.iter().position(|(code, _)| code.eq_ignore_ascii_case("ko")))
            .unwrap_or(0);
        let (title, message) = (self.tr("언어 선택", "Select language").into_owned(), self.tr("NDIR 메뉴에서 사용할 언어를 선택하세요.", "Select the language for the NDIR menus.").into_owned());
        self.choice_dialog(&title, &message, choices, selected, move |app, choice, _| {
            let Some(index) = choice else { return };
            let code = languages[index].0.clone();
            app.apply_language(&code);
        });
    }

    /// C# ViewTextAsync: 고른 첫 파일(폴더 제외)이 없으면 "텍스트 파일 하나를 선택하세요." 경고. 압축 안이면 임시로 푼다.
    /// TextViewerPath/TextEditorPath(ExpandSetting)가 있으면 그 프로그램에 파일 경로를 넘겨(작업 폴더 = 파일 폴더) 끝,
    /// 없으면 NDIR 글 창.
    pub(super) fn text_command(&mut self, edit: bool) {
        let i = self.active;
        // 원본 miTextViewerClick/miTextEditorClick → FUN_00430304: 커서 항목이 파일(종류 3)일 때만, 로컬·압축 패널에서(스킨 테마).
        let original = self.skin_active();
        let entry = if original {
            let pane = &self.panes[i];
            if pane.remote.is_some() {
                return;
            }
            let Some(entry) = pane.entries.get(pane.cursor).filter(|e| !e.is_dir && !e.drive).cloned() else { return };
            entry
        } else {
            let Some(entry) = self.target_entries(i).into_iter().find(|e| !e.is_dir) else {
                return self.warn("텍스트 파일 하나를 선택하세요.", "Select one text file.");
            };
            entry
        };
        match self.panes[i].archive.clone() {
            None => self.text_source_ready(Ok(entry.path), edit),
            Some(location) => {
                let tx = self.async_tx.clone();
                let key = entry.path.to_string_lossy().into_owned();
                thread::spawn(move || {
                    let password = (!location.password.is_empty()).then_some(location.password.as_str());
                    let path = ndir::archive::extract_entry_to_temp(&location.file, password, &key, &AtomicBool::new(false));
                    let _ = tx.send(AsyncResult::TextSource { path, edit });
                });
            }
        }
    }

    pub(super) fn text_source_ready(&mut self, path: Result<PathBuf, String>, edit: bool) {
        let path = match path {
            Ok(path) => path,
            Err(error) => return self.system_box("NDIR", &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        };
        let mut program = ndir::command_line::expand_setting(&self.settings.text(if edit { "TextEditorPath" } else { "TextViewerPath" }, ""));
        // 원본 FUN_00430304: 설정이 비면 "notepad.exe"에 "\"경로\""(스킨 테마). C# 테마는 NDIR 글 창.
        if program.trim().is_empty() && self.skin_active() {
            program = "notepad.exe".into();
        }
        if !program.trim().is_empty() {
            let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
            if let Err(error) = (self.shell_runner)(Path::new(&program), &native::quote_arg(&path.to_string_lossy()), &directory) {
                self.status = error;
            }
            return;
        }
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let result = commands::read_text(&path).map(|text| AsyncResult::Document { path, text, editable: edit }).unwrap_or_else(AsyncResult::Error);
            let _ = tx.send(result);
        });
    }

    /// C# ShowAbout: "NDIR {버전}\nWindows 10/11 · 64-bit\n사용자 설정: {프로필}", 캡션 "NDIR 정보", 정보 아이콘.
    pub(super) fn about_command(&mut self) {
        let text = format!(
            "NDIR {}\nWindows 10/11 · 64-bit\n{}: {}",
            env!("CARGO_PKG_VERSION"),
            self.tr("사용자 설정", "User settings"),
            self.profile.display()
        );
        let caption = self.tr("NDIR 정보", "About NDIR").into_owned();
        self.system_box(&caption, &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
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
        let inside = ArchiveMenuState { current: true, ..Default::default() };
        assert_eq!(inside.item("archiveProperties"), (true, true));
        assert_eq!(inside.item("archiveInstall"), (false, false));
        assert_eq!(inside.item("extractAuto"), (true, false));
        let picked = ArchiveMenuState { selected: true, ..Default::default() };
        assert_eq!(picked.item("extractCurrent"), (true, true));
        assert_eq!(picked.item("archiveInstall"), (true, true));
        assert_eq!(picked.item("archiveProperties"), (true, true));
        // 원본 pmArchivePopup(스킨 테마): 고른 것과 상관없이 압축 안이 아니면 켜짐.
        let original = ArchiveMenuState { original: true, ..Default::default() };
        assert_eq!((original.item("extractCurrent"), original.item("extractAuto")), ((true, true), (true, true)));
        let original_inside = ArchiveMenuState { original: true, in_archive: true, current: true, ..Default::default() };
        assert_eq!((original_inside.item("extractCurrent"), original_inside.item("extractAuto")), ((true, false), (true, false)));
    }

    #[test]
    fn theme_palettes_follow_csharp_apply_theme() {
        assert_eq!(theme_palette("Dark", true).panel, [0x11, 0x18, 0x27]);
        assert!(theme_palette("Dark", true).dark());
        assert_eq!(theme_palette("Light", false).panel, [0xFF, 0xFF, 0xFF]);
        assert!(!theme_palette("Light", false).dark());
        // System: light only when Windows apps use the light theme.
        assert_eq!(theme_palette("System", true), theme_palette("Light", false));
        assert_eq!(theme_palette("System", false), theme_palette("Dark", false));
        assert_eq!(theme_palette("Classic", false).app, [0xDE, 0xDE, 0xDE]);
        assert_eq!(theme_palette("Gray", false).panel, [0xD1, 0xD7, 0xE2]);
        assert!(!theme_palette("Gray", false).dark());
        assert_eq!(theme_palette("Black", false).text, [0xC0, 0xC0, 0xC0]);
        assert!(theme_palette("Black", false).dark());
        // unknown names fall back to Dark
        assert_eq!(theme_palette("NexusFile", false), theme_palette("Dark", false));
    }

    #[test]
    fn file_information_rows_follow_csharp_service() {
        let root = std::env::temp_dir().join(format!("ndir-fileinfo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let file = root.join("report.txt");
        fs::write(&file, b"12345").unwrap();
        let entry = Entry { path: file.clone(), name: "report.txt".into(), is_dir: false, size: 5, modified: None, hidden: false, attributes: 0, drive: false };
        let rows = file_information_rows(Some(&entry), "report.txt", Some(&file), &file.to_string_lossy(), Some("note"), "B", true, |ext, dir| format!("T{ext}{dir}"));
        let labels: Vec<_> = rows.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels, ["Name", "File", "Ext", "Location", "Size", "Date modified", "Attr", "Description"]);
        assert_eq!(rows[1].1, "T.txtfalse");
        assert_eq!(rows[2].1, ".txt");
        assert_eq!(rows[4].1, ndir::parity::format_size(5, "B", true));
        assert_ne!(rows[5].1, "—");
        assert_eq!(rows[7].1, "note");
        // Folder: size is "—", empty shell type falls back to "폴더", no extension shows "—".
        let rows = file_information_rows(None, "dir", Some(&root), &root.to_string_lossy(), None, "B", true, |_, _| String::new());
        assert_eq!(rows[1].1, "폴더");
        assert_eq!(rows[2].1, "—");
        assert_eq!(rows[4].1, "—");
        assert!(rows.iter().any(|(l, v)| *l == "Attr" && v.contains("Directory")));
        // Archive entry: no physical path, values from the listing.
        let rows = file_information_rows(Some(&entry), "report.txt", None, "a.zip!report.txt", None, "B", true, |_, _| String::new());
        assert_eq!(rows[3].1, "a.zip!report.txt");
        assert_eq!(rows[5].1, "—");
        assert!(!rows.iter().any(|(l, _)| *l == "Attr"));
        let _ = fs::remove_dir_all(&root);
    }
}
