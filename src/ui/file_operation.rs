//! C# FileConflictWindow(같은 이름 확인), FileOperationProgressWindow(파일 작업 진행), FileOperationErrorDialog(오류마다 계속할지),
//! ReportFileOperationResult(복사·이동·붙여넣기 결과), 삭제·완전 삭제 결과, TryExternalCopyAsync(외부 복사 프로그램).
//! 차이(HANDOFF): 영어 화면은 C#이 한국어로 고정한 글(진행 창 단추·상태, 결과 알림)도 영어로 보인다.
//! 압축·분할·합치기·폴더 크기 작업의 진행 창에는 일시 정지 단추가 없다(그 작업들은 멈춤 지점이 없다).
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{IDYES, MB_ICONASTERISK, MB_ICONEXCLAMATION, MB_ICONHAND, MB_ICONQUESTION, MB_OK, MB_YESNO};
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::time::SystemTime;

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// C# FileConflictInfo + 모두 적용 확인란.
pub(super) struct ConflictForm {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub source_dir: bool,
    pub destination_dir: bool,
    pub source_size: Option<u64>,
    pub destination_size: Option<u64>,
    pub source_modified: Option<SystemTime>,
    pub destination_modified: Option<SystemTime>,
    pub allow_resume: bool,
    pub apply_all: bool,
    /// 원본 imgOrg·imgNew(32x32 셸 아이콘): [기존, 새 것]의 RGBA와 그린 텍스처.
    pub icons: [Option<(usize, usize, Vec<u8>)>; 2],
    pub textures: [Option<egui::TextureHandle>; 2],
}

fn describe(path: &Path) -> (bool, Option<u64>, Option<SystemTime>) {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            let attributes = m.file_attributes();
            let directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
            let reparse = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
            (directory, (!directory && !reparse).then(|| m.len()), m.modified().ok())
        }
        Err(_) => (false, None, None),
    }
}

impl ConflictForm {
    pub(super) fn read(source: &Path, destination: &Path, allow_resume: bool) -> Self {
        let (source_dir, source_size, source_modified) = describe(source);
        let (destination_dir, destination_size, destination_modified) = describe(destination);
        Self { source: source.into(), destination: destination.into(), source_dir, destination_dir, source_size, destination_size, source_modified, destination_modified, allow_resume, apply_all: false, icons: [None, None], textures: [None, None] }
    }
    /// C# CanResume.
    pub(super) fn can_resume(&self) -> bool {
        self.allow_resume && !self.source_dir && !self.destination_dir && matches!((self.source_size, self.destination_size), (Some(s), Some(d)) if d < s)
    }
    /// C# ConflictPolicy.IsCompatible: 모두 적용한 답을 이 충돌에도 쓸 수 있는가.
    pub(super) fn compatible(&self, choice: ConflictChoice) -> bool {
        match choice {
            ConflictChoice::Resume => self.can_resume(),
            ConflictChoice::ReplaceFolder => self.source_dir || self.destination_dir,
            _ => true,
        }
    }
}

/// 원본 FUN_00422068: 파일 정보 글 "<b>%s</b> bytes ( 긴 날짜, 시간 )"(크기는 FloatToStrF ffNumber = 천 단위 쉼표).
/// highlight면 FUN_00422314처럼 <c0000FF>…</c>(빨강, 더 새로운 쪽).
pub(super) fn conflict_file_info(size: Option<u64>, modified: Option<SystemTime>, highlight: bool, ko: bool) -> String {
    let bytes = size.map(|s| format!("<b>{}</b> bytes", n0(s as usize))).unwrap_or_else(|| (if ko { "크기 확인 불가" } else { "Size unavailable" }).to_owned());
    let info = match modified.map(ndr::listing::long_date_and_time) {
        Some((date, time)) => format!("{bytes} ( {date}, {time} )"),
        None => bytes,
    };
    if highlight { format!("<c0000FF>{info}</c>") } else { info }
}

