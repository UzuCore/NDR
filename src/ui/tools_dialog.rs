//! C# ChecksumAsync(+ 체크섬 하위 메뉴), MergeTextFilesAsync, OpenWithAsync, RunCommandAsync(실행 파일 기록), OriginalFindAsync,
//! MapNetworkDriveAsync/UnmapNetworkDriveAsync(+ CredentialsWindow), ToggleExplorerIntegrationAsync, ImportLegacySettingsAsync,
//! InstallSelectedArchive. C#이 RunSafely로 보이는 예외는 `warn`(MessageBox(글, "NDR", 경고))이다.
//! 차이(HANDOFF): 원격 패널에서는 체크섬·합치기·다른 프로그램으로 열기 대상이 없다(C#에는 원격 패널이 없다).
//! ftp:// 명령은 원격 창 이식 전까지 옛 원격 연결 창을 연다.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::command_line::{self as cmdline, CommandKind};
use ndr::msgbox::{IDNO, IDYES, MB_ICONQUESTION, MB_OK, MB_YESNO, MB_YESNOCANCEL};

/// 바깥 시스템을 건드리는 함수들(테스트에서 바꿔 끼운다): 네트워크 드라이브, 탐색기 메뉴 등록, 동사 실행.
#[derive(Clone, Copy)]
pub(super) struct SystemHooks {
    pub network_drives: fn() -> Vec<String>,
    pub add_connection: fn(&str, &str, Option<&str>, Option<&str>) -> Result<(), (u32, String)>,
    pub cancel_connection: fn(&str) -> Result<(), (u32, String)>,
    pub explorer_enabled: fn() -> bool,
    pub set_explorer: fn(bool) -> Result<(), String>,
    pub run_verb: fn(&Path, &str) -> Result<(), String>,
    /// 원본 miPropertiesClick → FUN_00458da8: 고른 항목 전부의 IContextMenu "properties"(여러 파일 속성 창).
    pub properties: fn(&[PathBuf]) -> Result<(), String>,
    /// C# GetNetworkPathErrorCode / PromptAndConnectNetworkShare(Windows 자격 증명 창).
    pub network_path_error: fn(&Path) -> u32,
    pub connect_share: fn(&str) -> u32,
    /// C# OriginalShortcutService.ExistingTargetFolder(빠른 실행이면 true) / Create(대상, 폴더).
    pub shortcut_folder: fn(bool) -> Option<PathBuf>,
    pub create_shortcut: fn(&Path, &Path) -> Result<(), String>,
    /// C# _readClipboardFileDropList(CF_HDROP 목록). 테스트는 실제 클립보드를 건드리지 않는다.
    pub clipboard_files: fn() -> Vec<PathBuf>,
    /// 원본 miEmptyTempClick의 GetTempPathW + GetLongPathNameW. 시험에서는 실제 임시 폴더를 건드리지 않도록 늘 가짜.
    pub temp_folder: fn() -> Option<PathBuf>,
    /// 원본 TfCleanup의 SHGetSpecialFolderPathW(CSIDL_INTERNET_CACHE 0x20, CSIDL_COOKIES 0x21). 시험에서는 늘 None.
    pub special_folder: fn(u32) -> Option<PathBuf>,
    /// C# ListLocal("::drives::"): 준비된 드라이브 루트들.
    pub ready_drives: fn() -> Vec<PathBuf>,
    /// 드라이브 볼륨 이름 읽기(GetVolumeInformationW)와 바꾸기(원본 FUN_004351cc의 SetVolumeLabelW, 실패 = GetLastError).
    pub volume_label: fn(&Path) -> Option<String>,
    pub set_volume_label: fn(&Path, &str) -> Result<(), u32>,
    /// 원본 정보 표시줄의 드라이브 글·여유·전체(native::pane_disk_info). 시험에서는 늘 None.
    pub disk_info: fn(&Path) -> Option<(String, u64, u64)>,
    /// 원본 내 PC 드라이브 항목 정보(native::drive_item_info). 시험에서는 늘 None.
    pub drive_item: fn(&Path) -> Option<(String, u32, Option<(u64, u64)>)>,
    /// C# FileVisualConverter.LoadIcon(작은 셸 아이콘, 확장자·속성만): (너비, 높이, RGBA). 시험에서는 셸을 부르지 않는 가짜.
    pub file_icon: fn(&Path, bool) -> Option<(usize, usize, Vec<u8>)>,
    /// C# LoadShellOverlayIcon(실제 경로, 오버레이 포함). 따로 된 STA 스레드에서 부른다. 시험에서는 셸을 부르지 않는 가짜.
    pub overlay_icon: fn(&Path) -> Option<(usize, usize, Vec<u8>)>,
    /// 원본 GetIconIndex(실제 경로) 파일 아이콘(native::shell_path_icon, 오버레이 없음). 시험에서는 셸을 부르지 않는 가짜.
    pub path_icon: fn(&Path) -> Option<(usize, usize, Vec<u8>)>,
    /// 확장자(점 없이)의 셸 아이콘이 파일마다 다른가(native::extension_has_per_file_icon, HKCR). 시험에서는 늘 false(고정 목록만).
    pub per_file_icon: fn(&str) -> bool,
    /// 목록 글꼴(ListFont) 이름의 글꼴 파일(native::font_file). 시험에서는 늘 None(기본 글꼴).
    pub font_file: fn(&str) -> Option<(PathBuf, u32)>,
    /// 원본 TfFileExists imgOrg·imgNew(native::shell_large_icon, 32x32). 시험에서는 셸을 부르지 않는 가짜.
    pub large_icon: fn(&Path) -> Option<(usize, usize, Vec<u8>)>,
    /// 원본 DrawTextW(DT_PATH_ELLIPSIS) 경로 줄임(native::compact_path). 시험에서는 그대로 돌려준다.
    pub compact_path: fn(&str, i32) -> String,
    /// Delphi Mouse.WheelScrollLines(native::wheel_scroll_lines). 시험에서는 3.
    pub wheel_lines: fn() -> u32,
    /// 원본 PlaySoundW(파일, 0x22003)(native::play_wave). 시험에서는 소리를 내지 않고 PLAYED에 남긴다.
    pub play_wave: fn(Option<&Path>) -> bool,
    /// 폴더 트리 창의 뿌리(드라이브 뿌리 "C:\"). 시험에서는 빈 글(실제 드라이브를 훑지 않는다)이고, 트리 시험은 가짜 뿌리를 끼운다.
    pub tree_root: fn(&str) -> String,
    /// 원본 폴더 트리 삭제 FUN_004266ac의 SHFileOperationW(FO_DELETE)(native::shell_delete). 0 = 성공. 시험에서는 셸 창 없이 지운다.
    pub shell_delete: fn(&Path, bool) -> i32,
    /// 원본 GetShortcutTarget(바로 가기의 대상, 폴더인지)(native::shortcut_target).
    pub shortcut_target: fn(&Path) -> Option<(PathBuf, bool)>,
}

