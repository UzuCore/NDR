use ndr::{compat::{Language, available_codes}, markup::substitute};
use std::path::Path;

fn catalogs() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Resources/Languages"))
}

#[test]
fn all_language_choices_have_a_native_primary_menu() {
    let codes = available_codes(catalogs());
    let registered = ndr::compat::BUILTIN_LANGUAGES;
    assert_eq!(registered.len(), 40);
    assert_eq!(codes.len(), registered.len());
    for &(code, name) in registered {
        assert!(codes.iter().any(|file| file == code), "{code}: missing catalog");
        assert!(!name.is_empty());
    }
    for code in codes {
        let pack = Language::load(catalogs(), &code);
        assert_eq!(pack.code, code);
        assert!(pack.raw("ToolButton.tbFile").is_some());
        assert!(pack.primary_raw("Menu.miCopyTo").is_some(), "{code}: copy command not localized");
        assert!(pack.primary_raw("RES.MSG_COPY_PREPARING").is_some(), "{code}: copy progress not localized");
        assert!(pack.primary_raw("NDRUpdate.Title").is_some());
        assert!(pack.primary_raw("NDR.UPD_TITLE").is_some(), "{code}: update dialog must prefer its own label");
        assert!(pack.primary_raw("NDR.UPD_INSTALL_TITLE").is_some(), "{code}: installer must prefer its own label");
        assert_eq!(pack.text("RES.RENAME", "missing").contains("missing"), false);
    }
    let ja = Language::load(catalogs(), "ja");
    assert_eq!(ja.raw("Menu.miOpen").as_deref(), Some("&Q. 開く"));
    assert_eq!(ja.raw("Menu.miParameterOpen").as_deref(), Some("&W. Run with arguments…"));
    assert_eq!(ja.text("RES.MSG_COPY_PREPARING", "missing"), "コピーを準備しています…");
    assert_eq!(Language::load(catalogs(), "invalid").code, "ko");
}

#[test]
fn user_and_system_locale_spellings_resolve_without_losing_specific_variants() {
    for (requested, expected) in [
        ("KO_kr", "ko"), ("bg-BG", "bg"), ("DA_dk", "da"), ("el-GR", "el"),
        ("id-ID", "id"), ("in-ID", "id"), ("fil-PH", "fil"), ("tl_PH", "fil"),
        ("nb-NO", "nb"), ("no-NO", "nb"), ("pt", "pt-PT"), ("pt_BR", "pt-BR"),
        ("pt-AO", "pt-PT"), ("sr-Latn-RS", "sr-Latn"), ("zh_Hant_HK", "zh-TW"),
        ("zh-SG", "zh-CN"), ("vi-VN", "vi"), ("../../en", "ko"),
    ] {
        assert_eq!(Language::load(catalogs(), requested).code, expected, "{requested}");
    }
}

#[test]
fn new_catalogs_keep_format_strings_and_data_list_order() {
    for code in ["en", "ko"] {
        let pack = Language::load(catalogs(), code);
        let summary = substitute(&pack.text("RES.FOLDERINFO", "missing"), &["2", "9", "8 MB"]);
        assert!(summary.contains("<c000000><b>2</b></c>"));
        assert!(summary.contains("<c000000><b>9</b></c>"));
        assert!(summary.ends_with("<b>8 MB</b>"));
        let warning = substitute(&pack.text("RES.MSG_DELWARNING2", "missing"), &["ACTION", "27"]);
        assert!(warning.contains("ACTION") && warning.contains("27"));
        let filter = pack.text("RES.OD_PROGRAMFILTER", "missing");
        let parts: Vec<_> = filter.split('|').collect();
        assert_eq!(parts.len(), 4);
        assert_eq!((parts[1], parts[3]), ("*.exe", "*.*"));
        assert_eq!(pack.text("formPreferences.ComboBox.cbSort1", "missing").split(';').count(), 7);
        assert_eq!(pack.text("formAddToArchive.ComboBox.cbVolume", "missing").split(';').count(), 8);
    }
}

#[test]
fn all_forty_catalogs_directly_translate_startup_updating_and_common_file_operations() {
    let source = include_str!("../Resources/Languages/en.lang");
    let updater_keys: Vec<_> = source.lines().filter_map(|line| {
        let (key, _) = line.split_once('=')?;
        (key.starts_with("UPD_") || key.starts_with("UI_STARTUP_")).then(|| format!("NDR.{key}"))
    }).collect();
    assert_eq!(updater_keys.len(), 33);
    let file_operations = [
        "OP_TRANSFER_CANCEL", "OP_TRANSFER_VERB_COPY", "OP_TRANSFER_VERB_MOVE", "OP_TRANSFER_VERB_PASTE",
        "OP_TRANSFER_OPERATION_FAILED", "OP_TRANSFER_PREPARING_THE_OPERATION", "OP_TRANSFER_COPY_DONE",
        "OP_TRANSFER_MOVE_DONE", "OP_TRANSFER_DELETE_DONE", "OP_TRANSFER_PARTIAL",
        "OP_TRANSFER_PARTIAL_DETAIL", "OP_TRANSFER_CANCELLED", "OP_TRANSFER_SIZE_UNAVAILABLE",
        "OP_ARCHIVE_REPLACE_ARCHIVE", "OP_JOIN_EXISTS",
    ];
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let primary = Language::load(catalogs(), code);
        for key in updater_keys.iter().cloned().chain(file_operations.iter().map(|key| format!("NDR.{key}"))) {
            assert!(primary.primary_raw(&key).is_some_and(|line| !line.is_empty()), "{code}: missing local {key}");
        }
    }
}
