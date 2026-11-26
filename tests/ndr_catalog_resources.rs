use ndr::{compat::{Language, available_codes}, markup::substitute};
use std::path::Path;

#[test]
fn visual_regression_labels_are_localized_in_all_catalogs() {
    let english = Language::load(catalogs(), "en");
    let keys = [
        "ndrremotesites.passwordstoredwhensaved",
        "formmain.menu.miftpdisconnect", "formmain.menu.miftpget", "formmain.menu.miftpput",
        "formcomparefolder.caption.chkcomparecontents", "formcomparefolder.caption.chkrecursive",
        "res.func_recyclebin", "res.func_advrename",
        "formmain.menu.midropcopy", "formmain.menu.midropmove",
        "formmain.menu.midropshortcut", "formmain.menu.midropcancel",
        "formmain.menu.micheckupdate", "formmain.menu.mipopenwith",
    ];
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let pack = Language::load(catalogs(), code);
        for key in keys {
            let value = pack.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
            assert!(!value.trim().is_empty(), "{code}: empty {key}");
            if code != "en" {
                assert_ne!(Some(value), english.primary_raw(key), "{code}: English fallback {key}");
            }
        }
    }
}

fn catalogs() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Resources/Languages"))
}

#[test]
fn all_split_join_drive_kind_and_work_slot_labels_are_primary() {
    let keys = [
        "formfilesplit.caption.lpath", "formfilesplit.caption.lsplitsize",
        "formfilesplit.caption.btnok", "formfilejoin.caption.btnok",
        "formfilejoin.caption.btnadd", "formfilejoin.caption.chkistext",
        "formfilejoin.caption.lfiles", "res.wfgoto", "res.wfset",
        "ndr.ui_drive_kind_removable", "ndr.ui_drive_kind_fixed",
        "ndr.ui_drive_kind_network", "ndr.ui_drive_kind_ram",
        "ndr.ui_drive_kind_unknown", "ndr.ui_drive_kind_noroot",
        "formmain.menu.miarcmove", "formmain.menu.micleanupdisk",
        "formmain.menu.midiskcopy", "formmain.menu.mifiltercustom",
        "formmain.menu.miarcextracttofolder",
    ];
    for &(locale, _) in ndr::compat::BUILTIN_LANGUAGES {
        let pack = Language::load(catalogs(), locale);
        for key in keys {
            let value = pack.primary_raw(key).unwrap_or_else(|| panic!("{locale}: {key}"));
            assert!(!value.trim().is_empty(), "{locale}: {key}");
        }
        for key in ["res.wfgoto", "res.wfset"] {
            let template = pack.primary_raw(key).unwrap();
            assert_eq!(template.matches("%s").count(), 1, "{locale}: {key}");
            let value = substitute(template, &["3"]);
            assert!(value.contains('3') && !value.contains("%s"), "{locale}: {value}");
        }
    }
}

#[test]
fn all_security_vault_search_sftp_and_compatibility_keys_are_present() {
    let english = Language::load(catalogs(), "en");
    let source = include_str!("../Resources/Languages/en.lang");
    let groups = ["NDRVault", "NDRSearch", "NDRSFTPHostKey", "NDRCompatibility", "NDRCopyHandler"];
    let mut section = "";
    let mut keys = Vec::new();
    let mut sec_count = 0;
    let mut vault_count = 0;
    for line in source.lines() {
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name;
        } else if let Some((key, _)) = line.split_once('=') {
            if groups.contains(&section) || (section == "NDR" && key.starts_with("SEC_")) {
                keys.push(format!("{section}.{key}"));
                sec_count += usize::from(section == "NDR");
                vault_count += usize::from(section == "NDRVault");
            }
        }
    }
    assert_eq!(sec_count, 95);
    assert_eq!(vault_count, 81);
    assert_eq!(keys.len(), 193);
    let tokens = regex::Regex::new(r"\{(?:[0-9]+|[A-Za-z_][A-Za-z_0-9]*)\}|%[sd]|\\[rn]|</?[bc](?:[A-Fa-f0-9]{6})?>|\*\.[A-Za-z0-9*]+").unwrap();
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let pack = Language::load(catalogs(), code);
        for key in &keys {
            let base = english.primary_raw(key).unwrap();
            let local = pack.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
            assert!(!local.trim().is_empty() && !local.contains('\u{fffd}'), "{code}: invalid {key}");
            assert_eq!(tokens.find_iter(local).map(|m| m.as_str()).collect::<Vec<_>>(),
                tokens.find_iter(base).map(|m| m.as_str()).collect::<Vec<_>>(), "{code}: format mismatch {key}");
            if code != "en" && key.starts_with("NDR.SEC_") {
                assert_ne!(local, base, "{code}: untranslated security message {key}");
            }
        }
    }
}