#[cfg(test)]
thread_local! {
    /// 시험용 play_wave가 받은 경로(None = 멈추기).
    pub(super) static PLAYED: std::cell::RefCell<Vec<Option<PathBuf>>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
pub(super) fn record_wave(path: Option<&Path>) -> bool {
    PLAYED.with(|p| p.borrow_mut().push(path.map(Path::to_path_buf)));
    true
}

#[cfg(test)]
fn same_path(path: &str, _: i32) -> String {
    path.to_owned()
}

#[cfg(test)]
fn fake_overlay_icon(_: &Path) -> Option<(usize, usize, Vec<u8>)> {
    Some((3, 3, [255, 0, 0, 255].repeat(9)))
}

#[cfg(test)]
pub(super) fn fake_path_icon(_: &Path) -> Option<(usize, usize, Vec<u8>)> {
    Some((5, 5, [0, 255, 0, 255].repeat(25)))
}

#[cfg(test)]
pub(super) fn fake_large_icon(_: &Path) -> Option<(usize, usize, Vec<u8>)> {
    Some((4, 4, [0, 0, 255, 255].repeat(16)))
}

#[cfg(test)]
fn fake_file_icon(_: &Path, directory: bool) -> Option<(usize, usize, Vec<u8>)> {
    let pixel = if directory { [228, 188, 85, 255] } else { [200, 200, 200, 255] };
    Some((2, 2, pixel.repeat(4)))
}

#[cfg(test)]
fn no_drive_item(_: &Path) -> Option<(String, u32, Option<(u64, u64)>)> {
    None
}

#[cfg(test)]
fn no_disk_info(_: &Path) -> Option<(String, u64, u64)> {
    None
}

#[cfg(test)]
fn no_drives() -> Vec<PathBuf> {
    Vec::new()
}
#[cfg(test)]
fn no_volume_label(_: &Path) -> Option<String> {
    None
}
#[cfg(test)]
fn refuse_volume_label(_: &Path, _: &str) -> Result<(), u32> {
    Err(5)
}

#[cfg(test)]
fn no_temp_folder() -> Option<PathBuf> {
    None
}

/// CSIDL_INTERNET_CACHE(0x20), CSIDL_COOKIES(0x21).
#[cfg(not(test))]
fn real_special_folder(csidl: u32) -> Option<PathBuf> {
    match csidl {
        0x20 => native::internet_cache_folder(),
        0x21 => native::cookies_folder(),
        _ => None,
    }
}

fn system_clipboard_files() -> Vec<PathBuf> {
    native::read_clipboard_files().map(|(paths, _)| paths).unwrap_or_default()
}

/// C# connectNetworkShare: UNC 경로의 오류가 자격 증명 실패(5, 65, 86, 1326)면 연결 창, 아니면 그 오류 번호.
pub(super) fn network_share_code(path: &str, path_error: fn(&Path) -> u32, connect: fn(&str) -> u32) -> u32 {
    let code = path_error(Path::new(path));
    if matches!(code, 5 | 65 | 86 | 1326) { connect(path) } else { code }
}

fn shell_verb(path: &Path, verb: &str) -> Result<(), String> {
    native::shell_open(path, None, verb)
}

impl Default for SystemHooks {
    fn default() -> Self {
        Self {
            network_drives: native::network_drives,
            add_connection: native::wnet_add_connection,
            cancel_connection: native::wnet_cancel_connection,
            explorer_enabled: native::explorer_integration_enabled,
            set_explorer: native::set_explorer_integration,
            run_verb: shell_verb,
            #[cfg(not(test))]
            properties: ndr::shell_menu::properties,
            #[cfg(test)]
            properties: |_| Err("not in tests".into()),
            network_path_error: native::network_path_error,
            connect_share: native::connect_share_interactive,
            shortcut_folder: native::shortcut_target_folder,
            create_shortcut: native::create_original_shortcut,
            clipboard_files: system_clipboard_files,
            #[cfg(not(test))]
            temp_folder: native::temp_folder,
            #[cfg(test)]
            temp_folder: no_temp_folder,
            #[cfg(not(test))]
            special_folder: real_special_folder,
            #[cfg(test)]
            special_folder: |_| None,
            #[cfg(not(test))]
            ready_drives: native::ready_drives,
            #[cfg(test)]
            ready_drives: no_drives,
            #[cfg(not(test))]
            volume_label: native::volume_label,
            #[cfg(test)]
            volume_label: no_volume_label,
            #[cfg(not(test))]
            set_volume_label: native::set_volume_label,
            #[cfg(test)]
            set_volume_label: refuse_volume_label,
            #[cfg(not(test))]
            disk_info: native::pane_disk_info,
            #[cfg(test)]
            disk_info: no_disk_info,
            #[cfg(not(test))]
            drive_item: native::drive_item_info,
            #[cfg(test)]
            drive_item: no_drive_item,
            #[cfg(not(test))]
            file_icon: native::shell_small_icon,
            #[cfg(test)]
            file_icon: fake_file_icon,
            #[cfg(not(test))]
            overlay_icon: native::shell_overlay_icon,
            #[cfg(test)]
            overlay_icon: fake_overlay_icon,
            #[cfg(not(test))]
            path_icon: native::shell_path_icon,
            #[cfg(test)]
            path_icon: fake_path_icon,
            #[cfg(not(test))]
            per_file_icon: native::extension_has_per_file_icon,
            #[cfg(test)]
            per_file_icon: |_| false,
            #[cfg(not(test))]
            font_file: native::font_file,
            #[cfg(test)]
            font_file: |_| None,
            #[cfg(not(test))]
            large_icon: native::shell_large_icon,
            #[cfg(test)]
            large_icon: fake_large_icon,
            #[cfg(not(test))]
            compact_path: native::compact_path,
            #[cfg(test)]
            compact_path: same_path,
            #[cfg(not(test))]
            wheel_lines: native::wheel_scroll_lines,
            #[cfg(test)]
            wheel_lines: || 3,
            #[cfg(not(test))]
            play_wave: native::play_wave,
            #[cfg(test)]
            play_wave: record_wave,
            #[cfg(not(test))]
            tree_root: super::folder_tree_dialog::drive_root,
            #[cfg(test)]
            tree_root: |_| String::new(),
            #[cfg(not(test))]
            shell_delete: native::shell_delete,
            #[cfg(test)]
            shell_delete: |path, _| if std::fs::remove_dir_all(path).is_ok() { 0 } else { 2 },
            shortcut_target: native::shortcut_target,
        }
    }
}

/// C# ChecksumAlgorithm(하위 메뉴 checksum:Crc32 등). None이면 네 가지 모두.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChecksumAlgorithm {
    Crc32,
    Md5,
    Sha1,
    Sha256,
}