/// 정수에 천 단위 쉼표(C# N0).
pub(super) fn n0(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 원본 TfFileExists의 단추(DFM 오른쪽 열, 왼쪽 425, 너비 112, 높이 25).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConflictButton {
    Resume,
    Overwrite,
    Skip,
    OverwriteNewer,
    ReplaceFolder,
    Rename,
    Cancel,
}

impl ConflictButton {
    /// DFM Top(원본 btnResume 25, btnOverwrite 56, btnSkip 87, btnOverNewer·btnReplace 118, btnRename 149, btnAbort 220).
    pub(super) fn top(self) -> f32 {
        match self {
            Self::Resume => 25.0,
            Self::Overwrite => 56.0,
            Self::Skip => 87.0,
            Self::OverwriteNewer | Self::ReplaceFolder => 118.0,
            Self::Rename => 149.0,
            Self::Cancel => 220.0,
        }
    }
    pub(super) fn choice(self) -> ConflictChoice {
        match self {
            Self::Resume => ConflictChoice::Resume,
            Self::Overwrite => ConflictChoice::Overwrite,
            Self::Skip => ConflictChoice::Skip,
            Self::OverwriteNewer => ConflictChoice::OverwriteNewer,
            Self::ReplaceFolder => ConflictChoice::ReplaceFolder,
            Self::Rename => ConflictChoice::Rename,
            Self::Cancel => ConflictChoice::Cancel,
        }
    }
}

/// xilesShell::IsSubFolder(parent, child)(0x7b543c): child가 parent 안(같은 경로 아님)이면 true. 대소문자 무시.
pub(super) fn is_sub_folder(parent: &Path, child: &Path) -> bool {
    let normal = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let (parent, child) = (normal(parent), normal(child));
    child.len() > parent.len() && child.starts_with(&parent) && child.as_bytes().get(parent.len()) == Some(&b'\\')
}

/// 보이는 단추. 원본 FUN_004116dc(로컬 복사): 새 항목이 폴더면 btnReplace, 아니면 btnOverNewer(같은 자리).
/// FUN_00422870: 기존 = 새 경로면 덮어쓰기·새 파일 덮어쓰기·폴더 교체를 숨기고, 폴더 교체가 보일 때 새 항목이 기존 안이면 폴더 교체를,
/// 기존이 새 항목 안이면 폴더 교체와 덮어쓰기를 숨긴다. 이어서 전송(btnResume)은 C# 판처럼 이어 받을 수 있을 때만.
pub(super) fn conflict_buttons(form: &ConflictForm) -> Vec<ConflictButton> {
    let same = form.source.to_string_lossy().eq_ignore_ascii_case(&form.destination.to_string_lossy());
    let mut overwrite = !same;
    let mut replace = form.source_dir && !same;
    let newer = !form.source_dir && !same;
    if replace {
        if is_sub_folder(&form.destination, &form.source) {
            replace = false;
        }
        if is_sub_folder(&form.source, &form.destination) {
            replace = false;
            overwrite = false;
        }
    }
    let mut buttons = Vec::new();
    if form.can_resume() {
        buttons.push(ConflictButton::Resume);
    }
    if overwrite {
        buttons.push(ConflictButton::Overwrite);
    }
    buttons.push(ConflictButton::Skip);
    if newer {
        buttons.push(ConflictButton::OverwriteNewer);
    }
    if replace {
        buttons.push(ConflictButton::ReplaceFolder);
    }
    buttons.push(ConflictButton::Rename);
    buttons.push(ConflictButton::Cancel);
    buttons
}

/// 원본 TfFileExists(ClientWidth 545 × ClientHeight 253) 배치: lMsg(20,13), lFileName(20,34 굵게),
/// Panel1(17,56 396×155: lOrgLabel·imgOrg·lOrgDir·lOrgFile, Bevel1, lNewLabel·imgNew·lNewDir·lNewFile), chkApplyAll(20,221),
/// 오른쪽 단추 열(conflict_buttons). Enter = 건너뛰기(C# 판), Esc = 취소(창 닫기).
pub(super) fn conflict_ui(ui: &mut egui::Ui, form: &mut ConflictForm, language: &compat::Language, ko: bool, enter: bool) -> Option<ConflictChoice> {
    let text = |key: &str, fallback: &str| language.text(&format!("formFileExists.{key}"), fallback);
    let (area, _) = ui.allocate_exact_size(egui::vec2(545.0, 253.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let text_color = ui.visuals().text_color();
    super::original_dialogs::put_left(ui, at(20.0, 13.0, 390.0, 15.0), egui::Label::new(text("Caption.lMsg", "A destination folder already contains a file/folder named")).truncate());
    let name = form.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    super::original_dialogs::put_left(ui, at(20.0, 34.0, 390.0, 17.0), egui::Label::new(RichText::new(name).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()))).truncate());
    // Panel1(TNXPaintPanel, CustomDraw): 테두리 칸.
    let panel = at(17.0, 56.0, 396.0, 155.0);
    ui.painter().rect(panel, 4.0, ui.visuals().faint_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    ui.painter().hline(panel.min.x + 11.0..=panel.min.x + 382.0, panel.min.y + 72.0, ui.visuals().widgets.noninteractive.bg_stroke);
    for (slot, icon) in form.icons.iter().enumerate() {
        if form.textures[slot].is_none() {
            if let Some((width, height, rgba)) = icon.as_ref().filter(|(w, h, rgba)| rgba.len() == w * h * 4) {
                let image = egui::ColorImage::from_rgba_unmultiplied([*width, *height], rgba);
                form.textures[slot] = Some(ui.ctx().load_texture(format!("conflict-icon-{slot}"), image, Default::default()));
            }
        }
    }
    // 원본 FUN_00422314: CompareFileTime(기존, 새 것) — 더 새로운 쪽의 정보 글을 <c0000FF>(빨강)로 감싼다.
    let newer = match (form.destination_modified, form.source_modified) {
        (Some(existing), Some(new)) if existing < new => Some(false),
        (Some(existing), Some(new)) if existing > new => Some(true),
        _ => None,
    };
    let rows = [
        (0.0, text("Caption.lOrgLabel", "Existing File:"), form.destination.clone(), form.destination_dir, form.destination_size, form.destination_modified, newer == Some(true)),
        (77.0, text("Caption.lNewLabel", "Copying File:"), form.source.clone(), form.source_dir, form.source_size, form.source_modified, newer == Some(false)),
    ];
    for (slot, (dy, label, path, directory, size, modified, highlight)) in rows.into_iter().enumerate() {
        let local = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(panel.min + egui::vec2(x, y + dy), egui::vec2(w, h));
        super::original_dialogs::put_left(ui, local(14.0, 9.0, 370.0, 15.0), egui::Label::new(RichText::new(label).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()))).truncate());
        if let Some(texture) = &form.textures[slot] {
            ui.painter().image(texture.id(), local(14.0, 31.0 - if slot == 1 { 1.0 } else { 0.0 }, 32.0, 32.0), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        // 원본 lOrgDir/lNewDir: 파일 이름을 뗀 폴더(끝 "\" 포함, FUN_00421f40), epPathEllipsis.
        let folder = path.parent().map(|p| { let mut s = p.to_string_lossy().into_owned(); if !s.ends_with('\\') { s.push('\\'); } s }).unwrap_or_default();
        let folder_font = egui::FontId::proportional(13.0);
        let folder = super::original_dialogs::path_ellipsis(ui, &folder, 324.0, &folder_font);
        super::original_dialogs::put_left(ui, local(62.0, 30.0, 324.0, 18.0), egui::Label::new(RichText::new(folder).font(folder_font)));
        // 원본 FUN_00422314: 폴더 충돌(+0x460)이면 정보 글이 전체 경로, 아니면 FUN_00422068 글. 더 새로운 쪽은 빨강.
        let info = if directory {
            let full = path.to_string_lossy().into_owned();
            if highlight { format!("<c0000FF>{full}</c>") } else { full }
        } else {
            conflict_file_info(size, modified, highlight, ko)
        };
        super::original_dialogs::put_left(ui, local(62.0, if slot == 0 { 50.0 } else { 49.0 }, 324.0, 18.0), egui::Label::new(super::original_dialogs::markup_job(&info, 13.0, text_color, 324.0)).truncate());
    }
    // 원본 TfFileExists는 단추·확인란만 있어 글자만 쳐도 단축 문자가 먹는다(VCL CM_DIALOGCHAR).
    let raw = |key: &str, fallback: &str| language.raw(&format!("formFileExists.{key}")).unwrap_or_else(|| fallback.to_owned());
    super::original_dialogs::access_checkbox(ui, at(20.0, 221.0, 390.0, 17.0), &mut form.apply_all, &raw("Caption.chkApplyAll", "&Apply to all"));
    let mut chosen = None;
    for button in conflict_buttons(form) {
        // 원본 FUN_00422314: 폴더 충돌이면 덮어쓰기 단추 글이 MERGE_FOLDER("폴더 합치기(&G)")다.
        let label = match button {
            ConflictButton::Overwrite if form.source_dir && form.destination_dir => language.raw("RES.MERGE_FOLDER").unwrap_or_else(|| "Mer&ge".into()),
            ConflictButton::Overwrite => raw("Caption.btnOverwrite", "Over&write"),
            ConflictButton::Resume => raw("Caption.btnResume", "Res&ume"),
            ConflictButton::Skip => raw("Caption.btnSkip", "&Skip"),
            ConflictButton::OverwriteNewer => raw("Caption.btnOverNewer", "Overwrite N&ewer"),
            ConflictButton::ReplaceFolder => raw("Caption.btnReplace", "Replac&e Folder"),
            ConflictButton::Rename => raw("Caption.btnRename", "&Rename"),
            ConflictButton::Cancel => raw("Caption.btnAbort", "&Cancel"),
        };
        if super::original_dialogs::access_button(ui, at(425.0, button.top(), 112.0, 25.0), &label) && chosen.is_none() {
            chosen = Some(button.choice());
        }
    }
    if enter && chosen.is_none() {
        chosen = Some(ConflictChoice::Skip);
    }
    chosen
}

/// 작업이 끝나면 쓰는 C# 결과 알림 방식.
#[derive(Clone, Debug)]
pub(super) enum JobReport {
    /// 상태 줄 요약, 오류가 있으면 오류 글 창(압축·분할·합치기·폴더 크기 등).
    Generic,
    /// C# ReportFileOperationResult(verb). requested = 고른 항목 수.
    Transfer { verb: (&'static str, &'static str), requested: usize, clear_clipboard: bool },
    /// C# DeleteSelectedAsync 끝: skipped = 남기기로 한 보호 항목 수.
    Delete { recycle: bool, skipped: usize },
    /// C# ShredAsync 끝.
    Shred,
    /// 원본 TfCleanup 정리 끝(FUN_0041c77c): 정리 창을 다시 켜고 MSG_CLEANUP_DONE(정리 창 제목, MB_ICONINFORMATION).
    Cleanup,
}

/// C# FileOperationProgressWindow 인수.
#[derive(Clone, Debug, Default)]
pub(super) struct ProgressInfo {
    pub from: Option<String>,
    pub to: Option<String>,
    pub total_files: Option<u64>,
    pub total_bytes: Option<u64>,
    pub prefer_bytes: bool,
    pub allow_pause: bool,
    /// 원본 진행 창 캡션(COMPRESSING·EXTRACTING·SPLITTING·JOINING). 없으면 작업 종류로 고르거나 작업 제목.
    pub caption: Option<String>,
}

/// C# FormatDuration.
/// 원본 0x412308 lTo: 지금 대상 파일의 폴더. 작업이 지금 원본 파일(`current`)의 대상을 알려 왔으면(Event::Target) 그 폴더,
/// 아니면 시작 원본 폴더(`from`) 아래의 같은 상대 경로를 시작 대상 폴더(`to`)에 붙인 폴더(얻을 수 없으면 `to`).
pub(super) fn progress_target_line(from: Option<&str>, to: Option<&str>, current: &str, target: Option<&(PathBuf, PathBuf)>) -> Option<String> {
    if let Some((_, destination)) = target.filter(|(source, _)| source.as_os_str() == std::ffi::OsStr::new(current)) {
        if let Some(folder) = destination.parent() {
            return Some(folder.to_string_lossy().into_owned());
        }
    }
    let folder = Some(Path::new(current)).filter(|p| p.is_absolute()).and_then(Path::parent);
    match (from, to, folder) {
        (Some(base), Some(target), Some(folder)) => match folder.strip_prefix(base) {
            Ok(relative) if relative.as_os_str().is_empty() => Some(target.to_owned()),
            Ok(relative) => Some(Path::new(target).join(relative).to_string_lossy().into_owned()),
            Err(_) => Some(target.to_owned()),
        },
        _ => to.map(str::to_owned),
    }
}

fn format_duration(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (days, hours, minutes, secs) = (total / 86_400, total / 3600 % 24, total / 60 % 60, total % 60);
    if days >= 1 { format!("{days}.{hours:02}:{minutes:02}:{secs:02}") } else { format!("{hours:02}:{minutes:02}:{secs:02}") }
}

/// C# GetFileOperationSourcePath: 폴더면 그 폴더, 파일이면 들어 있는 폴더.
pub(super) fn source_folder(path: &Path) -> Option<String> {
    let full = std::path::absolute(path).ok()?;
    let metadata = fs::metadata(&full).ok()?;
    let folder = if metadata.is_dir() { full } else { full.parent()?.to_path_buf() };
    Some(folder.to_string_lossy().into_owned())
}

/// C# TryGetFlatFileProgressTotals: 모두 보통 파일일 때만 (개수, 크기).
pub(super) fn flat_totals(paths: &[PathBuf]) -> Option<(u64, u64)> {
    let mut bytes = 0u64;
    for path in paths {
        let m = fs::symlink_metadata(path).ok()?;
        if m.file_attributes() & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return None;
        }
        bytes = bytes.checked_add(m.len())?;
    }
    (!paths.is_empty()).then_some((paths.len() as u64, bytes))
}

/// 복사·이동을 부른 곳(C#의 상태 글이 다르다).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransferKind {
    OtherPane,
    Folder,
    Paste,
    Drag,
}

