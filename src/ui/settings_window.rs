//! 원본 TfPreferences("옵션") 쪽 차례(파일목록·폴더트리·색·옵션·프로그램·경로·기능키·기타)와 [formPreferences] 글로 C# SettingsWindow의 줄을 옮기고,
//! C# 판에만 있는 줄은 마지막 "NDR" 쪽에 둔다. 각 줄은 C#과 같은 키·처음 값,
//! 확인 = Commit(키마다 같은 변환·범위, 틀리면 "환경 설정" 경고 상자), 취소. 확장자 색 가져오기/내보내기(.col/.json), 기능키 편집 단추.
//! 확인한 뒤의 적용은 C# ShowOptionsAsync(언어·메뉴·테마·두 패널·보기·새로 고침, 저장).
//! 차이(HANDOFF): 영어 화면은 줄 이름이 모두 영어(C#은 언어 파일에 있는 것만), 한 번에 하나만 실행·활성화 단축키·창 투명도 바꾸기는 다음 시작부터.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{IDYES, MB_ICONEXCLAMATION, MB_ICONQUESTION, MB_OK, MB_YESNO};
use serde_json::{Map, Value, json};

fn t(ko: bool, korean: &str, english: &str) -> String {
    if ko { korean.into() } else { english.into() }
}

/// 한 줄(C# AddCheck/AddText/AddFolderPath/AddProgramPath/AddCombo/AddSlider/AddSection과 안내 글).
#[derive(Clone, Debug)]
pub(super) enum Row {
    Section(String),
    Note(String),
    Check { key: String, label: String, value: bool },
    Text { key: String, label: String, value: String, read_only: bool, max: usize, upper: bool },
    Folder { key: String, label: String, value: String, default: PathBuf },
    Program { key: String, label: String, value: String },
    /// (값, 보이는 글) 목록과 고른 줄. 값이 목록에 없으면 None(C# SelectedValue = null → 빈 값).
    Combo { key: String, label: String, choices: Vec<(String, String)>, selected: Option<usize> },
    Slider { key: String, label: String, value: f32 },
    ColorButtons,
    FunctionKeys,
    /// 원본 btnListFont / btnTreeFont("글꼴 변경", TFontDialog). tree = 폴더 트리 글꼴.
    FontButton { tree: bool, label: String },
}

pub(super) struct SettingsForm {
    ko: bool,
    pub tab: usize,
    pub tabs: Vec<Vec<Row>>,
    /// 쪽 이름(원본 TabSheet.ts* 8개 + "NDR").
    pub tab_titles: Vec<String>,
    /// C# _draft(원래 설정의 사본). 기능키 편집·색상표 가져오기가 바로 고친다.
    pub draft: ndr::settings::Settings,
    /// 원본 lbExtColor에서 고른 색 묶음(색이 같은 확장자들).
    pub color_selected: Option<usize>,
    /// 원본 TColorDialog.Color(마지막으로 고른 색, COLORREF). 추가할 때 처음 색.
    pub last_color: u32,
    /// 원본 tsColor 글: lExtColor, btnAddColor, btnEditColor, btnDelColor, btnSaveColorMap, btnImportColor, lHelp, lBasicColor.
    color_captions: [String; 8],
    /// 원본 lbBasicColor 줄(ListBox.lbBasicColor, ';' 구분 23줄: 0·16 머리, 15 빈 줄).
    basic_items: Vec<String>,
    /// 원본 btnOk, btnCancel, btnApply 글.
    button_captions: [String; 3],
    /// 원본 TfPreferences의 그룹·안내 글(쪽을 원본 자리로 그릴 때 쓴다).
    captions: std::collections::HashMap<&'static str, String>,
    /// 원본 tsFunctionKey lvFunctionKey(F1~F12, ^F1~^F11).
    pub function_keys: super::function_key_editor::FunctionKeyEditorForm,
}

/// 원본 FUN_00450b94/0x44ed5c: lbBasicColor 줄 번호 → TNXBasicColor 번호(머리 0·16과 빈 줄 15는 없음).
pub(super) fn basic_color_index(row: usize) -> Option<usize> {
    match row {
        0 | 15 | 16 => None,
        1..=14 => Some(row - 1),
        _ => Some(row - 3),
    }
}

/// 설정의 기본 색(NexusFileColorMapValues, 없으면 .col 기본값). "RRGGBB".
pub(super) fn basic_color(draft: &ndr::settings::Settings, index: usize) -> Option<String> {
    let (key, default) = ndr::styles::DEFAULT_ELEMENTS.get(index)?;
    let stored = object(draft, "NexusFileColorMapValues").get(*key).and_then(|v| v.as_str()).map(|v| v.trim_start_matches('#').to_owned());
    Some(stored.filter(|v| v.len() == 6).unwrap_or_else(|| (*default).to_owned()))
}

/// 원본 TNXFileColorMap의 lbExtColor 항목: 같은 색의 확장자를 한 줄로("TXT;DOC", 색). ExtensionColors 글(".txt=#RRGGBB; …")에서 처음 나온 색 순서로.
pub(super) fn color_groups(text: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (extension, color) in parse_extension_colors(text).unwrap_or_default() {
        let name = extension.trim_start_matches('.').to_uppercase();
        match groups.iter_mut().find(|(c, _)| c.eq_ignore_ascii_case(&color)) {
            Some((_, names)) => names.push(name),
            None => groups.push((color, vec![name])),
        }
    }
    groups
}

/// 묶음을 다시 ExtensionColors 글로.
fn groups_text(groups: &[(String, Vec<String>)]) -> String {
    groups.iter().flat_map(|(color, names)| names.iter().map(move |n| format!(".{}={color}", n.to_lowercase()))).collect::<Vec<_>>().join("; ")
}

/// 원본 FUN_0044ee6c(';'로 나눔, 빈 조각 빼기). 앞의 점은 뺀다.
fn split_extensions(text: &str) -> Vec<String> {
    text.split(';').map(|s| s.trim().trim_start_matches('.').to_uppercase()).filter(|s| !s.is_empty()).collect()
}

