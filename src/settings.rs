use serde_json::{Map, Value};
use std::path::Path;

pub const CURRENT_SCHEMA_VERSION: i64 = 5;

/// Default file list (pane) font size (ListFontSize, pixels). User decision: 16 px (the original default\nexusfile.ini has "Segoe UI",9 = 12 px).
pub const DEFAULT_LIST_FONT_PIXELS: f64 = 16.;

/// Original font size (points, TFont.Size) to pixels: original FUN_00644e70 Height = MulDiv(size, PixelsPerInch 96, 72) (rounded).
pub fn font_points_to_pixels(points: f64) -> f64 {
    (points * 96. / 72.).round()
}

/// Pixel font size to points for the Windows font box (rounded to 0.5).
pub fn font_pixels_to_points(pixels: f64) -> f64 {
    (pixels * 72. / 96. * 2.).round() / 2.
}
#[derive(Clone, Debug)]
pub struct Settings {
    pub values: Map<String, Value>,
}
impl Default for Settings {
    fn default() -> Self {
        let mut settings = Self { values: Map::new() };
        settings.normalize();
        settings
    }
}
impl Settings {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let values = serde_json::from_slice(bytes)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let mut settings = Self { values };
        settings.normalize();
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut normalized = self.clone();
        normalized.normalize();
        let bytes =
            serde_json::to_vec_pretty(&normalized.values).map_err(|error| error.to_string())?;
        atomic_write(path, &bytes)
    }
    pub fn text(&self, key: &str, default: &str) -> String {
        self.values
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_owned()
    }
    pub fn flag(&self, key: &str, default: bool) -> bool {
        self.values
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(default)
    }
    pub fn set_text(&mut self, key: &str, value: impl Into<String>) {
        self.values
            .insert(key.to_owned(), Value::String(value.into()));
    }
    pub fn set_flag(&mut self, key: &str, value: bool) {
        self.values.insert(key.to_owned(), Value::Bool(value));
    }
    pub fn strings(&self, key: &str) -> Vec<String> {
        self.values
            .get(key)
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn set_strings(&mut self, key: &str, value: Vec<String>) {
        self.values.insert(key.to_owned(), serde_json::json!(value));
    }
    /// Original schema migrations and essential bounds, retaining unknown keys and dictionary contents.
    pub fn normalize(&mut self) {
        use serde_json::json;
        let original_schema = self.integer("SchemaVersion", 0);
        let mut path_click = self.integer("PathClickMode", 0);
        if path_click == 0 && self.flag("NavigateOnPathClick", false) {
            path_click = 1;
        }
        path_click = path_click.clamp(0, 2);
        self.values
            .insert("PathClickMode".into(), json!(path_click));
        self.set_flag("NavigateOnPathClick", path_click == 1);
        if self
            .values
            .get("ShowDeleteToolbarButton")
            .and_then(Value::as_bool)
            .is_none()
        {
            self.set_flag(
                "ShowDeleteToolbarButton",
                if original_schema < 2 {
                    self.flag("ShowDeleteButton", true)
                } else {
                    true
                },
            );
            if original_schema < 2 {
                self.set_flag("ShowDeleteButton", true);
            }
        }
        for (key, default) in [
            ("ShowDeleteButton", true),
            ("UseLogicalSort", true),
            ("LogicalSort", true),
            ("DualPaneMode", true),
            ("ConfirmDelete", true),
            ("ConfirmDragDrop", true),
            ("UseRecycleBin", true),
            ("UseDescriptionFile", true),
        ] {
            self.set_flag(key, self.flag(key, default));
        }
        if original_schema < 1 && self.flag("ShowHiddenFiles", false) {
            self.set_flag("LeftShowHiddenFiles", true);
            self.set_flag("RightShowHiddenFiles", true);
        }
        for key in [
            "LeftShowHiddenFiles",
            "RightShowHiddenFiles",
            "ShowLeftFolderTree",
            "ShowRightFolderTree",
        ] {
            self.set_flag(key, self.flag(key, false));
        }
        self.set_flag(
            "ShowHiddenFiles",
            self.flag("LeftShowHiddenFiles", false) || self.flag("RightShowHiddenFiles", false),
        );
        if self.flag("ShowFolderTree", false)
            && !self.flag("ShowLeftFolderTree", false)
            && !self.flag("ShowRightFolderTree", false)
        {
            self.set_flag("ShowLeftFolderTree", true);
            self.set_flag("ShowRightFolderTree", true);
        }
        self.set_flag(
            "ShowFolderTree",
            self.flag("ShowLeftFolderTree", false) || self.flag("ShowRightFolderTree", false),
        );
        if original_schema < 3
            && self
                .text("Theme", "")
                .eq_ignore_ascii_case("NexusFile Color Map")
        {
            self.set_flag("UseNexusFileColorMap", true);
            self.set_text("Theme", "Dark");
        }
        // Original default [Window] skin=black (default\nexusfile.ini): the initial theme is Black. The C# default Dark from before schema 4 also moves to Black once.
        if original_schema < 4 && self.text("Theme", "Dark").eq_ignore_ascii_case("Dark") {
            self.set_text("Theme", "Black");
        }
        // Themes are only the original skins (black, classic). Old C# themes (Dark, Light, System) or unknown names become the original default Black.
        let theme = self.text("Theme", "Black");
        self.set_text(
            "Theme",
            if matches!(theme.as_str(), "Black" | "Classic") {
                theme.as_str()
            } else {
                "Black"
            },
        );
        let language = self.text("Language", "ko");
        self.set_text(
            "Language",
            if language.trim().is_empty() {
                "ko"
            } else {
                &language
            },
        );
        for (key, default, min, max) in [
            ("FolderTreeDepth", 10, 1, 64),
            ("DefaultDeleteButton", 2, 0, 2),
            ("MaxRecentFileCount", 10, 0, 1000),
            ("DefaultDestinationMode", 0, 0, 1),
            ("DefaultArchiveFormatIndex", 1, 0, 128),
            ("DefaultArchiveCompressionLevel", 2, 0, 9),
            // Original TNXScrollStep (default 2 at 0x46f20f): 1 = line, 2 = page.
            ("ScrollStep", 2, 1, 2),
            ("LeftScrollStep", 2, 1, 2),
            ("RightScrollStep", 2, 1, 2),
            ("ThumbnailSize", 120, 32, 512),
            ("LeftColumnCount", 0, 0, 19),
            ("RightColumnCount", 0, 0, 19),
            ("CharacterCase", 0, 0, 3),
            ("ListLineSpace", 0, 0, 32),
            ("WindowAlpha", 210, 0, 255),
        ] {
            let value = self.integer(key, default).clamp(min, max);
            self.values.insert(key.into(), json!(value));
        }
        if !self
            .values
            .get("DefaultArchiveCompressionPresetIndex")
            .is_none_or(Value::is_null)
        {
            let value = self
                .integer("DefaultArchiveCompressionPresetIndex", 1)
                .clamp(0, 2);
            self.values
                .insert("DefaultArchiveCompressionPresetIndex".into(), json!(value));
        }
        if let Some(value) = self.values.get("EscapeKeyAction")
            && !value.is_null()
            && !(0..=4).contains(&self.integer("EscapeKeyAction", -1))
        {
            self.values.insert("EscapeKeyAction".into(), Value::Null);
        }
        // Original default (default/nexusfile.ini esckey=0, constructor +0xd8 = 0): Esc = minimize. 0 when never chosen.
        if !self.values.contains_key("EscapeKeyAction") {
            self.values.insert("EscapeKeyAction".into(), json!(0));
        }
        for (key, default, min, max) in [
            // Original pane folder tree width (ini treewidth1/2 default 120, divider MinSize 50, FUN_0048da38).
            ("LeftFolderTreeWidth", 120., 50., 600.),
            ("RightFolderTreeWidth", 120., 50., 600.),
            ("LeftPaneWidth", 50., 20., 80.),
            ("RightPaneWidth", 50., 20., 80.),
            // Original TfMain Constraints.MinWidth/MinHeight 250.
            ("WindowWidth", 800., 250., 8192.),
            ("WindowHeight", 600., 250., 8192.),
            ("WindowLeft", 10., -16384., 16384.),
            ("WindowTop", 10., -16384., 16384.),
            ("TreeWindowLeft", 10., -16384., 16384.),
            ("TreeWindowTop", 10., -16384., 16384.),
            ("TreeWindowWidth", 800., 400., 8192.),
            ("TreeWindowHeight", 600., 300., 8192.),
            ("FolderTreeFontSize", 9., 8., 32.),
            ("FolderTreeItemWidth", 130., 32., 4096.),
            ("ListFontSize", DEFAULT_LIST_FONT_PIXELS, 8., 32.),
        ] {
            let value = self
                .values
                .get(key)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .unwrap_or(default)
                .clamp(min, max);
            self.values.insert(key.into(), json!(value));
        }
        for key in [
            "Favorites",
            "FavoriteNames",
            "LeftTabs",
            "RightTabs",
            "CommandHistory",
            "ExecutableHistory",
            "RecentRemoteHosts",
            "FolderTreeExcludedPaths",
            "LeftFolderTreeExpandedPaths",
            "RightFolderTreeExpandedPaths",
            "SavedSearchLocations",
            "SavedSearchPatterns",
        ] {
            let values = self.strings(key);
            self.set_strings(key, values);
        }
        let working = self.values.get("WorkingFolders").and_then(Value::as_array);
        let working = (0..5)
            .map(|index| {
                working
                    .and_then(|v| v.get(index))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned()
            })
            .collect();
        self.set_strings("WorkingFolders", working);
        for key in [
            "RemoteSites",
            "AdvancedRenameMacros",
            "MostUsedDestinations",
        ] {
            let values = self
                .values
                .get(key)
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter(|v| v.is_object())
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            self.values.insert(key.into(), json!(values));
        }
        for key in [
            "ExternalPrograms",
            "ExternalArchiveArgumentTemplates",
            "ExternalArchiveExtractArgumentTemplates",
            "FunctionKeys",
            "FunctionKeyBindings",
            "KeyboardShortcuts",
            "LegacyIniValues",
            "SpecialFolderPaths",
            "LastFoldersByDrive",
            "ExtensionColors",
            "NexusFileColorMapValues",
        ] {
            if !self.values.get(key).is_some_and(Value::is_object) {
                self.values.insert(key.into(), json!({}));
            }
        }
        // Original default color table (default\nexusfile.col [Ext]): filled once before schema 4 when there are no extension colors at all.
        if original_schema < 4 && self.values.get("ExtensionColors").and_then(Value::as_object).is_some_and(|m| m.is_empty()) {
            let colors: Map<String, Value> = crate::listing::ORIGINAL_EXTENSION_COLORS
                .iter()
                .flat_map(|(color, extensions)| extensions.iter().map(move |e| (format!(".{e}"), json!(color))))
                .collect();
            self.values.insert("ExtensionColors".into(), Value::Object(colors));
        }
        for (key, default) in [
            ("LastDestination", ""),
            ("LeftStartPath", ""),
            ("RightStartPath", ""),
            ("OpenWithParameters", ""),
            ("ExplorerShortcutKey", "Q"),
        ] {
            if !self.values.get(key).is_some_and(Value::is_string) {
                self.set_text(key, default);
            }
        }
        for key in ["ListFont", "FolderTreeFont"] {
            let font = self.text(key, "Segoe UI");
            self.set_text(
                key,
                if font.trim().is_empty() {
                    "Segoe UI"
                } else {
                    font.trim()
                },
            );
        }
        for key in ["LeftGroupMode", "RightGroupMode"] {
            let mode = self.text(key, "None");
            self.set_text(
                key,
                if matches!(
                    mode.as_str(),
                    "None" | "Extension" | "Size" | "Time" | "Category"
                ) {
                    mode.as_str()
                } else {
                    "None"
                },
            );
        }
        for key in ["LeftFolderSortMode", "RightFolderSortMode"] {
            let mut mode = self.text(key, "Auto");
            // Schema 5: folder sort "Auto" (original foldersort 0, see listing::effective_folder_sort) became the default. Earlier settings stored
            // the old default "Name" on every save (the original has no folder sort option in its menus or preferences), so it is reset once.
            if original_schema < 5 && mode == "Name" {
                mode = "Auto".into();
            }
            self.set_text(
                key,
                if matches!(
                    mode.as_str(),
                    "Auto" | "None" | "Name" | "Extension" | "Size" | "Modified" | "Attributes" | "Color"
                ) {
                    mode.as_str()
                } else {
                    "Auto"
                },
            );
        }
        for key in ["LeftFolderCategoryMode", "RightFolderCategoryMode"] {
            let mode = self.text(key, "Auto");
            self.set_text(
                key,
                if matches!(mode.as_str(), "Default" | "Normal") {
                    "Default"
                } else {
                    "Auto"
                },
            );
        }
        for key in ["SavedSearchPatterns", "RecentRemoteHosts"] {
            let mut seen = std::collections::HashSet::new();
            let values = self
                .strings(key)
                .into_iter()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty() && seen.insert(s.to_lowercase()))
                .take(20)
                .collect();
            self.set_strings(key, values);
        }
        let favorites = self.strings("Favorites");
        let mut names = self.strings("FavoriteNames");
        if names.len() != favorites.len() {
            names = (0..favorites.len())
                .map(|index| {
                    names.get(index).cloned().unwrap_or_else(|| {
                        Path::new(favorites[index].trim_end_matches(['\\', '/']))
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    })
                })
                .collect();
            self.set_strings("FavoriteNames", names);
        }
        self.values
            .insert("SchemaVersion".into(), json!(CURRENT_SCHEMA_VERSION));
    }
    fn integer(&self, key: &str, default: i64) -> i64 {
        self.values
            .get(key)
            .and_then(|value| {
                value.as_i64().or_else(|| {
                    value
                        .as_f64()
                        .filter(|v| {
                            v.is_finite()
                                && v.fract() == 0.
                                && *v >= i64::MIN as f64
                                && *v <= i64::MAX as f64
                        })
                        .map(|v| v as i64)
                })
            })
            .unwrap_or(default)
    }
}

