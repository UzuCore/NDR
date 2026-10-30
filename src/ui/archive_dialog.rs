//! C# CreateArchiveAsync / AddToArchiveAsync(move) / CreateZipAsync / ExtractArchiveAsync(Selected·CurrentFolder·NewFolder·Auto) /
//! 압축하기 창(원본 TfAddToArc, [formAddToArchive])과 압축 풀기·압축 파일 속성. 압축과 풀기는 늘 동봉한 7-Zip으로 한다.
//! 고친 것(사용자 요청): 원본의 외부 압축 프로그램 칸(eExternalProg)·외부 프로그램 설정은 쓰지 않는다. RAR·ARJ처럼 동봉 7-Zip이
//! 만들 수 없는 형식은 고를 수 없다. 이름 칸은 이름만 받고 압축 파일은 늘 지금 패널 폴더에 만든다.
//! 차이(HANDOFF): C#의 "내장" ZIP/7Z/TAR는 SharpCompress로 쓰지만 Rust는 동봉 7-Zip으로 쓴다(결과 형식은 같다). 다중 볼륨 압축 파일에
//! 더하기(볼륨 다시 만들기)는 아직 안 된다. 진행 창은 일반 작업 창이고 일시 정지가 없다.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::archive::{self, ExternalProgram};
use ndr::msgbox::{IDYES, MB_ICONEXCLAMATION, MB_OK, MB_YESNO};

/// C# ArchiveFormatOption(확장자, 표시 이름, 외부 프로그램, 내장, 최고 압축 수준).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ArchiveFormat {
    pub extension: String,
    pub label: String,
    pub program: Option<PathBuf>,
    pub builtin: bool,
    pub max_level: i32,
}

/// 압축 만들기 / 더하기 / 더한 뒤 원본 옮기기 / 빠른 ZIP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ArchiveKind {
    Create,
    Add,
    Move,
    Zip,
}

/// C# ExtractMode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExtractMode {
    Selected,
    CurrentFolder,
    NewFolder,
    Auto,
}

/// ArchiveCreateWindow의 입력.
#[derive(Clone, Debug)]
pub(super) struct ArchiveCreateForm {
    pub formats: Vec<ArchiveFormat>,
    pub format: usize,
    pub preset: usize,
    pub password: String,
    pub volume: String,
    pub recursive: bool,
    pub preset_labels: Vec<String>,
    pub volume_labels: Vec<String>,
    pub default_level: i32,
    /// 원본 TfAddToArc cbFileName: 압축 파일 이름(지금 패널 폴더에 만든다).
    pub file_name: String,
}

/// 원본 rbFormatClick(0x4169d4)의 PathRenameExtensionW: 이름 끝 확장자(.tar.gz·.tar.bz2는 둘 다)를 새 확장자로.
pub(super) fn rename_extension(name: &str, extension: &str) -> String {
    let split = name.rfind(['\\', '/']).map(|i| i + 1).unwrap_or(0);
    let (folder, file) = name.split_at(split);
    let lower = file.to_lowercase();
    let stem = if let Some(cut) = [".tar.gz", ".tar.bz2"].iter().find(|e| lower.ends_with(*e)).map(|e| file.len() - e.len()) {
        &file[..cut]
    } else {
        file.rfind('.').filter(|i| *i > 0).map(|i| &file[..i]).unwrap_or(file)
    };
    format!("{folder}{stem}.{}", extension.trim_start_matches('.'))
}

impl ArchiveCreateForm {
    /// 형식을 바꾸면 cbFileName 이름의 확장자를 그 형식으로(원본 rbFormatClick).
    pub(super) fn set_format(&mut self, index: usize) {
        let Some(format) = self.formats.get(index) else { return };
        self.format = index;
        let extension = format.extension.clone();
        if !self.file_name.trim().is_empty() {
            self.file_name = rename_extension(&self.file_name, &extension);
        }
    }
}

/// 창 결과(C# ArchiveCreateOptions + 고른 칸 번호).
#[derive(Clone, Debug)]
pub(super) struct ArchiveChoice {
    pub format: ArchiveFormat,
    pub format_index: usize,
    pub preset: usize,
    pub level: i32,
    pub password: String,
    pub volume: Option<u64>,
    pub recursive: bool,
    /// 지금 패널 폴더 + cbFileName의 이름. None이면 저장 상자로 고른다(빠른 ZIP, 빈 이름).
    pub output: Option<PathBuf>,
}

/// TryGetVolumeSizeBytes: 첫 항목(분할 안 함)이나 빈 글은 None, "숫자[단위]"(b/k/m/g, 1024배), 0은 None. 틀리면 Err.
pub(super) fn parse_volume(text: &str, no_volume: &str) -> Result<Option<u64>, ()> {
    let text = text.trim();
    if text.is_empty() || text.to_lowercase() == no_volume.trim().to_lowercase() {
        return Ok(None);
    }
    let digits_end = text.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(text.len());
    let (number, unit) = (&text[..digits_end], text[digits_end..].trim().to_lowercase());
    if number.is_empty() || number == "." || number.matches('.').count() > 1 {
        return Err(());
    }
    let value: f64 = number.parse().map_err(|_| ())?;
    let multiplier = match unit.as_str() {
        "" | "b" | "byte" | "bytes" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        _ => return Err(()),
    };
    let size = value * multiplier;
    if !(0.0..=i64::MAX as f64).contains(&size) {
        return Err(());
    }
    Ok((size.trunc() >= 1.0).then_some(size.trunc() as u64))
}

/// CompressionPresetIndex(level): 1 이하면 빠르게, 6 이상이면 최고, 아니면 보통.
fn preset_for_level(level: i32) -> usize {
    if level <= 1 {
        2
    } else if level >= 6 {
        0
    } else {
        1
    }
}

/// CompressionLevelForPreset. 내장 ZIP 보통(-1, 기본값)은 7-Zip 기본 5.
pub(super) fn level_for_preset(format: &ArchiveFormat, preset: usize, default_level: i32) -> i32 {
    let max = format.max_level.clamp(0, 9);
    match format.extension.trim_start_matches('.').to_lowercase().as_str() {
        "zip" | "7z" | "wim" => [9, 5, 0][preset.min(2)],
        "rar" => [5, 3, 0][preset.min(2)],
        "arj" => if preset == 2 { 0 } else { 1 },
        _ => match preset {
            0 => max,
            2 => 1.min(max),
            _ => default_level.clamp(0, max),
        },
    }
}