/// "#RRGGBB" ↔ COLORREF(0x00BBGGRR).
pub(super) fn colorref_of(hex: &str) -> u32 {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    ((value & 0xFF) << 16) | (value & 0xFF00) | ((value >> 16) & 0xFF)
}
pub(super) fn hex_of(colorref: u32) -> String {
    format!("#{:02X}{:02X}{:02X}", colorref & 0xFF, (colorref >> 8) & 0xFF, (colorref >> 16) & 0xFF)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsAction {
    BrowseFolder(usize),
    BrowseProgram(usize),
    ImportColors,
    ExportColors,
    /// 원본 btnAddColor·btnEditColor·btnDelColor·lbExtColor 두 번 누르기.
    AddColor,
    EditColor,
    DeleteColor,
    ChangeColor,
    /// 원본 lbBasicColorDblClick(0x44ed5c): 기본 색(TNXBasicColor 번호) 바꾸기.
    ChangeBasicColor(usize),
    /// 원본 btnFontClick(Tag 0 = 파일 목록, 1 = 폴더 트리).
    ChooseFont(bool),
    /// 원본 btnBrowse1/2(시작 폴더 "..."): 글 줄에 폴더를 고른다.
    BrowseText(usize),
    /// 원본 btnEditFunc(lvFunctionKey 두 번 누르기도): 고른 키를 TfEditFunction으로 고친다.
    EditFunctionKey,
    /// 원본 btnFuncDelete: 고른 키를 기본으로.
    ResetFunctionKey,
    /// 원본 btnApplyClick: 확인처럼 검사·적용하되 창은 그대로.
    Apply,
    Ok,
    Cancel,
}

/// C# LanguageService.Names(21개 언어).
const LANGUAGE_NAMES: [(&str, &str); 21] = [
    ("ca", "Català"), ("cs", "Čeština"), ("de", "Deutsch"), ("en", "English"), ("es", "Español"), ("fi", "Suomi"), ("fr", "Français"),
    ("hu", "Magyar"), ("it", "Italiano"), ("ja", "日本語"), ("ko", "한국어"), ("nl", "Nederlands"), ("pl", "Polski"), ("pt-BR", "Português (Brasil)"),
    ("ru", "Русский"), ("sk", "Slovenčina"), ("sv", "Svenska"), ("tr", "Türkçe"), ("uk", "Українська"), ("zh-CN", "简体中文"), ("zh-TW", "繁體中文"),
];

/// C# AvailableLanguages: Languages\*.lang 가운데 이름을 아는 것, 이름 순.
pub(super) fn available_languages(resources: &Path) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = std::fs::read_dir(resources.join("Languages"))
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter_map(|e| {
                    let path = e.path();
                    let stem = path.extension().filter(|x| x.eq_ignore_ascii_case("lang")).and(path.file_stem())?.to_string_lossy().into_owned();
                    LANGUAGE_NAMES.iter().find(|(code, _)| code.eq_ignore_ascii_case(&stem)).map(|(code, name)| ((*code).to_owned(), (*name).to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by(|a, b| a.1.cmp(&b.1));
    list
}

/// C# ParseExtensionColors: ".확장자=#RRGGBB"를 세미콜론으로, 확장자는 소문자, 색은 대문자.
pub(super) fn parse_extension_colors(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut result: Vec<(String, String)> = Vec::new();
    for entry in text.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let Some(separator) = entry.find('=').filter(|s| *s > 0) else {
            return Err(format!("확장자 색상 규칙 형식이 잘못되었습니다: {entry}"));
        };
        let mut extension = entry[..separator].trim().trim_start_matches('*').to_owned();
        if !extension.starts_with('.') {
            extension.insert(0, '.');
        }
        let color = entry[separator + 1..].trim();
        let bad_name = extension.chars().any(|c| "\"<>|:*?\\/".contains(c) || (c as u32) < 32);
        if !(2..=32).contains(&extension.chars().count()) || bad_name || color.len() != 7 || !color.starts_with('#') || !color[1..].chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("확장자 색상 규칙은 .확장자=#RRGGBB 형식이어야 합니다: {entry}"));
        }
        let extension = extension.to_lowercase();
        result.retain(|(k, _)| *k != extension);
        result.push((extension, color.to_uppercase()));
    }
    Ok(result)
}

fn row_key(row: &Row) -> Option<&str> {
    match row {
        Row::Check { key, .. } | Row::Text { key, .. } | Row::Folder { key, .. } | Row::Program { key, .. } | Row::Combo { key, .. } | Row::Slider { key, .. } => Some(key),
        _ => None,
    }
}

fn relabel(mut row: Row, text: String) -> Row {
    if let Row::Check { label, .. } | Row::Text { label, .. } | Row::Folder { label, .. } | Row::Program { label, .. } | Row::Combo { label, .. } | Row::Slider { label, .. } = &mut row {
        *label = text;
    }
    row
}

/// 원본 TfPreferences(DFM -Tree) 쪽 차례: tsDisplay, tsTree, tsColor, tsOption, tsExtProgram, tsPath, tsFunctionKey, tsEtc.
/// 원본에 같은 설정이 있는 줄은 그 쪽·차례·글([formPreferences] Caption.*, ComboBox.*)로 옮기고, C# 판에만 있는 줄은 마지막 "NDR" 쪽에
/// 원래 차례로 남긴다(그 줄이 하나도 남지 않은 머리·안내 글은 뺀다).
fn original_pages(built: Vec<Vec<Row>>, language: &compat::Language, ko: bool) -> (Vec<String>, Vec<Vec<Row>>) {
    let lang = |key: &str, fallback: &str| language.text(&format!("formPreferences.{key}"), fallback);
    let mut pool: Vec<Option<Row>> = built.into_iter().flatten().map(Some).collect();
    let mut take = |key: &str, label: Option<String>| -> Option<Row> {
        let index = pool.iter().position(|r| r.as_ref().and_then(row_key) == Some(key))?;
        let row = pool[index].take()?;
        Some(match label {
            Some(label) => relabel(row, label),
            None => row,
        })
    };
    // 원본 ComboBox 글이 같은 개수면 보이는 글만 원본으로(값은 그대로).
    let combo_texts = |row: Option<Row>, key: &str| -> Option<Row> {
        let mut row = row?;
        if let Row::Combo { choices, .. } = &mut row {
            let texts: Vec<String> = lang(key, "").split(';').map(str::to_owned).collect();
            if texts.len() == choices.len() {
                for ((_, display), text) in choices.iter_mut().zip(texts) {
                    *display = text;
                }
            }
        }
        Some(row)
    };
    let section = |key: &str, fallback: &str| Some(Row::Section(lang(key, fallback).replace('&', "")));
    let l = |key: &str, fallback: &str| Some(lang(key, fallback));
    let mut file_list = vec![
        section("GroupBox.gbFont", "Font"),
        take("ListFont", l("Caption.lListFont", "File List:")),
        take("ListFontSize", None),
        Some(Row::FontButton { tree: false, label: lang("Caption.btnListFont", "Change Font").replace('&', "") }),
        section("RadioGroup.rgCharCase", "Charcase"),
        take("CharacterCase", l("RadioGroup.rgCharCase", "Charcase")),
        section("GroupBox.gbViewStyle", "File List"),
        take("ShowIcons", l("Caption.chkShowIcon", "Show Icon")),
        take("ShowDriveList", l("Caption.chkDriveList", "Show Drive List")),
        take("ShowColumnLines", l("Caption.chkShowColLine", "Show Grid Column Line")),
        take("ShowRowLines", l("Caption.chkShowRowLine", "Show Grid Row Line")),
        take("UseLogicalSort", l("Caption.chkUseLogicalSort", "Use logical sort in file list")),
        take("ListLineSpace", l("Caption.lLineSpace", "Line Space:")),
        take("ThumbnailSize", l("Caption.lThumbSize", "Thumb Size:")),
    ];
    for (side, n) in [("Left", "1"), ("Right", "2")] {
        file_list.push(section(&format!("Caption.lListView{n}"), if n == "1" { "Left Pane" } else { "Right Pane" }));
        let view = take(&format!("{side}ViewStyle"), l(&format!("Caption.lViewStyle{n}"), "List Style"));
        file_list.push(combo_texts(view, &format!("ComboBox.cbViewStyle{n}")));
        file_list.push(take(&format!("{side}AlwaysShowFolderSize"), l(&format!("Caption.chkAlwaysShowSize{n}"), "Always Show Size")));
        file_list.push(take(&format!("{side}ScrollStep"), l(&format!("Caption.chkScrollStep{n}"), "Scroll by Page")));
        let sort = take(&format!("{side}SortMode"), l(&format!("Caption.lSort{n}"), "Sort"));
        file_list.push(combo_texts(sort, &format!("ComboBox.cbSort{n}")));
        file_list.push(take(&format!("{side}SortDescending"), l(&format!("Caption.chkSortDesc{n}"), "Descend")));
        file_list.push(take(&format!("{side}ShowHiddenFiles"), l(&format!("Caption.chkShowHidden{n}"), "Show Hidden Files")));
        // 원본 cbColumn1/2("자동;1열;…;6열"): 앞 7칸은 원본 글, NDR만의 7~19열은 같은 꼴("6열"의 6을 바꾼 글).
        let mut columns = take(&format!("{side}ColumnCount"), l(&format!("Caption.lColumn{n}"), "Column"));
        if let Some(Row::Combo { choices, .. }) = columns.as_mut() {
            let texts: Vec<String> = lang(&format!("ComboBox.cbColumn{n}"), "").split(';').map(str::to_owned).collect();
            if texts.len() == 7 {
                let pattern = texts[6].clone();
                for (index, (_, display)) in choices.iter_mut().enumerate() {
                    *display = texts.get(index).cloned().unwrap_or_else(|| pattern.replacen('6', &index.to_string(), 1));
                }
            }
        }
        file_list.push(columns);
    }
    file_list.extend([
        section("GroupBox.gbFormat", "Format"),
        take("DateFormat", l("Caption.lDateFormat", "Date Format:")),
        take("UseDateAge", l("Caption.chkUseDateAge", "Use Date Age")),
        take("TimeFormat", l("Caption.lTimeFormat", "Time Format:")),
        combo_texts(take("SizeUnit", l("Caption.lSizeFormat", "Size Format:")), "ComboBox.cbSizeFormat"),
        take("UseThousandsSeparator", l("Caption.chkThousandsSeparator", "Use Thousands Separator")),
        take("AttachExtensionToName", l("Caption.chkAttachExt", "Attach Extension to name")),
    ]);
    let tree = vec![
        take("TreeWindowMatchesMainSize", l("Caption.chkTreeMainWndSize", "Open folder tree window with size of main window")),
        take("SaveFolderTreeState", l("Caption.chkTreeFileInRoot", "Save folder tree data file in root folders of each drive")),
        take("ShowHiddenFoldersInTree", l("Caption.chkTreeShowHidden", "Show Hidden Folders")),
        take("ShowFolderTreeIcons", l("Caption.chkTreeShowIcon", "Show Icon")),
        take("FolderTreeItemWidth", l("Caption.lItemWidth", "Folder Name Width:")),
        section("GroupBox.gbTreeFont", "Font"),
        take("FolderTreeFont", l("Caption.lTreeFont", "Folder Tree:")),
        take("FolderTreeFontSize", None),
        Some(Row::FontButton { tree: true, label: lang("Caption.btnTreeFont", "Change Font").replace('&', "") }),
    ];
    let color = vec![take("UseNexusFileColorMap", None), take("ExtensionColors", None), Some(Row::ColorButtons)];
    let option = vec![
        take("SelectFolders", l("Caption.chkSelectAllFolder", "Select folder when select all")),
        take("UseExplorerMenu", l("Caption.chkExplorerMenu", "Use an explorer menu for right click")),

        take("RemoveReadOnlyWhenCopying", l("Caption.chkRemoveReadOnly", "Remove read only attribute when copying from CD/DVD")),
        take("SaveDriveFolders", l("Caption.chkDriveFolders", "Save last folder of each drive")),
        take("ConfirmDragDrop", l("Caption.chkConfirmDrop", "Confirm on drag and drop operation")),
        take("PlayCompletionSound", l("Caption.chkCompleteSound", "Play sound when operation is completed")),
        take("PlayWaveInProgram", l("Caption.chkPlayWave", "Play .wav(Wave Sound) file in this program.")),
        take("UseTabs", l("Caption.chkUseTab", "Use Tabs")),
        take("ShowDeleteButton", l("Caption.chkShowDeleteButton", "Show 'Delete' button on delete confirmation dialog")),
        take("DefaultDeleteButton", l("Caption.lDefDelBtn", "Default Button :")),
        take("EscapeKeyAction", l("Caption.lESC", "ESC function:")),
    ];
    // 원본 gbArchive(외부 압축 프로그램 ARJ·RAR·7Z·ALZ)는 없앴다(사용자 요청: 압축은 늘 동봉 7-Zip).
    let programs = vec![
        section("GroupBox.gbPrograms", "External Program"),
        take("TextViewerPath", l("Caption.lView", "View:")),
        take("TextEditorPath", l("Caption.lEdit", "Edit:")),
    ];
    let path = vec![
        section("GroupBox.gbMyFolders", "Paths"),
        Some(Row::Note(lang("Caption.lHelpPath", "Leave blank to use system default path"))),
        take("SpecialFolder:Documents", l("Caption.lMyDocuments", "Documents:")),
        take("SpecialFolder:Pictures", l("Caption.lMyPictures", "Pictures:")),
        take("SpecialFolder:Music", l("Caption.lMyMusic", "Music:")),
        take("SpecialFolder:Videos", l("Caption.lMyVideos", "Videos:")),
        take("SpecialFolder:Downloads", l("Caption.lPathDownloads", "Downloads:")),
        section("GroupBox.gbStartPath", "Start Folder"),
        Some(Row::Note(lang("Caption.lHelpStartPath", "Always start NexusFile  with following folders"))),
        take("LeftStartPathEnabled", l("Caption.chkStartPath1", "Left Pane:")),
        take("LeftStartPath", None),
        take("RightStartPathEnabled", l("Caption.chkStartPath2", "Right Pane:")),
        take("RightStartPath", None),
    ];
    let activation = format!("{} {}", lang("Caption.lActivateKey", "Bring NexusFile to front:").trim(), lang("Caption.lWinKey", "WinKey + ").trim());
    let etc = vec![
        take("MaxRecentFileCount", l("Caption.lMostUsedFolder", "The number of recent folder on Select Destination dialog:")),
        take("ExplorerShortcutKey", Some(activation)),
        Some(Row::Note(format!(
            "{} {}",
            lang("Caption.lApplyAfterRestart", "(Restart required)"),
            t(ko, "Win+문자 조합을 사용할 수 없으면 Win+Shift+문자 조합으로 등록을 시도합니다.", "If Win+letter is unavailable, NDR tries Win+Shift+letter.")
        ))),
        take("CheckForUpdatesOnStartup", l("Caption.chkCheckUpdate", "Check update when program starts")),
        take("UseDescriptionFile", l("Caption.chkUseION", "Read comment file \"descript.ion\" ")),
        take("UseSingleInstance", l("Caption.chkRunOnce", "Allow only one NexusFile running")),
        take("UseAlternateDriveHandling", l("Caption.chkHanShiftDrive", "Use Shift + Drive Letter regardless of input mode")),
        take("HideWhenMinimized", l("Caption.chkHideWhenMinimized", "Hide when minimized")),
        section("GroupBox.gbAlpha", "Window Transparency"),
        take("UseWindowAlpha", l("Caption.chkAlphaWindow", "Use Transparent Window")),
        take("WindowAlpha", Some(format!("{} — {}", lang("Caption.lTransMin", "Transparent"), lang("Caption.lTransMax", "Opaque")))),
        take("UseMenuAlpha", l("Caption.chkMenuAlpha", "Use Transparent Menu")),
    ];
    let function_keys = {
        let index = pool.iter().position(|r| matches!(r, Some(Row::FunctionKeys)));
        vec![index.and_then(|i| pool[i].take())]
    };
    // 남은 줄(C# 판에만 있는 것). 머리·안내 글은 그 묶음에 남은 줄이 있을 때만.
    let mut rest = Vec::new();
    for (i, row) in pool.iter().enumerate() {
        let Some(row) = row else { continue };
        if matches!(row, Row::ColorButtons) {
            continue;
        }
        if matches!(row, Row::Note(_)) {
            let previous_stays = i > 0 && matches!(&pool[i - 1], Some(r) if !matches!(r, Row::Section(_) | Row::Note(_) | Row::ColorButtons));
            if !previous_stays {
                continue;
            }
        }
        if matches!(row, Row::Section(_)) {
            let alive = pool[i + 1..].iter().take_while(|r| !matches!(r, Some(Row::Section(_)))).any(|r| matches!(r, Some(row) if !matches!(row, Row::Note(_) | Row::Section(_) | Row::ColorButtons)));
            if !alive {
                continue;
            }
        }
        rest.push(row.clone());
    }
    let titles = [
        ("TabSheet.tsDisplay", "File List"),
        ("TabSheet.tsTree", "Folder Tree"),
        ("TabSheet.tsColor", "Color"),
        ("TabSheet.tsOption", "Option"),
        ("TabSheet.tsExtProgram", "Programs"),
        ("TabSheet.tsPath", "Path"),
        ("TabSheet.tsFunctionKey", "Function Key"),
        ("TabSheet.tsEtc", "Etc"),
    ]
    .iter()
    .map(|(key, fallback)| lang(key, fallback))
    .chain(std::iter::once("NDR".to_owned()))
    .collect();
    let pages = [file_list, tree, color, option, programs, path, function_keys, etc].into_iter().map(|page| page.into_iter().flatten().collect()).chain(std::iter::once(rest)).collect();
    (titles, pages)
}

fn object(settings: &ndr::settings::Settings, key: &str) -> Map<String, Value> {
    settings.values.get(key).and_then(Value::as_object).cloned().unwrap_or_default()
}
fn text_of(settings: &ndr::settings::Settings, key: &str) -> String {
    match settings.values.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn number_text(settings: &ndr::settings::Settings, key: &str) -> String {
    settings.values.get(key).and_then(Value::as_f64).map(|v| if v.fract() == 0.0 { format!("{}", v as i64) } else { v.to_string() }).unwrap_or_default()
}

impl SettingsForm {
    pub(super) fn new(source: &ndr::settings::Settings, language: &compat::Language, resources: &Path) -> Self {
        let draft = source.clone();
        let ko = draft.text("Language", "ko").eq_ignore_ascii_case("ko");
        let lang = |key: &str, fallback: &str| language.text(&format!("formPreferences.{key}"), fallback);
        let flag = |key: &str, default: bool| draft.flag(key, default);
        let check = |key: &str, label: String, default: bool| Row::Check { key: key.into(), label, value: draft.flag(key, default) };
        let text = |key: &str, label: String, value: String| Row::Text { key: key.into(), label, value, read_only: false, max: usize::MAX, upper: false };
        let combo = |key: &str, label: String, values: &[&str], selected: &str| {
            let choices: Vec<(String, String)> = values.iter().map(|v| ((*v).to_owned(), (*v).to_owned())).collect();
            let selected = choices.iter().position(|(v, _)| v == selected);
            Row::Combo { key: key.into(), label, choices, selected }
        };
        let choice_row = |key: &str, label: String, choices: Vec<(String, String)>, selected: &str| {
            let selected = choices.iter().position(|(v, _)| v == selected);
            Row::Combo { key: key.into(), label, choices, selected }
        };
        // 일반.
        let mut general = vec![
            check("DualPaneMode", t(ko, "두 패널 표시", "Show two panes"), true),
            check("StartWithRightPaneActive", t(ko, "시작할 때 오른쪽 패널 활성화", "Activate the right pane at startup"), false),
            check("UseTabs", t(ko, "탭 사용", "Use tabs"), true),
            check("ShowToolBar", t(ko, "주요 도구 모음 표시", "Show the main toolbar"), true),
            check("ShowFunctionBar", t(ko, "기능키 표시", "Show function keys"), true),
            check("ShowVerticalBar", t(ko, "수직 도구 모음 표시(작업 폴더 단추, FTP 전송 막대)", "Show the vertical bar (work folder buttons, FTP transfer bar)"), true),
            check("ShowAddressBar", t(ko, "경로 표시", "Show the address bar"), true),
            check("ShowIcons", t(ko, "파일 아이콘 표시", "Show file icons"), true),
            check("UseOverlayIcons", lang("Caption.chkOverlayIcons", if ko { "셸 아이콘 오버레이 표시" } else { "Show Shell icon overlays" }), true),
            check("ConfirmDelete", t(ko, "삭제 전에 확인", "Confirm before deleting"), true),
            check("ConfirmDragDrop", t(ko, "드래그 작업 확인", "Confirm drag operations"), true),
            check("SelectFolders", lang("Caption.chkSelectAllFolder", "Select folders when execute select all function"), false),
            check("ShowDeleteButton", lang("Caption.chkShowDeleteButton", "Show 'Delete' button on delete confirmation dialog"), true),
            Row::Check { key: "ShowDeleteToolbarButton".into(), label: t(ko, "NDR 삭제 도구 버튼 표시", "Show the NDR delete toolbar button"), value: draft.values.get("ShowDeleteToolbarButton").and_then(Value::as_bool).unwrap_or(true) },
        ];
        let mut delete_labels: Vec<String> = lang("ComboBox.cbDefDelBtn", "Delete;Yes;No").split(';').map(|s| s.trim().to_owned()).collect();
        if delete_labels.len() != 3 {
            delete_labels = vec!["Delete".into(), "Yes".into(), "No".into()];
        }
        let delete_default = draft.values.get("DefaultDeleteButton").and_then(Value::as_i64).unwrap_or(2).clamp(0, 2).to_string();
        general.push(choice_row("DefaultDeleteButton", lang("Caption.lDefDelBtn", "Default button for a delete warning dialog:"), delete_labels.into_iter().enumerate().map(|(i, l)| (i.to_string(), l)).collect(), &delete_default));
        general.extend([
            check("UseRecycleBin", t(ko, "기본 삭제는 휴지통 사용", "Delete to the Recycle Bin by default"), true),
            check("UseDescriptionFile", lang("Caption.chkUseION", "Read comment file \"descript.ion\""), true),
            check("UseContentComparison", t(ko, "폴더 비교에서 파일 내용 확인", "Compare file contents in folder comparison"), true),
            check("ShowHiddenFoldersInTree", t(ko, "폴더 트리에서 숨김 폴더 표시", "Show hidden folders in the folder tree"), false),
            check("ShowFolderTreeIcons", t(ko, "폴더 트리 아이콘 표시", "Show folder tree icons"), true),
            check("PlayCompletionSound", t(ko, "작업 완료 소리", "Play a sound when an operation completes"), false),
            check("PlayWaveInProgram", lang("Caption.chkPlayWave", "Play .wav(Wave Sound) file in this program."), true),
            check("UseSingleInstance", t(ko, "한 번에 NDR 하나만 실행", "Run only one NDR at a time"), false),
        ]);
        let activation = format!("{} {}", lang("Caption.lActivateKey", "Program activation shortcut:").trim(), lang("Caption.lWinKey", "Windows key +").trim());
        general.push(Row::Text { key: "ExplorerShortcutKey".into(), label: activation, value: text_of(&draft, "ExplorerShortcutKey"), read_only: false, max: 1, upper: true });
        general.push(Row::Note(t(ko, "Win+문자 조합을 사용할 수 없으면 Win+Shift+문자 조합으로 등록을 시도합니다. 두 조합 모두 사용할 수 없으면 설정이 유지되고 안내합니다.",
            "If Win+letter is unavailable, NDR tries Win+Shift+letter. If both are unavailable, the previous setting is kept and a message is shown.")));
        general.extend([
            check("CheckForUpdatesOnStartup", t(ko, "설정된 피드에서 시작 시 업데이트 확인", "Check the configured feed for updates at startup"), true),
            check("HideWhenMinimized", t(ko, "최소화할 때 알림 영역으로 숨기기", "Hide in the notification area when minimized"), false),
            check("UseLogicalSort", t(ko, "숫자를 고려한 자연 정렬", "Natural sort that considers numbers"), true),
            check("UseAlternateDriveHandling", lang("Caption.chkHanShiftDrive", "Use Shift+Drive Letter when the input language is not English"), false),
            check("SaveDriveFolders", lang("Caption.chkDriveFolders", "Save each drive's last folder at exit"), true),
            check("SaveFolderTreeState", t(ko, "폴더 트리 펼침 상태 저장 (NDR 사용자 설정에 보관)", "Save the folder tree expansion state (in NDR user settings)"), false),
            check("RemoveReadOnlyWhenCopying", t(ko, "복사할 때 읽기 전용 속성 해제", "Clear the read-only attribute when copying"), true),
            combo("Theme", t(ko, "테마", "Theme"), &["Black", "Classic", "Gray"], &draft.text("Theme", "Black")),
        ]);
        let mut escape_labels: Vec<String> = lang("ComboBox.cbESC", "Minimize;Exit Program;Command Line;Deselect All;Go to Parent").split(';').map(|s| s.trim().to_owned()).collect();
        if escape_labels.len() != 5 {
            escape_labels = ["Minimize", "Exit Program", "Command Line", "Deselect All", "Go to Parent"].map(String::from).to_vec();
        }
        let mut escape: Vec<(String, String)> = vec![("Disabled".into(), t(ko, "사용 안 함", "Disabled"))];
        escape.extend(escape_labels.into_iter().enumerate().map(|(i, label)| (i.to_string(), label)));
        let escape_selected = draft.values.get("EscapeKeyAction").and_then(Value::as_i64).filter(|v| (0..=4).contains(v)).map_or("Disabled".to_owned(), |v| v.to_string());
        general.push(choice_row("EscapeKeyAction", lang("Caption.lESC", "ESC Key Function"), escape, &escape_selected));
        let languages: Vec<(String, String)> = available_languages(resources).into_iter().map(|(code, name)| (code.clone(), format!("{code} — {name}"))).collect();
        general.push(choice_row("Language", t(ko, "언어", "Language"), languages, &draft.text("Language", "ko")));

        // 표시.
        let mut cases: Vec<String> = lang("RadioGroupItems.rgCharCase", "Normal;Lower Case;Upper Case;Capitalize First Letter").split(';').map(str::to_owned).collect();
        if cases.len() != 4 {
            cases = ["Normal", "Lower Case", "Upper Case", "Capitalize First Letter"].map(String::from).to_vec();
        }
        let case_index = draft.values.get("CharacterCase").and_then(Value::as_i64).unwrap_or(0).clamp(0, 3) as usize;
        let views = ["Auto", "Names", "Details", "Thumbnails"];
        let sorts = ["None", "Name", "Extension", "Size", "Modified", "Attributes", "Color"];
        let folder_sorts = ["Name", "Extension", "Size", "Modified", "Attributes", "Color"];
        let groups = ["None", "Extension", "Size", "Time", "Category"];
        let categories = vec![("Auto".to_owned(), t(ko, "자동", "Auto")), ("Default".to_owned(), t(ko, "기본값", "Default"))];
        let columns: Vec<&str> = vec!["Auto", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19"];
        let column_value = |key: &str| draft.values.get(key).and_then(Value::as_i64).filter(|v| *v > 0).map_or("Auto".to_owned(), |v| v.to_string());
        let mut display = vec![
            Row::Combo { key: "CharacterCase".into(), label: lang("RadioGroup.rgCharCase", "Character Case"), choices: cases.iter().map(|c| (c.clone(), c.clone())).collect(), selected: Some(case_index) },
            check("AttachExtensionToName", lang("Caption.chkAttachExt", "Attach extension to the name"), false),
            text("ListLineSpace", lang("Caption.lLineSpace", "Line Space:"), number_text(&draft, "ListLineSpace")),
            combo("LeftViewStyle", t(ko, "왼쪽 기본 보기", "Left default view"), &views, &draft.text("LeftViewStyle", "Auto")),
            combo("RightViewStyle", t(ko, "오른쪽 기본 보기", "Right default view"), &views, &draft.text("RightViewStyle", "Auto")),
            combo("LeftSortMode", t(ko, "왼쪽 정렬", "Left sort"), &sorts, &draft.text("LeftSortMode", "Name")),
            combo("RightSortMode", t(ko, "오른쪽 정렬", "Right sort"), &sorts, &draft.text("RightSortMode", "Name")),
            combo("LeftFolderSortMode", t(ko, "왼쪽 폴더 정렬", "Left folder sort"), &folder_sorts, &draft.text("LeftFolderSortMode", "Name")),
            combo("RightFolderSortMode", t(ko, "오른쪽 폴더 정렬", "Right folder sort"), &folder_sorts, &draft.text("RightFolderSortMode", "Name")),
            combo("LeftGroupMode", t(ko, "왼쪽 그룹 기준", "Left grouping"), &groups, &draft.text("LeftGroupMode", "None")),
            combo("RightGroupMode", t(ko, "오른쪽 그룹 기준", "Right grouping"), &groups, &draft.text("RightGroupMode", "None")),
            choice_row("LeftFolderCategoryMode", t(ko, "왼쪽 폴더 범주", "Left folder category"), categories.clone(), &draft.text("LeftFolderCategoryMode", "Auto")),
            choice_row("RightFolderCategoryMode", t(ko, "오른쪽 폴더 범주", "Right folder category"), categories, &draft.text("RightFolderCategoryMode", "Auto")),
            combo("LeftColumnCount", t(ko, "왼쪽 이름 목록 열 수", "Left name list columns"), &columns, &column_value("LeftColumnCount")),
            combo("RightColumnCount", t(ko, "오른쪽 이름 목록 열 수", "Right name list columns"), &columns, &column_value("RightColumnCount")),
        ];
        for (key, korean, english, default) in [
            ("LeftSortDescending", "왼쪽 내림차순", "Left descending", false),
            ("RightSortDescending", "오른쪽 내림차순", "Right descending", false),
            ("LeftShowHiddenFiles", "왼쪽 숨김 / 시스템 파일 표시", "Left: show hidden / system files", false),
            ("RightShowHiddenFiles", "오른쪽 숨김 / 시스템 파일 표시", "Right: show hidden / system files", false),
            ("LeftFolderSortDescending", "왼쪽 폴더 내림차순", "Left folders descending", false),
            ("RightFolderSortDescending", "오른쪽 폴더 내림차순", "Right folders descending", false),
            ("LeftAlwaysShowFolderSize", "왼쪽 폴더 크기를 목록에 표시", "Left: show folder sizes in the list", false),
            ("RightAlwaysShowFolderSize", "오른쪽 폴더 크기를 목록에 표시", "Right: show folder sizes in the list", false),
        ] {
            display.push(check(key, t(ko, korean, english), default));
        }
        // 원본 chkScrollStep1/2(0x44cfa2: 값 2면 켜짐, 0x44da08: 켜짐 = 2, 꺼짐 = 1).
        for (key, korean, english) in [("LeftScrollStep", "왼쪽 페이지 단위 스크롤", "Left: scroll by page"), ("RightScrollStep", "오른쪽 페이지 단위 스크롤", "Right: scroll by page")] {
            let by_page = draft.values.get(key).and_then(Value::as_i64).unwrap_or(2) != 1;
            display.push(Row::Check { key: key.into(), label: t(ko, korean, english), value: by_page });
        }
        for (key, korean, english, default) in [
            ("LeftGroupDescending", "왼쪽 그룹 역순", "Left groups reversed", false),
            ("RightGroupDescending", "오른쪽 그룹 역순", "Right groups reversed", false),
            ("LeftShowGroupName", "왼쪽 그룹 이름 표시", "Left: show group names", true),
            ("RightShowGroupName", "오른쪽 그룹 이름 표시", "Right: show group names", true),
            ("ShowDriveList", "드라이브 목록 보기(목록 끝·내 PC)", "Show drive list (list end, This PC)", true),
            ("ShowDriveButtons", "빠른 드라이브 버튼 표시", "Show quick drive buttons", false),
            ("ShowFileType", "파일 종류 열 표시", "Show the file type column", true),
            ("ShowFolderDiskInfoBar", "폴더/디스크 정보 표시줄", "Folder/disk information bar", true),
            ("ShowFileInfoBar", "파일 정보 표시줄", "File information bar", true),
            ("ShowExtensionInfoBar", "확장자 정보 표시줄", "Extension information bar", false),
            ("ShowColumnLines", "목록 열 구분선 표시", "Show list column lines", true),
            ("ShowRowLines", "목록 행 구분선 표시", "Show list row lines", false),
        ] {
            display.push(check(key, t(ko, korean, english), default));
        }
        display.push(check("UseMenuAlpha", lang("Caption.chkMenuAlpha", "Use Transparent Menu"), true));
        display.push(Row::Section(lang("GroupBox.gbAlpha", "Window Transparency")));
        display.push(check("UseWindowAlpha", lang("Caption.chkAlphaWindow", "Use Transparent Window"), false));
        let alpha = draft.values.get("WindowAlpha").and_then(Value::as_f64).unwrap_or(210.0).clamp(0.0, 255.0) as f32;
        display.push(Row::Slider { key: "WindowAlpha".into(), label: format!("{} — {}", lang("Caption.lTransMin", "Transparent"), lang("Caption.lTransMax", "Opaque")), value: alpha });
        display.extend([
            check("UseDateAge", t(ko, "최근 날짜를 상대 시간으로 표시", "Show recent dates as relative times"), true),
            check("UseThousandsSeparator", t(ko, "파일 크기에 천 단위 구분자 표시", "Show thousands separators in file sizes"), true),
            text("DateFormat", t(ko, "날짜 형식", "Date format"), draft.text("DateFormat", "yyyy-MM-dd")),
            text("TimeFormat", t(ko, "시간 형식", "Time format"), draft.text("TimeFormat", "HH:mm")),
            combo("SizeUnit", t(ko, "크기 단위", "Size unit"), &["Auto", "B", "KB", "MB", "GB"], &draft.text("SizeUnit", "Auto")),
            text("ListFont", t(ko, "목록 글꼴", "List font"), text_of(&draft, "ListFont")),
            text("ListFontSize", t(ko, "글꼴 크기", "Font size"), number_text(&draft, "ListFontSize")),
            text("ThumbnailSize", t(ko, "미리 보기 크기 (px)", "Thumbnail size (px)"), number_text(&draft, "ThumbnailSize")),
            check("UseNexusFileColorMap", t(ko, "NexusFile .col 색상표 적용", "Use NexusFile .col color map"), false),
        ]);
        let colors = object(&draft, "ExtensionColors").iter().map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or(""))).collect::<Vec<_>>().join("; ");
        display.push(text("ExtensionColors", t(ko, "확장자 글자색 (.zip=#F59E0B; .txt=#60A5FA)", "Extension text colors (.zip=#F59E0B; .txt=#60A5FA)"), colors));
        display.push(Row::ColorButtons);

        // 경로.
        let profile = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default());
        let special = object(&draft, "SpecialFolderPaths");
        let special_value = |name: &str| special.get(name).and_then(Value::as_str).unwrap_or("").to_owned();
        let mut paths = vec![
            text("LeftPath", t(ko, "시작 경로 — 왼쪽", "Start path — left"), text_of(&draft, "LeftPath")),
            text("RightPath", t(ko, "시작 경로 — 오른쪽", "Start path — right"), text_of(&draft, "RightPath")),
            Row::Section(lang("GroupBox.gbMyFolders", "User Folders Location")),
        ];
        for (name, key, fallback, default) in [
            ("Documents", "Caption.lMyDocuments", "Documents:", native::documents_folder().unwrap_or_else(|| profile.join("Documents"))),
            ("Music", "Caption.lMyMusic", "Music:", profile.join("Music")),
            ("Pictures", "Caption.lMyPictures", "Pictures:", profile.join("Pictures")),
            ("Videos", "Caption.lMyVideos", "Videos:", profile.join("Videos")),
            ("Downloads", "Caption.lPathDownloads", "Downloads:", profile.join("Downloads")),
        ] {
            paths.push(Row::Folder { key: format!("SpecialFolder:{name}"), label: lang(key, fallback), value: special_value(name), default });
        }
        paths.push(Row::Note(lang("Caption.lHelpPath", "Leave blank to use the Windows default folder.")));
        let path_click = vec![
            ("0".to_owned(), t(ko, "기본 동작 (0)", "Default behavior (0)")),
            ("1".to_owned(), t(ko, "경로 클릭 이동, 빈 공간은 즐겨찾기 (1)", "Path clicks navigate; blank area opens Favorites (1)")),
            ("2".to_owned(), t(ko, "주소 표시줄 클릭 시 즐겨찾기 (2)", "Any address-bar click opens Favorites (2)")),
        ];
        let path_click_value = draft.values.get("PathClickMode").and_then(Value::as_i64).unwrap_or(0).clamp(0, 2).to_string();
        paths.push(choice_row("PathClickMode", t(ko, "주소 표시줄 클릭 동작", "Address bar click behavior"), path_click, &path_click_value));
        let destination = vec![("0".to_owned(), t(ko, "반대 패널 경로 (0)", "Opposite pane folder (0)")), ("1".to_owned(), t(ko, "현재 드라이브 루트 (1)", "Current drive root (1)"))];
        let destination_value = draft.values.get("DefaultDestinationMode").and_then(Value::as_i64).unwrap_or(0).clamp(0, 1).to_string();
        paths.push(choice_row("DefaultDestinationMode", t(ko, "대상 선택 기본 위치", "Default destination location"), destination, &destination_value));
        paths.push(text("MaxRecentFileCount", lang("Caption.lMostUsedFolder", if ko { "대상 선택 창에 표시할 자주 쓰는 폴더 수" } else { "Most used destination folders in chooser" }), number_text(&draft, "MaxRecentFileCount")));
        paths.extend([
            check("ShowLeftFolderTree", t(ko, "왼쪽 폴더 트리 표시", "Show the left folder tree"), false),
            check("ShowRightFolderTree", t(ko, "오른쪽 폴더 트리 표시", "Show the right folder tree"), false),
            check("UseExplorerMenu", t(ko, "오른쪽버튼 클릭: 탐색기 메뉴 사용하기", "Right-click: use the Explorer menu"), false),
            text("LeftFolderTreeWidth", t(ko, "왼쪽 폴더 트리 너비", "Left folder tree width"), number_text(&draft, "LeftFolderTreeWidth")),
            text("RightFolderTreeWidth", t(ko, "오른쪽 폴더 트리 너비", "Right folder tree width"), number_text(&draft, "RightFolderTreeWidth")),
            check("TreeWindowMatchesMainSize", t(ko, "별도 폴더 트리 창을 메인 창 크기로 열기", "Open the separate folder tree window at the main window size"), false),
            check("LeftStartPathEnabled", t(ko, "왼쪽 고정 시작 경로 사용", "Use a fixed left start path"), false),
            text("LeftStartPath", t(ko, "왼쪽 고정 시작 경로", "Fixed left start path"), text_of(&draft, "LeftStartPath")),
            check("RightStartPathEnabled", t(ko, "오른쪽 고정 시작 경로 사용", "Use a fixed right start path"), false),
            text("RightStartPath", t(ko, "오른쪽 고정 시작 경로", "Fixed right start path"), text_of(&draft, "RightStartPath")),
            Row::Section(t(ko, "작업 폴더 1–5", "Work folders 1–5")),
        ]);
        let work = draft.strings("WorkingFolders");
        for i in 0..5 {
            paths.push(text(&format!("WF{i}"), if ko { format!("작업 폴더 {}", i + 1) } else { format!("Work folder {}", i + 1) }, work.get(i).cloned().unwrap_or_default()));
        }
        paths.extend([
            check("ShowFavoritesBar", t(ko, "즐겨찾기 바 표시", "Show the favorites bar"), true),
            text("UserDefinedCleanupExtensions", t(ko, "정리 대상 확장자 (세미콜론 구분)", "Cleanup extensions (semicolon separated)"), text_of(&draft, "UserDefinedCleanupExtensions")),
            text("CleanupLocations", t(ko, "사용자 정리 위치 (세미콜론 구분)", "User cleanup locations (semicolon separated)"), draft.strings("UserDefinedCleanupLocations").join(";")),
            text("FolderTreeDepth", t(ko, "폴더 트리 탐색 깊이 (1–64)", "Folder tree scan depth (1–64)"), number_text(&draft, "FolderTreeDepth")),
            text("FolderTreeFont", t(ko, "폴더 트리 글꼴", "Folder tree font"), text_of(&draft, "FolderTreeFont")),
            text("FolderTreeFontSize", t(ko, "폴더 트리 글꼴 크기", "Folder tree font size"), number_text(&draft, "FolderTreeFontSize")),
            text("FolderTreeItemWidth", t(ko, "폴더 트리 이름 너비 (px)", "Folder tree name width (px)"), number_text(&draft, "FolderTreeItemWidth")),
            text("TreeExcludedPaths", t(ko, "폴더 트리 제외 경로 (세미콜론 구분)", "Folder tree excluded paths (semicolon separated)"), draft.strings("FolderTreeExcludedPaths").join(";")),
        ]);

        // 외부 프로그램.
        // 외부 압축 프로그램(원본 chkUseExtArcProgram, C# 판 압축 형식별 프로그램·인자)은 쓰지 않는다(사용자 요청).
        let mut programs = vec![
            check("UseCopyHandler", t(ko, "등록한 외부 복사 프로그램 사용", "Use the registered external copy program"), false),
            Row::Program { key: "TextViewerPath".into(), label: t(ko, "텍스트 보기 프로그램", "Text viewer program"), value: text_of(&draft, "TextViewerPath") },
            Row::Program { key: "TextEditorPath".into(), label: t(ko, "텍스트 편집 프로그램", "Text editor program"), value: text_of(&draft, "TextEditorPath") },
            Row::Program { key: "CopyHandlerPath".into(), label: language.text("NDRCopyHandler.Path", "External copy/move handler path (TeraCopy or KillCopy)"), value: text_of(&draft, "CopyHandlerPath") },
            text("UpdateManifestUrl", t(ko, "업데이트 JSON 피드 (HTTPS)", "Update JSON feed (HTTPS)"), text_of(&draft, "UpdateManifestUrl")),
        ];
        programs.push(Row::Section(language.text("formFunctionKeys.Form.Caption", "Function Keys")));
        programs.push(Row::Note(t(ko, "F1–F12별 표시 이름, 내부 동작 또는 사용자 프로그램/문서와 실행 인자를 편집합니다. 기존 NexusFile 기능키 설정도 자동으로 불러옵니다.",
            "Edit the display name, internal action or user program/document, and launch parameters for F1–F12. Existing NexusFile function key settings are imported automatically.")));
        programs.push(Row::FunctionKeys);
        let _ = flag;
        let pref = |key: &str, ko_text: &str, en_text: &str| language.text(&format!("formPreferences.{key}"), if ko { ko_text } else { en_text });
        let color_captions = [
            pref("Caption.lExtColor", "확장자별 색", "Extension"),
            pref("Caption.btnAddColor", "추가", "Add"),
            pref("Caption.btnEditColor", "편집", "Edit"),
            pref("Caption.btnDelColor", "삭제", "Delete"),
            pref("Caption.btnSaveColorMap", "내보내기", "Export"),
            pref("Caption.btnImportColor", "가져오기", "Import"),
            pref("Caption.lHelp", "색을 바꾸려면 해당 항목에서 더블클릭해주세요.", "Double Click to change the color"),
            pref("Caption.lBasicColor", "기본 색", "Basic"),
        ];
        let basic_items = language
            .text(
                "formPreferences.ListBox.lbBasicColor",
                "[File List];Background;Foreground;Folder;Drive;Readonly Attr(File Size Color);Hidden Attr(File Size Color);Comment;MB(Size);GB(Size);Size;Time;Attributes;Selected;Column Line;;[Folder Tree];Background;Foreground;Opened;Selected;Line;Hint Text",
            )
            .split(';')
            .map(str::to_owned)
            .collect();
        let (tab_titles, tabs) = original_pages(vec![general, display, paths, programs], language, ko);
        let button_captions = [pref("Caption.btnOk", "확인(&O)", "&OK"), pref("Caption.btnCancel", "취소", "Cancel"), pref("Caption.btnApply", "적용(&A)", "&Apply")];
        let captions = [
            ("GroupBox.gbFont", "Font"),
            ("RadioGroup.rgCharCase", "Charcase"),
            ("GroupBox.gbViewStyle", "File List"),
            ("Caption.lListView1", "Left Pane"),
            ("Caption.lListView2", "Right Pane"),
            ("GroupBox.gbFormat", "Format"),
            ("Caption.TntLabel3", "pixel"),
            ("GroupBox.gbTreeFont", "Font"),
            ("GroupBox.gbArchive", "External Archive Programs"),
            ("GroupBox.gbPrograms", "External Program"),
            ("GroupBox.gbMyFolders", "Paths"),
            ("Caption.lHelpPath", "Leave blank to use system default path"),
            ("GroupBox.gbStartPath", "Start Folder"),
            ("Caption.lHelpStartPath", "Always start NexusFile  with following folders"),
            ("ListViewCols.lvFunctionKey", "Key;Name;Command"),
            ("Caption.TntLabel66", "^F1 = Ctrl + F1"),
            ("Caption.btnEditFunc", "Edit"),
            ("Caption.btnFuncDelete", "Delete"),
            ("Caption.lActivateKey", "Bring NexusFile to front:"),
            ("Caption.lWinKey", "WinKey + "),
            ("Caption.lApplyAfterRestart", "(Restart required)"),
            ("GroupBox.gbAlpha", "Window Transparency"),
            ("Caption.lTransMin", "Transparent"),
            ("Caption.lTransMax", "Opaque"),
        ]
        .into_iter()
        .map(|(key, fallback)| (key, lang(key, fallback)))
        .collect();
        let function_keys = super::function_key_editor::FunctionKeyEditorForm::new(&draft, ko);
        Self { ko, tab: 0, tabs, tab_titles, draft, color_selected: None, last_color: 0, color_captions, basic_items, button_captions, captions, function_keys }
    }

    /// 기능키 목록을 고친 뒤(원본 btnEditFunc·btnFuncDelete): C# 판처럼 설정 사본의 FunctionKeyBindings를 바꾸고 옛 FunctionKeys의 F1~F12·^F1~^F11은 지운다.
    pub(super) fn sync_function_keys(&mut self) {
        self.draft.values.insert("FunctionKeyBindings".into(), self.function_keys.result());
        let mut legacy = object(&self.draft, "FunctionKeys");
        legacy.retain(|key, _| !(1..=12).any(|n| key.eq_ignore_ascii_case(&format!("F{n}")) || (n <= 11 && key.eq_ignore_ascii_case(&format!("^F{n}")))));
        self.draft.values.insert("FunctionKeys".into(), Value::Object(legacy));
    }

    pub(super) fn row_mut(&mut self, key: &str) -> Option<&mut Row> {
        self.tabs.iter_mut().flatten().find(|row| match row {
            Row::Check { key: k, .. } | Row::Text { key: k, .. } | Row::Folder { key: k, .. } | Row::Program { key: k, .. } | Row::Combo { key: k, .. } | Row::Slider { key: k, .. } => k == key,
            _ => false,
        })
    }

    /// C# Commit: 키마다 같은 변환을 해 사본을 돌려준다. 틀린 값이면 그 글(경고 상자).
    pub(super) fn commit(&self) -> Result<ndr::settings::Settings, String> {
        let mut draft = self.draft.clone();
        let values = &mut draft.values;
        let map_set = |values: &mut Map<String, Value>, name: &str, key: &str, value: &str, remove_empty: bool| {
            let mut map = values.get(name).and_then(Value::as_object).cloned().unwrap_or_default();
            if remove_empty && value.is_empty() {
                map.remove(key);
            } else {
                map.insert(key.into(), Value::String(value.into()));
            }
            values.insert(name.into(), Value::Object(map));
        };
        let full_paths = |text: &str, distinct: bool| -> Vec<String> {
            let mut list: Vec<String> = text
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    let expanded = native::expand_environment(s);
                    std::path::absolute(&expanded).map(|p| p.to_string_lossy().into_owned()).unwrap_or(expanded)
                })
                .collect();
            if distinct {
                let mut seen: Vec<String> = Vec::new();
                list.retain(|p| {
                    let lower = p.to_lowercase();
                    let fresh = !seen.contains(&lower);
                    seen.push(lower);
                    fresh
                });
            }
            list
        };
        for row in self.tabs.iter().flatten() {
            match row {
                Row::Check { key, value, .. } if key == "LeftScrollStep" || key == "RightScrollStep" => {
                    values.insert(key.clone(), json!(if *value { 2 } else { 1 }));
                }
                Row::Check { key, value, .. } => {
                    values.insert(key.clone(), Value::Bool(*value));
                }
                Row::Slider { key, value, .. } if key == "WindowAlpha" => {
                    values.insert(key.clone(), json!((value.round() as i64).clamp(0, 255)));
                }
                Row::Text { key, value, .. } | Row::Folder { key, value, .. } | Row::Program { key, value, .. } => {
                    let value = value.trim();
                    let int = |v: &str| v.parse::<i64>().ok();
                    let float = |v: &str| v.parse::<f64>().ok();
                    match key.as_str() {
                        "ExtensionColors" => {
                            let colors = parse_extension_colors(value)?;
                            values.insert(key.clone(), Value::Object(colors.into_iter().map(|(k, v)| (k, Value::String(v))).collect()));
                        }
                        "LeftPath" | "RightPath" | "DateFormat" | "TimeFormat" | "ListFont" | "UserDefinedCleanupExtensions" | "TextViewerPath" | "TextEditorPath" | "CopyHandlerPath"
                        | "UpdateManifestUrl" | "FolderTreeFont" => {
                            values.insert(key.clone(), Value::String(value.into()));
                        }
                        "LeftStartPath" | "RightStartPath" => {
                            values.insert(key.clone(), Value::String(native::expand_environment(value)));
                        }
                        "ExplorerShortcutKey" => {
                            let mut chars = value.chars();
                            let accepted = match (chars.next(), chars.next()) {
                                (None, _) => String::new(),
                                (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase().to_string(),
                                _ => return Err(if self.ko { "활성화 단축키에는 영문자 또는 숫자 한 글자만 입력하세요.".into() } else { "Enter one letter or digit for the activation shortcut.".into() }),
                            };
                            values.insert(key.clone(), Value::String(accepted));
                        }
                        "CleanupLocations" => {
                            values.insert("UserDefinedCleanupLocations".into(), json!(full_paths(value, false)));
                        }
                        "TreeExcludedPaths" => {
                            values.insert("FolderTreeExcludedPaths".into(), json!(full_paths(value, true)));
                        }
                        "ListFontSize" | "LeftFolderTreeWidth" | "RightFolderTreeWidth" | "FolderTreeItemWidth" | "FolderTreeFontSize" => {
                            if let Some(v) = float(value) {
                                values.insert(key.clone(), json!(v));
                            }
                        }
                        "ThumbnailSize" => {
                            if let Some(v) = int(value) {
                                values.insert(key.clone(), json!(v));
                            }
                        }
                        "ListLineSpace" => {
                            if let Some(v) = int(value) {
                                values.insert(key.clone(), json!(v.clamp(0, 32)));
                            }
                        }
                        "MaxRecentFileCount" => {
                            if let Some(v) = int(value) {
                                values.insert(key.clone(), json!(v.clamp(0, 1000)));
                            }
                        }
                        "FolderTreeDepth" => {
                            if let Some(v) = int(value) {
                                values.insert(key.clone(), json!(v.clamp(1, 64)));
                            }
                        }
                        other => {
                            if let Some(name) = other.strip_prefix("SpecialFolder:") {
                                map_set(values, "SpecialFolderPaths", name, value, true);
                            }
                        }
                    }
                }
                Row::Combo { key, choices, selected, .. } => {
                    let value = selected.and_then(|i| choices.get(i)).map(|(v, _)| v.clone()).unwrap_or_default();
                    match key.as_str() {
                        "Theme" | "LeftViewStyle" | "RightViewStyle" | "LeftSortMode" | "RightSortMode" | "LeftFolderSortMode" | "RightFolderSortMode" | "LeftGroupMode" | "RightGroupMode"
                        | "LeftFolderCategoryMode" | "RightFolderCategoryMode" | "SizeUnit" | "Language" => {
                            values.insert(key.clone(), Value::String(value));
                        }
                        "LeftColumnCount" | "RightColumnCount" => {
                            let count = if value == "Auto" { 0 } else { value.parse::<i64>().unwrap_or(0).clamp(1, 19) };
                            values.insert(key.clone(), json!(count));
                        }
                        "DefaultDestinationMode" => {
                            values.insert(key.clone(), json!(value.parse::<i64>().map_or(0, |v| v.clamp(0, 1))));
                        }
                        "PathClickMode" => {
                            let mode = value.parse::<i64>().map_or(0, |v| v.clamp(0, 2));
                            values.insert(key.clone(), json!(mode));
                            values.insert("NavigateOnPathClick".into(), Value::Bool(mode == 1));
                        }
                        "CharacterCase" => {
                            values.insert(key.clone(), json!(selected.unwrap_or(0).min(3)));
                        }
                        "DefaultDeleteButton" => {
                            values.insert(key.clone(), json!(value.parse::<i64>().map_or(2, |v| v.clamp(0, 2))));
                        }
                        "EscapeKeyAction" => {
                            values.insert(key.clone(), value.parse::<i64>().ok().filter(|v| (0..=4).contains(v)).map_or(Value::Null, |v| json!(v)));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // 작업 폴더 1~5(앞뒤 공백 뺌).
        let mut work = draft.strings("WorkingFolders");
        work.resize(5, String::new());
        for row in self.tabs.iter().flatten() {
            if let Row::Text { key, value, .. } = row {
                if let Some(index) = key.strip_prefix("WF").and_then(|n| n.parse::<usize>().ok()).filter(|i| *i < 5) {
                    work[index] = value.trim().to_owned();
                }
            }
        }
        draft.set_strings("WorkingFolders", work);
        Ok(draft)
    }
}

fn index_of(rows: &[Row], key: &str) -> Option<usize> {
    rows.iter().position(|row| row_key(row) == Some(key))
}

fn label_of(rows: &[Row], key: &str) -> String {
    match index_of(rows, key).map(|i| &rows[i]) {
        Some(Row::Check { label, .. } | Row::Text { label, .. } | Row::Folder { label, .. } | Row::Program { label, .. } | Row::Combo { label, .. } | Row::Slider { label, .. }) => label.replace('&', ""),
        _ => String::new(),
    }
}

/// 글 줄의 지금 값(글꼴 이름·크기 표시용).
fn label_value(rows: &[Row], key: &str) -> String {
    match index_of(rows, key).map(|i| &rows[i]) {
        Some(Row::Text { value, .. }) => value.trim().to_owned(),
        _ => String::new(),
    }
}

fn put_label(ui: &mut egui::Ui, rect: egui::Rect, text: &str) {
    super::original_dialogs::put_left(ui, rect, egui::Label::new(text.replace('&', "")).truncate());
}

fn put_check(ui: &mut egui::Ui, rows: &mut [Row], key: &str, rect: egui::Rect) {
    if let Some(Row::Check { label, value, .. }) = index_of(rows, key).map(|i| &mut rows[i]) {
        super::original_dialogs::put_left(ui, rect, egui::Checkbox::new(value, label.replace('&', "")));
    }
}

/// 원본 TEdit 자리(글 줄, 폴더·프로그램 경로 칸).
fn put_edit(ui: &mut egui::Ui, rows: &mut [Row], key: &str, rect: egui::Rect) {
    match index_of(rows, key).map(|i| &mut rows[i]) {
        Some(Row::Text { value, read_only: false, max, upper, .. }) => {
            if ui.put(rect, egui::TextEdit::singleline(value).char_limit(*max)).changed() && *upper {
                *value = value.to_uppercase();
            }
        }
        Some(Row::Folder { value, .. } | Row::Program { value, .. }) => {
            ui.put(rect, egui::TextEdit::singleline(value));
        }
        _ => {}
    }
}

/// 원본 TComboBox(csDropDownList) 자리.
fn put_combo(ui: &mut egui::Ui, rows: &mut [Row], key: &str, rect: egui::Rect) {
    if let Some(Row::Combo { key: id, choices, selected, .. }) = index_of(rows, key).map(|i| &mut rows[i]) {
        let text = selected.and_then(|i| choices.get(i)).map(|(_, d)| d.clone()).unwrap_or_default();
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ComboBox::from_id_salt(("settings-combo", id.as_str())).width(rect.width() - 8.0).selected_text(text).show_opaque(ui, |ui| {
                for (i, (_, display)) in choices.iter().enumerate() {
                    ui.selectable_value(selected, Some(i), display.as_str());
                }
            });
        });
    }
}

/// 원본 TComboBox(csDropDown, 글을 쳐 넣을 수 있음) 자리: 글 칸과 원본 Items 목록 단추.
fn put_dropdown_edit(ui: &mut egui::Ui, rows: &mut [Row], key: &str, rect: egui::Rect, items: &[&str]) {
    let Some(Row::Text { value, .. }) = index_of(rows, key).map(|i| &mut rows[i]) else { return };
    ui.put(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() - 18.0, rect.height())), egui::TextEdit::singleline(value));
    ui.scope_builder(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(rect.min + egui::vec2(rect.width() - 17.0, 0.0), egui::vec2(17.0, rect.height()))), |ui| {
        ui.menu_button("▼", |ui| {
            for item in items {
                if ui.button(*item).clicked() {
                    *value = (*item).to_owned();
                    ui.close();
                }
            }
        });
    });
}

