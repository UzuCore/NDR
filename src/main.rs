#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;

use std::path::PathBuf;

/// Copies a whole folder (moving an old settings folder over). Existing files are not touched.
fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[derive(Default, Clone)]
pub struct Options {
    profile: Option<PathBuf>,
    left: Option<PathBuf>,
    right: Option<PathBuf>,
    /// Original /dir2: (an existing path). The original opens the right pane there only in dual-pane view.
    dir2: Option<PathBuf>,
    /// Original /pane:single (false) / /pane:dual (true). Otherwise the setting (DualPaneMode).
    dual: Option<bool>,
    language: Option<String>,
    smoke_seconds: Option<u64>,
    report: Option<PathBuf>,
    /// For tests: during the (off-screen) startup check, opens a sample of this kind of original window and saves that screen as PNG (`--shot kind file`).
    shot: Option<(String, PathBuf)>,
    /// Start maximized: (restore outer x, y, width, height (egui points), whether the first frame finished). App maximizes after the first frame.
    maximize_restore: Option<((f32, f32, f32, f32), bool)>,
}
/// Settings file name. This is the only settings file.
const SETTINGS_FILE: &str = "settings.json";
/// Items moved from an old settings folder to the executable folder (the settings file moves last, so an interrupted move is redone on the next start).
const PROFILE_ITEMS: [&str; 4] = ["toolbar.xml", "advrename.save", "treedata", SETTINGS_FILE];

/// Whether files can be created in that folder (creates one and deletes it at once).
fn writable(directory: &std::path::Path) -> bool {
    let probe = directory.join(format!(".ndr-write-probe-{}", std::process::id()));
    let made = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).is_ok();
    made && std::fs::remove_file(&probe).is_ok()
}

/// Moves one file or folder (copy then delete the original across drives). Leaves it alone when the destination already exists.
fn move_item(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if to.exists() || std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        copy_tree(from, to)?;
        std::fs::remove_dir_all(from)
    } else {
        std::fs::copy(from, to)?;
        if std::fs::metadata(to)?.len() != std::fs::metadata(from)?.len() {
            return Err(std::io::Error::other(format!("copy of {} is incomplete", from.display())));
        }
        std::fs::remove_file(from)
    }
}

/// Settings folder = the executable folder (`beside`). The only settings file is settings.json in that folder.
/// When it does not exist yet, the settings (PROFILE_ITEMS) of the first old settings folder (`old` order, %LOCALAPPDATA%\NDR -> NDIR-Rust) that has a settings file
/// move to the executable folder. The old ones are deleted so settings do not remain in two places, and an empty old folder is removed too.
/// When the executable folder is not writable (e.g. Program Files), nothing moves and the old folder is used as is.
fn beside_profile(beside: &std::path::Path, old: &[PathBuf]) -> PathBuf {
    if beside.join(SETTINGS_FILE).is_file() {
        return beside.to_path_buf();
    }
    let source = old.iter().find(|folder| folder.join(SETTINGS_FILE).is_file());
    if !writable(beside) {
        return source.or(old.first()).cloned().unwrap_or_else(|| beside.to_path_buf());
    }
    if let Some(source) = source {
        for name in PROFILE_ITEMS {
            let from = source.join(name);
            if from.exists() && move_item(&from, &beside.join(name)).is_err() && name == SETTINGS_FILE {
                // If the settings file could not be moved, use the old folder (so settings are not lost).
                return source.clone();
            }
        }
        let _ = std::fs::remove_dir(source);
    }
    beside.to_path_buf()
}

impl Options {
    fn profile_directory(&self) -> PathBuf {
        if let Some(profile) = &self.profile {
            return profile.clone();
        }
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()));
        // Before the program was renamed from NDIR to NDR, the settings folder was NDIR-Rust.
        let old = [local.join("NDR"), local.join("NDIR-Rust")];
        match std::env::current_exe().ok().and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)) {
            Some(beside) => beside_profile(&beside, &old),
            None => old[0].clone(),
        }
    }
    fn parse() -> Result<Self, String> {
        Self::parse_from(std::env::args().skip(1))
    }
    fn parse_from(arguments: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut result = Self::default();
        let mut args = arguments;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--profile" => {
                    result.profile = Some(PathBuf::from(
                        args.next().ok_or("--profile needs a directory")?,
                    ))
                }
                "--left" => {
                    result.left = Some(PathBuf::from(args.next().ok_or("--left needs a path")?))
                }
                "--right" => {
                    result.right = Some(PathBuf::from(args.next().ok_or("--right needs a path")?))
                }
                "--language" => {
                    result.language = Some(args.next().ok_or("--language needs a code")?)
                }
                "--smoke-seconds" => {
                    result.smoke_seconds = Some(
                        args.next()
                            .ok_or("--smoke-seconds needs a number")?
                            .parse()
                            .map_err(|_| "Invalid smoke duration")?,
                    )
                }
                "--report" => {
                    result.report = Some(PathBuf::from(args.next().ok_or("--report needs a file")?))
                }
                "--shot" => {
                    let kind = args.next().ok_or("--shot needs a dialog kind")?;
                    let path = PathBuf::from(args.next().ok_or("--shot needs a PNG path")?);
                    result.shot = Some((kind, path));
                }
                _ => {
                    // Original FUN_004295b0: the part before each argument's first ':' is the key (case-insensitive). Quotes around values are removed,
                    // /dir: and /dir1: = left pane, /dir2: = right pane (only existing paths), /userdir: = user folder,
                    // /pane:single and dual = one or two panes. Unknown arguments are ignored.
                    let Some((key, value)) = arg.split_once(':') else { continue };
                    let value = value.strip_prefix('"').unwrap_or(value);
                    let value = value.strip_suffix('"').unwrap_or(value);
                    let exists = std::path::Path::new(value).exists();
                    match key.to_ascii_lowercase().as_str() {
                        "/dir" | "/dir1" if exists => result.left = Some(value.into()),
                        "/dir2" if exists => result.dir2 = Some(value.into()),
                        "/userdir" => result.profile = Some(user_directory(value)?),
                        "/pane" if value.eq_ignore_ascii_case("single") => result.dual = Some(false),
                        "/pane" if value.eq_ignore_ascii_case("dual") => result.dual = Some(true),
                        _ => {}
                    }
                }
            }
        }
        if result.smoke_seconds.is_some() && result.profile.is_none() {
            return Err("Smoke checks require --profile (isolated directory)".into());
        }
        if result.shot.is_some() && result.smoke_seconds.is_none() {
            return Err("--shot requires --smoke-seconds".into());
        }
        Ok(result)
    }
}