impl App {
    /// 원본 FUN_0042bdc4(복사·이동)·FUN_0042c734(삭제): 표시에서 작업을 만들면(끌어 놓기·붙여넣기가 아니면) 그 패널의 표시를
    /// "마지막 선택"(+0xd08, CopySelection)에 남기고 표시를 지운다(ClearSelection). 표시가 없으면 마지막 선택도 빈다. 스킨 테마.
    pub(super) fn original_take_marks(&mut self, sources: &[PathBuf]) {
        let Some(parent) = sources.first().and_then(|p| p.parent()) else { return };
        let order = [self.active, 1 - self.active];
        let Some(i) = order.into_iter().find(|&i| {
            let pane = &self.panes[i];
            pane.archive.is_none() && pane.remote.is_none() && ndr::names::equals_ignore_case(&pane.path.to_string_lossy(), &parent.to_string_lossy())
        }) else {
            return;
        };
        self.original_take_pane_marks(i);
    }

    /// original_take_marks의 패널을 아는 판(끌어 놓기: 원본 FUN_004856d8이 놓기 뒤 끈 목록에 CopySelection + ClearSelection). 스킨 테마.
    pub(super) fn original_take_pane_marks(&mut self, i: usize) {
        if i > 1 {
            return;
        }
        let pane = &mut self.panes[i];
        self.last_selection[i] = pane.selected.iter().filter_map(|&row| pane.entries.get(row)).filter(|e| !e.drive && !e.is_parent()).map(|e| e.path.clone()).collect();
        pane.selected.clear();
    }