/// 원본 TEdit + TUpDown(Associate) 자리: 글 칸 오른쪽에 16 너비 위·아래 단추(Min~Max, 1씩).
fn put_updown(ui: &mut egui::Ui, rows: &mut [Row], key: &str, edit: egui::Rect, min: i64, max: i64) {
    let Some(Row::Text { value, .. }) = index_of(rows, key).map(|i| &mut rows[i]) else { return };
    ui.put(edit, egui::TextEdit::singleline(value));
    let half = edit.height() / 2.0;
    let current = value.trim().parse::<i64>().ok();
    for (dy, glyph, step) in [(0.0, "▲", 1i64), (half, "▼", -1)] {
        let rect = egui::Rect::from_min_size(edit.right_top() + egui::vec2(0.0, dy), egui::vec2(16.0, half));
        let clicked = ui
            .scope(|ui| {
                ui.spacing_mut().button_padding = egui::Vec2::ZERO;
                ui.put(rect, egui::Button::new(RichText::new(glyph).size(7.0))).clicked()
            })
            .inner;
        if clicked {
            *value = current.map_or(min, |v| v + step).clamp(min, max).to_string();
        }
    }
}

/// 원본 TListBox 칸: 바탕과 테두리.
fn list_frame(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().rect(rect, 0.0, ui.visuals().extreme_bg_color, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
}

/// 원본 쪽 하나(TabSheet 바탕 = origin). DFM 자리 그대로이고, 그룹 안 자리는 그룹 Left/Top을 더한다.
fn original_page(ui: &mut egui::Ui, form: &mut SettingsForm, tab: usize, origin: egui::Pos2, enter: bool, delete: bool) -> Option<SettingsAction> {
    use super::original_dialogs::{group_box, put_left};
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(origin + egui::vec2(x, y), egui::vec2(w, h));
    let captions = form.captions.clone();
    let cap = |key: &str, fallback: &str| captions.get(key).cloned().unwrap_or_else(|| fallback.to_owned()).replace('&', "");
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    let blue = Color32::from_rgb(0x3a, 0x8a, 0xde);
    let mut action = None;
    match tab {
        // tsDisplay: gbFont(10,7 208×94), rgCharCase(7,107 208×112), gbFormat(10,225 208×199), gbViewStyle(224,8 320×415).
        0 => {
            group_box(ui, at(10.0, 7.0, 208.0, 94.0), &cap("GroupBox.gbFont", "Font"));
            // lListFont(34,29) + lListFontValue(81,29) 그룹 안 자리: 한국어 글은 원본 칸보다 넓어 한 줄로 잇는다.
            // 원본 FUN_0044cd58: wsprintfW("\"%s\",%d,%d", 이름, 크기, 모양(TFontStyles 바이트)).
            let style = form.draft.values.get("ListFontStyle").and_then(Value::as_i64).unwrap_or(0) & 0x0F;
            let rows = &mut form.tabs[tab];
            let font = format!("{} \"{}\",{},{style}", label_of(rows, "ListFont"), label_value(rows, "ListFont"), label_value(rows, "ListFontSize"));
            put_label(ui, at(44.0, 36.0, 170.0, 15.0), &font);
            if let Some(Row::FontButton { label, .. }) = rows.iter().find(|r| matches!(r, Row::FontButton { tree: false, .. })) {
                if ui.put(at(51.0, 62.0, 140.0, 25.0), egui::Button::new(label.as_str())).clicked() {
                    action = Some(SettingsAction::ChooseFont(false));
                }
            }
            // 원본 TRadioGroup: 제목 아래로 항목을 고르게 나눈다(112 - 16 = 96, 항목마다 24).
            group_box(ui, at(7.0, 107.0, 208.0, 112.0), &cap("RadioGroup.rgCharCase", "Charcase"));
            if let Some(Row::Combo { choices, selected, .. }) = index_of(rows, "CharacterCase").map(|i| &mut rows[i]) {
                for (i, (_, display)) in choices.iter().enumerate() {
                    if put_left(ui, at(15.0, 107.0 + 20.0 + i as f32 * 24.0, 196.0, 17.0), egui::RadioButton::new(*selected == Some(i), display.as_str())).clicked() {
                        *selected = Some(i);
                    }
                }
            }
            let (fx, fy) = (10.0, 225.0);
            group_box(ui, at(fx, fy, 208.0, 199.0), &cap("GroupBox.gbFormat", "Format"));
            put_label(ui, at(fx + 10.0, fy + 28.0, 88.0, 15.0), &label_of(rows, "DateFormat"));
            put_dropdown_edit(ui, rows, "DateFormat", at(fx + 101.0, fy + 25.0, 96.0, 21.0), &["yyyy/MM/dd", "yyyy-MM-dd", "M/d/yyyy", "MM/dd/yyyy", "dd-MMM-yyyy", "MMM dd, yy"]);
            put_check(ui, rows, "UseDateAge", at(fx + 10.0, fy + 50.0, 195.0, 17.0));
            put_label(ui, at(fx + 10.0, fy + 77.0, 88.0, 15.0), &label_of(rows, "TimeFormat"));
            put_dropdown_edit(ui, rows, "TimeFormat", at(fx + 101.0, fy + 74.0, 96.0, 21.0), &["HH:mm", "H:mm", "hh:mm tt", "h:mm tt", "HH:mm:ss", "H:mm:ss"]);
            put_label(ui, at(fx + 10.0, fy + 109.0, 88.0, 15.0), &label_of(rows, "SizeUnit"));
            put_combo(ui, rows, "SizeUnit", at(fx + 101.0, fy + 106.0, 96.0, 21.0));
            put_check(ui, rows, "UseThousandsSeparator", at(fx + 10.0, fy + 135.0, 195.0, 17.0));
            put_check(ui, rows, "AttachExtensionToName", at(fx + 10.0, fy + 169.0, 195.0, 17.0));
            let (vx, vy) = (224.0, 8.0);
            group_box(ui, at(vx, vy, 320.0, 415.0), &cap("GroupBox.gbViewStyle", "File List"));
            for (top, key) in [(23.0, "ShowIcons"), (42.0, "ShowDriveList"), (61.0, "ShowColumnLines"), (81.0, "ShowRowLines"), (102.0, "UseLogicalSort")] {
                put_check(ui, rows, key, at(vx + 12.0, vy + top, 300.0, 17.0));
            }
            put_label(ui, at(vx + 12.0, vy + 131.0, 88.0, 15.0), &label_of(rows, "ListLineSpace"));
            put_updown(ui, rows, "ListLineSpace", at(vx + 102.0, vy + 128.0, 40.0, 21.0), 0, 10);
            put_label(ui, at(vx + 166.0, vy + 131.0, 82.0, 15.0), &label_of(rows, "ThumbnailSize"));
            put_updown(ui, rows, "ThumbnailSize", at(vx + 249.0, vy + 128.0, 40.0, 21.0), 64, 300);
            ui.painter().hline(origin.x + vx + 10.0..=origin.x + vx + 304.0, origin.y + vy + 160.0, stroke);
            ui.painter().vline(origin.x + vx + 160.0, origin.y + vy + 166.0..=origin.y + vy + 405.0, stroke);
            for (side, n, x) in [("Left", "1", 10.0), ("Right", "2", 167.0)] {
                let title = cap(&format!("Caption.lListView{n}"), if n == "1" { "Left Pane" } else { "Right Pane" });
                put_left(ui, at(vx + x, vy + 164.0, 140.0, 16.0), egui::Label::new(RichText::new(title).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())).color(blue)));
                put_label(ui, at(vx + x, vy + 184.0, 140.0, 15.0), &label_of(rows, &format!("{side}ViewStyle")));
                put_combo(ui, rows, &format!("{side}ViewStyle"), at(vx + x, vy + 203.0, 126.0, 21.0));
                put_check(ui, rows, &format!("{side}AlwaysShowFolderSize"), at(vx + x, vy + 230.0, 146.0, 17.0));
                put_check(ui, rows, &format!("{side}ScrollStep"), at(vx + x, vy + 253.0, 146.0, 17.0));
                put_label(ui, at(vx + x, vy + 272.0, 140.0, 15.0), &label_of(rows, &format!("{side}SortMode")));
                put_combo(ui, rows, &format!("{side}SortMode"), at(vx + x, vy + 290.0, 126.0, 21.0));
                put_check(ui, rows, &format!("{side}SortDescending"), at(vx + x, vy + 318.0, 143.0, 17.0));
                put_check(ui, rows, &format!("{side}ShowHiddenFiles"), at(vx + x, vy + 339.0, 143.0, 17.0));
                put_label(ui, at(vx + x, vy + 362.0, 140.0, 15.0), &label_of(rows, &format!("{side}ColumnCount")));
                put_combo(ui, rows, &format!("{side}ColumnCount"), at(vx + x, vy + 381.0, 126.0, 21.0));
            }
        }
        // tsTree: 확인란(32, 24/47/70/93), lItemWidth(32,138) eTreeItemWidth(169,135 65)+udTreeItemWidth, "pixel"(257,138), gbTreeFont(32,173 225×96).
        1 => {
            let rows = &mut form.tabs[tab];
            for (top, key) in [(24.0, "TreeWindowMatchesMainSize"), (47.0, "SaveFolderTreeState"), (70.0, "ShowHiddenFoldersInTree"), (93.0, "ShowFolderTreeIcons")] {
                put_check(ui, rows, key, at(32.0, top, 470.0, 17.0));
            }
            put_label(ui, at(32.0, 138.0, 135.0, 15.0), &label_of(rows, "FolderTreeItemWidth"));
            put_updown(ui, rows, "FolderTreeItemWidth", at(169.0, 135.0, 65.0, 21.0), 60, 400);
            put_label(ui, at(257.0, 138.0, 60.0, 15.0), &cap("Caption.TntLabel3", "pixel"));
            group_box(ui, at(32.0, 173.0, 225.0, 96.0), &cap("GroupBox.gbTreeFont", "Font"));
            let tree_style = form.draft.values.get("FolderTreeFontStyle").and_then(Value::as_i64).unwrap_or(0) & 0x0F;
            let font = format!("{} \"{}\",{},{tree_style}", label_of(rows, "FolderTreeFont"), label_value(rows, "FolderTreeFont"), label_value(rows, "FolderTreeFontSize"));
            put_label(ui, at(55.0, 205.0, 198.0, 15.0), &font);
            if let Some(Row::FontButton { label, .. }) = rows.iter().find(|r| matches!(r, Row::FontButton { tree: true, .. })) {
                if ui.put(at(73.0, 232.0, 140.0, 25.0), egui::Button::new(label.as_str())).clicked() {
                    action = Some(SettingsAction::ChooseFont(true));
                }
            }
        }
        // tsColor: lBasicColor(12,17) lbBasicColor(12,36 165×349), lExtColor(193,17) lbExtColor(193,36 265×349),
        // btnAddColor·btnEditColor·btnDelColor(467,36/67/98) btnSaveColorMap·btnImportColor(467,281/312) 81×25, lHelp(12,399).
        2 => {
            let color_text = form.tabs[tab].iter().find_map(|row| match row {
                Row::Text { key, value, .. } if key == "ExtensionColors" => Some(value.clone()),
                _ => None,
            });
            let groups = color_groups(color_text.as_deref().unwrap_or(""));
            let mut color_selected = form.color_selected.filter(|i| *i < groups.len());
            let captions = form.color_captions.clone();
            put_label(ui, at(12.0, 17.0, 165.0, 15.0), &captions[7]);
            let basic = at(12.0, 36.0, 165.0, 349.0);
            list_frame(ui, basic);
            ui.scope_builder(egui::UiBuilder::new().max_rect(basic.shrink(2.0)), |ui| {
                egui::ScrollArea::vertical().id_salt("settings-basic-colors").auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (row, text) in form.basic_items.iter().enumerate() {
                        match basic_color_index(row).and_then(|index| basic_color(&form.draft, index).map(|color| (index, color))) {
                            Some((index, color)) => {
                                let rgb = u32::from_str_radix(&color, 16).unwrap_or(0);
                                let fill = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
                                let response = ui
                                    .horizontal(|ui| {
                                        let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 12.0), egui::Sense::hover());
                                        ui.painter().rect_filled(rect, 0.0, fill);
                                        ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
                                        ui.add(egui::Button::new(text.as_str()).frame(false))
                                    })
                                    .inner;
                                if super::panes::windows_double_clicked(&response) {
                                    action = Some(SettingsAction::ChangeBasicColor(index));
                                }
                            }
                            // 원본: 줄 0·16(머리)은 파란 글(0xFF0000 BGR), 15는 빈 줄. 어두운 바탕에서도 읽히게 NDR 파랑으로.
                            None => {
                                ui.label(RichText::new(text.as_str()).color(blue));
                            }
                        }
                    }
                });
            });
            put_label(ui, at(193.0, 17.0, 265.0, 15.0), &captions[0]);
            let extensions = at(193.0, 36.0, 265.0, 349.0);
            list_frame(ui, extensions);
            ui.scope_builder(egui::UiBuilder::new().max_rect(extensions.shrink(2.0)), |ui| {
                egui::ScrollArea::vertical().id_salt("settings-ext-colors").auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (i, (color, names)) in groups.iter().enumerate() {
                        let rgb = colorref_of(color);
                        let fill = Color32::from_rgb((rgb & 0xFF) as u8, ((rgb >> 8) & 0xFF) as u8, ((rgb >> 16) & 0xFF) as u8);
                        let response = ui
                            .horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 12.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 0.0, fill);
                                ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Inside);
                                ui.add(egui::Button::selectable(color_selected == Some(i), RichText::new(names.join(";")).color(fill)).truncate())
                            })
                            .inner;
                        if response.clicked() {
                            color_selected = Some(i);
                        }
                        if super::panes::windows_double_clicked(&response) {
                            color_selected = Some(i);
                            action = Some(SettingsAction::ChangeColor);
                        }
                    }
                });
            });
            for (top, caption, act) in [
                (36.0, &captions[1], SettingsAction::AddColor),
                (67.0, &captions[2], SettingsAction::EditColor),
                (98.0, &captions[3], SettingsAction::DeleteColor),
                (281.0, &captions[4], SettingsAction::ExportColors),
                (312.0, &captions[5], SettingsAction::ImportColors),
            ] {
                if ui.put(at(467.0, top, 81.0, 25.0), egui::Button::new(caption.replace('&', ""))).clicked() {
                    action = Some(act);
                }
            }
            put_left(ui, at(12.0, 395.0, 175.0, 36.0), egui::Label::new(captions[6].replace('&', "")).wrap());
            // C# 판에만 있는 "NexusFile .col 색상표 적용"(원본은 늘 적용): 확장자 목록 아래.
            put_check(ui, &mut form.tabs[tab], "UseNexusFileColorMap", at(193.0, 397.0, 265.0, 17.0));
            form.color_selected = color_selected;
        }
        // tsOption: 확인란(24, 16/38/61/84/107/129/152/175, 198/224), lDefDelBtn(46,253) cbDefDelBtn(46,270 101), lESC(24,310) cbESC(24,327 145).
        3 => {
            let rows = &mut form.tabs[tab];
            for (top, key) in [
                (16.0, "SelectFolders"),
                (38.0, "UseExplorerMenu"),

                (84.0, "RemoveReadOnlyWhenCopying"),
                (107.0, "SaveDriveFolders"),
                (129.0, "ConfirmDragDrop"),
                (152.0, "PlayCompletionSound"),
                (175.0, "PlayWaveInProgram"),
                (201.0, "UseTabs"),
                (227.0, "ShowDeleteButton"),
            ] {
                put_check(ui, rows, key, at(24.0, top, 505.0, 17.0));
            }
            put_label(ui, at(46.0, 253.0, 300.0, 15.0), &label_of(rows, "DefaultDeleteButton"));
            put_combo(ui, rows, "DefaultDeleteButton", at(46.0, 270.0, 101.0, 21.0));
            put_label(ui, at(24.0, 310.0, 300.0, 15.0), &label_of(rows, "EscapeKeyAction"));
            put_combo(ui, rows, "EscapeKeyAction", at(24.0, 327.0, 145.0, 21.0));
        }
        // tsExtProgram: gbPrograms(32,208 481×101) 보기·편집(칸 123 316). 원본 gbArchive(32,24 481×170, 외부 압축 프로그램)는 두지 않는다.
        4 => {
            let rows = &mut form.tabs[tab];
            group_box(ui, at(32.0, 208.0, 481.0, 101.0), &cap("GroupBox.gbPrograms", "External Program"));
            for (ly, key) in [(29.0, "TextViewerPath"), (65.0, "TextEditorPath")] {
                put_label(ui, at(42.0, 208.0 + ly, 110.0, 15.0), &label_of(rows, key));
                put_edit(ui, rows, key, at(32.0 + 123.0, 208.0 + ly - 4.0, 316.0, 21.0));
                if let Some(index) = index_of(rows, key) {
                    if ui.put(at(32.0 + 439.0, 208.0 + ly - 6.0, 30.0, 23.0), egui::Button::new("...")).clicked() {
                        action = Some(SettingsAction::BrowseProgram(index));
                    }
                }
            }
        }
        // tsPath: gbMyFolders(24,16 501×209) lHelpPath(16,28), 문서·그림·음악·비디오·다운로드 줄(칸 133 327, "..." 463 30×25),
        // gbStartPath(24,240 501×125) lHelpStartPath(17,24), chkStartPath1/2(17,55/91) eStartPath1/2(124,53/89 336) btnBrowse1/2(463,52/88).
        5 => {
            let rows = &mut form.tabs[tab];
            group_box(ui, at(24.0, 16.0, 501.0, 209.0), &cap("GroupBox.gbMyFolders", "Paths"));
            put_label(ui, at(40.0, 44.0, 460.0, 15.0), &cap("Caption.lHelpPath", "Leave blank to use system default path"));
            for (ly, ey, by, name) in [(60.0, 57.0, 55.0, "Documents"), (88.0, 85.0, 83.0, "Pictures"), (116.0, 113.0, 111.0, "Music"), (144.0, 141.0, 140.0, "Videos"), (174.0, 171.0, 170.0, "Downloads")] {
                let key = format!("SpecialFolder:{name}");
                put_label(ui, at(40.0, 16.0 + ly, 110.0, 15.0), &label_of(rows, &key));
                put_edit(ui, rows, &key, at(24.0 + 133.0, 16.0 + ey, 327.0, 21.0));
                if let Some(index) = index_of(rows, &key) {
                    if ui.put(at(24.0 + 463.0, 16.0 + by, 30.0, 25.0), egui::Button::new("...")).clicked() {
                        action = Some(SettingsAction::BrowseFolder(index));
                    }
                }
            }
            group_box(ui, at(24.0, 240.0, 501.0, 125.0), &cap("GroupBox.gbStartPath", "Start Folder"));
            put_label(ui, at(41.0, 264.0, 460.0, 15.0), &cap("Caption.lHelpStartPath", "Always start NexusFile  with following folders"));
            for (y, side) in [(55.0, "Left"), (91.0, "Right")] {
                put_check(ui, rows, &format!("{side}StartPathEnabled"), at(41.0, 240.0 + y, 104.0, 17.0));
                put_edit(ui, rows, &format!("{side}StartPath"), at(24.0 + 124.0, 240.0 + y - 2.0, 336.0, 21.0));
                if let Some(index) = index_of(rows, &format!("{side}StartPath")) {
                    if ui.put(at(24.0 + 463.0, 240.0 + y - 3.0, 30.0, 25.0), egui::Button::new("...")).clicked() {
                        action = Some(SettingsAction::BrowseText(index));
                    }
                }
            }
        }
        // tsFunctionKey: lvFunctionKey(20,31 504×360: 키·이름 120·명령 300), TntLabel66 "^F1 = Ctrl + F1"(20,399), btnEditFunc(368,399)·btnFuncDelete(449,399) 75×25.
        6 => {
            let headers: Vec<String> = cap("ListViewCols.lvFunctionKey", "Key;Name;Command").split(';').map(str::to_owned).collect();
            let header = |i: usize, fallback: &str| headers.get(i).cloned().unwrap_or_else(|| fallback.into());
            let columns = [super::grid::Column::text(header(0, "Key"), 60.0), super::grid::Column::text(header(1, "Name"), 120.0), super::grid::Column::text(header(2, "Command"), 300.0)];
            let keys = &mut form.function_keys;
            let rows: Vec<[String; 5]> = (0..keys.bindings.len()).map(|i| keys.row(i)).collect();
            let mut double_clicked = false;
            ui.scope_builder(egui::UiBuilder::new().max_rect(at(20.0, 31.0, 504.0, 360.0)), |ui| {
                let view = super::grid::GridView { id: "settings-function-keys", columns: &columns, height: 330.0, enabled: true, sortable: false, multi_select: false, keyboard: true, context_menu: None, row_color: None, background: None };
                let events = super::grid::show(ui, &view, &mut keys.grid, &|row, column| rows[row][[0, 1, 3][column]].clone(), &|_| false);
                double_clicked = events.double_clicked.is_some();
            });
            if double_clicked {
                action = Some(SettingsAction::EditFunctionKey);
            }
            // 원본 lvFunctionKey OnKeyDown: Enter = 편집, Delete = 삭제(이 쪽에는 글 칸이 없어 목록이 글쇠를 받는다).
            if !ui.ctx().wants_keyboard_input() {
                if enter {
                    action = Some(SettingsAction::EditFunctionKey);
                } else if delete {
                    action = Some(SettingsAction::ResetFunctionKey);
                }
            }
            put_label(ui, at(20.0, 399.0, 200.0, 15.0), &cap("Caption.TntLabel66", "^F1 = Ctrl + F1"));
            if ui.put(at(368.0, 399.0, 75.0, 25.0), egui::Button::new(cap("Caption.btnEditFunc", "Edit"))).clicked() {
                action = Some(SettingsAction::EditFunctionKey);
            }
            if ui.put(at(449.0, 399.0, 75.0, 25.0), egui::Button::new(cap("Caption.btnFuncDelete", "Delete"))).clicked() {
                action = Some(SettingsAction::ResetFunctionKey);
            }
        }
        // tsEtc: lMostUsedFolder(24,21) eMostUsedFolder(377,18 49)+ud, lActivateKey(24,56) lWinKey(221,56) eActivateKey(293,51 25) lApplyAfterRestart(324,54),
        // 확인란(24, 84/104/125/145/166), gbAlpha(24,210 211×157): chkAlphaWindow(16,34) tbAlphaValue(11,60 189×36) lTransMin(19,96) lTransMax(153,96) chkMenuAlpha(16,126).
        _ => {
            let rows = &mut form.tabs[tab];
            put_label(ui, at(24.0, 21.0, 350.0, 15.0), &label_of(rows, "MaxRecentFileCount"));
            put_updown(ui, rows, "MaxRecentFileCount", at(377.0, 18.0, 49.0, 21.0), 0, 10);
            put_label(ui, at(24.0, 56.0, 195.0, 15.0), &cap("Caption.lActivateKey", "Bring NexusFile to front:"));
            put_label(ui, at(221.0, 56.0, 70.0, 15.0), &cap("Caption.lWinKey", "WinKey + "));
            put_edit(ui, rows, "ExplorerShortcutKey", at(293.0, 51.0, 25.0, 21.0));
            put_label(ui, at(324.0, 54.0, 225.0, 15.0), &cap("Caption.lApplyAfterRestart", "(Restart required)"));
            for (top, key) in [(87.0, "CheckForUpdatesOnStartup"), (107.0, "UseDescriptionFile"), (128.0, "UseSingleInstance"), (148.0, "UseAlternateDriveHandling"), (169.0, "HideWhenMinimized")] {
                put_check(ui, rows, key, at(24.0, top, 505.0, 17.0));
            }
            group_box(ui, at(24.0, 210.0, 211.0, 157.0), &cap("GroupBox.gbAlpha", "Window Transparency"));
            put_check(ui, rows, "UseWindowAlpha", at(40.0, 244.0, 190.0, 17.0));
            if let Some(Row::Slider { value, .. }) = index_of(rows, "WindowAlpha").map(|i| &mut rows[i]) {
                ui.scope_builder(egui::UiBuilder::new().max_rect(at(35.0, 270.0, 189.0, 36.0)), |ui| {
                    ui.spacing_mut().slider_width = 180.0;
                    ui.add(egui::Slider::new(value, 20.0..=255.0).step_by(1.0).show_value(false));
                });
            }
            put_label(ui, at(43.0, 306.0, 110.0, 15.0), &cap("Caption.lTransMin", "Transparent"));
            put_label(ui, at(177.0, 306.0, 55.0, 15.0), &cap("Caption.lTransMax", "Opaque"));
            put_check(ui, rows, "UseMenuAlpha", at(40.0, 336.0, 190.0, 17.0));
        }
    }
    action
}