/// Original /userdir: (FUN_004295b0): a relative path is based on the executable folder. A missing folder shows MessageBoxW("\"%s\"\n%s", the path and
/// FormatMessage(3 = ERROR_PATH_NOT_FOUND), MB_ICONHAND) and exits.
fn user_directory(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    let path = if path.is_relative() {
        std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(&path))).unwrap_or(path)
    } else {
        path
    };
    if path.exists() {
        return Ok(path);
    }
    let message = format!("\"{}\"\n{}", path.display(), ndr::names::system_message(3).unwrap_or_default().trim_end());
    let wide = |text: &str| text.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, caption) = (wide(&message), wide("NDR"));
    // SAFETY: both strings are NUL-terminated UTF-16 and outlive the call; no owner window.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONHAND);
    }
    Err(message)
}

fn main() {
    let result = (|| -> Result<(), String> {
        let mut options = Options::parse()?;
        let settings =
            ndr::settings::Settings::load(&options.profile_directory().join("settings.json"))
                .unwrap_or_default();
        let (mut instance_guard, mut navigation) = (None::<ndr::lifecycle::InstanceGuard>, None);
        if settings.flag("UseSingleInstance", false) && options.smoke_seconds.is_none() {
            match ndr::lifecycle::InstanceGuard::acquire(
                &options.profile_directory(),
                ndr::lifecycle::NavigationRequest {
                    left: options.left.clone(),
                    right: options.right.clone().or_else(|| options.dir2.clone()),
                },
            )? {
                ndr::lifecycle::InstanceOutcome::Primary { guard, requests } => {
                    instance_guard = Some(guard);
                    navigation = Some(requests);
                }
                ndr::lifecycle::InstanceOutcome::Forwarded => return Ok(()),
            }
        }
        let mut viewport = eframe::egui::ViewportBuilder::default()
            .with_title("NDR")
            .with_inner_size([1400.0, 850.0])
            // Original TfMain DFM Constraints.MinWidth/MinHeight = 250 (outer size). Fixed: previously the window could not shrink below 800x480.
            .with_min_inner_size([250.0, 250.0]);
        // C# ApplyWindowBoundsFromSettings: with a saved window (HasWindowBounds), its size, position (kept on screen) and maximized state. Otherwise the original default settings
        // (default/nexusfile.ini [Window] left=10 top=10 width=800 height=600 maximized=0). Fixed: previously the first run was 1400x850.
        if options.smoke_seconds.is_none() {
            let saved = settings.flag("HasWindowBounds", false);
            let number = |key: &str, default: f64| if saved { settings.values.get(key).and_then(|v| v.as_f64()).unwrap_or(default) as f32 } else { default as f32 };
            let (width, height) = (number("WindowWidth", 800.0), number("WindowHeight", 600.0));
            let (mut left, mut top) = (number("WindowLeft", 10.0), number("WindowTop", 10.0));
            if let Some(screen) = ndr::native::virtual_screen_points() {
                (left, top) = ndr::native::clamp_window_position(left, top, width, height, screen);
            }
            // Saved values are outer sizes including the frame like the original and C# version, so the frame and title bar are subtracted to open with the inner size.
            let (frame_width, frame_height) = ndr::native::window_frame_points().unwrap_or((16.0, 39.0));
            let maximized = saved && settings.flag("WindowMaximized", false);
            let work = maximized.then(|| ndr::native::work_area_points(left + width / 2.0, top + height / 2.0)).flatten();
            if let Some((work_left, work_top, work_width, work_height)) = work {
                // Start maximized: winit with_maximized showed the window before the first frame and flickered. Draw into a hidden window of work area size, then maximize.
                viewport = viewport.with_inner_size([(work_width - frame_width).max(250.0), (work_height - frame_height).max(250.0)]).with_position([work_left, work_top]);
                options.maximize_restore = Some(((left, top, width, height), false));
            } else {
                viewport = viewport
                    .with_inner_size([(width - frame_width).max(250.0), (height - frame_height).max(250.0)])
                    .with_position([left, top])
                    .with_maximized(maximized);
            }
        }
        if options.smoke_seconds.is_some() {
            // The startup check opens off screen, without taking focus and without a taskbar entry (it does not cover the user's screen).
            viewport = viewport.with_position([-30000.0, -30000.0]).with_active(false).with_taskbar(false);
        } else {
            // Fixed: so folders extracted to %TEMP%\NDR when opening archive items do not pile up, ones unchanged for over 7 days and not in use are deleted.
            std::thread::spawn(|| {
                ndr::archive::clean_stale_temp_entries(&ndr::archive::temp_entry_root(), std::time::Duration::from_secs(7 * 24 * 3600), std::time::SystemTime::now());
            });
        }
        let native = eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        };
        eframe::run_native(
            "NDR",
            native,
            Box::new(move |cc| Ok(Box::new(ui::App::new(cc, options, navigation, instance_guard)))),
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(message) = result {
        eprintln!("{message}");
        let _ = std::fs::write(
            std::env::temp_dir().join("ndr-rust-startup-error.txt"),
            message,
        );
        std::process::exit(1);
    }
}


#[cfg(test)]
mod options_tests {
    use super::Options;

    /// Original FUN_004295b0 command line: /dir and /dir1 = left, /dir2 (kept separately), existing paths only, quotes removed, /pane, unknown arguments ignored.
    #[test]
    fn original_command_line_switches() {
        let root = std::env::temp_dir().join(format!("ndr-options-{}", std::process::id()));
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let parse = |list: Vec<String>| Options::parse_from(list.into_iter()).unwrap();
        let options = parse(vec![format!("/DIR:\"{}\"", a.display()), format!("/dir2:{}", b.display()), "/pane:Single".into(), "C:\\stray".into(), "/unknown:x".into()]);
        assert_eq!(options.left.as_deref(), Some(a.as_path()));
        assert_eq!(options.right, None, "/dir is the left pane only");
        assert_eq!(options.dir2.as_deref(), Some(b.as_path()));
        assert_eq!(options.dual, Some(false));
        let options = parse(vec![format!("/dir1:{}", root.join("missing").display()), "/pane:dual".into()]);
        assert_eq!(options.left, None, "paths that do not exist are ignored");
        assert_eq!(options.dual, Some(true));
        let options = parse(vec![format!("/userdir:{}", root.display())]);
        assert_eq!(options.profile.as_deref(), Some(root.as_path()));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The only settings file is settings.json next to the executable: when missing, settings of an old settings folder are moved over (the old ones deleted); when present it is used as is.
    #[test]
    fn settings_live_beside_the_executable_and_old_settings_move_there() {
        let root = std::env::temp_dir().join(format!("ndr-beside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (beside, old, older) = (root.join("app"), root.join("NDR"), root.join("NDIR-Rust"));
        for folder in [&beside, &old.join("treedata"), &older] {
            std::fs::create_dir_all(folder).unwrap();
        }
        std::fs::write(old.join("settings.json"), "{\"Theme\":\"Black\"}").unwrap();
        std::fs::write(old.join("toolbar.xml"), "<toolbar/>").unwrap();
        std::fs::write(old.join("treedata").join("1234.xcd"), "tree").unwrap();
        std::fs::write(older.join("settings.json"), "{\"Theme\":\"Dark\"}").unwrap();
        let folders = [old.clone(), older.clone()];
        assert_eq!(super::beside_profile(&beside, &folders), beside);
        assert_eq!(std::fs::read_to_string(beside.join("settings.json")).unwrap(), "{\"Theme\":\"Black\"}", "the first old folder with settings wins");
        assert_eq!(std::fs::read_to_string(beside.join("toolbar.xml")).unwrap(), "<toolbar/>");
        assert_eq!(std::fs::read_to_string(beside.join("treedata").join("1234.xcd")).unwrap(), "tree");
        assert!(!old.exists(), "the old folder is emptied and removed so only one settings file remains");
        assert!(older.join("settings.json").is_file(), "other old folders are left alone");
        let leftovers: Vec<_> = std::fs::read_dir(&beside).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(leftovers.len(), 3, "no write probe is left behind: {leftovers:?}");
        // Settings next to the executable are used as is (old folders are not checked again).
        std::fs::write(beside.join("settings.json"), "{}").unwrap();
        assert_eq!(super::beside_profile(&beside, &folders), beside);
        assert_eq!(std::fs::read_to_string(beside.join("settings.json")).unwrap(), "{}");
        assert!(older.join("settings.json").is_file());
        // With no settings at all, the executable folder (created on the first save).
        let fresh = root.join("fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        assert_eq!(super::beside_profile(&fresh, &[root.join("none")]), fresh);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
