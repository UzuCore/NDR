//! C# CreateArchiveAsync / AddToArchiveAsync(move) / CreateZipAsync / ExtractArchiveAsync(Selected·CurrentFolder·NewFolder·Auto) /
//! ShowArchivePropertiesAsync와 ArchiveCreateWindow([formAddToArchive]). 압축은 동봉한 7-Zip(또는 설정한 외부 프로그램)으로 한다.
//! 차이(HANDOFF): C#의 "내장" ZIP/7Z/TAR는 SharpCompress로 쓰지만 Rust는 동봉 7-Zip으로 쓴다(결과 형식은 같다). 다중 볼륨 압축 파일에
//! 더하기(볼륨 다시 만들기)는 아직 안 된다. 진행 창은 일반 작업 창이고 일시 정지가 없다.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndir::archive::{self, ExternalProgram};
use ndir::msgbox::{IDYES, MB_ICONEXCLAMATION, MB_OK, MB_YESNO};

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
    pub external: String,
    pub preset: usize,
    pub password: String,
    pub volume: String,
    pub recursive: bool,
    pub preset_labels: Vec<String>,
    pub volume_labels: Vec<String>,
    pub default_level: i32,
    /// 원본 TfAddToArc cbFileName: 지금 이름과 목록(기본 이름, 다른 패널 폴더의 같은 이름).
    pub file_name: String,
    pub file_names: Vec<String>,
    /// 원본 btnOkClick(0x417074): 외부 프로그램 칸 파일이 없으면 창을 닫지 않고 그 칸에 초점(SetFocus + SelectAll).
    pub focus_external: bool,
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
    /// 형식을 바꾸면 외부 프로그램 칸을 그 형식 것으로, cbFileName의 이름과 목록 확장자를 그 형식으로(원본 rbFormatClick).
    pub(super) fn set_format(&mut self, index: usize) {
        let Some(format) = self.formats.get(index) else { return };
        self.format = index;
        self.external = format.program.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let extension = format.extension.clone();
        if !self.file_name.trim().is_empty() {
            self.file_name = rename_extension(&self.file_name, &extension);
        }
        for name in &mut self.file_names {
            *name = rename_extension(name, &extension);
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
    pub external: Option<String>,
    /// 원본 cbFileName의 글(상대 경로면 지금 폴더 기준). None이면 저장 상자로 고른다(빠른 ZIP, 빈 이름).
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

/// 원본 miArcAddToArchiveClick 끝: cbFileName 글이 상대 경로(PathIsRelativeW)면 PathCombineW(지금 패널 폴더, 글), 아니면 그대로.
/// 빈 글이면 None.
pub(super) fn archive_output_path(text: &str, current: &Path) -> Option<PathBuf> {
    let text = text.trim().trim_matches('"');
    if text.is_empty() {
        return None;
    }
    let path = Path::new(text);
    let combined = if path.is_absolute() || text.starts_with("\\\\") { path.to_path_buf() } else { current.join(path) };
    Some(std::path::absolute(&combined).unwrap_or(combined))
}

/// EnsureArchiveOutputSafe: 결과가 원본 파일 자신이거나 원본 폴더 안이면 Err(글).
pub(super) fn output_conflict(sources: &[PathBuf], output: &Path, ko: bool) -> Option<String> {
    let full = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().into_owned();
    let output = full(output);
    for source in sources {
        let source_full = full(source);
        if source.is_file() && ndir::names::equals_ignore_case(&source_full, &output) {
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

/// 원본 TfAddToArc(ClientWidth 443 × ClientHeight 354): lSecName(20,16 굵게 파랑), lFileName(44,43) cbFileName(106,40 275×21)
/// btnBrowseFile(387,40 37×21), lSecFormat(20,76), rbZIP(44,104 굵게)·rbRAR(108)·rb7z(173)·rbALZ(238)·rbARJ(303),
/// lExtProg(44,129) eExternalProg(171,127 211×21) btnBrowseExtProg(387,127), lHelp(45,157 회색), lSecOptions(20,179),
/// lLevel(44,209) cbCompLevel(143,206 121×21) chkRecursive(276,208), lVolume(44,243) cbVolume(143,240), lPassword(44,276)
/// ePassword(143,273), Bevel1(20,310 404), btnOk(253,320) btnCancel(341,320) 83×25.
/// 원본 다섯 형식 밖의 C# 판 형식(TAR·GZ·EGG 등)은 라디오 줄 오른쪽 작은 목록에서 고른다.
/// 0 = 압축, 1 = 취소, 2 = 외부 프로그램 찾아보기, 3 = 이름 찾아보기.
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
    let name_width = if form.file_names.is_empty() { 275.0 } else { 253.0 };
    ui.put(at(106.0, 40.0, name_width, 21.0), egui::TextEdit::singleline(&mut form.file_name));
    if !form.file_names.is_empty() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(at(359.0, 40.0, 22.0, 21.0)), |ui| {
            ui.menu_button("▼", |ui| {
                for name in form.file_names.clone() {
                    if ui.button(&name).clicked() {
                        form.file_name = name;
                        ui.close();
                    }
                }
            });
        });
    }
    if ui.put(at(387.0, 40.0, 37.0, 21.0), egui::Button::new("...")).clicked() {
        choice = Some(3);
    }
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
    put_left(ui, at(44.0, 129.0, 125.0, 15.0), egui::Label::new(text("Caption.lExtProg", "E&xternal Program:")).truncate());
    let external = ui.put(at(171.0, 127.0, 211.0, 21.0), egui::TextEdit::singleline(&mut form.external));
    if std::mem::take(&mut form.focus_external) {
        // 원본 eExtProg SetFocus + SelectAll.
        external.request_focus();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), external.id) {
            let length = form.external.chars().count();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(length))));
            state.store(ui.ctx(), external.id);
        }
    }
    if ui.put(at(387.0, 127.0, 37.0, 21.0), egui::Button::new("...")).clicked() {
        choice = Some(2);
    }
    put_left(ui, at(45.0, 157.0, 380.0, 16.0), egui::Label::new(RichText::new(text("Caption.lHelp", "* Only ZIP format does not need an external program.")).color(Color32::GRAY)).truncate());
    put_left(ui, at(44.0, 209.0, 97.0, 15.0), egui::Label::new(text("Caption.lLevel", "Le&vel:")).truncate());
    let current = form.preset_labels.get(form.preset).cloned().unwrap_or_default();
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(143.0, 206.0, 121.0, 21.0)), |ui| {
        egui::ComboBox::from_id_salt("archive-level").width(113.0).selected_text(current).show_opaque(ui, |ui| {
            for (index, label) in form.preset_labels.iter().enumerate() {
                ui.selectable_value(&mut form.preset, index, label.as_str());
            }
        });
    });
    put_left(ui, at(276.0, 208.0, 156.0, 17.0), egui::Checkbox::new(&mut form.recursive, text("Caption.chkRecursive", "Includ&e Subfolders")));
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
    ndir::parity::format_size(value as u64, "B", true).trim_end_matches(" B").to_owned()
}