/// C# 판에만 있는 줄의 "NDR" 쪽: 위에서 아래로 늘어놓는다(C# OptionsWindow).
fn list_page(ui: &mut egui::Ui, form: &mut SettingsForm) -> Option<SettingsAction> {
    let ko = form.ko;
    let mut action = None;
    let browse_folder = t(ko, "찾아보기...", "Browse...");
    let browse_program = t(ko, "찾아보기…", "Browse…");
    let tab = form.tab;
    egui::ScrollArea::vertical().id_salt(("settings-tab", tab)).auto_shrink([false, false]).show(ui, |ui| {
        let label_cell = |ui: &mut egui::Ui, text: &str| {
            ui.allocate_ui_with_layout(egui::vec2(200.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(200.0);
                ui.add(egui::Label::new(text).truncate());
            });
        };
        for (index, row) in form.tabs[tab].iter_mut().enumerate() {
            match row {
                Row::Section(title) => {
                    ui.add_space(8.0);
                    ui.label(RichText::new(title.as_str()).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())).color(Color32::from_rgb(0x3a, 0x8a, 0xde)));
                    ui.add_space(2.0);
                }
                Row::Note(text) => {
                    ui.add(egui::Label::new(RichText::new(text.as_str()).weak()).wrap());
                }
                Row::Check { label, value, .. } => {
                    ui.checkbox(value, label.as_str());
                }
                Row::Text { label, value, read_only, max, upper, .. } => {
                    ui.horizontal(|ui| {
                        label_cell(ui, label);
                        if *read_only {
                            ui.add(egui::TextEdit::singleline(&mut value.as_str()).desired_width(f32::INFINITY));
                        } else if ui.add(egui::TextEdit::singleline(value).char_limit(*max).desired_width(f32::INFINITY)).changed() && *upper {
                            *value = value.to_uppercase();
                        }
                    });
                }
                Row::Folder { label, value, .. } => {
                    ui.horizontal(|ui| {
                        label_cell(ui, label);
                        ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width() - 96.0));
                        if ui.button(browse_folder.as_str()).clicked() {
                            action = Some(SettingsAction::BrowseFolder(index));
                        }
                    });
                }
                Row::Program { label, value, .. } => {
                    ui.horizontal(|ui| {
                        label_cell(ui, label);
                        ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width() - 96.0));
                        if ui.button(browse_program.as_str()).clicked() {
                            action = Some(SettingsAction::BrowseProgram(index));
                        }
                    });
                }
                Row::Combo { key, label, choices, selected } => {
                    ui.horizontal(|ui| {
                        label_cell(ui, label);
                        let text = selected.and_then(|i| choices.get(i)).map(|(_, d)| d.clone()).unwrap_or_default();
                        egui::ComboBox::from_id_salt(("settings-combo", key.as_str())).width(300.0).selected_text(text).show_opaque(ui, |ui| {
                            for (i, (_, display)) in choices.iter().enumerate() {
                                ui.selectable_value(selected, Some(i), display.as_str());
                            }
                        });
                    });
                }
                Row::Slider { label, value, .. } => {
                    ui.horizontal(|ui| {
                        label_cell(ui, label);
                        ui.add(egui::Slider::new(value, 0.0..=255.0).step_by(1.0).show_value(false));
                    });
                }
                Row::ColorButtons | Row::FunctionKeys | Row::FontButton { .. } => {}
            }
        }
    });
    action
}