#[test]
fn completed_operation_and_remote_groups_are_present_and_preserve_format_tokens() {
    let english = Language::load(catalogs(), "en");
    let source = include_str!("../Resources/Languages/en.lang");
    let prefixes = ["OP_DISK_", "UI_TOOLS_", "OP_DIAG_", "OP_ARCHIVE_", "NET_", "OP_ATTRIBUTES_", "OP_PANE_", "OP_SHORTCUTS_", "OP_SPLIT_"];
    let keys: Vec<_> = source.lines().filter_map(|line| line.split_once('='))
        .filter(|(key, _)| prefixes.iter().any(|prefix| key.starts_with(prefix)))
        .map(|(key, _)| format!("NDR.{key}")).collect();
    assert!(keys.len() >= 350);
    let tokens = regex::Regex::new(r"\{(?:[0-9]+|[A-Za-z_][A-Za-z_0-9]*)\}|%[sd]|</?[bc](?:[A-Fa-f0-9]{6})?>|\*\.[A-Za-z0-9*]+").unwrap();
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let lang = Language::load(catalogs(), code);
        for key in &keys {
            let base = english.primary_raw(key).unwrap();
            let selected = lang.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
            assert!(!selected.trim().is_empty() && !selected.contains('\u{fffd}') && !selected.contains("???"), "{code}: invalid {key}");
            let mut expected: Vec<_> = tokens.find_iter(base).map(|m| m.as_str()).collect();
            let mut actual: Vec<_> = tokens.find_iter(selected).map(|m| m.as_str()).collect();
            expected.sort_unstable(); actual.sort_unstable();
            assert_eq!(actual, expected, "{code}: format mismatch {key}");
        }
    }
    let ko = Language::load(catalogs(), "ko");
    assert_eq!(ko.primary_raw("NDR.SEC_CHOOSE_A_DESTINATION_INSIDE_THE_ENCRYPTED_FOLDER"), Some("암호화 폴더 안의 대상 위치를 선택하세요."));
    let it = Language::load(catalogs(), "it");
    assert_eq!(it.text("formFTPQuickConnect.Caption.lPassword", "missing"), "Password");
}

#[test]
fn every_locale_has_primary_dynamic_error_and_menu_captions() {
    let errors = [
        "7ZDLL_LOAD_FAILED", "CANNOT_CREATEFILE", "CANNOT_FIND_NEXTVOLUME",
        "CANNOT_OPEN_FILE", "CANNOT_READFILE", "WRITE_FAILED",
        "RAR_INVALIDFILE_OR_CORRUPTED", "UNSUPPORTED_ARCHIVE_FILE",
        "FTP_PUTCRITERIA", "FTPENGINE_BUSY", "FTPSITE_EXISTS", "FTPSITE_INVALIDINFO",
        "SHORTCUT_ALREADY_ASSIGNED", "SHORTCUT_PREVENTED", "TITLE_EDITPARAM", "MSG_EDITPARAM",
    ];
    let menus = [
        "miAddRemoveProg", "miAdvDeselect", "miAdvSelect", "miArcAdd",
        "miArcExtractAuto", "miArcExtractToCurr", "miClearDescriptions", "miCommandLine",
        "miCompareFolder", "miControlPanel", "miCopyFilename", "miCopyFullPath",
        "miCopyPath", "miEmptyRecycleBin", "miEmptyTemp", "miFind", "miFTPQuickConnect",
        "miHelp", "miHomePage", "miNetwork", "miPasteAsShortcut", "miRestoreLastSel",
        "miSelectSameExt", "miSelectSameName", "miShowDriveButtons", "miShowFolders",
        "miTextEditor", "miTextViewer", "miToLastDest", "miToTemp", "miViewButtonBar",
        "miViewFileInfoBar", "miViewFindResult", "miViewFolderInfoBar", "miViewVertBar",
    ];
    let english = Language::load(catalogs(), "en");
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let lang = Language::load(catalogs(), code);
        for key in errors.iter().map(|key| format!("RES.{key}")) {
            let selected = lang.primary_raw(&key).unwrap_or_else(|| panic!("{code}: {key}"));
            assert!(!selected.trim().is_empty(), "{code}: {key}");
            if code != "en" { assert_ne!(Some(selected), english.primary_raw(&key), "{code}: {key}"); }
        }
        for key in menus.iter().map(|key| format!("Menu.{key}")) {
            let selected = lang.primary_raw(&key).unwrap_or_else(|| panic!("{code}: {key}"));
            let base = english.primary_raw(&key).unwrap();
            assert_eq!(ndr::msgbox::display_caption(selected).1, ndr::msgbox::display_caption(base).1, "{code}: {key}");
        }
    }
}

