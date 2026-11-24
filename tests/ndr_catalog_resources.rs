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
    assert_eq!(ja.raw("Menu.miParameterOpen").as_deref(), Some("&W. 引数を指定して実行…"));
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

#[test]
fn every_locale_supplies_core_pane_view_and_downloads_captions() {
    let keys = [
        "RES.FOLDERINFO", "RES.DISKINFO_SHORT", "RES.DISKINFO_LONG", "RES.SELECTED_INFO",
        "RES.FOLDER", "RES.DIR", "RES.FOLDERS", "RES.DISKFREE", "RES.TODAY", "RES.YESTERDAY",
        "RES.DATEAGE", "RES.FUNC_GOTODOWNLOADS", "Menu.miToDownloads", "Menu.miViewToolbar",
        "Menu.miDualView", "Menu.miListStyle", "Menu.miListStyleAuto", "Menu.miListStyleNames",
        "Menu.miListStyleDetails", "Menu.miListStyleThumbs", "Menu.miSkin", "Menu.miSortBy",
        "Menu.miSortDescend", "Menu.miShowHidden", "formPreferences.ComboBox.cbViewStyle1",
        "formPreferences.ComboBox.cbViewStyle2", "formNDRSearch.Caption.chkFiles",
        "formNDRSearch.Caption.chkFolders", "NDR.UI_PANE_FOLDER", "NDR.UI_PANE_NO_EXTENSION",
        "NDR.UI_PANE_NO_EXTENSION_INFO", "NDR.UI_PANE_NO_SELECTION", "NDR.UI_PANE_DRIVE_COUNT",
        "NDR.UI_SIZE_BYTES", "NDR.UI_PANE_SELECTION_SUMMARY",
    ];
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let primary = Language::load(catalogs(), code);
        for key in keys {
            assert!(primary.primary_raw(key).is_some_and(|label| !label.is_empty()), "{code}: {key}");
        }
        let label = ndr::markup::substitute(
            &primary.text("NDR.UI_PANE_SELECTION_SUMMARY", "missing"),
            &[], // braces are interpolated separately at runtime
        );
        assert!((0..4).all(|index| label.contains(&format!("{{{index}}}"))), "{code}: {label}");
    }
}

#[test]
fn common_settings_and_open_with_arguments_use_primary_locale() {
    let english = Language::load(catalogs(), "en");
    let keys = [
        "UI_SETTINGS_LANGUAGE", "UI_SETTINGS_THEME_A980E4", "UI_SETTINGS_CAPTION",
        "UI_SETTINGS_CONFIRM_BEFORE_DELETING_C13428", "UI_SETTINGS_DELETE_TO_THE_RECYCLE_B00473",
        "UI_SETTINGS_SHOW_TWO_PANES_FF5C36", "UI_SETTINGS_SHOW_THE_MAIN_TOOLBAR_1B9356",
        "UI_SETTINGS_SHOW_ON_SCREEN_NOTIFICATIONS_462688", "UI_SETTINGS_WORK_FOLDER_NUMBER",
        "UI_SETTINGS_DISABLED_D41047", "UI_SETTINGS_BROWSE_PROGRAM", "UI_SETTINGS_BROWSE_FOLDER",
    ];
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let primary = Language::load(catalogs(), code);
        for key in keys {
            let qualified = format!("NDR.{key}");
            assert!(primary.primary_raw(&qualified).is_some_and(|value| !value.is_empty()), "{code}: {qualified}");
        }
        let run = primary.primary_raw("Menu.miParameterOpen").expect("native command caption");
        assert!(run.starts_with("&W. "), "{code}: missing mnemonic");
        if code != "en" {
            assert_ne!(Some(run), english.primary_raw("Menu.miParameterOpen"), "{code}: English command caption");
            for key in ["UI_SETTINGS_SHOW_TWO_PANES_FF5C36", "UI_SETTINGS_WORK_FOLDER_NUMBER", "UI_SETTINGS_SHOW_THE_MAIN_TOOLBAR_1B9356"] {
                let qualified = format!("NDR.{key}");
                assert_ne!(primary.primary_raw(&qualified), english.primary_raw(&qualified), "{code}: {qualified}");
            }
        }
        assert!(primary.text("NDR.UI_SETTINGS_WORK_FOLDER_NUMBER", "").contains("{0}"), "{code}: missing work folder argument");
    }
}

#[test]
fn six_deeper_catalogs_cover_all_explicit_ndr_settings_labels() {
    let source = include_str!("../src/ui/settings_window.rs");
    let keys: std::collections::BTreeSet<_> = source.split(".localized(\"NDR.UI_SETTINGS_").skip(1)
        .filter_map(|rest| rest.split_once('"').map(|(suffix, _)| format!("NDR.UI_SETTINGS_{suffix}")))
        .collect();
    assert_eq!(keys.len(), 87);
    for code in ["ja", "zh-CN", "zh-TW", "de", "fr", "es"] {
        let primary = Language::load(catalogs(), code);
        for key in &keys {
            assert!(primary.primary_raw(key).is_some_and(|value| !value.is_empty()), "{code}: {key}");
        }
    }
}