/// 원본 TfPreferences(ClientWidth 577 × ClientHeight 512): TntPageControl(7,8 561×465, 쪽 8개 + C# 판에만 있는 줄의 "NDR" 쪽),
/// btnApply(321,480)·btnOk(405,480)·btnCancel(488,480) 79×25. 쪽 바탕은 PageControl 안쪽(4,24)이고 쪽 안 자리는 DFM 그대로.
pub(super) fn settings_ui(ui: &mut egui::Ui, form: &mut SettingsForm, enter: bool, delete: bool) -> Option<SettingsAction> {
    let mut action = None;
    let (area, _) = ui.allocate_exact_size(egui::vec2(577.0, 512.0), egui::Sense::hover());
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    ui.scope_builder(egui::UiBuilder::new().max_rect(at(9.0, 8.0, 557.0, 22.0)).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        for (index, title) in form.tab_titles.iter().enumerate() {
            if ui.selectable_label(form.tab == index, title.replace('&', "")).clicked() {
                form.tab = index;
            }
        }
    });
    ui.painter().rect_stroke(at(7.0, 30.0, 561.0, 443.0), 2.0, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    let tab = form.tab.min(form.tabs.len().saturating_sub(1));
    let page = at(11.0, 32.0, 553.0, 437.0);
    let chosen = if tab < 8 { original_page(ui, form, tab, page.min, enter, delete) } else { ui.scope_builder(egui::UiBuilder::new().max_rect(page.shrink(4.0)), |ui| list_page(ui, form)).inner };
    if chosen.is_some() {
        action = chosen;
    }
    let [ok, cancel, apply] = form.button_captions.clone();
    for (left, caption, act) in [(321.0, apply, SettingsAction::Apply), (405.0, ok, SettingsAction::Ok), (488.0, cancel, SettingsAction::Cancel)] {
        if ui.put(at(left, 480.0, 79.0, 25.0), egui::Button::new(caption.replace('&', ""))).clicked() {
            action = Some(act);
        }
    }
    if enter && action.is_none() && !ui.ctx().wants_keyboard_input() {
        action = Some(SettingsAction::Ok);
    }
    action
}