#[test]
fn every_locale_supplies_folder_comparison_and_synchronization_catalog() {
    let english = Language::load(catalogs(), "en");
    let source = include_str!("../Resources/Languages/en.lang");
    let keys: Vec<_> = source
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| key.starts_with("UI_COMPARE_"))
        .map(|(key, _)| format!("NDR.{key}"))
        .collect();
    assert_eq!(keys.len(), 94);
    let tokens = regex::Regex::new(r"\{(?:[0-9]+|[A-Za-z_][A-Za-z_0-9]*)\}|\\[rn]|\*\.[A-Za-z0-9*]+").unwrap();
    let mut identical = Vec::new();
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let pack = Language::load(catalogs(), code);
        for key in &keys {
            let base = english.primary_raw(key).unwrap();
            let text = pack.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
            assert!(!text.trim().is_empty() && !text.contains('\u{fffd}'), "{code}: invalid {key}");
            assert_eq!(pack.raw(key).as_deref(), Some(text), "{code}: fallback in {key}");
            assert_eq!(
                tokens.find_iter(text).map(|m| m.as_str()).collect::<Vec<_>>(),
                tokens.find_iter(base).map(|m| m.as_str()).collect::<Vec<_>>(),
                "{code}: format tokens changed in {key}"
            );
            if code != "en" && text == base {
                identical.push((code, key.as_str()));
            }
        }
    }
    // Legitimate shared spellings, not missing translations.
    for (code, key) in identical {
        let accepted = match key {
            "NDR.UI_COMPARE_COLUMN_STATUS" => matches!(code, "af" | "da" | "de" | "id" | "ms" | "nb" | "nl" | "pt-BR" | "sv"),
            "NDR.UI_COMPARE_COLUMN_TYPE" => matches!(code, "da" | "fr" | "nb" | "nl"),
            "NDR.UI_COMPARE_TYPE_FILE" => matches!(code, "fil" | "it"),
            "NDR.UI_COMPARE_ERROR_COUNT" => code == "ca",
            _ => false,
        };
        assert!(accepted, "{code}: unexpected English text in {key}");
    }
    let ko = Language::load(catalogs(), "ko");
    assert_eq!(ko.primary_raw("NDR.UI_COMPARE_LOCAL_ONLY"), Some("폴더 비교는 일반 폴더에서만 사용할 수 있습니다."));
    assert_eq!(ko.primary_raw("NDR.UI_COMPARE_SYNC_LOCAL_ONLY"), Some("폴더 동기화는 일반 폴더에서만 사용할 수 있습니다."));
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
    assert_eq!(updater_keys.len(), 42);
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
fn all_catalogs_cover_explicit_ndr_settings_labels_and_format_tokens() {
    let source = include_str!("../src/ui/settings_window.rs");
    let keys: std::collections::BTreeSet<_> = source.split('"')
        .filter(|candidate| candidate.starts_with("NDR.UI_SETTINGS_")
            && candidate.bytes().all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || matches!(ch, b'.' | b'_')))
        .collect();
    assert!(keys.len() >= 86);
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let primary = Language::load(catalogs(), code);
        for key in &keys {
            assert!(primary.primary_raw(key).is_some_and(|value| !value.trim().is_empty() && !value.contains('\u{fffd}')), "{code}: {key}");
        }
        for (key, token) in [
            ("COLOR_FILTER_COL", "*.col"),
            ("COLOR_FILTER_JSON", "*.json"),
            ("COLOR_FILTER_ALL", "*.*"),
            ("COLOR_IMPORT_ERROR", "{0}"),
            ("COLOR_EXPORT_ERROR", "{0}"),
            ("UPDATE_JSON_FEED_HTTPS_C1FD01", "HTTPS"),
            ("EXTENSION_TEXT_COLORS_ZIP_B1DE31", ".zip=#F59E0B"),
            ("EXTENSION_TEXT_COLORS_ZIP_B1DE31", ".txt=#60A5FA"),
        ] {
            let value = primary.text(&format!("NDR.UI_SETTINGS_{key}"), "");
            assert_eq!(value.matches(token).count(), 1, "{code}: {key}: {value:?}");
        }
    }
}