impl ChecksumAlgorithm {
    pub(super) fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "crc32" => Some(Self::Crc32),
            "md5" => Some(Self::Md5),
            "sha1" => Some(Self::Sha1),
            "sha256" => Some(Self::Sha256),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Crc32 => "CRC",
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
        }
    }
}

/// 체크섬 창 글: "이름\r\n결과"를 빈 줄로 잇는다.
pub(super) fn checksum_text(items: &[(String, ndr::tools::Checksums)], algorithm: Option<ChecksumAlgorithm>) -> String {
    items
        .iter()
        .map(|(name, hash)| {
            let result = match algorithm {
                Some(ChecksumAlgorithm::Crc32) => format!("CRC {}", hash.crc32),
                Some(ChecksumAlgorithm::Md5) => format!("MD5 {}", hash.md5),
                Some(ChecksumAlgorithm::Sha1) => format!("SHA-1 {}", hash.sha1),
                Some(ChecksumAlgorithm::Sha256) => format!("SHA-256 {}", hash.sha256),
                None => format!("CRC32   {}\r\nMD5     {}\r\nSHA-1   {}\r\nSHA-256 {}", hash.crc32, hash.md5, hash.sha1, hash.sha256),
            };
            format!("{name}\r\n{result}")
        })
        .collect::<Vec<_>>()
        .join("\r\n\r\n")
}

/// 대상 파일(압축 파일 안이면 항목 키): (보이는 이름, 경로 또는 키).
struct SourceFiles {
    archive: Option<ArchiveLocation>,
    files: Vec<(String, PathBuf)>,
}

impl SourceFiles {
    /// 압축 파일 안이면 임시 폴더에 하나씩 풀어 실제 경로로(C# ExtractEntryToTempAsync).
    fn resolve(&self, cancel: &AtomicBool) -> Result<Vec<PathBuf>, String> {
        match &self.archive {
            None => Ok(self.files.iter().map(|(_, p)| p.clone()).collect()),
            Some(location) => {
                let password = (!location.password.is_empty()).then_some(location.password.as_str());
                self.files.iter().map(|(_, key)| ndr::archive::extract_entry_to_temp(&location.file, password, &key.to_string_lossy(), cancel)).collect()
            }
        }
    }
}