impl App {
    pub(super) fn settings_form(&mut self) -> Option<&mut SettingsForm> {
        self.original_dialogs.iter_mut().rev().find_map(|d| match &mut d.kind {
            DialogKind::Settings(form) => Some(form.as_mut()),
            _ => None,
        })
    }

    /// C# ShowOptionsAsync: 환경 설정 창.
    pub(super) fn open_settings_window(&mut self) {
        let mut form = SettingsForm::new(&self.settings, &self.language_pack, &self.resources);
        // 기능키 목록의 "기본/내부 명령/사용자 명령" 글은 지금 화면 언어(C# FunctionKeyEditorWindow처럼).
        form.function_keys = super::function_key_editor::FunctionKeyEditorForm::new(&self.settings, self.language == "ko");
        // 원본 TfPreferences 제목([formPreferences] Form.Caption, 한국어 "옵션").
        let title = self.language_pack.text("formPreferences.Form.Caption", "Options");
        self.push_original_dialog(title, DialogKind::Settings(Box::new(form)), |app, result, _| {
            if let DialogResult::Settings(Some(settings)) = result {
                app.apply_settings(settings);
            }
        });
    }

    /// 확인한 뒤(C# ShowOptionsAsync의 나머지): 정규화, 언어·테마·두 패널·숨김 표시·보기, 저장, 두 패널 새로 고침.
    pub(super) fn apply_settings(&mut self, settings: ndr::settings::Settings) {
        let previous_single = self.settings.flag("UseSingleInstance", false);
        self.settings = settings;
        self.settings.normalize();
        // C# ShowOptionsAsync: UseSingleInstance가 바뀌면 ApplySingleInstanceModeAsync. 실패면 이전 값으로 되돌리고 알린다.
        let wanted_single = self.settings.flag("UseSingleInstance", false);
        if wanted_single != previous_single && !self.apply_single_instance(wanted_single) {
            self.settings.set_flag("UseSingleInstance", previous_single);
            let text = self.language_pack.text(
                "NDRCompatibility.SingleInstanceConflict",
                "Another NDR instance already uses this user data directory, or single-instance mode could not be enabled. The previous setting was kept.",
            );
            self.system_box("NDR", &text, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONASTERISK, |_, _, _| {});
        }
        self.language = self.settings.text("Language", &self.language);
        self.dark = super::system_commands::theme_palette(&self.settings.text("Theme", "Black")).dark();
        self.dual = self.settings.flag("DualPaneMode", true);
        self.profile_load_error = false;
        self.preferences_dirty = true;
        self.save();
        for index in 0..2 {
            self.panes[index].show_hidden = self.settings.flag(if index == 0 { "LeftShowHiddenFiles" } else { "RightShowHiddenFiles" }, false);
            self.refresh(index);
        }
    }