#[test]
fn batch_rename_catalogs_keep_six_action_slots_and_confirmation_layout() {
    let english = Language::load(catalogs(), "en");
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let pack = Language::load(catalogs(), code);
        let raw = |key| pack.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
        let actions = "RES.ADVRENAME_ACTIONS";
        let action_text = raw(actions);
        let labels: Vec<_> = action_text.split(';').map(str::trim).collect();
        assert_eq!(labels.len(), 6, "{code}: {labels:?}");
        assert!(labels.iter().all(|text| !text.is_empty()), "{code}: {labels:?}");
        if code != "en" {
            assert_ne!(Some(action_text), english.primary_raw(actions), "{code}");
        }
        let confirm = raw("NDR.OP_RENAME_CONFIRM");
        assert!(confirm.starts_with("{0}\n\n"), "{code}: {confirm:?}");
        assert_eq!(confirm.matches("{0}").count(), 1, "{code}: {confirm:?}");
        let invalid = raw("NDR.OP_RENAME_INVALID_RESULT");
        for slot in ["{0}", "{1}"] {
            assert_eq!(invalid.matches(slot).count(), 1, "{code}: {invalid:?}");
        }
        let limits = raw("formAdvRename.Caption.numericRangeError");
        for number in ["255", "999999", "-999999", "250"] {
            assert!(limits.contains(number), "{code}: {limits:?}");
        }
    }
}

#[test]
fn every_locale_supplies_native_transfer_feedback_with_required_arguments() {
    let english = Language::load(catalogs(), "en");
    let keyed = [
        ("NDR.OP_TRANSFER_COPY_DONE", 0),
        ("NDR.OP_TRANSFER_MOVE_DONE", 0),
        ("NDR.OP_TRANSFER_DELETE_DONE", 0),
        ("NDR.OP_TRANSFER_DONE_DETAIL", 2),
        ("NDR.OP_TRANSFER_DONE_SUMMARY", 2),
        ("NDR.OP_TRANSFER_FAILED_SUMMARY", 4),
        ("NDR.UI_PANE_FOLDER_NOT_FOUND", 1),
        ("NDR.OP_JOB_FINISH_FIRST", 0),
        ("NDR.OP_JOB_PATH_BUSY", 0),
        ("NDR.OP_JOB_LIMIT", 0),
        ("formFolderTree.Menu.miShowHiddenFolder", 0),
        ("formFolderTree.Menu.miSubRefresh", 0),
        ("formFolderTree.Menu.miTreeRefresh", 0),
    ];
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let language = Language::load(catalogs(), code);
        for (key, required) in keyed {
            let text = language.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
            if code != "en" {
                assert_ne!(Some(text), english.primary_raw(key), "{code}: {key} is English");
            }
            assert!(!text.contains('\u{fffd}'), "{code}: {key} contains broken Unicode");
            for number in 0..required {
                assert_eq!(text.matches(&format!("{{{number}}}")).count(), 1, "{code}: {key}");
            }
        }
    }
}