/// GetDefaultArchiveFileName: 하나면 그 이름(확장자 뺌), 아니면 현재 폴더 이름(루트면 드라이브 글자), 그래도 없으면 archive.
pub(super) fn default_archive_name(paths: &[PathBuf], current: &Path, extension: &str) -> String {
    let mut base = if paths.len() == 1 { paths[0].file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() } else { String::new() };
    if base.trim().is_empty() {
        let text = current.to_string_lossy();
        let trimmed = if text.len() > 3 { text.trim_end_matches(['\\', '/']) } else { &text };
        base = Path::new(trimmed).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if base.trim().is_empty() {
            base = text.trim_end_matches(['\\', '/']).trim_end_matches(':').to_owned();
        }
    }
    if base.trim().is_empty() {
        base = "archive".into();
    }
    format!("{base}.{extension}")
}

/// 압축 파일 경로 = 지금 패널 폴더 + cbFileName의 이름. 고친 것(사용자 요청): 원본은 경로를 적으면(PathIsRelativeW가 거짓) 그 경로에
/// 만들었지만, 여기서는 적은 글의 파일 이름만 쓰고 늘 지금 폴더에 만든다. 빈 글이면 None.
pub(super) fn archive_output_path(text: &str, current: &Path) -> Option<PathBuf> {
    let text = text.trim().trim_matches('"');
    let name = text.rsplit(['\\', '/']).next().unwrap_or("").trim();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    let combined = current.join(name);
    Some(std::path::absolute(&combined).unwrap_or(combined))
}

/// EnsureArchiveOutputSafe: 결과가 원본 파일 자신이거나 원본 폴더 안이면 Err(글).
pub(super) fn output_conflict(sources: &[PathBuf], output: &Path, ko: bool) -> Option<String> {
    let full = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().into_owned();
    let output = full(output);
    for source in sources {
        let source_full = full(source);
        if source.is_file() && ndr::names::equals_ignore_case(&source_full, &output) {
            return Some(if ko { "압축 결과물 경로와 원본 파일 경로가 같습니다." } else { "The archive path is the same as a source file." }.into());
        }
        if source.is_dir() {
            let prefix = format!("{}\\", source_full.trim_end_matches(['\\', '/']));
            if output.to_lowercase().starts_with(&prefix.to_lowercase()) {
                return Some(if ko { "압축 결과물을 압축 대상 폴더 안에 저장할 수 없습니다." } else { "The archive cannot be saved inside a folder being archived." }.into());
            }
        }
    }
    None
}

/// 원본 TfAddToArc(ClientWidth 443 × ClientHeight 354): lSecName(20,16 굵게 파랑), lFileName(44,43) cbFileName(106,40 275×21),
/// lSecFormat(20,76), rbZIP(44,104 굵게)·rb7z(173)·rbALZ(238), lSecOptions(20,179), lLevel(44,209) cbCompLevel(143,206 121×21)
/// chkRecursive(276,208), lVolume(44,243) cbVolume(143,240), lPassword(44,276) ePassword(143,273), Bevel1(20,310 404),
/// btnOk(253,320) btnCancel(341,320) 83×25. 외부 프로그램 줄(lExtProg·eExternalProg·btnBrowseExtProg)·lHelp·rbRAR·rbARJ는 두지 않는다.
/// 원본 다섯 형식 밖의 동봉 7-Zip 형식(TAR 등)은 라디오 줄 오른쪽 작은 목록에서 고른다. 0 = 압축, 1 = 취소.
pub(super) fn archive_create_ui(ui: &mut egui::Ui, form: &mut ArchiveCreateForm, language: &compat::Language, enter: bool) -> Option<usize> {
    use super::original_dialogs::put_left;
    let text = |key: &str, fallback: &str| language.text(&format!("formAddToArchive.{key}"), fallback);
    let mut choice = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(443.0, 354.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let blue = Color32::from_rgb(0x3f, 0x66, 0xe9);
    for (top, key, fallback) in [(16.0, "Caption.lSecName", "1. File Name"), (76.0, "Caption.lSecFormat", "2. Archive Type"), (179.0, "Caption.lSecOptions", "3. Options")] {
        put_left(ui, at(20.0, top, 400.0, 16.0), egui::Label::new(RichText::new(text(key, fallback)).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())).color(blue)));
    }
    put_left(ui, at(44.0, 43.0, 60.0, 15.0), egui::Label::new(text("Caption.lFileName", "&Filename:")).truncate());
    ui.put(at(106.0, 40.0, 275.0, 21.0), egui::TextEdit::singleline(&mut form.file_name));
    // 원본 rbZIP·rbRAR·rb7z·rbALZ·rbARJ(Tag 1~5).
    let original = [("zip", 44.0, "Caption.rbZIP", "&ZIP"), ("rar", 108.0, "Caption.rbRAR", "&RAR"), ("7z", 173.0, "Caption.rb7z", "&7z"), ("alz", 238.0, "Caption.rbALZ", "&ALZ"), ("arj", 303.0, "Caption.rbARJ", "AR&J")];
    let index_of = |extension: &str| form.formats.iter().position(|f| f.extension.trim_start_matches('.').eq_ignore_ascii_case(extension));
    let mut picked = form.format;
    for (extension, left, key, fallback) in original {
        let Some(index) = index_of(extension) else { continue };
        let mut label = RichText::new(text(key, fallback));
        if extension == "zip" {
            label = label.strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()));
        }
        if put_left(ui, at(left, 104.0, 59.0, 17.0), egui::RadioButton::new(form.format == index, label)).clicked() {
            picked = index;
        }
    }
    let others: Vec<usize> = (0..form.formats.len()).filter(|i| !original.iter().any(|(e, ..)| form.formats[*i].extension.trim_start_matches('.').eq_ignore_ascii_case(e))).collect();
    if !others.is_empty() {
        let current = if others.contains(&form.format) { form.formats[form.format].label.clone() } else { "…".into() };
        ui.scope_builder(egui::UiBuilder::new().max_rect(at(364.0, 101.0, 60.0, 21.0)), |ui| {
            egui::ComboBox::from_id_salt("archive-format").width(56.0).selected_text(current).show_opaque(ui, |ui| {
                for index in &others {
                    ui.selectable_value(&mut picked, *index, form.formats[*index].label.as_str());
                }
            });
        });
    }
    if picked != form.format {
        form.set_format(picked);
    }
    put_left(ui, at(44.0, 209.0, 97.0, 15.0), egui::Label::new(text("Caption.lLevel", "Le&vel:")).truncate());
    let current = form.preset_labels.get(form.preset).cloned().unwrap_or_default();
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(143.0, 206.0, 121.0, 21.0)), |ui| {
        egui::ComboBox::from_id_salt("archive-level").width(113.0).selected_text(current).show_opaque(ui, |ui| {
            for (index, label) in form.preset_labels.iter().enumerate() {
                ui.selectable_value(&mut form.preset, index, label.as_str());
            }
        });
    });
    super::original_dialogs::access_checkbox(ui, at(276.0, 208.0, 156.0, 17.0), &mut form.recursive, &language.raw("formAddToArchive.Caption.chkRecursive").unwrap_or_else(|| "Includ&e Subfolders".into()));
    put_left(ui, at(44.0, 243.0, 97.0, 15.0), egui::Label::new(text("Caption.lVolume", "Volume(&S):")).truncate());
    ui.put(at(143.0, 240.0, 99.0, 21.0), egui::TextEdit::singleline(&mut form.volume));
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(242.0, 240.0, 22.0, 21.0)), |ui| {
        ui.menu_button("▼", |ui| {
            for label in form.volume_labels.clone() {
                if ui.button(&label).clicked() {
                    form.volume = label;
                    ui.close();
                }
            }
        });
    });
    put_left(ui, at(44.0, 276.0, 97.0, 15.0), egui::Label::new(text("Caption.lPassword", "&Password:")).truncate());
    ui.put(at(143.0, 273.0, 121.0, 21.0), egui::TextEdit::singleline(&mut form.password).password(true));
    ui.painter().hline(area.min.x + 20.0..=area.min.x + 424.0, area.min.y + 310.0, ui.visuals().widgets.noninteractive.bg_stroke);
    if ui.put(at(253.0, 320.0, 83.0, 25.0), egui::Button::new(text("Caption.btnOk", "Compress"))).clicked() {
        choice = Some(0);
    }
    if ui.put(at(341.0, 320.0, 83.0, 25.0), egui::Button::new(text("Caption.btnCancel", "Cancel"))).clicked() {
        choice = Some(1);
    }
    if enter && choice.is_none() {
        choice = Some(0);
    }
    choice
}
fn n0(value: usize) -> String {
    ndr::parity::format_size(value as u64, "B", true).trim_end_matches(" B").to_owned()
}