/// Commit through a unique same-directory file. A failed replacement leaves the old file intact.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::{
        fs::OpenOptions,
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temporary;
    let mut file;
    loop {
        temporary = parent.join(format!(
            ".ndr-save-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(opened) => {
                file = opened;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, path)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("{}: {error}", path.display()));
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_fields_survive_save_and_malformed_input_is_reported() {
        let root = std::env::temp_dir().join(format!("ndr-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        std::fs::write(
            &path,
            br#"{"FutureField":{"Nested":[1,true]},"LeftPath":"old"}"#,
        )
        .unwrap();
        let mut settings = Settings::load(&path).unwrap();
        settings.set_text("LeftPath", "한글");
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path).unwrap();
        assert_eq!(
            loaded.values["FutureField"],
            serde_json::json!({"Nested":[1,true]})
        );
        assert_eq!(loaded.text("LeftPath", ""), "한글");
        std::fs::write(&path, b"{bad").unwrap();
        assert!(Settings::load(&path).is_err());
        std::fs::write(&path, b"[]").unwrap();
        assert!(Settings::load(&path).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn failed_atomic_save_preserves_old_data_and_removes_temporary_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let root =
            std::env::temp_dir().join(format!("ndr-settings-locked-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        std::fs::write(&path, b"{\"Keep\":true}").unwrap();
        let guard = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let mut settings = Settings::default();
        settings.set_flag("Keep", false);
        assert!(settings.save(&path).is_err());
        drop(guard);
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"Keep\":true}");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn defaults_follow_the_original_default_ini_and_color_map() {
        // Original default\nexusfile.ini skin=black, default\nexusfile.col [Ext].
        let fresh = Settings::default();
        assert_eq!(fresh.text("Theme", ""), "Black");
        assert_eq!(fresh.values["ExtensionColors"][".zip"], "#808000");
        assert_eq!(fresh.values["ExtensionColors"][".jpg"], "#D857EA");
        assert_eq!(fresh.values["ExtensionColors"].as_object().unwrap().len(), 85);
        // Default pane (file list) font 16 px. Original points (MulDiv(size, 96, 72)): 12pt = 16px, 9pt = 12px.
        assert_eq!(fresh.values["ListFontSize"].as_f64(), Some(16.0));
        assert_eq!((font_points_to_pixels(12.), font_points_to_pixels(9.), font_points_to_pixels(10.5)), (16., 12., 14.));
        assert_eq!((font_pixels_to_points(14.), font_pixels_to_points(16.)), (10.5, 12.));
        // The C# default Dark from before schema 4 becomes Black once; extension colors the user chose stay.
        let mut old = Settings { values: serde_json::from_value(serde_json::json!({"SchemaVersion": 3, "Theme": "Dark", "ExtensionColors": {".txt": "#112233"}})).unwrap() };
        old.normalize();
        assert_eq!(old.text("Theme", ""), "Black");
        assert_eq!(old.values["ExtensionColors"], serde_json::json!({".txt": "#112233"}));
        // Themes are only original skins: a previously chosen C# theme Dark becomes Black too. Emptied extension colors stay empty.
        let mut chosen = Settings { values: serde_json::from_value(serde_json::json!({"SchemaVersion": 4, "Theme": "Dark", "ExtensionColors": {}})).unwrap() };
        chosen.normalize();
        assert_eq!(chosen.text("Theme", ""), "Black");
        assert!(chosen.values["ExtensionColors"].as_object().unwrap().is_empty());
    }
    #[test]
    fn schema_migrations_preserve_original_toolbar_hidden_tree_and_theme_behavior() {
        let mut settings=Settings{values:serde_json::from_value(serde_json::json!({"SchemaVersion":0,"ShowDeleteButton":false,"ShowHiddenFiles":true,"ShowFolderTree":true,"NavigateOnPathClick":true,"Theme":"NexusFile Color Map","UseLogicalSort":false,"LogicalSort":true,"Future":{"Keep":[1,2]},"ExternalPrograms":{"rar":"C:\\Tools\\rar.exe"}})).unwrap()};
        settings.normalize();
        assert_eq!(settings.values["SchemaVersion"], CURRENT_SCHEMA_VERSION);
        assert_eq!(settings.text("LeftFolderSortMode", ""), "Auto", "original default foldersort=0");
        // Schema 5: the old stored default "Name" becomes "Auto" once; a choice saved after that stays.
        let mut old = Settings { values: serde_json::from_value(serde_json::json!({"SchemaVersion": 4, "LeftFolderSortMode": "Name", "RightFolderSortMode": "Size"})).unwrap() };
        old.normalize();
        assert_eq!((old.text("LeftFolderSortMode", ""), old.text("RightFolderSortMode", "")), ("Auto".to_owned(), "Size".to_owned()));
        old.set_text("LeftFolderSortMode", "Name");
        old.normalize();
        assert_eq!(old.text("LeftFolderSortMode", ""), "Name");
        assert!(!settings.flag("ShowDeleteToolbarButton", true));
        assert!(settings.flag("ShowDeleteButton", false));
        assert!(settings.flag("LeftShowHiddenFiles", false));
        assert!(settings.flag("RightShowHiddenFiles", false));
        assert!(settings.flag("ShowLeftFolderTree", false));
        assert!(settings.flag("ShowRightFolderTree", false));
        assert_eq!(settings.values["PathClickMode"], 1);
        assert_eq!(settings.values["EscapeKeyAction"], 0, "original default ini esckey=0 (minimize)");
        assert!(settings.flag("UseNexusFileColorMap", false));
        assert_eq!(settings.text("Theme", ""), "Black");
        assert!(!settings.flag("UseLogicalSort", true));
        assert!(settings.flag("LogicalSort", false));
        assert_eq!(settings.values["Future"]["Keep"], serde_json::json!([1, 2]));
        assert_eq!(
            settings.values["ExternalPrograms"]["rar"],
            r"C:\Tools\rar.exe"
        );
    }
    #[test]
    fn normalization_handles_wrong_shapes_and_bounds_without_losing_unknown_objects() {
        let mut settings=Settings{values:serde_json::from_value(serde_json::json!({"SchemaVersion":3,"ShowDeleteToolbarButton":false,"WindowWidth":1,"WindowHeight":99999,"FolderTreeDepth":0,"LeftColumnCount":99,"ThumbnailSize":4,"Language":" ","WorkingFolders":[null,7,"C:\\docs"],"Favorites":["C:\\first",null],"FavoriteNames":null,"ExternalPrograms":{"future":{"args":["keep"]}},"CommandHistory":{"bad":true},"RemoteSites":null,"WindowTop":{},"FunctionKeys":[]})).unwrap()};
        settings.normalize();
        assert_eq!(settings.values["WindowWidth"].as_f64(), Some(250.));
        assert_eq!(settings.values["WindowHeight"].as_f64(), Some(8192.));
        assert_eq!(settings.values["FolderTreeDepth"], 1);
        assert_eq!(settings.values["LeftColumnCount"], 19);
        assert_eq!(settings.values["ThumbnailSize"], 32);
        assert_eq!(
            settings.strings("WorkingFolders"),
            ["", "", r"C:\docs", "", ""]
        );
        assert_eq!(settings.strings("FavoriteNames"), ["first"]);
        assert_eq!(settings.text("Language", ""), "ko");
        assert!(settings.values["CommandHistory"].is_array());
        assert!(settings.values["RemoteSites"].is_array());
        assert!(settings.values["FunctionKeys"].is_object());
        assert_eq!(
            settings.values["ExternalPrograms"]["future"]["args"],
            serde_json::json!(["keep"])
        );
        let once = settings.values.clone();
        settings.normalize();
        assert_eq!(settings.values, once);
    }
}