#[test]
fn every_locale_uses_its_own_destructive_action_and_update_messages() {
    let english = Language::load(catalogs(), "en");
    let keys: &[(&str, &[(&str, usize)])] = &[
        ("RES.MSG_DELWARNING1", &[("%s", 2), ("\n", 2), ("<b>", 1), ("</b>", 1), ("<c0000FF>", 1), ("</c>", 1)]),
        ("RES.MSG_DELWARNING2", &[("%s", 1), ("%d", 1), ("\n", 2), ("<c0000FF>", 1), ("</c>", 1)]),
        ("RES.MSG_DELWARN_RECYCLEBIN", &[("<b>", 1), ("</b>", 1)]),
        ("RES.MSG_DELWARN_DELETE", &[("<b>", 1), ("</b>", 1)]),
        ("RES.MSG_DELWARN_SHRED", &[("<b>", 1), ("</b>", 1), ("<cFF0000>", 1), ("</c>", 1)]),
        ("NDR.OP_DISK_RESTORING_OVERWRITES_THE_EXISTING_DATA_ON_THE_TARGET_VOLUME", &[]),
        ("NDR.OP_DISK_THIS_OVERWRITES_ALL_EXISTING_DATA_ON_THE_DESTINATION_DRIVE", &[]),
        ("NDR.OP_DISK_THIS_OVERWRITES_THE_TARGET_DISK_S_PARTITION_TABLE_AND", &[]),
        ("NDR.OP_DEST_MOVE_VAULT_UNSUPPORTED", &[]),
        ("NDR.UI_VAULT_REGULAR_MOVE", &[]),
        ("RES.MSG_WANTTOCONTINUE", &[("%s", 1), ("\n", 1)]),
        ("RES.MSG_WANTTOCONTINUE2", &[]),
        ("NDR.OP_TRANSFER_SOME_ITEMS_COULD_NOT_BE_SHREDDED", &[]),
        ("NDR.UI_VAULT_COPY_COMBINATION", &[]),
        ("NDR.UI_VAULT_COMPACT_MENU", &[]),
        ("NDR.OP_DISK_CONFIRM_PHYSICAL", &[("{0}", 1), ("{1}", 1), ("\n", 2)]),
        ("NDR.OP_DISK_CONFIRM_VOLUME", &[("{0}", 1), ("{1}", 1), ("\n", 2)]),
        ("NDR.OP_TRANSFER_DELETE_SUMMARY", &[("{0}", 1), ("{1}", 1), ("{2}", 1)]),
        ("NDR.OP_TRANSFER_RECYCLE_SUMMARY", &[("{0}", 1), ("{1}", 1), ("{2}", 1)]),
        ("NDR.OP_TRANSFER_ITEMS_THAT_COULD_NOT_BE_DELETED", &[]),
        ("NDR.OP_TRANSFER_DELETE_STOPPED", &[("{0}", 1), ("{1}", 1)]),
        ("NDR.OP_TRANSFER_DELETE_SKIPPED", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_THE_MULTI_VOLUME_ARCHIVE_UPDATE_WAS_CANCELLED_THE_EXISTING", &[]),
        ("NDR.OP_ARCHIVE_THE_0_FORMAT_CANNOT_BE_SPLIT_INTO_VOLUMES", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_THE_0_FORMAT_CANNOT_BE_ENCRYPTED", &[("{0}", 1)]),
        ("NDR.OP_TRANSFER_ERRORS_TITLE", &[("{0}", 1), ("{1}", 1)]),
        ("NDR.OP_TRANSFER_EXTERNAL_FILE_OPERATION_FAILED", &[]),
        ("NDR.OP_TRANSFER_FAILED_SUMMARY", &[("{0}", 1), ("{1}", 1), ("{2}", 1), ("{3}", 1)]),
        ("NDR.OP_ARCHIVE_ITEMS_CANNOT_BE_ADDED_TO_AN_INCOMPLETE_ARCHIVE_SET", &[]),
        ("NDR.OP_ARCHIVE_THE_ARCHIVE_CANNOT_BE_SAVED_INSIDE_A_FOLDER_BEING", &[]),
        ("NDR.OP_ARCHIVE_A_VOLUME_SIZE_CANNOT_BE_USED_WHEN_ADDING_TO", &[]),
        ("NDR.OP_ARCHIVE_A_NEW_ARCHIVE_CANNOT_BE_CREATED_INSIDE_AN_ARCHIVE", &[]),
        ("NDR.OP_ARCHIVE_CANNOT_OPEN_FORMAT", &[("{0}", 1)]),
        ("RES.RECYCLEBIN_HINT", &[("Shift", 2), ("Ctrl+Shift", 1), ("Ctrl+Alt", 1)]),
        ("RES.WORKFOLDER_SETHINT", &[]),
        ("RES.WORKFOLDER_UNSETHINT", &[]),
        ("NDR.UI_TOOLS_TOOLBAR_CREATE_ZIP", &[("ZIP", 1)]),
        ("NDR.UI_MORE_LANGUAGES_HELP", &[]),
        ("NDR.UI_OPERATION_RUNNING", &[]),
        ("RES.TITLE_DELDESCFILE", &[]),
        ("RES.MSG_DELDESCFILE", &[]),
        ("NDR.UI_CLEAR_COMMENT_LIMIT", &[]),
        ("NDR.NET_LOCAL_CONFLICT_PROMPT", &[("{count}", 1)]),
        ("RES.MSG_TOLEFT", &[("%s", 1)]),
        ("RES.MSG_TORIGHT", &[("%s", 1)]),
        ("RES.TITLE_FILEOPERATION", &[]),
        ("RES.BACKHINT", &[("%s", 1)]),
        ("RES.FORWARDHINT", &[("%s", 1)]),
        ("NDR.UI_UNKNOWN_COMMAND", &[("{0}", 1)]),
        ("RES.FILESPLIT", &[]),
        ("RES.SPLITTING", &[]),
        ("RES.JOINING", &[]),
        ("RES.SPLITTED_INFO", &[("%d", 1)]),
        ("NDR.OP_SPLIT_CREATED_PARTS", &[("{0}", 1)]),
        ("NDR.OP_SPLIT_FILE_SPLIT_COMPLETED", &[]),
        ("NDR.OP_SPLIT_FILE_SPLIT_CANCELED", &[]),
        ("NDR.OP_SPLIT_FILE_JOIN_COMPLETED", &[]),
        ("NDR.OP_SPLIT_FILE_JOIN_CANCELED", &[]),
        ("NDR.OP_SPLIT_THE_OUTPUT_FOLDER_DOES_NOT_EXIST", &[]),
        ("NDR.OP_JOIN_CREATED_FILES", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_WRONG_FORMAT", &[("{0}", 1), (".{1}", 1)]),
        ("NDR.OP_ARCHIVE_SELECT_THE_FILES_OR_FOLDERS_TO_ARCHIVE", &[]),
        ("NDR.OP_ARCHIVE_SELECT_AN_ARCHIVE_OR_OPEN_ONE_FIRST", &[]),
        ("NDR.OP_ARCHIVE_UNKNOWN", &[]),
        ("NDR.OP_ARCHIVE_READ_ONLY", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_THE_ARCHIVE_PATH_IS_THE_SAME_AS_A_SOURCE", &[]),
        ("NDR.OP_ARCHIVE_PROP_FILE", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_PROP_FILES", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_PROP_FOLDERS", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_PROP_FORMAT", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_PROP_MODIFIED", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_PROP_SIZE", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_PROP_UNPACKED", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_OPEN_OR_SELECT_AN_ARCHIVE_TO_SEE_ITS_PROPERTIES", &[]),
        ("NDR.OP_ARCHIVE_UNSUPPORTED_FORMAT", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_REPLACE_SET", &[("{0}", 1), ("\n\n{1}", 1)]),
        ("NDR.OP_ARCHIVE_REBUILD_PASSWORD", &[("{0}\n", 1)]),
        ("NDR.OP_ARCHIVE_REBUILT_EMPTY", &[]),
        ("NDR.OP_ARCHIVE_STAGE_CLEANUP_FAILED", &[("{0}", 1)]),
        ("NDR.OP_ARCHIVE_THE_ARCHIVE_WAS_CREATED_BUT_DELETING_THE_ORIGINALS_STOPPED", &[("{0}", 1)]),
        ("NDR.NET_REMOTE_CONFLICT_PROMPT", &[("{count}", 1)]),
        ("RES.UPDATE_NOUPDATE", &[]),
        ("RES.UPDATE_NEWUPDATE", &[]),
    ];
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let language = Language::load(catalogs(), code);
        for &(key, tokens) in keys {
            let text = language.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
            assert!(!text.trim().is_empty(), "{code}: empty {key}");
            assert!(!text.contains('\u{fffd}'), "{code}: invalid Unicode in {key}");
            if code != "en" {
                assert_ne!(Some(text), english.primary_raw(key), "{code}: {key} is English");
            }
            for &(token, count) in tokens {
                assert_eq!(text.matches(token).count(), count, "{code}: invalid {token} in {key}");
            }
        }
    }
}


#[test]
fn file_menu_labels_are_localized_without_changing_access_keys() {
    let english = Language::load(catalogs(), "en");
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let pack = Language::load(catalogs(), code);
        for (key, mnemonic, dialog) in [
            ("Menu.miAdvRename", "&A. ", true),
            ("Menu.miJoinFile", "&A. ", true),
            ("Menu.miRecycleBin", "&R. ", false),
            ("Menu.miSaveList", "&Z. ", true),
            ("Menu.miSplitFile", "&S. ", true),
        ] {
            let value = pack.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
            assert!(value.starts_with(mnemonic), "{code}: access key changed: {key}");
            assert_eq!(value.ends_with('\u{2026}'), dialog, "{code}: wrong dialog marker in {key}");
            assert!(!value.contains('\u{fffd}'), "{code}: broken character in {key}");
            assert_eq!(pack.raw(key).as_deref(), Some(value), "{code}: menu used a fallback for {key}");
            if code != "en" {
                assert_ne!(Some(value), english.primary_raw(key), "{code}: untranslated {key}");
            }
        }
        for key in ["Menu.miArcExtract", "ToolButton.tbNewUpdateAvailable", "NDR.UI_TOOLS_TOOLBAR_COMMAND_UNAVAILABLE"] {
            let value = pack.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
            assert!(!value.trim().is_empty() && !value.contains('\u{fffd}'), "{code}: invalid value in {key}");
            if code != "en" {
                assert_ne!(Some(value), english.primary_raw(key), "{code}: untranslated {key}");
            }
        }
    }
}


#[test]
fn file_list_export_captions_have_primary_language_text() {
    let english = Language::load(catalogs(), "en");
    let keys = [
        "formsavelist.form.caption",
        "formsavelist.groupbox.gbcolumns",
        "formsavelist.radiogroup.rgdelimiter",
        "formsavelist.radiogroupitems.rgdelimiter",
        "formsavelist.caption.lhelp",
        "formsavelist.caption.chkattributes",
        "formsavelist.caption.chkdate",
        "formsavelist.caption.chkdescription",
        "formsavelist.caption.chkextension",
        "formsavelist.caption.chkfilename",
        "formsavelist.caption.chkfilesize",
        "formsavelist.caption.chksizecomma",
        "formsavelist.caption.chktime",
        "formsavelist.caption.btnsave",
        "formsavelist.caption.btncancel",
    ];
    for &(code, _) in ndr::compat::BUILTIN_LANGUAGES {
        let pack = Language::load(catalogs(), code);
        for (index, key) in keys.iter().enumerate() {
            let value = pack.primary_raw(key).unwrap_or_else(|| panic!("{code}: missing {key}"));
            assert!(!value.trim().is_empty() && !value.contains('\u{fffd}'), "{code}: invalid {key}");
            if index < 5 && code != "en" {
                assert_ne!(Some(value), english.primary_raw(key), "{code}: untranslated {key}");
            }
            if index == 3 {
                let options = value.split(';').collect::<Vec<_>>();
                assert_eq!(options.len(), 3, "{code}: expected tab, comma and custom options");
                assert!(options.iter().all(|item| !item.trim().is_empty()), "{code}: empty separator label");
            }
        }
    }
}
