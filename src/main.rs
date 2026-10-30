#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ui;

use std::path::PathBuf;

/// 폴더를 통째로 복사한다(예전 설정 폴더 옮겨 오기). 이미 있는 파일은 건드리지 않는다.
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
    /// 원본 /dir2:(있는 경로). 원본은 두 패널 보기일 때만 오른쪽 패널을 그리로 연다.
    dir2: Option<PathBuf>,
    /// 원본 /pane:single(false)·/pane:dual(true). 없으면 설정(DualPaneMode).
    dual: Option<bool>,
    language: Option<String>,
    smoke_seconds: Option<u64>,
    report: Option<PathBuf>,
    /// 시험용: 시작 확인(화면 밖) 중에 이 종류의 원본 창 견본을 열고 그 화면을 PNG로 저장한다(`--shot 종류 파일`).
    shot: Option<(String, PathBuf)>,
    /// 최대화로 시작: (되돌릴 바깥 x, y, 너비, 높이(egui 점), 첫 그림이 끝났는지). App이 첫 그림 뒤에 최대화한다.
    maximize_restore: Option<((f32, f32, f32, f32), bool)>,
}
/// 설정 파일 이름. 설정 파일은 이것 하나만 쓴다.
const SETTINGS_FILE: &str = "settings.json";
/// 예전 설정 폴더에서 실행 파일 폴더로 옮겨 오는 것(설정 파일은 마지막에 옮긴다: 도중에 멈추면 다음 시작에 다시 한다).
const PROFILE_ITEMS: [&str; 4] = ["toolbar.xml", "advrename.save", "treedata", SETTINGS_FILE];

/// 그 폴더에 파일을 만들 수 있는가(만들어 보고 바로 지운다).
fn writable(directory: &std::path::Path) -> bool {
    let probe = directory.join(format!(".ndr-write-probe-{}", std::process::id()));
    let made = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).is_ok();
    made && std::fs::remove_file(&probe).is_ok()
}

/// 파일·폴더 하나를 옮긴다(다른 드라이브면 복사한 뒤 원래 것을 지운다). 대상이 이미 있으면 그대로 둔다.
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

/// 설정 폴더 = 실행 파일 폴더(`beside`). 설정 파일은 그 폴더의 settings.json 하나뿐이다.
/// 그 파일이 아직 없으면 예전 설정 폴더(`old` 차례, %LOCALAPPDATA%\NDR → NDIR-Rust) 가운데 설정 파일이 있는 첫 폴더의
/// 설정(PROFILE_ITEMS)을 실행 파일 폴더로 옮긴다. 두 곳에 설정이 남지 않게 예전 것은 지우고, 빈 예전 폴더도 지운다.
/// 실행 파일 폴더에 쓸 수 없으면(예: Program Files) 옮기지 않고 예전 폴더를 그대로 쓴다.
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
                // 설정 파일을 옮기지 못했으면 예전 폴더를 쓴다(설정을 잃지 않게).
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
        // 프로그램 이름을 NDIR에서 NDR로 바꾸기 전의 설정 폴더는 NDIR-Rust였다.
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
                    // 원본 FUN_004295b0: 인자마다 첫 ':' 앞을 열쇠로 본다(대소문자 무시). 값의 앞뒤 따옴표를 떼고,
                    // /dir:·/dir1: = 왼쪽 패널, /dir2: = 오른쪽 패널(둘 다 있는 경로일 때만), /userdir: = 사용자 폴더,
                    // /pane:single·dual = 한/두 패널. 모르는 인자는 무시한다.
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

/// 원본 /userdir:(FUN_004295b0): 상대 경로면 실행 파일 폴더 기준. 없는 폴더면 MessageBoxW("\"%s\"\n%s", 경로와
/// FormatMessage(3 = ERROR_PATH_NOT_FOUND), MB_ICONHAND)를 보이고 끝낸다.
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
            // 원본 TfMain DFM Constraints.MinWidth·MinHeight = 250(바깥 크기). 고친 것: 전에는 800×480보다 작게 줄일 수 없었다.
            .with_min_inner_size([250.0, 250.0]);
        // C# ApplyWindowBoundsFromSettings: 저장한 창이 있으면(HasWindowBounds) 그 크기·위치(화면 안으로)·최대화.
        if settings.flag("HasWindowBounds", false) && options.smoke_seconds.is_none() {
            let number = |key: &str, default: f64| settings.values.get(key).and_then(|v| v.as_f64()).unwrap_or(default) as f32;
            let (width, height) = (number("WindowWidth", 1450.0), number("WindowHeight", 900.0));
            let (mut left, mut top) = (number("WindowLeft", 10.0), number("WindowTop", 10.0));
            if let Some(screen) = ndr::native::virtual_screen_points() {
                (left, top) = ndr::native::clamp_window_position(left, top, width, height, screen);
            }
            // 저장 값은 원본·C# 판처럼 테두리를 포함한 바깥 크기이므로, 테두리·제목 줄 크기를 빼서 안쪽 크기로 연다.
            let (frame_width, frame_height) = ndr::native::window_frame_points().unwrap_or((16.0, 39.0));
            let work = settings.flag("WindowMaximized", false).then(|| ndr::native::work_area_points(left + width / 2.0, top + height / 2.0)).flatten();
            if let Some((work_left, work_top, work_width, work_height)) = work {
                // 최대화로 시작: winit with_maximized는 첫 그림 전에 창을 보여 깜박였다. 작업 영역 크기의 숨은 창으로 그린 뒤 최대화한다.
                viewport = viewport.with_inner_size([(work_width - frame_width).max(800.0), (work_height - frame_height).max(480.0)]).with_position([work_left, work_top]);
                options.maximize_restore = Some(((left, top, width, height), false));
            } else {
                viewport = viewport
                    .with_inner_size([(width - frame_width).max(800.0), (height - frame_height).max(480.0)])
                    .with_position([left, top])
                    .with_maximized(settings.flag("WindowMaximized", false));
            }
        }
        if options.smoke_seconds.is_some() {
            // 시작 확인은 화면 밖에서, 초점을 뺏지 않고, 작업 표시줄에도 안 보이게 띄운다(사용자 화면을 가리지 않는다).
            viewport = viewport.with_position([-30000.0, -30000.0]).with_active(false).with_taskbar(false);
        } else {
            // 고친 것: 압축 안 항목을 열 때 %TEMP%\NDR에 푼 폴더가 쌓이지 않게, 7일 넘게 바뀌지 않았고 쓰는 중이 아닌 것을 지운다.
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

    /// 원본 FUN_004295b0 명령줄: /dir·/dir1 = 왼쪽, /dir2(따로 둠), 있는 경로만, 따옴표 뗌, /pane, 모르는 인자 무시.
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

    /// 설정 파일은 실행 파일 옆 settings.json 하나: 없으면 예전 설정 폴더의 설정을 옮기고(예전 것은 지움), 있으면 그대로 쓴다.
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
        // 실행 파일 옆에 설정이 있으면 그대로 쓴다(예전 폴더를 다시 보지 않는다).
        std::fs::write(beside.join("settings.json"), "{}").unwrap();
        assert_eq!(super::beside_profile(&beside, &folders), beside);
        assert_eq!(std::fs::read_to_string(beside.join("settings.json")).unwrap(), "{}");
        assert!(older.join("settings.json").is_file());
        // 아무 설정도 없으면 실행 파일 폴더(처음 저장할 때 만든다).
        let fresh = root.join("fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        assert_eq!(super::beside_profile(&fresh, &[root.join("none")]), fresh);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