    pub(super) fn settings_action(&mut self, action: SettingsAction, ctx: &egui::Context) {
        let ko = self.language == "ko";
        match action {
            SettingsAction::Ok => {
                let Some(form) = self.settings_form() else { return };
                match form.commit() {
                    Ok(settings) => self.answer_dialog(DialogResult::Settings(Some(settings)), ctx),
                    Err(error) => {
                        let caption = self.tr("환경 설정", "Settings").into_owned();
                        self.system_box(&caption, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                    }
                }
            }
            SettingsAction::Cancel => self.answer_dialog(DialogResult::Settings(None), ctx),
            SettingsAction::Apply => {
                let Some(form) = self.settings_form() else { return };
                match form.commit() {
                    Ok(settings) => {
                        self.apply_settings(settings);
                        // 창은 그대로 두고, 다음 확인·적용의 바탕(_draft)을 적용한 설정으로.
                        let applied = self.settings.clone();
                        if let Some(form) = self.settings_form() {
                            form.draft = applied;
                        }
                    }
                    Err(error) => {
                        let caption = self.tr("환경 설정", "Settings").into_owned();
                        self.system_box(&caption, &error, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
                    }
                }
            }
            SettingsAction::BrowseFolder(index) => {
                let Some(form) = self.settings_form() else { return };
                let tab = form.tab;
                let Some(Row::Folder { label, value, default, .. }) = form.tabs[tab].get(index).cloned() else { return };
                let configured = PathBuf::from(native::expand_environment(value.trim().trim_matches('"')));
                let profile = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default());
                let initial = if configured.is_dir() { configured } else if default.is_dir() { default } else { profile };
                let title = label.trim_end_matches([':', ' ']).to_owned();
                if let Some(path) = (self.folder_picker)(&title, Some(&initial)) {
                    if let Some(Row::Folder { value, .. }) = self.settings_form().and_then(|f| f.tabs[tab].get_mut(index)) {
                        *value = path.to_string_lossy().into_owned();
                    }
                }
            }
            SettingsAction::BrowseProgram(index) => {
                let Some(form) = self.settings_form() else { return };
                let tab = form.tab;
                let Some(Row::Program { value, .. }) = form.tabs[tab].get(index).cloned() else { return };
                let current = PathBuf::from(native::expand_environment(value.trim().trim_matches('"')));
                let mut initial = if current.is_file() { current.parent().map(Path::to_path_buf).unwrap_or_default() } else { current.clone() };
                if !initial.is_dir() {
                    initial = PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_default());
                }
                let name = if current.is_file() { current.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { String::new() };
                let filters = super::original_dialogs::program_filter(&self.language_pack);
                let filters: Vec<(&str, &str)> = filters.iter().map(|(n, p)| (n.as_str(), p.as_str())).collect();
                let title = if ko { "프로그램 선택" } else { "Select a program" };
                if let Some(path) = (self.open_dialog)(title, Some(&initial), &name, &filters) {
                    if let Some(Row::Program { value, .. }) = self.settings_form().and_then(|f| f.tabs[tab].get_mut(index)) {
                        *value = path.to_string_lossy().into_owned();
                    }
                }
            }
            SettingsAction::ImportColors => self.import_extension_colors(),
            SettingsAction::ExportColors => self.export_extension_colors(),
            SettingsAction::AddColor => self.add_extension_color(),
            SettingsAction::EditColor => self.edit_extension_color(),
            SettingsAction::DeleteColor => self.change_color_groups(|groups, selected| {
                // 원본 btnDelColorClick(0x44fb98): 고른 줄의 확장자를 모두 지운다(묻지 않음).
                if selected < groups.len() {
                    groups.remove(selected);
                }
            }),
            SettingsAction::ChangeColor => self.change_extension_color(),
            SettingsAction::ChangeBasicColor(index) => self.change_basic_color(index),
            SettingsAction::ChooseFont(tree) => self.choose_settings_font(tree),
            SettingsAction::EditFunctionKey => {
                let Some(form) = self.settings_form() else { return };
                let Some(index) = form.function_keys.selected() else { return };
                let (key, current) = form.function_keys.bindings[index].clone();
                let current = current.unwrap_or(super::function_key_editor::Binding { name: key.clone(), ..Default::default() });
                let edit = super::function_key_editor::FunctionKeyEditForm::new(&key, &current, ko);
                let title = format!("{} — {key}", self.language_pack.text("formEditFunction.Form.Caption", "Edit Function Key"));
                self.push_original_dialog(title, DialogKind::FunctionKeyEdit(Box::new(edit)), move |app, result, _| {
                    let DialogResult::FunctionKeyEdit(Some(binding)) = result else { return };
                    if let Some(form) = app.settings_form() {
                        form.function_keys.bindings[index].1 = Some(binding);
                        form.sync_function_keys();
                    }
                });
            }
            SettingsAction::ResetFunctionKey => {
                let Some(form) = self.settings_form() else { return };
                if let Some(index) = form.function_keys.selected() {
                    form.function_keys.bindings[index].1 = None;
                    form.sync_function_keys();
                }
            }
            SettingsAction::BrowseText(index) => {
                let Some(form) = self.settings_form() else { return };
                let tab = form.tab;
                let Some(Row::Text { label, value, .. }) = form.tabs[tab].get(index).cloned() else { return };
                let configured = PathBuf::from(native::expand_environment(value.trim().trim_matches('"')));
                let initial = if configured.is_dir() { configured } else { PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()) };
                let title = label.trim_end_matches([':', ' ']).to_owned();
                if let Some(path) = (self.folder_picker)(&title, Some(&initial)) {
                    if let Some(Row::Text { value, .. }) = self.settings_form().and_then(|f| f.tabs[tab].get_mut(index)) {
                        *value = path.to_string_lossy().into_owned();
                    }
                }
            }
        }
    }