impl App {
    /// 원본 FUN_00404890(압축 엔진 오류 → [RES] 글): 7-Zip이 낸 오류 글이면 원본 압축 오류 글로 바꿔 보인다(보일 때만 바꾼다).
    /// 분류하지 못한 글과 7-Zip이 아닌 글은 그대로.
    pub(super) fn archive_error_text(&self, raw: &str) -> String {
        if !(raw.contains("7-Zip failed") || raw.contains("Bundled 7-Zip")) {
            return raw.to_owned();
        }
        let Some(key) = ndir::archive::original_error_key(raw) else { return raw.to_owned() };
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
    /// ExternalPrograms[key]를 ExpandSetting한 경로가 있는 파일이면.
    fn configured_program(&self, key: &str) -> Option<PathBuf> {
        let value = self.settings.values.get("ExternalPrograms")?.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?.1.as_str()?.to_owned();
        let path = PathBuf::from(ndir::command_line::expand_setting(&value));
        path.is_file().then_some(path)
    }

    fn template(&self, section: &str, key: &str) -> Option<String> {
        self.settings.values.get(section)?.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?.1.as_str().map(str::to_owned).filter(|t| !t.trim().is_empty())
    }

    /// GetArchiveFormatOptions(includeWimCreation).
    pub(super) fn archive_formats(&self, include_wim: bool) -> Vec<ArchiveFormat> {
        let ko = self.language == "ko";
        let format = |extension: &str, label: String, program: Option<PathBuf>, builtin: bool, max_level: i32| ArchiveFormat { extension: extension.into(), label, program, builtin, max_level };
        let mut formats = vec![format("zip", if ko { "ZIP (내장)" } else { "ZIP (built-in)" }.into(), None, true, 9)];
        if let Some(program) = self.configured_program("7z") {
            formats.push(format("zip", if ko { "ZIP (7-Zip 명령줄)" } else { "ZIP (7-Zip command line)" }.into(), Some(program), false, 9));
        }
        for extension in ["7z", "rar", "arj", "ace", "alz", "cab", "iso", "egg", "tar", "gz", "bz2", "bzip2", "z", "xz", "tar.gz", "tar.bz2"] {
            let Some(program) = self.configured_program(extension) else { continue };
            let label = if extension == "rar" {
                if ko { "RAR (RAR CLI 프로그램)".into() } else { "RAR (RAR command line)".into() }
            } else if ko {
                format!(".{} (외부 프로그램)", extension.to_uppercase())
            } else {
                format!(".{} (external program)", extension.to_uppercase())
            };
            let max = match extension {
                "rar" => 5,
                "arj" => 4,
                _ => 9,
            };
            formats.push(format(extension, label, Some(program), false, max));
        }
        formats.push(format("7z", if ko { "7Z (내장 LZMA2)" } else { "7Z (built-in LZMA2)" }.into(), None, true, 9));
        formats.push(format("tar", if ko { "TAR (내장)" } else { "TAR (built-in)" }.into(), None, true, 0));
        formats.push(format("tar.gz", if ko { "TAR.GZ (내장 GZip)" } else { "TAR.GZ (built-in GZip)" }.into(), None, true, 9));
        formats.push(format("tar.bz2", if ko { "TAR.BZ2 (내장 BZip2)" } else { "TAR.BZ2 (built-in BZip2)" }.into(), None, true, 9));
        // 원본 arcformat 4 = ALZ(외부 alzprog). 동봉 7-Zip(UzuCore 7zip+α)이 ALZ를 만들므로 내장으로 둔다.
        // 동봉 7zip+α의 EGG 만들기(원본·C# 판에 없음)는 사용자 요청으로 넣지 않는다(.egg 읽기·풀기는 그대로).
        formats.push(format("alz", if ko { "ALZ (내장)" } else { "ALZ (built-in)" }.into(), None, true, 9));
        for (extension, ko_label, en_label, max) in [("rar", "RAR (외부 프로그램 선택)", "RAR (choose an external program)", 5), ("arj", "ARJ (외부 프로그램 선택)", "ARJ (choose an external program)", 4)] {
            if !formats.iter().any(|f| f.extension.eq_ignore_ascii_case(extension)) {
                formats.push(format(extension, if ko { ko_label } else { en_label }.into(), None, false, max));
            }
        }
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
            let choice = ArchiveChoice { format: zip, format_index: 0, preset: 1, level: 2, password: String::new(), volume: None, recursive: true, external: None, output: None };
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
        // 원본 miArcAddToArchiveClick(0x43a7f8): cbFileName = 기본 이름(하나면 그 이름, 아니면 폴더 이름), 다른 패널이 로컬 폴더면
        // 그 폴더 + 기본 이름도 목록에 넣고 첫 줄을 고른다. 확장자는 고른 형식(rbFormatClick).
        let current = self.panes[self.active].path.clone();
        let name = default_archive_name(&paths, &current, &formats[format].extension);
        let mut file_names = vec![name.clone()];
        let other = &self.panes[1 - self.active];
        if self.dual && other.archive.is_none() && other.remote.is_none() && !super::panes::is_drives(&other.path) && !ndir::names::equals_ignore_case(&other.path.to_string_lossy(), &current.to_string_lossy()) {
            file_names.push(other.path.join(&name).to_string_lossy().into_owned());
        }
        let form = ArchiveCreateForm {
            external: formats[format].program.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
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
            file_names,
            focus_external: false,
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

    /// 창 단추: 압축(분할 크기 검사), 취소, 외부 프로그램 찾아보기.
    pub(super) fn submit_archive_create(&mut self, choice: usize, ctx: &egui::Context) {
        let open = self.open_dialog;
        let save = self.save_dialog;
        let current = self.panes[self.active].path.clone();
        let browse_title = self.language_pack.text("formAddToArchive.Caption.lExtProg", "Select external archive program");
        let name_title = self.language_pack.text("formAddToArchive.Caption.lFileName", "&Filename:").replace('&', "");
        let program_filters = super::original_dialogs::program_filter(&self.language_pack);
        let all_files = self.tr("모든 파일 (*.*)", "All files (*.*)").into_owned();
        let Some(form) = self.archive_form() else { return };
        match choice {
            1 => self.answer_dialog(DialogResult::ArchiveCreate(None), ctx),
            2 => {
                let filters: Vec<(&str, &str)> = program_filters.iter().map(|(n, p)| (n.as_str(), p.as_str())).collect();
                if let Some(path) = open(&browse_title, None, "", &filters) {
                    if let Some(form) = self.archive_form() {
                        form.external = path.to_string_lossy().into_owned();
                    }
                }
            }
            3 => {
                // cbFileName 옆 찾아보기(원본 btnBrowseFile은 숨김): 저장 상자로 이름을 고른다.
                let extension = form.formats.get(form.format).map(|f| f.extension.clone()).unwrap_or_default();
                let named = archive_output_path(&form.file_name, &current);
                let folder = named.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or(current.clone());
                let file = named.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let spec = format!("*.{extension}");
                let label = format!(".{} (*.{extension})", extension.to_uppercase());
                if let Some(path) = save(&name_title, Some(&folder), &file, &[(label.as_str(), spec.as_str()), (all_files.as_str(), "*.*")], &extension, false) {
                    if let Some(form) = self.archive_form() {
                        form.file_name = path.to_string_lossy().into_owned();
                    }
                }
            }
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
                    external: Some(form.external.trim().to_owned()).filter(|e| !e.is_empty()),
                    output: archive_output_path(&form.file_name, &current),
                };
                // 원본 btnOkClick(0x417074): ZIP이 아니면 외부 프로그램 칸의 파일이 있어야(PathFileExistsW) 창을 닫고,
                // 없으면 창을 그대로 두고 그 칸에 초점을 준다. NDIR은 C# 판처럼 동봉 7-Zip이 만들 수 있으면 빈 칸도 받는다.
                let zip = choice.format.extension.trim_start_matches('.').eq_ignore_ascii_case("zip");
                let missing = !zip
                    && match &choice.external {
                        Some(path) => !Path::new(path).exists(),
                        None => self.archive_program(&choice).is_err(),
                    };
                if missing {
                    if let Some(form) = self.archive_form() {
                        form.focus_external = true;
                    }
                    return;
                }
                self.answer_dialog(DialogResult::ArchiveCreate(Some(choice)), ctx);
            }
        }
    }

    /// 외부 프로그램이 필요하면 그 경로(C# useExternalProgram / ResolveArchiveProgram). Ok(None) = 동봉 7-Zip.
    pub(super) fn archive_program(&self, choice: &ArchiveChoice) -> Result<Option<ExternalProgram>, String> {
        let extension = choice.format.extension.as_str();
        // 동봉 7zip+α는 ALZ·EGG도 나눠 만든다(.001…). 암호는 넣지 않으므로 그때만 외부 프로그램.
        let splits = matches!(extension, "zip" | "7z") || (choice.format.builtin && matches!(extension, "alz" | "egg"));
        let needs_program = !choice.format.builtin || choice.external.is_some() || (!choice.password.is_empty() && !matches!(extension, "zip" | "7z")) || (choice.volume.is_some() && !splits);
        if !needs_program {
            return Ok(None);
        }
        let key = if matches!(extension, "zip" | "7z") { "7z" } else { extension };
        let program = choice.external.as_ref().map(PathBuf::from).or_else(|| choice.format.program.clone()).or_else(|| self.configured_program(key));
        match program.filter(|p| p.is_file()) {
            Some(program) => Ok(Some(ExternalProgram { program, create_template: self.template("ExternalArchiveArgumentTemplates", extension), extract_template: None })),
            None if matches!(extension, "zip" | "7z") && choice.format.builtin => Ok(None),
            None => Err(if self.language == "ko" { format!(".{extension} 압축을 외부 처리하려면 환경 설정에 사용할 명령줄 압축 프로그램을 등록해야 합니다.") } else { format!("Register a command-line archive program in the settings to create .{extension} archives.") }),
        }
    }

    /// 저장 대화 상자부터 검사·확인·실행까지.
    fn archive_save_step(&mut self, kind: ArchiveKind, paths: Vec<PathBuf>, choice: ArchiveChoice, _unused: Option<()>) {
        let ko = self.language == "ko";
        let program = match self.archive_program(&choice) {
            Ok(program) => program,
            Err(error) => return self.system_box("NDIR", &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {}),
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
            return self.system_box("NDIR", &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
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
        let options = ndir::commands::ArchiveOptions {
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
        }
    }

    /// TryGetExternalArchiveProgram: UseExternalArchivePrograms이거나 NDIR이 직접 못 읽는 형식이면 ExternalPrograms[형식] 또는 [7z].
    /// 고른 항목만 풀 때 풀기 인자 형식이 없고 직접 읽을 수 있으면 동봉 7-Zip.
    fn extract_program(&self, archive: &Path, selected_keys: bool) -> Option<ExternalProgram> {
        let format = archive::archive_format(archive)?;
        let internal = archive::can_read_internally(archive);
        if !(self.settings.flag("UseExternalArchivePrograms", false) || !internal) {
            return None;
        }
        let program = self.configured_program(&format).or_else(|| self.configured_program("7z"))?;
        let template = self.template("ExternalArchiveExtractArgumentTemplates", &format);
        if selected_keys && template.is_none() && internal {
            return None;
        }
        Some(ExternalProgram { program, create_template: None, extract_template: template })
    }

    /// C# OpenItemAsync(동봉 7-Zip이 못 읽는 압축, 예: .ace): TryGetExternalArchiveProgram이 있으면 "{폴더}\{확장자 뗀 이름}"에
    /// 그 프로그램으로 통째로 풀고 그 폴더로 간다. 로컬 압축 파일(`ask_if_not_empty`)은 대상 폴더가 비어 있지 않으면
    /// C# 판처럼 "여기에 추가할까요?"를 묻는다. 프로그램이 없으면 C# NotSupportedException 글.
    pub(super) fn open_with_external_archive_program(&mut self, pane: usize, archive_path: PathBuf, ask_if_not_empty: bool) {
        let Some(program) = self.extract_program(&archive_path, false) else {
            let extension = archive_path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            return self.warn(
                &format!("{extension} 형식은 내부에서 열 수 없습니다. 환경 설정에서 외부 압축 프로그램을 지정하세요."),
                &format!("The {extension} format cannot be opened internally. Choose an external archive program in Options."),
            );
        };
        let parent = archive_path.parent().map(Path::to_path_buf).unwrap_or_default();
        let destination = parent.join(archive::extract_folder_name(&archive_path));
        let not_empty = destination.read_dir().is_ok_and(|mut entries| entries.next().is_some());
        let start = move |app: &mut App| {
            let tx = app.async_tx.clone();
            let title = app.tr("압축 열기", "Open archive").into_owned();
            let target = destination.clone();
            app.start_task(title, move |events, cancel| {
                // C# RunExternalExtractWithPasswordPromptAsync: 암호 진단이면 TITLE_PASSWORD·MSG_PASSWORD로 묻고 다시 푼다. 취소면 조용히 끝.
                let mut passwords = |archive: &Path| super::commands::ask_password(&events, &cancel, archive);
                match archive::extract_whole_with_program_with_passwords(&archive_path, &target, None, &program, &cancel, &mut passwords) {
                    Err(error) if error == "Archive password request cancelled" => return Ok(()),
                    other => other?,
                }
                let _ = tx.send(AsyncResult::OpenFolder { pane, path: target });
                Ok(())
            });
        };
        if ask_if_not_empty && not_empty {
            let (title, text) = (self.tr("압축 열기", "Open archive").into_owned(), self.tr("압축 풀기 대상 폴더가 비어 있지 않습니다. 여기에 추가할까요?", "The extraction folder is not empty. Add the files there?").into_owned());
            self.system_box(&title, &text, MB_YESNO | MB_ICONEXCLAMATION, move |app, answer, _| {
                if answer == IDYES {
                    start(app);
                }
            });
        } else {
            start(self);
        }
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
                    if !list.iter().any(|p| ndir::names::equals_ignore_case(&p.to_string_lossy(), &first.to_string_lossy())) {
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
        let programs: Vec<Option<ExternalProgram>> = archives.iter().map(|a| self.extract_program(a, !keys.is_empty())).collect();
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
                let modified = ndir::listing::date_text(metadata.modified().ok(), &date_format, &time_format, false, ko, std::time::SystemTime::now());
                let name = archive_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let size = |b: u64| ndir::parity::format_size(b, &unit, thousands);
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

/// 다중 볼륨 압축 파일 다시 만들기(C# UpdateMultiVolumeArchiveAsync): `<폴더>\.ndir-multivolume-update-…\existing`에 기존 항목을 모두 풀고
/// 고른 항목을 덮어 합친 뒤 볼륨 이름의 묶음 이름("x.7z")으로 같은 분할 크기의 새 묶음을 만든다(남는 옛 볼륨은 지운다). 작업 폴더는 끝나면 지운다.
fn rebuild_multivolume(paths: &[PathBuf], first_volume: &Path, options: &ndir::commands::ArchiveOptions, events: &Sender<Event>, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let parent = first_volume.parent().ok_or("Invalid archive path")?;
    let mut random = [0u8; 16];
    native::random_bytes(&mut random)?;
    let stage = parent.join(format!(".ndir-multivolume-update-{}", random.iter().map(|b| format!("{b:02x}")).collect::<String>()));
    let existing = stage.join("existing");
    let result = (|| {
        std::fs::create_dir_all(&existing).map_err(|e| e.to_string())?;
        let _ = native::set_attributes(&stage, false, true);
        let mut overwrite = |_: &Path, _: &Path| ndir::fsops::ConflictChoice::Overwrite;
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
        let rebuilt = ndir::commands::ArchiveOptions { append: false, recursive: true, ..options.clone() };
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
    if ndir::fsops::is_reparse(&metadata) {
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