    /// C# RunLocalFileOperationWithProgress(title, paths, destination, move): 진행 창과 결과 알림. 외부 복사 프로그램이
    /// 설정돼 있으면 그쪽으로 넘긴다(C# TryExternalCopyAsync).
    pub(super) fn local_transfer(&mut self, sources: Vec<PathBuf>, destination: PathBuf, moving: bool, kind: TransferKind) {
        if matches!(kind, TransferKind::OtherPane | TransferKind::Folder) {
            self.original_take_marks(&sources);
        }
        if self.try_external_copy(&sources, &destination, moving, kind) {
            return;
        }
        self.remember_destination(&destination);
        let ko = self.language == "ko";
        let (title, verb) = match (moving, kind) {
            (true, _) => (if ko { "항목 이동" } else { "Move items" }, ("이동", "move")),
            (false, TransferKind::Paste) => (if ko { "항목 붙여넣기" } else { "Paste items" }, ("붙여넣기", "paste")),
            (false, _) => (if ko { "항목 복사" } else { "Copy items" }, ("복사", "copy")),
        };
        let totals = flat_totals(&sources);
        let progress = ProgressInfo {
            from: if sources.len() == 1 { source_folder(&sources[0]) } else { None },
            to: Some(destination.to_string_lossy().into_owned()),
            total_files: totals.map(|t| t.0),
            total_bytes: totals.map(|t| t.1),
            prefer_bytes: !moving,
            allow_pause: true,
            caption: None,
        };
        let report = JobReport::Transfer { verb, requested: sources.len(), clear_clipboard: moving && kind == TransferKind::Paste };
        self.start_job(title.into(), Operation::Copy { sources, destination, move_files: moving }, report, Some(progress));
    }

    /// 충돌 질문: 모두 적용한 답이 맞으면 바로, 아니면 FileConflictWindow.
    pub(super) fn ask_file_conflict(&mut self, source: PathBuf, destination: PathBuf, allow_resume: bool, reply: Sender<ConflictChoice>) {
        let mut form = ConflictForm::read(&source, &destination, allow_resume);
        if let Some(choice) = self.conflict_all.filter(|c| form.compatible(*c)) {
            let _ = reply.send(choice);
            return;
        }
        // 원본 FUN_00422314: imgOrg = 기존 항목, imgNew = 복사하려는 항목의 셸 아이콘(SHGetFileInfoW SHGFI_ICON).
        form.icons = [(self.system.large_icon)(&destination), (self.system.large_icon)(&source)];
        let title = self.language_pack.text("formFileExists.Form.Caption", "Confirm File/Folder Replace");
        self.push_original_dialog(title, DialogKind::FileConflict(Box::new(form)), move |app, result, _| {
            let choice = match result {
                DialogResult::FileConflict(Some((choice, all))) => {
                    if all && choice != ConflictChoice::Cancel {
                        app.conflict_all = Some(choice);
                    }
                    choice
                }
                _ => ConflictChoice::Cancel,
            };
            let _ = reply.send(choice);
        });
    }

    /// 원본 FUN_0041190c: TfGetText 모양의 창(TITLE_RENAME_AND_COPY, MSG_RENAME_AND_COPY의 %s = 원래 이름, 처음 값 = 원래 이름).
    pub(super) fn ask_rename_target(&mut self, name: String, reply: Sender<Option<String>>) {
        let title = self.language_pack.text("RES.TITLE_RENAME_AND_COPY", "Rename and Copy");
        let message = self
            .language_pack
            .text("RES.MSG_RENAME_AND_COPY", "Type a new name.\nThis file will be copied with a new name.\nOriginal filename:%s")
            .replace("%s", &name);
        self.prompt(&title, &message, &name, None, false, move |_, value, _| {
            let _ = reply.send(value);
        });
    }

    /// C# FileOperationErrorDialog: 오류 상자(TITLE_FILEOPERROR, 오류 아이콘), 계속할 수 있으면 MSG_WANTTOCONTINUE 예/아니요.
    pub(super) fn ask_file_failure(&mut self, path: PathBuf, message: String, can_continue: bool, reply: Sender<bool>) {
        let title = self.language_pack.text("RES.TITLE_FILEOPERROR", "File Operation Error");
        let detail = if message.trim().is_empty() { self.language_pack.text("RES.MSG_UNKNOWN_ERROR", "Unknown error occurred. Error number: %d").replace("%d", "0") } else { message };
        self.system_box(&title.clone(), &detail, MB_OK | MB_ICONHAND, move |app, _, _| {
            if !can_continue {
                let _ = reply.send(true);
                return;
            }
            let prompt = app
                .language_pack
                .text("RES.MSG_WANTTOCONTINUE", "%s\nError occurred while doing operation for this file. Do you want to keep working on the rest of the file/folder(s)?")
                .replace("%s", &path.to_string_lossy());
            app.system_box(&title, &prompt, MB_YESNO | MB_ICONQUESTION, move |_, answer, _| {
                let _ = reply.send(answer == IDYES);
            });
        });
    }