    /// 설정 창의 확장자 색 묶음(lbExtColor)과 고른 줄.
    fn color_state(&mut self) -> Option<(Vec<(String, Vec<String>)>, Option<usize>)> {
        let form = self.settings_form()?;
        let selected = form.color_selected;
        let Some(Row::Text { value, .. }) = form.row_mut("ExtensionColors") else { return None };
        Some((color_groups(value), selected))
    }

    /// 고른 줄이 있으면 묶음을 고치고 ExtensionColors 글로 되돌린다(원본 FUN_0044fd48: 목록을 다시 채움, 고른 줄 없음).
    fn change_color_groups(&mut self, change: impl FnOnce(&mut Vec<(String, Vec<String>)>, usize)) {
        let Some((mut groups, Some(selected))) = self.color_state() else { return };
        change(&mut groups, selected);
        self.store_color_groups(groups);
    }

    fn store_color_groups(&mut self, mut groups: Vec<(String, Vec<String>)>) {
        groups.retain(|(_, names)| !names.is_empty());
        let Some(form) = self.settings_form() else { return };
        if let Some(Row::Text { value, .. }) = form.row_mut("ExtensionColors") {
            *value = groups_text(&groups);
        }
        form.color_selected = None;
    }

    /// TNXFileColorMap.Add/SetExtension: 그 확장자를 다른 색 묶음에서 빼고 이 색 묶음(없으면 새 줄)에 넣는다.
    fn put_extensions(groups: &mut Vec<(String, Vec<String>)>, names: &[String], color: &str) {
        for name in names {
            for (_, existing) in groups.iter_mut() {
                existing.retain(|n| n != name);
            }
            match groups.iter_mut().find(|(c, _)| c.eq_ignore_ascii_case(color)) {
                Some((_, existing)) => existing.push(name.clone()),
                None => groups.push((color.to_owned(), vec![name.clone()])),
            }
        }
    }

    /// 원본 PREF_COLOREXISTS: 그 색을 이미 쓰는 확장자가 있으면 TfMsgBox(PREF_SETCOLOR, PREF_COLOREXISTS, 예/아니요)로 합칠지 묻는다.
    /// 예이거나 쓰는 확장자가 없으면 apply.
    fn confirm_color_merge(&mut self, groups: &[(String, Vec<String>)], color: &str, apply: impl FnOnce(&mut App) + 'static) {
        if !groups.iter().any(|(c, names)| c.eq_ignore_ascii_case(color) && !names.is_empty()) {
            return apply(self);
        }
        let title = self.language_pack.text("RES.PREF_SETCOLOR", "Color Setting");
        let text = self.language_pack.text("RES.PREF_COLOREXISTS", "There are extensions with specified color. You want to merge with those extensions?");
        self.message_box(&title, "", &text, MB_YESNO | MB_ICONQUESTION, move |app, answer, _| {
            if answer == IDYES {
                apply(app);
            }
        });
    }

    /// 원본 btnAddColorClick(0x44f360): TfGetText(PREF_TITLE_ADDEXTS, PREF_MSG_ADDEXTS)로 확장자(';' 구분)를 받고, 비어 있지 않으면
    /// 색 고르기(TColorDialog, 처음 색 = 마지막 색). 그 색을 쓰는 확장자가 있으면 합칠지 묻고, 예일 때만 더한다.
    fn add_extension_color(&mut self) {
        let title = self.language_pack.text("RES.PREF_TITLE_ADDEXTS", "Add Extentions");
        let message = self.language_pack.text("RES.PREF_MSG_ADDEXTS", "Type extensions to add with semi-colon separator.\n(ex: TXT;DOC;XLS)");
        self.prompt(&title, &message, "", None, false, |app, value, _| {
            let Some(value) = value.filter(|v| !v.is_empty()) else { return };
            let names = split_extensions(&value);
            let Some(initial) = app.settings_form().map(|f| f.last_color) else { return };
            let Some(color) = (app.color_picker)(initial) else { return };
            if let Some(form) = app.settings_form() {
                form.last_color = color;
            }
            let Some((groups, _)) = app.color_state() else { return };
            let hex = hex_of(color);
            let check = hex.clone();
            app.confirm_color_merge(&groups, &check, move |app| {
                let Some((mut groups, _)) = app.color_state() else { return };
                Self::put_extensions(&mut groups, &names, &hex);
                app.store_color_groups(groups);
            });
        });
    }

    /// 원본 btnEditColorClick(0x44f7c0): 고른 줄의 확장자 글을 TfGetText(PREF_TITLE_EDITEXTS, PREF_MSG_EDITEXTS, 처음 값 = 그 글)로 고치고,
    /// 확인이면 옛 확장자를 지우고 새 확장자를 같은 색으로 넣는다.
    fn edit_extension_color(&mut self) {
        let Some((groups, Some(selected))) = self.color_state() else { return };
        let Some((color, names)) = groups.get(selected).cloned() else { return };
        let title = self.language_pack.text("RES.PREF_TITLE_EDITEXTS", "Edit Extensions");
        let message = self.language_pack.text("RES.PREF_MSG_EDITEXTS", "Type extensions with semi-colon separator.\n(ex: TXT;DOC;XLS)");
        self.prompt(&title, &message, &names.join(";"), None, false, move |app, value, _| {
            let Some(value) = value else { return };
            let Some((mut groups, _)) = app.color_state() else { return };
            for (_, existing) in groups.iter_mut() {
                existing.retain(|n| !names.contains(n));
            }
            Self::put_extensions(&mut groups, &split_extensions(&value), &color);
            app.store_color_groups(groups);
        });
    }

    /// 원본 lbExtColorDblClick(0x44ef68): 고른 줄의 색으로 색 고르기를 열고, 확인이면(그 색을 쓰는 확장자가 있으면 합칠지 물은 뒤) 그 줄의 확장자 색을 바꾼다.
    fn change_extension_color(&mut self) {
        let Some((groups, Some(selected))) = self.color_state() else { return };
        let Some((color, names)) = groups.get(selected).cloned() else { return };
        let Some(chosen) = (self.color_picker)(colorref_of(&color)) else { return };
        if let Some(form) = self.settings_form() {
            form.last_color = chosen;
        }
        let hex = hex_of(chosen);
        let check = hex.clone();
        self.confirm_color_merge(&groups, &check, move |app| {
            let Some((mut groups, _)) = app.color_state() else { return };
            Self::put_extensions(&mut groups, &names, &hex);
            app.store_color_groups(groups);
        });
    }

    /// 원본 lbBasicColorDblClick(0x44ed5c): 그 기본 색으로 색 고르기를 열고, 확인이면 SetBasic. NDR은 NexusFileColorMapValues에 넣고
    /// 색상표 적용(UseNexusFileColorMap)을 켠다(원본은 기본 색을 늘 쓴다).
    fn change_basic_color(&mut self, index: usize) {
        let Some((key, _)) = ndr::styles::DEFAULT_ELEMENTS.get(index) else { return };
        let Some(current) = self.settings_form().and_then(|f| basic_color(&f.draft, index)) else { return };
        let rgb = u32::from_str_radix(&current, 16).unwrap_or(0);
        let colorref = ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF);
        let Some(chosen) = (self.color_picker)(colorref) else { return };
        let Some(form) = self.settings_form() else { return };
        form.last_color = chosen;
        let mut values = object(&form.draft, "NexusFileColorMapValues");
        values.insert((*key).to_owned(), Value::String(hex_of(chosen).trim_start_matches('#').to_owned()));
        form.draft.values.insert("NexusFileColorMapValues".into(), Value::Object(values));
        form.draft.set_flag("UseNexusFileColorMap", true);
        if let Some(Row::Check { value, .. }) = form.row_mut("UseNexusFileColorMap") {
            *value = true;
        }
    }

    /// 원본 btnFontClick: 글꼴 상자(처음 값 = 지금 이름·크기·모양)를 열고 확인이면 이름·크기 줄과 모양(원본 TFontStyles 바이트)을 바꾼다.
    fn choose_settings_font(&mut self, tree: bool) {
        let (name_key, size_key, style_key) = if tree { ("FolderTreeFont", "FolderTreeFontSize", "FolderTreeFontStyle") } else { ("ListFont", "ListFontSize", "ListFontStyle") };
        let Some(form) = self.settings_form() else { return };
        let read = |form: &mut SettingsForm, key: &str| match form.row_mut(key) {
            Some(Row::Text { value, .. }) => value.clone(),
            _ => String::new(),
        };
        let name = read(form, name_key);
        let size = read(form, size_key).trim().parse::<f32>().unwrap_or(0.0);
        let style = (form.draft.values.get(style_key).and_then(Value::as_i64).unwrap_or(0) & 0x0F) as u8;
        let Some((name, size, style)) = (self.font_picker)(name.trim(), size, style) else { return };
        let Some(form) = self.settings_form() else { return };
        if let Some(Row::Text { value, .. }) = form.row_mut(name_key) {
            *value = name;
        }
        if let Some(Row::Text { value, .. }) = form.row_mut(size_key) {
            *value = if size.fract() == 0.0 { format!("{}", size as i64) } else { format!("{size}") };
        }
        form.draft.values.insert(style_key.into(), Value::from(style & 0x0F));
    }

    /// C# ImportExtensionColors.
    fn import_extension_colors(&mut self) {
        let ko = self.language == "ko";
        let filters = if ko {
            [("NexusFile 색상표 (*.col)", "*.col"), ("NDR 색상표 (*.json)", "*.json"), ("모든 파일 (*.*)", "*.*")]
        } else {
            [("NexusFile color map (*.col)", "*.col"), ("NDR color map (*.json)", "*.json"), ("All files (*.*)", "*.*")]
        };
        let Some(path) = (self.open_dialog)("", None, "", &filters) else { return };
        let result: Result<(), String> = (|| {
            let Some(form) = self.settings_form() else { return Ok(()) };
            let colors: Vec<(String, String)> = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("col")) {
                let map = ndr::styles::ColorMap::load(&path)?;
                let hex = |c: &[u8; 4]| format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
                let elements: Map<String, Value> = map.elements.iter().map(|(k, c)| (k.clone(), Value::String(hex(c)))).collect();
                let used = !elements.is_empty();
                form.draft.values.insert("NexusFileColorMapValues".into(), Value::Object(elements));
                form.draft.set_flag("UseNexusFileColorMap", used);
                if let Some(Row::Check { value, .. }) = form.row_mut("UseNexusFileColorMap") {
                    *value = used;
                }
                map.extensions.iter().map(|(k, c)| (k.clone(), format!("#{}", hex(c)))).collect()
            } else {
                let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                let parsed: Map<String, Value> = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
                form.draft.values.insert("NexusFileColorMapValues".into(), Value::Object(Map::new()));
                form.draft.set_flag("UseNexusFileColorMap", false);
                if let Some(Row::Check { value, .. }) = form.row_mut("UseNexusFileColorMap") {
                    *value = false;
                }
                parsed.into_iter().map(|(k, v)| (k, v.as_str().unwrap_or("").to_owned())).collect()
            };
            let text = colors.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
            parse_extension_colors(&text)?;
            if let Some(Row::Text { value, .. }) = form.row_mut("ExtensionColors") {
                *value = text;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let title = self.tr("NDR 환경 설정", "NDR Settings").into_owned();
            let text = if ko { format!("색상표를 읽을 수 없습니다: {error}") } else { format!("The color map could not be read: {error}") };
            self.system_box(&title, &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        }
    }

    /// C# ExportExtensionColors.
    fn export_extension_colors(&mut self) {
        let ko = self.language == "ko";
        let title = self.tr("NDR 환경 설정", "NDR Settings").into_owned();
        let fail = |app: &mut App, error: String| {
            let text = if ko { format!("색상표를 내보낼 수 없습니다: {error}") } else { format!("The color map could not be exported: {error}") };
            app.system_box(&title, &text, MB_OK | MB_ICONEXCLAMATION, |_, _, _| {});
        };
        let Some(form) = self.settings_form() else { return };
        let text = match form.tabs.iter().flatten().find_map(|r| match r {
            Row::Text { key, value, .. } if key == "ExtensionColors" => Some(value.clone()),
            _ => None,
        }) {
            Some(text) => text,
            None => String::new(),
        };
        let colors = match parse_extension_colors(&text) {
            Ok(colors) => colors,
            Err(error) => return fail(self, error),
        };
        let elements = object(&form.draft, "NexusFileColorMapValues");
        let filters = if ko { [("NexusFile 색상표 (*.col)", "*.col"), ("NDR 색상표 (*.json)", "*.json")] } else { [("NexusFile color map (*.col)", "*.col"), ("NDR color map (*.json)", "*.json")] };
        let Some(path) = (self.save_dialog)("", None, "NDR-extension-colors.col", &filters, "col", true) else { return };
        let body = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("col")) {
            let mut map = ndr::styles::ColorMap::default();
            for (key, value) in &elements {
                if let Some(color) = value.as_str().and_then(ndr::styles::parse_color) {
                    map.elements.insert(key.to_ascii_uppercase(), color);
                }
            }
            for (extension, color) in &colors {
                if let Some(color) = ndr::styles::parse_color(color) {
                    map.extensions.insert(extension.clone(), color);
                }
            }
            map.serialize()
        } else {
            serde_json::to_string_pretty(&Value::Object(colors.into_iter().map(|(k, v)| (k, Value::String(v))).collect())).unwrap_or_default()
        };
        if let Err(error) = std::fs::write(&path, body) {
            fail(self, error.to_string());
        }
    }
}