impl App {
    /// 원본 FUN_00404890(압축 엔진 오류 → [RES] 글): 7-Zip이 낸 오류 글이면 원본 압축 오류 글로 바꿔 보인다(보일 때만 바꾼다).
    /// 분류하지 못한 글과 7-Zip이 아닌 글은 그대로.
    pub(super) fn archive_error_text(&self, raw: &str) -> String {
        if !(raw.contains("7-Zip failed") || raw.contains("Bundled 7-Zip")) {
            return raw.to_owned();
        }
        let Some(key) = ndr::archive::original_error_key(raw) else { return raw.to_owned() };
        let fallback = match key {
            "INVALID_PASSWORD" => "Invalid Password",
            "DISKISFULL" => "Disk is full.",
            "CANNOT_OPEN_FILE" => "Failed to open a compressed file.",
            "WRITE_FAILED" => "Couldn't write a file.",
            "7ZDLL_LOAD_FAILED" => "Couldn't load 7z.dll file.",
            "UNSUPPORTED_ARCHIVE_FILE" => "Unspported compressed file. Or not a compressed file.",
            "RAR_INVALIDFILE_OR_CORRUPTED" => "Invalid password or a compressed file may be corrupted.",
            "CANNOT_READFILE" => "Couldn't read a file.",
            "CANNOT_CREATEFILE" => "Couldn't create a file.",
            _ => "Cannot find next part of a compressed file.",
        };
        self.language_pack.text(&format!("RES.{key}"), fallback)
    }
    /// 동봉 7-Zip이 만들 수 있는 형식(원본 rbZIP·rb7z·rbALZ, 그 밖의 TAR 등).
    pub(super) fn archive_formats(&self, include_wim: bool) -> Vec<ArchiveFormat> {
        let ko = self.language == "ko";
        let format = |extension: &str, label: String, program: Option<PathBuf>, builtin: bool, max_level: i32| ArchiveFormat { extension: extension.into(), label, program, builtin, max_level };
        let mut formats = vec![format("zip", if ko { "ZIP (내장)" } else { "ZIP (built-in)" }.into(), None, true, 9)];
        formats.push(format("7z", if ko { "7Z (내장 LZMA2)" } else { "7Z (built-in LZMA2)" }.into(), None, true, 9));
        formats.push(format("tar", if ko { "TAR (내장)" } else { "TAR (built-in)" }.into(), None, true, 0));
        formats.push(format("tar.gz", if ko { "TAR.GZ (내장 GZip)" } else { "TAR.GZ (built-in GZip)" }.into(), None, true, 9));
        formats.push(format("tar.bz2", if ko { "TAR.BZ2 (내장 BZip2)" } else { "TAR.BZ2 (built-in BZip2)" }.into(), None, true, 9));
        // 원본 arcformat 4 = ALZ(외부 alzprog). 동봉 7-Zip(UzuCore 7zip+α)이 ALZ를 만들므로 내장으로 둔다.
        // 동봉 7zip+α의 EGG 만들기(원본·C# 판에 없음)는 사용자 요청으로 넣지 않는다(.egg 읽기·풀기는 그대로).
        formats.push(format("alz", if ko { "ALZ (내장)" } else { "ALZ (built-in)" }.into(), None, true, 9));
        if include_wim && !formats.iter().any(|f| f.extension == "wim") && archive::seven_zip_path().is_ok_and(|p| p.ends_with("Full\\7z.exe") || p.ends_with("Full/7z.exe")) {
            formats.push(format("wim", "WIM (7-Zip 26.05)".into(), None, true, 9));
        }
        formats
    }

