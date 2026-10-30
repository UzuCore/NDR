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
}
impl Options {
    fn profile_directory(&self) -> PathBuf {
        self.profile.clone().unwrap_or_else(|| {
            let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()));
            let profile = local.join("NDR");
            // 프로그램 이름을 NDIR에서 NDR로 바꿨다. 새 폴더가 아직 없으면 예전 설정 폴더(NDIR-Rust)를 복사해 온다(예전 폴더는 그대로 둔다).
            let old = local.join("NDIR-Rust");
            if !profile.exists() && old.is_dir() {
                let _ = copy_tree(&old, &profile);
            }
            profile
        })
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
        let options = Options::parse()?;
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
            .with_min_inner_size([800.0, 480.0]);
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
            viewport = viewport
                .with_inner_size([(width - frame_width).max(800.0), (height - frame_height).max(480.0)])
                .with_position([left, top])
                .with_maximized(settings.flag("WindowMaximized", false));
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
}