/// 원본 TfPassword("인증", ClientWidth 249 × ClientHeight 142): lUser(12,24) eUser(82,21 145×21), lPassword(12,64) ePassword(82,61 145×21, '*'),
/// btnOk(46,105)·btnCancel(135,105) 75×25. NDR은 C# CredentialsWindow(네트워크 드라이브 다른 계정) 자리에 쓴다. 확인은 사용자 이름이 있을 때만.
pub(super) fn credentials_ui(ui: &mut egui::Ui, user: &mut String, password: &mut String, language: &compat::Language, enter: bool, first_frame: bool) -> Option<bool> {
    use super::original_dialogs::put_left;
    let text = |key: &str, fallback: &str| language.text(&format!("formPassword.{key}"), fallback).replace('&', "");
    let mut result = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(249.0, 142.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    put_left(ui, at(12.0, 24.0, 68.0, 15.0), egui::Label::new(text("Caption.lUser", "User:")).truncate());
    let response = ui.put(at(82.0, 21.0, 145.0, 21.0), egui::TextEdit::singleline(user));
    if first_frame {
        response.request_focus();
    }
    put_left(ui, at(12.0, 64.0, 68.0, 15.0), egui::Label::new(text("Caption.lPassword", "Password:")).truncate());
    ui.put(at(82.0, 61.0, 145.0, 21.0), egui::TextEdit::singleline(password).password(true));
    if ui.put(at(46.0, 105.0, 75.0, 25.0), egui::Button::new(text("Caption.btnOk", "OK"))).clicked() {
        result = Some(true);
    }
    if ui.put(at(135.0, 105.0, 75.0, 25.0), egui::Button::new(text("Caption.btnCancel", "Cancel"))).clicked() {
        result = Some(false);
    }
    if enter && result.is_none() {
        result = Some(true);
    }
    result.filter(|ok| !*ok || !user.trim().is_empty())
}

impl App {
    /// C# SelectedItems에서 폴더를 뺀 파일들(원격 패널은 없음).
    fn selected_files(&self) -> SourceFiles {
        let pane = &self.panes[self.active];
        let files = self.target_entries(self.active).into_iter().filter(|e| !e.is_dir).map(|e| (e.name.clone(), e.path.clone())).collect();
        SourceFiles { archive: pane.archive.clone(), files }
    }

    /// C# ChecksumAsync: 파일마다 네 가지(또는 고른 하나)를 계산해 글 창("체크섬" 또는 "CRC 체크섬" 등).
    /// 원본 하위 메뉴에 있는 CRC·MD5·SHA1은 원본 TfChecksum(`original_checksum`)으로, SHA-256과 네 가지 모두는 C# 글 창으로.
    pub(super) fn checksum_command(&mut self, algorithm: Option<ChecksumAlgorithm>) {
        if let Some(algorithm) = algorithm.filter(|a| *a != ChecksumAlgorithm::Sha256) {
            return self.original_checksum(algorithm);
        }
        let sources = self.selected_files();
        if sources.files.is_empty() {
            return self.notice("체크섬을 계산할 파일을 선택하세요.", "Select the files to checksum.");
        }
        let base = self.tr("체크섬", "Checksum").into_owned();
        let title = match algorithm {
            Some(a) => format!("{} {base}", a.label()),
            None => base,
        };
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            let outcome = sources.resolve(&cancel).and_then(|paths| {
                sources.files.iter().zip(paths).map(|((name, _), path)| ndr::tools::checksums(&path, &cancel).map(|hash| (name.clone(), hash))).collect::<Result<Vec<_>, _>>()
            });
            let _ = tx.send(match outcome {
                Ok(items) => AsyncResult::Text { title, text: checksum_text(&items, algorithm) },
                Err(error) => AsyncResult::Warning(error),
            });
        });
    }

    /// 원본 ChecksumClick(0x445bbc) + TfChecksum(FormShow 0x41af0c, 끝 0x41b164): 초점 항목이 파일이고 로컬·압축 패널일 때만.
    /// 제목 = "CRC"/"MD5"/"SHA1", 글은 CALCULATING으로 시작해 계산 스레드가 끝나면 결과(CRC = IntToHex(8자, 대문자),
    /// MD5/SHA1 = CryptoAPI 해시의 "0123456789abcdef" 소문자)나 ERROR. 압축 안 항목은 임시로 풀어 계산한다.
    fn original_checksum(&mut self, algorithm: ChecksumAlgorithm) {
        let pane = &self.panes[self.active];
        if super::panes::is_drives(&pane.path) {
            return;
        }
        let Some(entry) = pane.entries.get(pane.cursor).filter(|e| !e.is_dir) else { return };
        let sources = SourceFiles { archive: pane.archive.clone(), files: vec![(entry.name.clone(), entry.path.clone())] };
        let (caption, name) = match algorithm {
            ChecksumAlgorithm::Crc32 => ("CRC", "crc32"),
            ChecksumAlgorithm::Md5 => ("MD5", "md5"),
            _ => ("SHA1", "sha1"),
        };
        let calculating = self.language_pack.text("RES.CALCULATING", "Calculating...");
        self.push_original_dialog(caption.into(), DialogKind::Checksum { value: calculating, ready: false }, |_, _, _| {});
        let serial = self.msgbox_serial;
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            let path = sources.resolve(&cancel).ok().and_then(|paths| paths.into_iter().next());
            let value = path.and_then(|path| ndr::tools::checksum(&path, name).ok()).map(|hash| if name == "crc32" { hash } else { hash.to_ascii_lowercase() });
            let _ = tx.send(AsyncResult::ChecksumDone { serial, value });
        });
    }

    /// 원본 FUN_0041b164: 결과가 비면 ERROR와 꺼진 복사 단추, 아니면 결과와 켜진 복사 단추. 이미 닫힌 창이면 버린다.
    pub(super) fn checksum_done(&mut self, serial: u64, value: Option<String>) {
        let error = self.language_pack.text("RES.ERROR", "ERROR");
        let Some(dialog) = self.original_dialogs.iter_mut().find(|d| d.serial == serial) else { return };
        if let DialogKind::Checksum { value: shown, ready } = &mut dialog.kind {
            *ready = value.as_ref().is_some_and(|v| !v.is_empty());
            *shown = value.filter(|v| !v.is_empty()).unwrap_or(error);
        }
    }

    /// 원본 btnCopyClick(0x41b0ec): 맨 위 체크섬 창의 글을 클립보드로.
    pub(super) fn copy_checksum(&mut self) {
        let Some(DialogKind::Checksum { value, ready: true }) = self.original_dialogs.last().map(|d| &d.kind) else { return };
        let text = value.clone();
        if let Err(error) = (self.clipboard_writer)(&text) {
            self.status = error;
        }
    }

    /// C# MergeTextFilesAsync: 파일 둘 이상 → 저장 대화 상자 → (압축 안이면 임시로 풀기) → 같은 경로 검사 → 구분 고르기 → 이어 쓰기.
    pub(super) fn merge_text_files_command(&mut self) {
        let sources = self.selected_files();
        if sources.files.len() < 2 {
            return self.warn("합칠 텍스트 파일을 둘 이상 선택하세요.", "Select at least two text files to merge.");
        }
        let folder = PathBuf::from(self.favorites_current());
        let ko = self.language == "ko";
        let title = self.tr("텍스트 파일 합치기", "Merge text files").into_owned();
        let filters: [(&str, &str); 2] = if ko { [("텍스트 파일 (*.txt)", "*.txt"), ("모든 파일 (*.*)", "*.*")] } else { [("Text files (*.txt)", "*.txt"), ("All files (*.*)", "*.*")] };
        let Some(output) = (self.save_dialog)(&title, Some(&folder), "merged.txt", &filters, "", true) else { return };
        if sources.archive.is_none() {
            let paths = sources.files.into_iter().map(|(_, p)| p).collect();
            return self.merge_sources_ready(output, Ok(paths));
        }
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let outcome = sources.resolve(&AtomicBool::new(false));
            let _ = tx.send(AsyncResult::MergeSources(output, outcome));
        });
    }

    pub(super) fn merge_sources_ready(&mut self, output: PathBuf, sources: Result<Vec<PathBuf>, String>) {
        let sources = match sources {
            Ok(sources) => sources,
            Err(error) => return self.system_box("NDR", &error, MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {}),
        };
        let full = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().into_owned();
        if sources.iter().any(|p| ndr::names::equals_ignore_case(&full(p), &full(&output))) {
            return self.warn("결과 파일을 입력 파일과 같은 경로에 저장할 수 없습니다.", "The result cannot be saved over one of the input files.");
        }
        let ko = self.language == "ko";
        let title = self.tr("파일 사이 줄바꿈", "Line breaks between files").into_owned();
        let message = self.tr("각 파일 뒤에 구분 줄을 넣을까요?", "Add separator lines after each file?").into_owned();
        let options = if ko { ["구분 없이 이어 붙이기", "줄바꿈 두 개 추가"] } else { ["Join without separators", "Add two line breaks"] };
        self.choice_dialog(&title, &message, options.map(String::from).to_vec(), 0, move |app, choice, _| {
            let Some(choice) = choice else { return };
            let tx = app.async_tx.clone();
            thread::spawn(move || {
                let outcome = (|| -> Result<(), String> {
                    use std::io::Write;
                    let mut writer = std::io::BufWriter::with_capacity(1024 * 1024, std::fs::File::create(&output).map_err(|e| e.to_string())?);
                    for source in &sources {
                        let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
                        std::io::copy(&mut input, &mut writer).map_err(|e| e.to_string())?;
                        if choice == 1 {
                            writer.write_all(b"\r\n\r\n").map_err(|e| e.to_string())?;
                        }
                    }
                    writer.flush().map_err(|e| e.to_string())
                })();
                let _ = tx.send(AsyncResult::MergeDone(sources.len(), outcome));
            });
        });
    }

    pub(super) fn merge_done(&mut self, count: usize, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => {
                let n = ndr::parity::format_size(count as u64, "B", true).trim_end_matches(" B").to_owned();
                self.status = if self.language == "ko" { format!("{n}개 텍스트 파일을 합쳤습니다.") } else { format!("Merged {n} text files.") };
                self.refresh(self.active);
            }
            Err(error) => self.system_box("NDR", &error, MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {}),
        }
    }

    /// C# OpenWithAsync: 프로그램(처음 값 TextViewerPath, 환경 변수·$(PROGRAMDIR) 풀기) → 인수 템플릿 → {files}면 한 번, 아니면 파일마다 실행.
    /// 스킨 테마는 원본 mipOpenWithClick(0x43da54): 커서 항목(FUN_0042d3b8 = 지금 폴더 + 이름)을
    /// ExecuteFile("rundll32.exe", "shell32.dll,OpenAs_RunDLL 경로")로 Windows "연결 프로그램" 창에 넘긴다.
    pub(super) fn open_with_command(&mut self) {
        let pane = &self.panes[self.active];
        if pane.archive.is_some() || pane.remote.is_some() {
            return;
        }
        let Some(entry) = pane.entries.get(pane.cursor).filter(|e| !e.is_dir && !e.drive) else { return };
        let path = entry.path.clone();
        let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
        if let Err(error) = (self.shell_runner)(Path::new("rundll32.exe"), &format!("shell32.dll,OpenAs_RunDLL {}", path.display()), &directory) {
            self.status = error;
        }
    }

    /// C# RunCommandAsync의 입력 상자 = 원본 miCommandLineClick(0x438598): TfGetText(TITLE_CMD, MSG_CMD, 빈 글, 기록).
    pub(super) fn command_prompt(&mut self) {
        let title = self.language_pack.text("RES.TITLE_CMD", "Command Line");
        let message = self.language_pack.text("RES.MSG_CMD", "Type a command to execute.");
        let history = self.settings.strings("CommandHistory");
        self.prompt(&title, &message, "", Some(history), false, |app, command, _| {
            if let Some(command) = command {
                app.run_command(command);
            }
        });
    }

    /// C# RunCommandAsync(입력 뒤): 기록(로컬 명령이면 20줄), ftp:// → 원격, http:// → 열기, 압축 안이면 그만, cd → 폴더 이동,
    /// 실행 파일 기록에 같은 이름이 하나면 그 경로(인수 없이), 여럿이면 고르는 메뉴, 아니면 ShellExecuteEx(이름, 인수, 작업 폴더).
    pub(super) fn run_command(&mut self, command: String) {
        let i = self.active;
        let parsed = cmdline::parse(&command);
        let local = self.panes[i].archive.is_none() && matches!(parsed.kind, CommandKind::Local | CommandKind::ChangeDirectory);
        let mut history = self.settings.strings("CommandHistory");
        cmdline::remember_local_command(&mut history, &command, local);
        self.settings.set_strings("CommandHistory", history);
        self.save();
        match parsed.kind {
            CommandKind::Ftp => {
                // 원본 miCommandLineClick: "ftp://"로 시작하면 FUN_0043e9f8(창, 0 = 지금 패널, 명령) = TfFTPQuickConnect(cbHost = 명령).
                // (C# OpenRemoteWindow(initialHost: 명령) 대신 원본처럼 패널 FTP 연결 창.)
                self.ftp_quick_connect(command);
                return;
            }
            CommandKind::Http => {
                let directory = self.panes[i].path.clone();
                if let Err(error) = (self.shell_runner)(Path::new(&command), "", &directory) {
                    self.status = error;
                }
                return;
            }
            _ => {}
        }
        if self.panes[i].archive.is_some() {
            return;
        }
        if self.panes[i].remote.is_some() {
            return self.ftp_raw_command(i, command);
        }
        let directory = if self.panes[i].path.is_dir() {
            self.panes[i].path.clone()
        } else {
            self.last_command_directory.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        };
        self.last_command_directory = Some(directory.clone());
        if parsed.kind == CommandKind::ChangeDirectory {
            if let Some(target) = cmdline::resolve_directory(&parsed.arguments, &directory) {
                self.navigate(i, target, true);
            }
            return;
        }
        if command.trim().is_empty() {
            return;
        }
        let candidates = cmdline::find_candidates(&self.settings.strings("ExecutableHistory"), &command, Some(&directory));
        match candidates.len() {
            1 => self.run_cached_executable(&candidates[0], &directory),
            0 => {
                if let Err(error) = (self.shell_runner)(Path::new(&parsed.file_name), &parsed.arguments, &directory) {
                    self.status = error;
                }
            }
            _ => {
                let items = candidates
                    .into_iter()
                    .map(|path| super::favorites_dialog::PopupItem::Entry { label: path.clone(), access: None, gesture: String::new(), enabled: true, checked: false, action: super::favorites_dialog::PopupAction::RunExecutable { path, directory: directory.clone() } })
                    .collect();
                // 원본 pmTemp(+0x870): OnPopup = pmTempPopup(0x442b78)이 아래 화살표 하나를 보내 첫 줄을 골라 둔다.
                self.popup = Some(super::favorites_dialog::PopupMenu { items, position: Some(egui::pos2(4.0, 4.0)), highlight: Some(0), opened: true });
            }
        }
    }

    /// RunCachedExecutable: 경로가 있으면 인수 없이 실행.
    pub(super) fn run_cached_executable(&mut self, path: &str, directory: &Path) {
        if !cmdline::executable_exists(path, Some(directory)) {
            return;
        }
        if let Err(error) = (self.shell_runner)(Path::new(path), "", directory) {
            self.status = error;
        }
    }

    /// C# OriginalFindAsync(miFindClick 0x437b2c): TfGetText(TITLE_FIND, MSG_FIND, 빈 글). OK면 찾기.
    pub(super) fn find_command(&mut self) {
        let title = self.language_pack.text("RES.TITLE_FIND", "Find");
        let message = self.language_pack.text("RES.MSG_FIND", "Find a file containing following text and move a select bar to that file.");
        self.prompt(&title, &message, "", None, false, |app, text, _| {
            if let Some(text) = text {
                app.original_find(&text);
            }
        });
    }

    /// C# MapNetworkDriveAsync.
    pub(super) fn map_drive_command(&mut self) {
        let ko = self.language == "ko";
        let title = if ko { "네트워크 드라이브 연결" } else { "Connect network drive" };
        let message = if ko { "사용할 드라이브 문자 (예: Z:)" } else { "Drive letter to use (e.g. Z:)" };
        self.prompt(title, message, "Z:", None, false, move |app, letter, _| {
            let Some(letter) = letter else { return };
            let mut letter = letter.trim().trim_end_matches('\\').to_owned();
            if letter.chars().count() == 1 {
                letter.push(':');
            }
            let chars: Vec<char> = letter.chars().collect();
            if chars.len() != 2 || !chars[0].is_alphabetic() || chars[1] != ':' {
                return app.warn("드라이브 문자는 Z: 형식으로 입력하세요.", "Enter the drive letter as Z:.");
            }
            if native::drives().iter().any(|d| d.to_string_lossy().to_lowercase().starts_with(&letter.to_lowercase())) {
                let (k, e) = (format!("{letter} 드라이브 문자는 이미 사용 중입니다."), format!("Drive letter {letter} is already in use."));
                return app.warn(&k, &e);
            }
            let message = if app.language == "ko" { "공유 경로 (예: \\\\server\\share)" } else { "Share path (e.g. \\\\server\\share)" };
            app.prompt(title, message, "", None, false, move |app, remote, _| {
                let Some(remote) = remote.filter(|r| !r.trim().is_empty() && r.starts_with("\\\\")) else { return };
                let (caption, question) = if app.language == "ko" {
                    ("네트워크 인증", "현재 Windows 계정으로 연결할까요?\n아니요를 선택하면 별도 계정 정보를 입력합니다.")
                } else {
                    ("Network authentication", "Connect with the current Windows account?\nChoose No to enter other credentials.")
                };
                app.system_box(caption, question, MB_YESNOCANCEL | MB_ICONQUESTION, move |app, answer, _| match answer {
                    IDYES => app.connect_drive(&letter, &remote, None),
                    IDNO => {
                        let title = app.tr("네트워크 드라이브 인증", "Network drive credentials").into_owned();
                        app.credentials_dialog(&title, move |app, credentials| {
                            if let Some(credentials) = credentials {
                                app.connect_drive(&letter, &remote, Some(credentials));
                            }
                        });
                    }
                    _ => {}
                });
            });
        });
    }

    /// 원본 miNetwork(0x4416ec, 내부 명령 0x26 NetworkDialog): 네트워크를 뿌리로 한 폴더 고르기(SHBrowseForFolderW, 제목 [RES] NETWORK).
    /// 로컬 패널이면 지금 폴더를 고른 채 열고, 고르면 활성 패널을 그 폴더로 옮긴다(FUN_0048d6e4, FTP 패널이면 끊고 간다).
    /// (C# 판은 탐색기 shell:NetworkPlacesFolder를 열었다.)
    pub(super) fn browse_network_command(&mut self) {
        let i = self.active;
        let local = self.panes[i].archive.is_none() && self.panes[i].remote.is_none();
        let initial = local.then(|| self.panes[i].path.clone());
        let title = self.language_pack.text("RES.NETWORK", "Network");
        if let Some(folder) = (self.network_picker)(&title, initial.as_deref()) {
            self.navigate(i, folder, true);
        }
    }

    fn connect_drive(&mut self, letter: &str, remote: &str, credentials: Option<(String, String)>) {
        let remote = remote.trim();
        let (user, password) = match &credentials {
            Some((user, password)) => (Some(user.as_str()), Some(password.as_str())),
            None => (None, None),
        };
        if let Err((code, message)) = (self.system.add_connection)(letter, remote, user, password) {
            let (k, e) = (format!("네트워크 드라이브 연결에 실패했습니다. Windows 오류 {code}: {message}"), format!("Could not connect the network drive. Windows error {code}: {message}"));
            return self.warn(&k, &e);
        }
        // 원본 FUN_00435144: 활성 패널이 로컬 모드(0)일 때만 새로 읽는다.
        super::panes::DriveListCache::invalidate(&self.drive_list);
        if self.panes[self.active].archive.is_none() && self.panes[self.active].remote.is_none() {
            self.refresh(self.active);
        }
        self.status = if self.language == "ko" { format!("{letter} → {remote} 연결됨") } else { format!("{letter} → {remote} connected") };
    }

    /// CredentialsWindow를 연다.
    pub(super) fn credentials_dialog(&mut self, title: &str, on_result: impl FnOnce(&mut App, Option<(String, String)>) + 'static) {
        self.push_original_dialog(title.into(), DialogKind::Credentials { user: String::new(), password: String::new() }, move |app, result, _| {
            if let DialogResult::Credentials(value) = result {
                on_result(app, value);
            }
        });
    }

    /// C# UnmapNetworkDriveAsync: 연결된 네트워크 드라이브 고르기 → 연결 해제 → 활성 패널 새로 고침.
    pub(super) fn unmap_drive_command(&mut self) {
        let mapped = (self.system.network_drives)();
        if mapped.is_empty() {
            return self.notice("연결된 네트워크 드라이브가 없습니다.", "No network drives are connected.");
        }
        let title = self.tr("네트워크 드라이브 연결 해제", "Disconnect network drive").into_owned();
        let message = self.tr("연결을 해제할 드라이브를 고르세요.", "Choose the drive to disconnect.").into_owned();
        self.choice_dialog(&title, &message, mapped.clone(), 0, move |app, choice, _| {
            let Some(selected) = choice.and_then(|c| mapped.get(c).cloned()) else { return };
            if let Err((code, message)) = (app.system.cancel_connection)(&selected) {
                let (k, e) = (format!("네트워크 드라이브 연결 해제에 실패했습니다. Windows 오류 {code}: {message}"), format!("Could not disconnect the network drive. Windows error {code}: {message}"));
                return app.warn(&k, &e);
            }
            // 원본 FUN_004350bc: 활성 패널이 로컬 모드(0)일 때만 새로 읽는다.
            super::panes::DriveListCache::invalidate(&app.drive_list);
            if app.panes[app.active].archive.is_none() && app.panes[app.active].remote.is_none() {
                app.refresh(app.active);
            }
        });
    }

    /// C# ToggleExplorerIntegrationAsync: 등록돼 있으면 제거를, 아니면 추가를 묻는다.
    pub(super) fn explorer_integration_command(&mut self) {
        let caption = self.tr("탐색기 통합", "Explorer integration").into_owned();
        let enabled = (self.system.explorer_enabled)();
        let question = if enabled {
            self.tr("NDR의 탐색기 폴더 메뉴 등록을 제거할까요?", "Remove NDR from the Explorer folder menu?")
        } else {
            self.tr("현재 계정의 Windows 탐색기 폴더 메뉴에 NDR을 추가할까요?", "Add NDR to the Windows Explorer folder menu for the current account?")
        }
        .into_owned();
        self.system_box(&caption, &question, MB_YESNO | MB_ICONQUESTION, move |app, answer, _| {
            if answer != IDYES {
                return;
            }
            match (app.system.set_explorer)(!enabled) {
                Ok(()) => {
                    app.status = if enabled { app.tr("탐색기 메뉴 등록을 제거했습니다.", "Removed NDR from the Explorer menu.") } else { app.tr("탐색기 메뉴에 NDR을 추가했습니다.", "Added NDR to the Explorer menu.") }.into_owned();
                }
                Err(error) => app.system_box("NDR", &error, MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {}),
            }
        });
    }

    /// C# ImportLegacySettingsAsync: INI 고르기 → 설정 바꾸기, 옆의 toolbar.xml도 가져오기(있던 것은 .backup-시각으로 보관),
    /// 언어·테마·두 패널·시작 위치를 다시 적용.
    pub(super) fn import_legacy_command(&mut self) {
        let ko = self.language == "ko";
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
        let suggested = std::path::absolute(exe_dir.join("..").join("nexusfile").join("default")).unwrap_or_default();
        let initial = if suggested.is_dir() { suggested } else { PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Documents") };
        let title = if ko { "NexusFile INI 설정 가져오기" } else { "Import NexusFile INI settings" };
        let filters: [(&str, &str); 2] = if ko { [("NexusFile 설정 (*.ini)", "*.ini"), ("모든 파일 (*.*)", "*.*")] } else { [("NexusFile settings (*.ini)", "*.ini"), ("All files (*.*)", "*.*")] };
        let Some(path) = (self.open_dialog)(title, Some(&initial), "nexusfile.ini", &filters) else { return };
        let imported = match compat::import_ini(&path) {
            Ok(settings) => settings,
            Err(error) => return self.system_box("NDR", &error, MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {}),
        };
        import_toolbar_beside(&path, &self.profile);
        self.settings = imported;
        self.settings.normalize();
        self.profile_load_error = false;
        self.language = self.settings.text("Language", &self.language);
        self.dark = self.settings.text("Theme", "Black") != "Light";
        self.dual = self.settings.flag("DualPaneMode", true);
        self.active = usize::from(self.dual && self.settings.flag("StartWithRightPaneActive", false));
        self.preferences_dirty = true;
        self.toolbar = ndr::commands::load_toolbar(&self.profile, include_bytes!("../../Resources/toolbar.xml")).unwrap_or(ndr::commands::ToolbarConfig { items: vec![], warning: None });
        for (index, side) in ["Left", "Right"].into_iter().enumerate() {
            self.panes[index].show_hidden = self.settings.flag(&format!("{side}ShowHiddenFiles"), false);
            let start = self.settings.text(&format!("{side}StartPath"), "");
            let path = if self.settings.flag(&format!("{side}StartPathEnabled"), false) && !start.trim().is_empty() { start } else { self.settings.text(&format!("{side}Path"), "") };
            if !path.trim().is_empty() {
                self.navigate(index, PathBuf::from(path), true);
            }
            self.refresh(index);
        }
        self.save();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.status = if self.language == "ko" { format!("레거시 설정을 가져왔습니다: {name}") } else { format!("Imported legacy settings: {name}") };
    }

    /// C# InstallSelectedArchive: 압축 파일에 셸 동사 "install".
    pub(super) fn install_archive_command(&mut self) {
        let i = self.active;
        if self.panes[i].archive.is_some() {
            return self.warn("압축 파일 안의 항목을 설치할 수 없습니다.", "Items inside an archive cannot be installed.");
        }
        let item = self.target_entries(i).into_iter().find(|e| !e.is_dir && ndr::archive::archive_format(&e.path).is_some());
        let Some(item) = item else {
            return self.warn("설치할 압축 파일을 선택하세요.", "Select an archive to install.");
        };
        if let Err(error) = (self.system.run_verb)(&item.path, "install") {
            self.system_box("NDR", &error, MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
        }
    }
}

/// NexusFileToolbarService.ImportBesideLegacyIni: INI 옆 toolbar.xml이 읽히면 프로필로 복사(있던 파일은 .backup-yyyyMMdd-HHmmss[-N]).
pub(super) fn import_toolbar_beside(ini: &Path, profile: &Path) -> Option<Result<(), String>> {
    let source = ini.parent()?.join("toolbar.xml");
    if !source.is_file() {
        return None;
    }
    Some((|| {
        let bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
        ndr::commands::parse_toolbar(&bytes)?;
        std::fs::create_dir_all(profile).map_err(|e| e.to_string())?;
        let destination = profile.join("toolbar.xml");
        if destination.is_file() {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
            let mut backup = PathBuf::from(format!("{}.backup-{stamp}", destination.display()));
            let mut suffix = 1;
            while backup.exists() {
                backup = PathBuf::from(format!("{}.backup-{stamp}-{suffix}", destination.display()));
                suffix += 1;
            }
            std::fs::copy(&destination, &backup).map_err(|e| e.to_string())?;
        }
        std::fs::copy(&source, &destination).map_err(|e| e.to_string())?;
        Ok(())
    })())
}