    fn archive_form(&mut self) -> Option<&mut ArchiveCreateForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::ArchiveCreate(form)) => Some(form),
            _ => None,
        }
    }

    /// 압축 대상(일반 폴더의 표시 항목).
    fn archive_sources(&self) -> Vec<PathBuf> {
        let p = &self.panes[self.active];
        if p.archive.is_some() {
            return vec![];
        }
        p.targets()
    }

    /// archiveAdd / archiveMove / createArchive / createZip.
    pub(super) fn archive_command(&mut self, kind: ArchiveKind) {
        if self.panes[self.active].archive.is_some() {
            return match kind {
                ArchiveKind::Create => self.warn("압축 파일 안에서 새 압축 파일을 만들 수 없습니다.", "A new archive cannot be created inside an archive."),
                ArchiveKind::Zip => self.notice("압축 파일 안에서는 새 압축 파일을 만들 수 없습니다.", "A new archive cannot be created inside an archive."),
                _ => self.warn("압축 파일 안에서는 새 압축 파일을 만들 수 없습니다.", "A new archive cannot be created inside an archive."),
            };
        }
        let paths = self.archive_sources();
        if paths.is_empty() {
            return if kind == ArchiveKind::Zip { self.notice("압축할 파일이나 폴더를 선택하세요.", "Select the files or folders to archive.") } else { self.warn("압축할 파일이나 폴더를 선택하세요.", "Select the files or folders to archive.") };
        }
        if kind == ArchiveKind::Zip {
            let zip = self.archive_formats(false).remove(0);
            let choice = ArchiveChoice { format: zip, format_index: 0, preset: 1, level: 2, password: String::new(), volume: None, recursive: true, output: None };
            return self.archive_save_step(kind, paths, choice, None);
        }
        let formats = self.archive_formats(kind == ArchiveKind::Create);
        let default_level = self.settings.values.get("DefaultArchiveCompressionLevel").and_then(|v| v.as_i64()).unwrap_or(2).clamp(0, 9) as i32;
        let preset = self.settings.values.get("DefaultArchiveCompressionPresetIndex").and_then(|v| v.as_i64()).map(|v| v.clamp(0, 2) as usize).unwrap_or_else(|| preset_for_level(default_level));
        let default_index = self.settings.values.get("DefaultArchiveFormatIndex").and_then(|v| v.as_i64()).unwrap_or(1);
        let default_extension = self.settings.text("DefaultArchiveFormatExtension", "");
        let format = if !default_extension.trim().is_empty() { formats.iter().position(|f| f.extension.eq_ignore_ascii_case(&default_extension)) } else { None }.unwrap_or_else(|| ((default_index - 1).max(0) as usize).min(formats.len() - 1));
        let split = |key: &str, fallback: &str| self.language_pack.text(&format!("formAddToArchive.{key}"), fallback).split(';').map(str::to_owned).collect::<Vec<_>>();
        let volume_labels = split("ComboBox.cbVolume", "No Volume;1440 KB;2880 KB;5 MB;10 MB;20 MB;50 MB;100 MB");
        // 원본 miArcAddToArchiveClick(0x43a7f8): cbFileName = 기본 이름(하나면 그 이름, 아니면 폴더 이름), 확장자는 고른 형식
        // (rbFormatClick). 고친 것(사용자 요청): 다른 패널 폴더 + 이름 줄은 넣지 않는다(늘 지금 폴더에 만든다).
        let current = self.panes[self.active].path.clone();
        let name = default_archive_name(&paths, &current, &formats[format].extension);
        let form = ArchiveCreateForm {
            formats,
            format,
            preset,
            password: String::new(),
            volume: volume_labels.first().cloned().unwrap_or_default(),
            recursive: paths.iter().any(|p| p.is_dir()),
            preset_labels: split("ComboBox.cbCompLevel", "Best;Normal;Fast"),
            volume_labels,
            default_level,
            file_name: name,
        };
        let title = self.language_pack.text("formAddToArchive.Form.Caption", "Add to Archive");
        self.push_original_dialog(title, DialogKind::ArchiveCreate(Box::new(form)), move |app, result, _| {
            let DialogResult::ArchiveCreate(Some(choice)) = result else { return };
            // RememberArchiveDialogChoices.
            app.settings.values.insert("DefaultArchiveFormatIndex".into(), serde_json::json!((choice.format_index + 1).max(1)));
            app.settings.set_text("DefaultArchiveFormatExtension", &choice.format.extension);
            app.settings.values.insert("DefaultArchiveCompressionPresetIndex".into(), serde_json::json!(choice.preset));
            app.save();
            if choice.format.extension == "wim" && (!choice.password.is_empty() || choice.volume.is_some()) {
                return app.warn("WIM은 이 흐름에서 비밀번호나 분할 크기를 지원하지 않습니다.", "WIM does not support password encryption or split volumes in this workflow.");
            }
            app.archive_save_step(kind, paths, choice, None);
        });
    }

    /// 창 단추: 압축(분할 크기 검사), 취소.
    pub(super) fn submit_archive_create(&mut self, choice: usize, ctx: &egui::Context) {
        let current = self.panes[self.active].path.clone();
        let Some(form) = self.archive_form() else { return };
        match choice {
            1 => self.answer_dialog(DialogResult::ArchiveCreate(None), ctx),
            _ => {
                let Some(format) = form.formats.get(form.format).cloned() else { return };
                let no_volume = form.volume_labels.first().cloned().unwrap_or_default();
                let Ok(volume) = parse_volume(&form.volume, &no_volume) else {
                    let title = self.language_pack.text("formAddToArchive.Form.Caption", "Add to Archive");
                    let message = self.language_pack.text("formAddToArchive.Message.InvalidVolume", "Enter a valid volume size such as 10 MB, or 0 for no splitting.");
                    return self.system_box(&title, &message, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                };
                let choice = ArchiveChoice {
                    level: level_for_preset(&format, form.preset, form.default_level),
                    format,
                    format_index: form.format,
                    preset: form.preset,
                    password: form.password.clone(),
                    volume,
                    recursive: form.recursive,
                    output: archive_output_path(&form.file_name, &current),
                };
                self.answer_dialog(DialogResult::ArchiveCreate(Some(choice)), ctx);
            }
        }
    }

    /// 동봉 7-Zip으로 만들 수 있는지(외부 프로그램은 쓰지 않는다). Ok(None) = 동봉 7-Zip.
    pub(super) fn archive_program(&self, choice: &ArchiveChoice) -> Result<Option<ExternalProgram>, String> {
        let extension = choice.format.extension.as_str();
        let ko = self.language == "ko";
        // 동봉 7zip+α는 ZIP·7Z에 암호를 넣고, ZIP·7Z·ALZ·EGG를 나눠 만든다(.001…).
        if !choice.password.is_empty() && !matches!(extension, "zip" | "7z") {
            return Err(if ko { format!(".{extension} 형식은 암호를 넣을 수 없습니다.") } else { format!("The .{extension} format cannot be encrypted.") });
        }
        if choice.volume.is_some() && !matches!(extension, "zip" | "7z" | "alz" | "egg") {
            return Err(if ko { format!(".{extension} 형식은 분할 압축을 할 수 없습니다.") } else { format!("The .{extension} format cannot be split into volumes.") });
        }
        Ok(None)
    }

    /// 저장 대화 상자부터 검사·확인·실행까지.
    fn archive_save_step(&mut self, kind: ArchiveKind, paths: Vec<PathBuf>, choice: ArchiveChoice, _unused: Option<()>) {
        let ko = self.language == "ko";
        let program = match self.archive_program(&choice) {
            Ok(program) => program,
            Err(error) => return self.system_box("NDR", &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
        };
        let extension = choice.format.extension.clone();
        let folder = self.panes[self.active].path.clone();
        let upper = extension.to_uppercase();
        let (title, filter_name, filter_spec, name) = match kind {
            ArchiveKind::Zip => (
                if ko { "ZIP 압축 파일 만들기" } else { "Create ZIP archive" }.to_owned(),
                if ko { "ZIP 압축 파일 (*.zip)" } else { "ZIP archives (*.zip)" }.to_owned(),
                "*.zip".to_owned(),
                if ko { "새 압축 파일.zip" } else { "New archive.zip" }.to_owned(),
            ),
            ArchiveKind::Create => (
                if ko { "압축 파일 만들기" } else { "Create archive" }.to_owned(),
                if ko { format!(".{upper} 파일 (*.{extension})") } else { format!(".{upper} files (*.{extension})") },
                format!("*.{extension}"),
                default_archive_name(&paths, &folder, &extension),
            ),
            _ => {
                let mut specs: Vec<String> = vec![];
                for format in self.archive_formats(false) {
                    let spec = format!("*.{}", format.extension);
                    if !specs.iter().any(|s| s.eq_ignore_ascii_case(&spec)) {
                        specs.push(spec);
                    }
                }
                let joined = specs.join(";");
                (
                    if kind == ArchiveKind::Move { if ko { "압축 파일로 이동" } else { "Move to archive" } } else if ko { "압축 파일에 추가" } else { "Add to archive" }.to_owned(),
                    if ko { format!("압축 파일 ({joined})") } else { format!("Archives ({joined})") },
                    joined,
                    default_archive_name(&paths, &folder, &extension),
                )
            }
        };
        let all = if ko { "모든 파일 (*.*)" } else { "All files (*.*)" };
        let filters: Vec<(&str, &str)> = if kind == ArchiveKind::Zip { vec![(filter_name.as_str(), filter_spec.as_str())] } else { vec![(filter_name.as_str(), filter_spec.as_str()), (all, "*.*")] };
        let chosen = match choice.output.clone() {
            // 원본 TfAddToArc: 창의 cbFileName이 결과 이름이다(저장 상자 없음).
            Some(path) => path,
            None => {
                let Some(chosen) = (self.save_dialog)(&title, Some(&folder), &name, &filters, &extension, false) else { return };
                chosen
            }
        };
        let output = if matches!(kind, ArchiveKind::Add | ArchiveKind::Move) { archive::first_volume_path(&chosen) } else { std::path::absolute(&chosen).unwrap_or(chosen) };
        if let Some(error) = output_conflict(&paths, &output, ko) {
            return self.system_box("NDR", &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
        let family = archive::output_family(&output);
        if matches!(kind, ArchiveKind::Create | ArchiveKind::Zip) {
            if family.is_empty() {
                return self.run_archive_task(kind, paths, output, choice, program);
            }
            let mut preview = family.iter().take(8).filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect::<Vec<_>>().join("\r\n");
            if family.len() > 8 {
                preview += &if ko { format!("\r\n외 {}개", n0(family.len() - 8)) } else { format!("\r\nand {} more", n0(family.len() - 8)) };
            }
            let text = if ko { format!("기존 압축 파일 묶음 {}개를 새 결과로 교체할까요?\r\n\r\n{preview}", n0(family.len())) } else { format!("Replace the {} existing archive files with the new result?\r\n\r\n{preview}", n0(family.len())) };
            let caption = self.tr("압축 파일 교체", "Replace archive").into_owned();
            return self.system_box(&caption, &text, MB_YESNO | MB_ICONEXCLAMATION, move |app, answer, _| {
                if answer == IDYES {
                    app.run_archive_task(kind, paths, output, choice, program);
                }
            });
        }
        // 더하기: 형식, 묶음, 분할 크기, 읽기 전용 검사.
        let exists = output.is_file();
        let target_format = archive::archive_format(&output);
        if !target_format.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&extension)) {
            let shown = target_format.unwrap_or_else(|| if ko { "알 수 없음".into() } else { "unknown".into() });
            let (k, e) = (format!("대상 파일의 형식({shown})과 선택한 형식(.{extension})이 다릅니다."), format!("The target file format ({shown}) differs from the selected format (.{extension})."));
            return self.warn(&k, &e);
        }
        if !exists && !family.is_empty() {
            return self.warn("완전하지 않은 압축 파일 묶음에는 추가할 수 없습니다. 모든 볼륨이 있는 압축 파일을 선택하세요.", "Items cannot be added to an incomplete archive set. Choose an archive with all of its volumes.");
        }
        let mut choice = choice;
        if exists && family.len() > 1 {
            // C# 다중 볼륨 업데이트: 내장 7-Zip(7z, zip)만. 분할 크기 = 고른 값, 없으면 가장 큰 볼륨(MiB 내림).
            if !matches!(extension.as_str(), "7z" | "zip") || program.is_some() {
                let (k, e) = (
                    format!(".{extension} 다중 볼륨 파일을 업데이트하려면 설정에서 {extension}용 외부 압축 인자에 {{archive}}, {{sources}}와 볼륨 크기 자리표시자를 지정하세요."),
                    format!("To update a .{extension} multi-volume archive, set an external archive template for {extension} with {{archive}}, {{sources}} and a volume size placeholder."),
                );
                return self.warn(&k, &e);
            }
            if choice.volume.is_none() {
                let largest = family.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).max().unwrap_or(0);
                const MIB: u64 = 1024 * 1024;
                choice.volume = (largest > 0).then(|| (largest / MIB).max(1) * MIB);
            }
            if choice.volume.is_none() {
                return self.warn("기존 분할 크기를 확인하지 못했습니다. 압축 추가 창에서 새 분할 크기를 지정하세요.", "The existing volume size could not be determined. Choose a new volume size in the add-to-archive window.");
            }
        } else if exists && choice.volume.is_some() {
            return self.warn("기존 압축 파일에 추가할 때는 분할 크기를 지정할 수 없습니다.", "A volume size cannot be used when adding to an existing archive.");
        }
        if exists && std::fs::metadata(&output).is_ok_and(|m| m.permissions().readonly()) {
            let name = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (k, e) = (format!("기존 압축 파일이 읽기 전용입니다: {name}"), format!("The existing archive is read-only: {name}"));
            return self.warn(&k, &e);
        }
        if kind == ArchiveKind::Move {
            let caption = self.tr("압축 후 이동", "Move after archiving").into_owned();
            let text = self.tr("압축 파일을 만든 뒤 원본을 휴지통으로 이동할까요?", "Move the originals to the Recycle Bin after creating the archive?").into_owned();
            return self.system_box(&caption, &text, MB_YESNO | MB_ICONEXCLAMATION, move |app, answer, _| {
                if answer == IDYES {
                    app.run_archive_task(kind, paths, output, choice, program);
                }
            });
        }
        self.run_archive_task(kind, paths, output, choice, program);
    }

    /// 압축 작업(일반 작업 창). 끝나면 상태 줄은 결과 글(ArchiveStatus).
    fn run_archive_task(&mut self, kind: ArchiveKind, paths: Vec<PathBuf>, output: PathBuf, choice: ArchiveChoice, program: Option<ExternalProgram>) {
        let ko = self.language == "ko";
        let options = ndr::commands::ArchiveOptions {
            extension: choice.format.extension.clone(),
            level: choice.level,
            password: choice.password.clone(),
            volume_bytes: choice.volume,
            recursive: choice.recursive,
            append: matches!(kind, ArchiveKind::Add | ArchiveKind::Move),
        };
        let recycle = self.settings.flag("UseRecycleBin", true);
        let name = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let upper = choice.format.extension.to_uppercase();
        let count = n0(paths.len());
        let done = match kind {
            ArchiveKind::Create => if ko { format!("{upper} 압축 파일을 만들었습니다.") } else { format!("Created the {upper} archive.") },
            ArchiveKind::Zip => if ko { format!("ZIP 파일을 만들었습니다: {name}") } else { format!("Created the ZIP file: {name}") },
            _ => if ko { format!("{count}개 항목을 {name}에 추가했습니다.") } else { format!("Added {count} items to {name}.") },
        };
        let stopped = if ko { format!("압축은 만들었지만 원본 삭제를 중단했습니다: {name}") } else { format!("The archive was created but deleting the originals stopped: {name}") };
        let title = match kind {
            ArchiveKind::Create => if ko { format!("{upper} 압축 만들기") } else { format!("Create {upper} archive") },
            ArchiveKind::Zip => if ko { "ZIP 압축 만들기".into() } else { "Create ZIP archive".into() },
            ArchiveKind::Add => if ko { "압축 파일에 추가".into() } else { "Add to archive".into() },
            ArchiveKind::Move => if ko { "압축 파일로 이동".into() } else { "Move to archive".into() },
        };
        let tx = self.async_tx.clone();
        let moving = kind == ArchiveKind::Move;
        let focus_output = output.clone();
        let rebuild = options.append && archive::output_family(&output).len() > 1;
        let rebuild_cancelled = if ko { "분할 압축 파일 업데이트를 취소했습니다. 기존 압축 파일은 유지됩니다." } else { "The multi-volume archive update was cancelled. The existing archive was kept." }.to_owned();
        // C# RunArchiveCreationWithProgress = FileOperationProgressWindow.Run(allowPause: true). 외부 프로그램은 일시 정지 없음.
        let external = program.is_some();
        let work = move |events: Sender<Event>, cancel: Arc<AtomicBool>| -> Result<(), String> {
            if rebuild {
                // C# "기존 분할 압축 파일 준비": 숨긴 작업 폴더에 모두 풀고, 고른 항목을 덮어 합친 뒤, 같은 분할 크기로 다시 만든다.
                let result = rebuild_multivolume(&paths, &output, &options, &events, &cancel);
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(AsyncResult::ArchiveStatus(rebuild_cancelled));
                    return Ok(());
                }
                result?;
            } else {
                archive::create_with_program(&paths, &output, &options, program.as_ref(), &cancel)?;
            }
            let _ = events.send(Event::Progress { path: output.clone(), bytes: 0, files: paths.len() as u64 });
            if moving {
                let deleted = super::commands::run_nested(Operation::Delete { sources: paths, recycle }, &events, &cancel);
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(AsyncResult::ArchiveStatus(stopped));
                    return Ok(());
                }
                deleted?;
            }
            let _ = tx.send(AsyncResult::ArchiveStatus(done));
            Ok(())
        };
        let started = if external {
            self.start_task(title, work)
        } else {
            let progress = super::file_operation::ProgressInfo { caption: Some(self.language_pack.text("RES.COMPRESSING", "Adding to the archive...")), ..Default::default() };
            self.start_pausable_task(title, progress, move |events, cancel, pause| archive::with_pause(pause, || work(events, cancel)))
        };
        if started {
            if let Some(job) = self.job.as_mut() {
                job.custom_status = true;
                job.chime = kind == ArchiveKind::Create;
            }
            // 압축이 끝나면 만든 압축 파일에 커서를 둔다(사용자 요청).
            self.archive_focus = Some((self.active, focus_output));
        }
    }

    /// 동봉 7-Zip이 못 읽는 압축(예: .ace)을 열려고 할 때. 외부 압축 프로그램은 쓰지 않으므로 열 수 없다고 알린다.
    pub(super) fn open_with_external_archive_program(&mut self, _pane: usize, archive_path: PathBuf, _ask_if_not_empty: bool) {
        let extension = archive_path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        self.warn(&format!("{extension} 형식은 열 수 없습니다."), &format!("The {extension} format cannot be opened."));
    }

    /// C# ExtractArchiveAsync.
    pub(super) fn extract_command(&mut self, mode: ExtractMode) {
        let i = self.active;
        let in_archive = self.panes[i].archive.clone();
        if in_archive.is_some() && matches!(mode, ExtractMode::CurrentFolder | ExtractMode::Auto) {
            return;
        }
        let selected = self.target_entries(i);
        let archives: Vec<PathBuf> = match &in_archive {
            Some(location) => vec![archive::first_volume_path(&location.file)],
            None => {
                let mut list: Vec<PathBuf> = vec![];
                for entry in selected.iter().filter(|e| !e.is_dir && archive::archive_format(&e.path).is_some()) {
                    let first = archive::first_volume_path(&entry.path);
                    if !list.iter().any(|p| ndr::names::equals_ignore_case(&p.to_string_lossy(), &first.to_string_lossy())) {
                        list.push(first);
                    }
                }
                list
            }
        };
        if archives.is_empty() {
            return self.warn("압축 파일을 선택하거나 압축 파일 안으로 이동하세요.", "Select an archive or open one first.");
        }
        if in_archive.is_some() && selected.is_empty() {
            return;
        }
        let keys: Vec<String> = if in_archive.is_some() { selected.iter().map(|e| e.path.to_string_lossy().replace('\\', "/")).collect() } else { vec![] };
        let password = in_archive.as_ref().map(|l| l.password.clone()).unwrap_or_default();
        let parent = archives[0].parent().map(Path::to_path_buf).unwrap_or_default();
        if mode == ExtractMode::Selected || in_archive.is_some() {
            let other = &self.panes[1 - i];
            let initial = if other.archive.is_none() { other.path.clone() } else { parent };
            let current = self.panes[i].path.clone();
            return self.select_destination(super::destination_dialog::DestinationPurpose::Extract, initial, Some(current), move |app, base| {
                app.run_extract(archives, keys, password, base, mode);
            });
        }
        self.run_extract(archives, keys, password, parent, mode);
    }

    /// 압축 파일 안 항목을 그 폴더로 복사(C# CopyToOtherAsync·CopyMoveToFolderAsync의 압축 쪽).
    pub(super) fn extract_keys_to(&mut self, pane: usize, keys: Vec<PathBuf>, destination: PathBuf) {
        let Some(location) = self.panes[pane].archive.clone() else { return };
        let keys = keys.into_iter().map(|k| k.to_string_lossy().replace('\\', "/")).collect();
        self.run_extract(vec![archive::first_volume_path(&location.file)], keys, location.password, destination, ExtractMode::Selected);
    }

    fn run_extract(&mut self, archives: Vec<PathBuf>, keys: Vec<String>, password: String, base: PathBuf, mode: ExtractMode) {
        self.remember_destination(&base);
        let programs: Vec<Option<ExternalProgram>> = archives.iter().map(|_| None).collect();
        let tx = self.async_tx.clone();
        let ko = self.language == "ko";
        let done = if ko { format!("압축을 풀었습니다: {}", base.display()) } else { format!("Extracted to {}", base.display()) };
        let title = self.tr("압축 풀기", "Extract").into_owned();
        // C# RunArchiveExtractionWithProgress: 내장 경로는 FileOperationProgressWindow.Run(원본 = 압축 파일, 대상, 일시 정지 가능),
        // 외부 프로그램은 "외부 압축 풀기"(allowPause: false).
        let external = programs.iter().any(Option::is_some);
        let progress = super::file_operation::ProgressInfo { from: archives.first().map(|a| a.to_string_lossy().into_owned()), to: Some(base.to_string_lossy().into_owned()), caption: Some(self.language_pack.text("RES.EXTRACTING", "Extracting...")), ..Default::default() };
        let work = move |events: Sender<Event>, cancel: Arc<AtomicBool>| -> Result<(), String> {
            let password = (!password.is_empty()).then_some(password);
            for (archive_path, program) in archives.iter().zip(programs) {
                let mut conflicts = |source: &Path, target: &Path| super::commands::ask_conflict(&events, &cancel, source, target);
                let mut passwords = |archive: &Path, _: Option<&str>| super::commands::ask_password(&events, &cancel, archive);
                let mut destination = base.clone();
                let mut known = password.clone();
                match mode {
                    ExtractMode::NewFolder => destination = archive::create_unique_extract_folder(&base, &archive::extract_folder_name(archive_path))?,
                    ExtractMode::Auto => {
                        let single = match archive::list_with_passwords(archive_path, known.as_deref(), &cancel, &mut passwords) {
                            Ok((entries, used)) => {
                                known = used;
                                archive::has_single_root_folder(&entries)
                            }
                            Err(error) if error == "Archive password request cancelled" => return Ok(()),
                            Err(_) => false,
                        };
                        if !single {
                            destination = archive::create_unique_extract_folder(&base, &archive::extract_folder_name(archive_path))?;
                        }
                    }
                    _ => std::fs::create_dir_all(&base).map_err(|e| e.to_string())?,
                }
                let shown = archive_path.clone();
                // 원본 0x439593: 다음 볼륨이 없으면 ARC_CHANGEVOL로 묻고, 확인이면 다시 시도한다(취소면 그 오류로 끝).
                loop {
                    let outcome = archive::extract_entries_with_program(archive_path, &destination, &keys, known.as_deref(), &cancel, &mut conflicts, &mut passwords, program.as_ref(), &mut |done, _, _| {
                        let _ = events.send(Event::Progress { path: shown.clone(), bytes: 0, files: done as u64 });
                    });
                    match outcome {
                        Err(error) if !cancel.load(Ordering::Relaxed) => {
                            if let Some(volume) = archive::missing_volume(&error) {
                                if super::commands::ask_change_volume(&events, &cancel, &volume) {
                                    continue;
                                }
                            }
                            return Err(error);
                        }
                        outcome => {
                            outcome?;
                            break;
                        }
                    }
                }
            }
            let _ = tx.send(AsyncResult::ArchiveStatus(done));
            Ok(())
        };
        let started = if external {
            self.start_task(title, work)
        } else {
            self.start_pausable_task(title, progress, move |events, cancel, pause| archive::with_pause(pause, || work(events, cancel)))
        };
        if started {
            if let Some(job) = self.job.as_mut() {
                job.custom_status = true;
            }
        }
    }

    /// C# ShowArchivePropertiesAsync: 글 창 "압축 파일 속성".
    pub(super) fn archive_properties_command(&mut self) {
        let i = self.active;
        let archive_path = match &self.panes[i].archive {
            Some(location) => Some(location.file.clone()),
            None => self.target_entries(i).into_iter().find(|e| !e.is_dir && archive::archive_format(&e.path).is_some()).map(|e| e.path),
        };
        let Some(archive_path) = archive_path else {
            return self.warn("속성을 볼 압축 파일을 열거나 선택하세요.", "Open or select an archive to see its properties.");
        };
        let ko = self.language == "ko";
        let unit = self.settings.text("SizeUnit", "Auto");
        let thousands = self.settings.flag("UseThousandsSeparator", true);
        let (date_format, time_format) = (self.settings.text("DateFormat", "yyyy-MM-dd"), self.settings.text("TimeFormat", "HH:mm"));
        let password = self.panes[i].archive.as_ref().map(|l| l.password.clone()).filter(|p| !p.is_empty());
        let title = self.tr("압축 파일 속성", "Archive properties").into_owned();
        let tx = self.async_tx.clone();
        thread::spawn(move || {
            let outcome = (|| -> Result<String, String> {
                let metadata = std::fs::metadata(&archive_path).map_err(|e| e.to_string())?;
                let entries = archive::list(&archive_path, password.as_deref())?;
                let (files, folders, bytes) = archive::summary(&entries);
                let format = archive::archive_format(&archive_path).map(|f| f.to_uppercase()).unwrap_or_else(|| if ko { "알 수 없음".into() } else { "unknown".into() });
                let modified = ndr::listing::date_text(metadata.modified().ok(), &date_format, &time_format, false, ko, std::time::SystemTime::now());
                let name = archive_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let size = |b: u64| ndr::parity::format_size(b, &unit, thousands);
                let lines = if ko {
                    [format!("파일: {name}"), format!("형식: {format}"), format!("크기: {}", size(metadata.len())), format!("수정한 날짜: {modified}"), format!("파일 항목: {}", n0(files)), format!("폴더 항목: {}", n0(folders)), format!("압축 해제 크기: {}", size(bytes))]
                } else {
                    [format!("File: {name}"), format!("Format: {format}"), format!("Size: {}", size(metadata.len())), format!("Modified: {modified}"), format!("File entries: {}", n0(files)), format!("Folder entries: {}", n0(folders)), format!("Extracted size: {}", size(bytes))]
                };
                Ok(lines.join("\r\n"))
            })();
            let _ = tx.send(match outcome {
                Ok(text) => AsyncResult::Text { title, text },
                Err(error) => AsyncResult::Warning(error),
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_window_rules_follow_archivecreatewindow() {
        assert_eq!(parse_volume("No Volume", "No Volume"), Ok(None));
        assert_eq!(parse_volume("", "분할 안함"), Ok(None));
        assert_eq!(parse_volume("1440 KB", "x"), Ok(Some(1440 * 1024)));
        assert_eq!(parse_volume("1.5m", "x"), Ok(Some(1572864)));
        assert_eq!(parse_volume("0", "x"), Ok(None), "0 means no splitting");
        assert_eq!(parse_volume("12 bytes", "x"), Ok(Some(12)));
        assert!(parse_volume("ten MB", "x").is_err() && parse_volume("5 TB", "x").is_err() && parse_volume("1.2.3", "x").is_err());
        let zip = ArchiveFormat { extension: "zip".into(), label: String::new(), program: None, builtin: true, max_level: 9 };
        let tar = ArchiveFormat { extension: "tar".into(), max_level: 0, ..zip.clone() };
        let rar = ArchiveFormat { extension: "rar".into(), max_level: 5, ..zip.clone() };
        assert_eq!([0, 1, 2].map(|p| level_for_preset(&zip, p, 2)), [9, 5, 0]);
        assert_eq!([0, 1, 2].map(|p| level_for_preset(&rar, p, 2)), [5, 3, 0]);
        assert_eq!([0, 1, 2].map(|p| level_for_preset(&tar, p, 2)), [0, 0, 0]);
        assert_eq!([preset_for_level(0), preset_for_level(2), preset_for_level(7)], [2, 1, 0]);
        assert_eq!(default_archive_name(&[PathBuf::from("C:\\x\\report.final.docx")], Path::new("C:\\x"), "zip"), "report.final.zip");
        assert_eq!(default_archive_name(&[PathBuf::from("a"), PathBuf::from("b")], Path::new("C:\\data\\work\\"), "7z"), "work.7z");
        assert_eq!(default_archive_name(&[PathBuf::from("a"), PathBuf::from("b")], Path::new("D:\\"), "zip"), "D.zip");
    }
}

/// 다중 볼륨 압축 파일 다시 만들기(C# UpdateMultiVolumeArchiveAsync): `<폴더>\.ndr-multivolume-update-…\existing`에 기존 항목을 모두 풀고
/// 고른 항목을 덮어 합친 뒤 볼륨 이름의 묶음 이름("x.7z")으로 같은 분할 크기의 새 묶음을 만든다(남는 옛 볼륨은 지운다). 작업 폴더는 끝나면 지운다.
fn rebuild_multivolume(paths: &[PathBuf], first_volume: &Path, options: &ndr::commands::ArchiveOptions, events: &Sender<Event>, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let parent = first_volume.parent().ok_or("Invalid archive path")?;
    let mut random = [0u8; 16];
    native::random_bytes(&mut random)?;
    let stage = parent.join(format!(".ndr-multivolume-update-{}", random.iter().map(|b| format!("{b:02x}")).collect::<String>()));
    let existing = stage.join("existing");
    let result = (|| {
        std::fs::create_dir_all(&existing).map_err(|e| e.to_string())?;
        let _ = native::set_attributes(&stage, false, true);
        let mut overwrite = |_: &Path, _: &Path| ndr::fsops::ConflictChoice::Overwrite;
        let mut passwords = |archive: &Path, _: Option<&str>| super::commands::ask_password(events, cancel, archive);
        let initial = (!options.password.is_empty()).then_some(options.password.as_str());
        archive::extract_entries_with_program(first_volume, &existing, &[], initial, cancel, &mut overwrite, &mut passwords, None, &mut |done, _, _| {
            let _ = events.send(Event::Progress { path: first_volume.to_path_buf(), bytes: 0, files: done as u64 });
        })
        .map_err(|error| {
            let lower = error.to_lowercase();
            if lower.contains("password") || lower.contains("encrypted") || lower.contains("crypt") {
                format!("{error}\r\n압축 추가 창의 암호 칸에 기존 압축 파일 암호를 입력하세요.")
            } else {
                error
            }
        })?;
        for path in paths {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            let name = path.file_name().ok_or("Cannot archive a filesystem root")?;
            merge_tree(path, &existing.join(name))?;
        }
        let roots: Vec<PathBuf> = std::fs::read_dir(&existing).map_err(|e| e.to_string())?.filter_map(Result::ok).map(|e| e.path()).collect();
        if roots.is_empty() {
            return Err("업데이트된 압축 파일에 항목이 없습니다.".into());
        }
        // "x.7z.001" → "x.7z"(7-Zip이 .001, .002 …를 붙인다).
        let name = first_volume.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let base = match name.rsplit_once('.') {
            Some((stem, tail)) if tail.len() >= 3 && tail.bytes().all(|b| b.is_ascii_digit()) => parent.join(stem),
            _ => first_volume.to_path_buf(),
        };
        let rebuilt = ndr::commands::ArchiveOptions { append: false, recursive: true, ..options.clone() };
        archive::create_with_program(&roots, &base, &rebuilt, None, cancel)
    })();
    if stage.exists() {
        if let Err(error) = std::fs::remove_dir_all(&stage) {
            let _ = events.send(Event::Error { path: stage.clone(), message: format!("압축 파일 업데이트는 끝났지만 임시 파일을 정리하지 못했습니다. 폴더를 확인하세요. {error}") });
        }
    }
    result
}

/// 고른 항목을 작업 폴더에 덮어 합친다(C# MergeArchiveInputSnapshot: 같은 이름 파일은 바꾸고 폴더는 합친다).
fn merge_tree(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if ndr::fsops::is_reparse(&metadata) {
        return Err(format!("Links are not archived: {}", source.display()));
    }
    if metadata.is_dir() {
        if target.is_file() {
            std::fs::remove_file(target).map_err(|e| e.to_string())?;
        }
        std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
        for item in std::fs::read_dir(source).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            merge_tree(&item.path(), &target.join(item.file_name()))?;
        }
        return Ok(());
    }
    if target.is_dir() {
        std::fs::remove_dir_all(target).map_err(|e| e.to_string())?;
    }
    std::fs::copy(source, target).map(|_| ()).map_err(|e| e.to_string())
}