    /// 원본 TfProgress(ClientWidth 441 × ClientHeight 143): lFrom(20,23 353×16)·lTo(20,42 353×15) 경로 줄(epPathEllipsis),
    /// lFileName(20,71 317×17, 끝 생략), ProgressBar1(18,94 327×16), btnCancel(355,90 78×25), lRemaining(18,116), lCount(오른쪽 끝 345, 116).
    /// 원본 FUN_00412308(파일마다): lFrom = 원본 파일의 폴더(PathRemoveFileSpecW), lTo = 대상 폴더, lFileName = 파일 이름,
    /// lCount = "(%d)" 남은 항목 수(전체 - 끝난 것). FUN_00412558(바이트): 막대 = 백분율, 복사·이동이면 lRemaining =
    /// StrFormatByteSizeW(남은 바이트). FUN_00411cbc(준비): lFileName = MSG_*_PREPARING, 다른 글은 빈 글.
    /// C# 판에만 있는 것: 일시 정지 단추(btnCancel 위 355,60), 바이트 합계가 없는 작업의 남은 시간(lRemaining 자리), 전체 수를 모르면 끝난 수.
    /// 창 닫기(×, Esc)는 취소 요청이다(C# OnClosing). 위에 다른 창이 있으면 그리기만 한다.
    pub(super) fn file_progress_ui(&mut self, ctx: &egui::Context) {
        use super::original_dialogs::{path_ellipsis, put_enabled, put_left};
        let ko = self.language == "ko";
        let covered = !self.msgboxes.is_empty() || !self.original_dialogs.is_empty();
        let remaining_label = self.language_pack.text("formProgress.Caption.lRemaining", "Remaining:");
        let cancel_label = super::favorites_dialog::access_key(&self.language_pack.text("formProgress.Caption.btnCancel", "&Cancel")).0;
        let Some(job) = self.job.as_mut().filter(|j| !j.finished) else { return };
        let Some(info) = job.progress.clone() else { return };
        // 원본 줄: 처음 준 경로, 없으면 처음 알려 온 경로의 폴더.
        if info.from.is_none() && job.from_seen.is_none() && !job.path.is_empty() {
            job.from_seen = Path::new(&job.path).parent().map(|p| p.to_string_lossy().into_owned());
        }
        let from = info.from.clone().or_else(|| job.from_seen.clone());
        // 원본 0x412308: 파일마다 lFrom = 지금 원본 파일의 폴더, lTo = 지금 대상 파일의 폴더(PathRemoveFileSpecW).
        // 파일 작업(fsops)은 대상 파일을 알려 준다(Event::Target, 충돌로 바뀐 이름 포함). 알려 주지 않는 작업은
        // 시작 폴더 아래의 같은 상대 경로로 얻는다(얻을 수 없으면 시작 대상 폴더).
        let current_folder = Some(Path::new(&job.path)).filter(|p| p.is_absolute()).and_then(Path::parent).map(Path::to_path_buf);
        let to_line = progress_target_line(info.from.as_deref(), info.to.as_deref(), &job.path, job.target.as_ref());
        let from = current_folder.map(|f| f.to_string_lossy().into_owned()).or(from);
        let fraction = if info.prefer_bytes && info.total_bytes.is_some_and(|t| t > 0) {
            Some((job.bytes as f64 / info.total_bytes.unwrap_or(1) as f64).clamp(0.0, 1.0))
        } else if info.total_files.is_some_and(|t| t > 0) {
            Some((job.files as f64 / info.total_files.unwrap_or(1) as f64).clamp(0.0, 1.0))
        } else {
            None
        };
        // lRemaining: 원본은 남은 바이트(StrFormatByteSizeW). 바이트 합계가 없으면 C# 판의 남은 시간.
        let remaining = match info.total_bytes.filter(|t| *t > 0) {
            Some(total) if info.prefer_bytes => ndr::native::format_byte_size(total.saturating_sub(job.bytes)),
            _ => match fraction {
                Some(f) if f > 0.0 && f < 1.0 => format!("{remaining_label} {}", format_duration(job.started.elapsed().as_secs_f64() * (1.0 - f) / f)),
                _ => String::new(),
            },
        };
        // lCount: 원본 "(%d)" = 남은 항목 수. 전체를 모르면 C# 판처럼 끝난 수.
        let count = match info.total_files {
            Some(total) => format!("({})", n0(total.saturating_sub(job.files) as usize)),
            None => format!("({})", n0(job.files as usize)),
        };
        let cancelled = job.cancel.load(Ordering::Relaxed);
        let paused = job.pause.load(Ordering::Relaxed);
        // lFileName: 파일 이름. 준비 중이면 원본 MSG_*_PREPARING, 취소·일시 정지 안내는 C# 판 글.
        let file_line = if cancelled {
            (if ko { "작업을 취소하고 있습니다." } else { "Cancelling the operation." }).to_owned()
        } else if let Some(note) = &job.note {
            note.clone()
        } else if job.path.is_empty() {
            // 원본 FUN_00411cbc: 준비 창 글 = 종류별 MSG_COPY/MOVE/DEL_PREPARING. 그 밖의 작업은 C# 글.
            match &job.report {
                JobReport::Transfer { verb: (_, "move"), .. } => self.language_pack.text("RES.MSG_MOVE_PREPARING", "Preparing for moving file/folder(s)"),
                JobReport::Transfer { .. } => self.language_pack.text("RES.MSG_COPY_PREPARING", "Preparing for copying file/folder(s)"),
                JobReport::Delete { .. } | JobReport::Shred | JobReport::Cleanup => self.language_pack.text("RES.MSG_DEL_PREPARING", "Preparing for deleting file/folder(s)"),
                _ => (if ko { "작업을 준비하고 있습니다." } else { "Preparing the operation." }).to_owned(),
            }
        } else {
            Path::new(&job.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| job.path.clone())
        };
        let mut cancel = !covered && ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut toggle_pause = false;
        // 원본 진행 창 캡션(FUN_00411cbc 종류별): COPYING / MOVING / DELETING / RECYCLING. 그 밖의 작업은 C# 제목.
        let caption = match &job.report {
            _ if info.caption.is_some() => info.caption.clone().unwrap_or_default(),
            JobReport::Transfer { verb: (_, "move"), .. } => self.language_pack.text("RES.MOVING", "Moving..."),
            JobReport::Transfer { .. } => self.language_pack.text("RES.COPYING", "Copying..."),
            JobReport::Delete { recycle: true, .. } => self.language_pack.text("RES.RECYCLING", "Sending to the RecycleBin..."),
            JobReport::Delete { .. } | JobReport::Shred | JobReport::Cleanup => self.language_pack.text("RES.DELETING", "Deleting..."),
            _ => job.title.clone(),
        };
        super::keep_opaque(ctx, egui::Id::new("ndr-file-progress"));
        let progress_id = egui::Id::new("ndr-file-progress");
        super::movable_modal(ctx, progress_id).show(ctx, |ui| {
            ui.set_width(441.0);
            if super::modal_title_bar(ui, progress_id, &caption, None) {
                cancel = true;
            }
            ui.separator();
            let (area, _) = ui.allocate_exact_size(egui::vec2(441.0, 143.0), egui::Sense::hover());
            let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
            let font = egui::FontId::proportional(13.0);
            for (top, height, value) in [(23.0, 16.0, from.as_deref()), (42.0, 15.0, to_line.as_deref())] {
                if let Some(value) = value.filter(|v| !v.trim().is_empty()) {
                    let shown = path_ellipsis(ui, value, 353.0, &font);
                    put_left(ui, at(20.0, top, 353.0, height), egui::Label::new(RichText::new(shown).font(font.clone())));
                }
            }
            put_left(ui, at(20.0, 71.0, 317.0, 17.0), egui::Label::new(RichText::new(&file_line).font(font.clone())).truncate());
            let bar = match fraction {
                Some(value) => egui::ProgressBar::new(value as f32),
                None => egui::ProgressBar::new(0.0).animate(true),
            };
            ui.put(at(18.0, 94.0, 327.0, 16.0), bar.desired_height(16.0));
            if info.allow_pause {
                let label = if paused { if ko { "계속" } else { "Resume" } } else if ko { "일시 정지" } else { "Pause" };
                if put_enabled(ui, at(355.0, 60.0, 78.0, 25.0), !cancelled, egui::Button::new(label)).clicked() {
                    toggle_pause = true;
                }
            }
            if put_enabled(ui, at(355.0, 90.0, 78.0, 25.0), !cancelled, egui::Button::new(&cancel_label)).clicked() {
                cancel = true;
            }
            put_left(ui, at(18.0, 116.0, 268.0, 14.0), egui::Label::new(RichText::new(&remaining).size(12.0)).truncate());
            ui.painter().text(area.min + egui::vec2(345.0, 116.0), egui::Align2::RIGHT_TOP, &count, egui::FontId::proportional(12.0), ui.visuals().text_color());
        });
        if covered {
            return;
        }
        if toggle_pause {
            let now = !job.pause.load(Ordering::Relaxed);
            job.pause.store(now, Ordering::Relaxed);
            job.note = Some(match (now, ko) {
                (true, true) => "작업을 일시 정지했습니다.".into(),
                (true, false) => "The operation is paused.".into(),
                (false, true) => "작업을 계속합니다.".into(),
                (false, false) => "Continuing the operation.".into(),
            });
        }
        if cancel {
            job.pause.store(false, Ordering::Relaxed);
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// 작업이 끝났을 때(Event::Finished) 결과 알림. job은 끝난 채로 남는다(다음 작업을 막지 않는다).
    pub(super) fn report_job(&mut self, cancelled: bool) {
        let ko = self.language == "ko";
        let Some(job) = self.job.as_ref() else { return };
        let report = job.report.clone();
        let errors = job.errors.clone();
        let fatal = job.fatal.clone();
        let (files, interactive, skipped, title) = (job.files, job.interactive, job.skipped, job.title.clone());
        let (chime, custom_status, bytes) = (job.chime, job.custom_status, job.bytes);
        let unreported: Vec<String> = errors.iter().skip(interactive.min(errors.len())).cloned().collect();
        let sound = |app: &App| {
            // 원본 0x43d9fc(completesound): PlaySoundW("<프로그램 폴더>\complete.wav", 0x22003). 낼 수 없으면 C#처럼 MessageBeep.
            if app.settings.flag("PlayCompletionSound", false) && !(app.system.play_wave)(Some(&app.resources.join("complete.wav"))) {
                native::completion_sound();
            }
        };
        // 압축을 마쳤으면 만든 압축 파일이 있는 패널의 새 목록에서 그 파일에 커서를 둔다.
        if let Some((pane, output)) = self.archive_focus.take() {
            let in_pane = output.parent().is_some_and(|folder| ndr::names::equals_ignore_case(&folder.to_string_lossy(), &self.panes[pane].path.to_string_lossy()));
            if !cancelled && fatal.is_none() && output.exists() && in_pane && self.panes[pane].archive.is_none() {
                self.pending_focus[pane] = output.file_name().map(|n| n.to_string_lossy().into_owned());
            }
        }
        self.refresh(0);
        self.refresh(1);
        self.ftp_queue_done();
        match report {
            JobReport::Generic => {
                if chime && !cancelled && errors.is_empty() && fatal.is_none() {
                    sound(self);
                }
                if let Some(message) = &fatal {
                    let shown = self.archive_error_text(message);
                    self.system_box("NDR", &shown, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                }
                if custom_status {
                    return;
                }
                self.status = format!(
                    "{title}: {} files, {}{}{}",
                    files,
                    format_size(bytes),
                    if cancelled { " (cancelled)" } else { "" },
                    if errors.is_empty() { String::new() } else { format!(" · {} errors", errors.len()) }
                );
                if !errors.is_empty() {
                    self.show_error_text(&title, &errors);
                }
            }
            JobReport::Transfer { verb, requested, clear_clipboard } => {
                let v = if ko { verb.0 } else { verb.1 };
                if let Some(message) = fatal {
                    let caption = if ko { format!("{v} 오류") } else { format!("{} error", capitalize(v)) };
                    self.system_box(&caption, &message, MB_OK | MB_ICONHAND, |_, _, _| {});
                    self.status = if ko { format!("{v} 작업을 마치지 못했습니다.") } else { format!("The {v} operation did not finish.") };
                    return;
                }
                if cancelled {
                    self.status = if ko { format!("{v} 작업을 취소했습니다. 완료된 항목은 유지됩니다.") } else { format!("The {v} operation was cancelled. Completed items were kept.") };
                    return;
                }
                if !errors.is_empty() {
                    let mut details = unreported.iter().take(30).cloned().collect::<Vec<_>>().join("\r\n");
                    if unreported.len() > 30 {
                        details += &if ko { format!("\r\n외 {}건", n0(unreported.len() - 30)) } else { format!("\r\nand {} more", n0(unreported.len() - 30)) };
                    }
                    if skipped > 0 {
                        details += &if ko { format!("\r\n\r\n사용자 선택으로 건너뛴 항목: {}개", n0(skipped)) } else { format!("\r\n\r\nItems skipped by choice: {}", n0(skipped)) };
                    }
                    if !unreported.is_empty() || skipped > 0 {
                        let caption = if ko { format!("일부 {v} 실패 ({}건)", n0(errors.len())) } else { format!("Some items failed to {v} ({})", n0(errors.len())) };
                        self.system_box(&caption, &details, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                    }
                    self.status = if ko {
                        format!("{v} 완료: 요청 {}개, 실패 {}건, 건너뜀 {}개", n0(requested), n0(errors.len()), n0(skipped))
                    } else {
                        format!("{} finished: {} requested, {} failed, {} skipped", capitalize(v), n0(requested), n0(errors.len()), n0(skipped))
                    };
                    return;
                }
                if skipped > 0 {
                    self.status = if ko { format!("{v} 완료: {}개 항목을 건너뛰었습니다.", n0(skipped)) } else { format!("{} finished: skipped {} item(s).", capitalize(v), n0(skipped)) };
                    return;
                }
                if clear_clipboard {
                    self.clipboard = None;
                }
                self.status = if ko { format!("요청한 {}개 항목을 {v}했습니다.", n0(requested)) } else { format!("Finished: {v} {} requested item(s).", n0(requested)) };
                sound(self);
            }
            JobReport::Delete { recycle, skipped } => {
                let failed = errors.len() + usize::from(fatal.is_some());
                let summary = if skipped > 0 { if ko { format!(" 건너뜀 {}개", n0(skipped)) } else { format!(" Skipped {}.", n0(skipped)) } } else { String::new() };
                let completed = n0(files as usize);
                self.status = match (cancelled, recycle, ko) {
                    (true, _, true) => format!("삭제 작업을 중단했습니다. 완료 {completed}개, 실패 {}개", n0(failed)),
                    (true, _, false) => format!("Deletion stopped. Completed {completed}, failed {}.", n0(failed)),
                    (false, true, true) => format!("{completed}개 항목을 휴지통으로 이동했습니다.{summary} 실패 {}개", n0(failed)),
                    (false, true, false) => format!("Moved {completed} item(s) to the Recycle Bin.{summary} Failed {}.", n0(failed)),
                    (false, false, true) => format!("{completed}개 항목을 삭제했습니다.{summary} 실패 {}개", n0(failed)),
                    (false, false, false) => format!("Deleted {completed} item(s).{summary} Failed {}.", n0(failed)),
                };
                if !cancelled && failed == 0 && skipped == 0 && files > 0 {
                    sound(self);
                }
                let mut rest = unreported;
                rest.extend(fatal);
                if !rest.is_empty() {
                    let title = self.tr("삭제하지 못한 항목", "Items that could not be deleted").into_owned();
                    self.show_error_text(&title, &rest);
                }
            }
            JobReport::Shred => {
                let mut all = errors;
                all.extend(fatal);
                if all.is_empty() {
                    self.status = self.tr("완전 삭제 작업이 끝났습니다.", "Shredding finished.").into_owned();
                    sound(self);
                } else {
                    let title = self.tr("일부 항목을 완전히 삭제하지 못했습니다.", "Some items could not be shredded.").into_owned();
                    self.text = Some(TextDocument { path: None, title, text: all.join("\r\n"), editable: false, dirty: false });
                }
            }
            JobReport::Cleanup => {
                // 원본 FUN_0041c77c: 오류는 알리지 않고(플래그 0x80), 끝나면 MessageBoxW(MSG_CLEANUP_DONE, 정리 창 제목, MB_ICONINFORMATION).
                self.status.clear();
                if !cancelled {
                    let title = self.language_pack.text("formCleanup.Form.Caption", "Disk Clean-Up");
                    let text = self.language_pack.text("RES.MSG_CLEANUP_DONE", "Cleanup completed.");
                    self.system_box(&title, &text, MB_OK | MB_ICONASTERISK, |_, _, _| {});
                }
            }
        }
    }

    /// C# TextWindow(제목, 오류 500줄 + "… 나머지 N개 오류").
    fn show_error_text(&mut self, title: &str, errors: &[String]) {
        let mut text = errors.iter().take(500).map(|e| self.archive_error_text(e)).collect::<Vec<_>>().join("\r\n");
        if errors.len() > 500 {
            text += &if self.language == "ko" { format!("\r\n… 나머지 {}개 오류", n0(errors.len() - 500)) } else { format!("\r\n… {} more errors", n0(errors.len() - 500)) };
        }
        self.text = Some(TextDocument { path: None, title: title.into(), text, editable: false, dirty: false });
    }

    /// C# TryExternalCopyAsync: UseCopyHandler이고 CopyHandlerPath(환경 변수 풀기)가 있는 파일이면 그 프로그램에 넘기고 true.
    /// KillCopy(GUI)·kc(콘솔)는 ANSI 목록 파일, TeraCopy 4 이상은 여러 항목 복사에 SendTo, 나머지는 "Copy|Move 원본|*목록 대상 /Close".
    /// 프로그램이 끝나면 두 패널을 새로 고치고 C#처럼 상태 글을 쓴다. 실패는 경고 상자(C# RunSafely).
    fn try_external_copy(&mut self, sources: &[PathBuf], destination: &Path, moving: bool, kind: TransferKind) -> bool {
        if !self.settings.flag("UseCopyHandler", false) {
            return false;
        }
        let program = compat::expand_setting(&self.settings.text("CopyHandlerPath", ""));
        if program.trim().is_empty() || !Path::new(&program).is_file() {
            return false;
        }
        let mut paths: Vec<String> = Vec::new();
        for source in sources.iter().filter(|p| p.exists()) {
            let full = std::path::absolute(source).unwrap_or_else(|_| source.clone()).to_string_lossy().into_owned();
            if !paths.iter().any(|p| p.eq_ignore_ascii_case(&full)) {
                paths.push(full);
            }
        }
        if paths.is_empty() {
            return false;
        }
        let target = std::path::absolute(destination).unwrap_or_else(|_| destination.to_path_buf());
        self.remember_destination(&target);
        let target = target.to_string_lossy().into_owned();
        let ko = self.language == "ko";
        let name = Path::new(&program).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let kill_gui = name.contains("killcopy");
        let kill_console = name == "kc";
        let count = paths.len();
        self.status = if kill_gui || kill_console || !ko {
            format!("External copy handler is processing {} item(s).", n0(count))
        } else {
            format!("외부 복사 프로그램에서 {}개 항목을 처리하고 있습니다.", n0(count))
        };
        let done = match (kind, moving, ko) {
            (TransferKind::OtherPane, false, true) => Some(format!("{}개 항목을 복사했습니다.", n0(count))),
            (TransferKind::OtherPane, true, true) => Some(format!("{}개 항목을 이동했습니다.", n0(count))),
            (TransferKind::OtherPane, false, false) => Some(format!("Copied {} item(s).", n0(count))),
            (TransferKind::OtherPane, true, false) => Some(format!("Moved {} item(s).", n0(count))),
            (TransferKind::Drag, false, true) => Some(format!("드래그한 {}개 항목을 복사했습니다.", n0(count))),
            (TransferKind::Drag, true, true) => Some(format!("드래그한 {}개 항목을 이동했습니다.", n0(count))),
            (TransferKind::Drag, false, false) => Some(format!("Copied {} dragged item(s).", n0(count))),
            (TransferKind::Drag, true, false) => Some(format!("Moved {} dragged item(s).", n0(count))),
            _ => None,
        };
        let clear_clipboard = moving && kind == TransferKind::Paste;
        let runner = self.program_runner;
        let tx = self.async_tx.clone();
        let teracopy = !moving && paths.len() >= 2 && name.contains("teracopy") && native::file_major_version(Path::new(&program)).is_some_and(|v| v >= 4) && {
            // C#: 명령 줄 길이 어림(따옴표 포함) 30,000 미만.
            let estimate = program.len() * 2 + target.len() * 2 + 256 + paths.iter().map(|p| p.len() * 2 + 8).sum::<usize>();
            estimate < 30_000
        };
        thread::spawn(move || {
            let result = run_external_copy(runner, &program, &paths, &target, moving, kill_gui, kill_console, teracopy);
            let _ = tx.send(AsyncResult::ExternalCopy { result, status: done, clear_clipboard });
        });
        true
    }

    pub(super) fn external_copy_done(&mut self, result: Result<(), String>, status: Option<String>, clear_clipboard: bool) {
        self.refresh(0);
        self.refresh(1);
        match result {
            Ok(()) => {
                if clear_clipboard {
                    self.clipboard = None;
                }
                if let Some(status) = status {
                    self.status = status;
                }
            }
            Err(message) => self.system_box("NDR", &message, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        }
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// 임시 목록 파일 이름(C# ndr-copy-{Guid:N}).
fn list_path(extension: &str) -> PathBuf {
    let mut bytes = [0u8; 16];
    let _ = native::random_bytes(&mut bytes);
    std::env::temp_dir().join(format!("ndr-copy-{}.{extension}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

#[allow(clippy::too_many_arguments)]
fn run_external_copy(runner: fn(&Path, &[String], &Path) -> Result<i32, String>, program: &str, paths: &[String], target: &str, moving: bool, kill_gui: bool, kill_console: bool, teracopy: bool) -> Result<(), String> {
    let directory = Path::new(program).parent().map(Path::to_path_buf).unwrap_or_default();
    let mut list = None;
    let result = (|| {
        let mut args = Vec::new();
        if kill_gui || kill_console {
            // KillCopy는 현재 ANSI 코드 페이지로 목록을 읽는다.
            let mut bytes = Vec::new();
            for path in paths {
                bytes.extend(ndr::split::ansi_bytes(path).ok_or("KillCopy reads file lists using the current Windows ANSI code page, which cannot represent one or more selected paths. Use a Unicode-capable handler such as TeraCopy for these paths.")?);
                bytes.extend(b"\r\n");
            }
            let file = list_path("lst");
            fs::write(&file, bytes).map_err(|e| e.to_string())?;
            let shown = file.to_string_lossy().into_owned();
            list = Some(file);
            if kill_gui {
                args.push(format!("/{shown}/{target}/{}", if moving { " -m" } else { "" }));
            } else {
                args.push(shown);
                args.push(target.to_owned());
                if moving {
                    args.push("-m".into());
                }
            }
            let code = runner(Path::new(program), &args, &directory).map_err(|e| format!("KillCopy could not be started. {e}"))?;
            if code != 0 {
                return Err(format!("KillCopy exited with error code {code}."));
            }
            return Ok(());
        }
        if teracopy {
            args.extend(["SendTo".to_owned(), target.to_owned(), "/Close".to_owned()]);
            args.extend(paths.iter().cloned());
        } else {
            args.push(if moving { "Move" } else { "Copy" }.into());
            if paths.len() == 1 {
                args.push(paths[0].clone());
            } else {
                let file = list_path("txt");
                let mut text = String::from("\u{feff}");
                for path in paths {
                    text += path;
                    text += "\r\n";
                }
                fs::write(&file, text).map_err(|e| e.to_string())?;
                args.push(format!("*{}", file.display()));
                list = Some(file);
            }
            args.push(target.to_owned());
            args.push("/Close".into());
        }
        let code = runner(Path::new(program), &args, &directory)?;
        if code != 0 {
            return Err(format!("외부 복사 프로그램이 오류 코드 {code}로 종료되었습니다."));
        }
        Ok(())
    })();
    if let Some(file) = list {
        let _ = fs::remove_file(file);
    }
    result
}

/// 프로그램을 바로 실행하고 끝날 때까지 기다린다(C# UseShellExecute = false, WaitForExitAsync).
pub(super) fn run_program(program: &Path, args: &[String], directory: &Path) -> Result<i32, String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    if directory.is_dir() {
        command.current_dir(directory);
    }
    command.status().map(|s| s.code().unwrap_or(-1)).map_err(|e| e.to_string())
}
