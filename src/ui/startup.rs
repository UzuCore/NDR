//! Paint the main window before waiting on saved disks, paths, or resources.
use super::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

type PaneStartup = (Vec<PathBuf>, usize, bool);
type Navigation = Receiver<ndr::lifecycle::NavigationRequest>;

pub(super) struct PreparedStartup {
    pub(super) profile: PathBuf,
    pub(super) settings: Settings,
    pub(super) error: Option<String>,
    pub(super) language: String,
    pub(super) dual: bool,
    pub(super) panes_init: [PaneStartup; 2],
    pub(super) resources: PathBuf,
    pub(super) language_pack: compat::Language,
    pub(super) toolbar: ndr::commands::ToolbarConfig,
    pub(super) list_font:Option<PreparedListFont>,
    pub(super) tree_font:Option<PreparedListFont>,
    pub(super) prepare_ms:u64,
}

pub(super) struct PreparedListFont{
    pub(super) key:String,
    pub(super) data:Option<(&'static [u8],u32)>,
    pub(super) korean_scale:f32,
}
pub(super) fn list_font_config(settings:&Settings,language:&str)->(String,u8,f32,String){
    let face=settings.text("ListFont","Segoe UI");
    let style=(settings.values.get("ListFontStyle").and_then(|v|v.as_i64()).unwrap_or(0)&0x0F) as u8;
    let size=ndr::settings::list_font_pixels(settings) as f32;
    let pixels=size.round() as i32;
    let key=format!("{face}\u{0}{}\u{0}{pixels}\u{0}{}",style&3,super::preferred_cjk_font(language).unwrap_or(""));
    (face,style,size,key)
}
pub(super) fn tree_font_config(settings:&Settings,language:&str)->(String,u8,f32,String){
    let face=settings.text("FolderTreeFont","Segoe UI");
    let style=(settings.values.get("FolderTreeFontStyle").and_then(|v|v.as_i64()).unwrap_or(0)&0x0F) as u8;
    let size=ndr::settings::font_layout_pixels(ndr::settings::tree_font_points(settings)) as f32;
    let key=format!("{face}\u{0}{}\u{0}{}\u{0}{}",style&3,size.round() as i32,super::preferred_cjk_font(language).unwrap_or(""));
    (face,style,size,key)
}
pub(super) fn prepare_list_font(face:&str,style:u8,size:f32,key:String,find:fn(&str)->Option<(PathBuf,u32)>)->PreparedListFont{
    let data=native::styled_font_names(face,style).iter().find_map(|name|{
        let (path,index)=find(name)?;
        let bytes=native::mapped_file(&path)?;
        ab_glyph::FontRef::try_from_slice_and_index(bytes,index).ok()?;
        Some((bytes,index))
    });
    let hangul="\u{ac00}".repeat(10);
    let pixels=size.round() as i32;
    let korean_scale=native::gdi_text_widths(face,pixels,style,&[hangul.as_str()])
        .zip(native::gdi_text_widths("Malgun Gothic",pixels,style,&[hangul.as_str()]))
        .map(|(a,b)|a[0] as f32/b[0].max(1) as f32)
        .filter(|s|(0.5..=1.5).contains(s)).unwrap_or(1.0);
    PreparedListFont{key,data,korean_scale}
}

fn startup_language(options: &Options, settings: Option<&Settings>) -> String {
    options.language.clone().unwrap_or_else(|| settings.map_or("ko".into(), |settings| settings.text("Language", "ko")))
}

impl PreparedStartup {
    pub(super) fn prepare(options: &Options, profile: PathBuf, loaded: Result<Settings, String>) -> Self {
        let start=Instant::now();
        let error = loaded.as_ref().err().cloned();
        let settings = loaded.unwrap_or_default();
        let language = startup_language(options, Some(&settings));
        let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_else(|| "C:\\".into()));
        // C# start folder: command line -> "Always start in the following folder" (if StartPathEnabled and StartPath exist) -> last folder (LeftPath).
        // The second value is whether the folder was given explicitly like a command line / start folder (in which case that folder takes priority over the saved selected tab).
        let start_path = |side: &str, explicit: &Option<PathBuf>| -> (PathBuf, bool) {
            if let Some(path) = explicit {
                return (path.clone(), true);
            }
            let start = settings.text(&format!("{side}StartPath"), "");
            if settings.flag(&format!("{side}StartPathEnabled"), false) && !start.trim().is_empty() {
                return (PathBuf::from(start.trim()), true);
            }
            (PathBuf::from(settings.text(&format!("{side}Path"), &home.to_string_lossy())), false)
        };
        let (left, left_fixed) = start_path("Left", &options.left);
        // Original FUN_004295b0: /pane:single/dual takes precedence over the setting (dualview), and /dir2 opens the right side only in dual-pane view.
        let dual = options.dual.unwrap_or_else(|| settings.flag("DualPaneMode", true));
        let right_explicit = options.right.clone().or_else(|| options.dir2.clone().filter(|_| dual));
        let (right, right_fixed) = start_path("Right", &right_explicit);
        // C# ValidStartPath: an existing folder as-is, otherwise the user folder, otherwise "::drives::" (My PC).
        // Both panes often restore the same paths (including network folders).
        // Probe each path only once during this startup, not on later navigation.
        let mut checked = HashMap::new();
        let mut exists = |path: &Path| {
            if panes::is_drives(path) {
                return true;
            }
            if let Some(&found) = checked.get(path) {
                return found;
            }
            let found = path.is_dir();
            checked.insert(path.to_path_buf(), found);
            found
        };
        let mut valid_start = |candidate: PathBuf| {
            if exists(&candidate) {
                candidate
            } else if exists(&home) {
                home.clone()
            } else {
                PathBuf::from(remote_window::DRIVES)
            }
        };
        let (left, right) = (valid_start(left), valid_start(right));
        let [left, right] = [(left, left_fixed, "Left"), (right, right_fixed, "Right")].map(|(start, fixed, side)| {
            let selected = settings.values.get(&format!("Selected{side}Tab")).and_then(|v| v.as_u64()).map(|v| v as usize);
            let (tabs, tab) = restore_tabs(&settings.strings(&format!("{side}Tabs")), selected, &start, fixed, &mut exists);
            (tabs, tab, settings.flag(&format!("{side}ShowHiddenFiles"), false))
        });
        let resources = resource_dir();
        let language_pack = compat::Language::load(&resources.join("Languages"), &language);
        let language = language_pack.code.clone();
        let toolbar = ndr::commands::load_toolbar(&profile);
        let list_font=(!cfg!(test)).then(||{
            let (face,style,size,key)=list_font_config(&settings,&language);
            prepare_list_font(&face,style,size,key,native::font_file)
        });
        let tree_font=(!cfg!(test)).then(||{
            let (face,style,size,key)=tree_font_config(&settings,&language);
            prepare_list_font(&face,style,size,key,native::font_file)
        });
        Self { profile, settings, error, language, dual, panes_init: [left, right], resources, language_pack, toolbar,
            list_font,tree_font,prepare_ms:start.elapsed().as_millis() as u64 }
    }
}

enum StartupPhase {
    Loading(Box<Pending>),
    Ready(Box<App>),
    Failed { message: String },
}

struct Pending {
    options: Options,
    navigation: Option<Navigation>,
    guard: Option<ndr::lifecycle::InstanceGuard>,
    hwnd: Option<isize>,
    prepared: Receiver<PreparedStartup>,
    painted: bool,
    created:Instant,
    shell_ms:u64,
    loading_text: String,
    failure_text: String,
}

pub struct StartupApp {
    phase: StartupPhase,
}

impl StartupApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        options: Options,
        profile: PathBuf,
        loaded: Result<Settings, String>,
        navigation: Option<Navigation>,
        guard: Option<ndr::lifecycle::InstanceGuard>,
    ) -> Self {
        let created=Instant::now();
        let ctx = &cc.egui_ctx;
        let hwnd = cc.window_handle().ok().and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        });
        let settings = loaded.as_ref().ok();
        let labels = compat::Language::load_labels(&resource_dir().join("Languages"), &startup_language(&options, settings));
        let loading_text = labels.localized("NDR.UI_STARTUP_LOADING", "폴더와 설정을 준비하는 중…", "Preparing folders and settings…");
        let failure_text = labels.localized("NDR.UI_STARTUP_FAILED", "시작 준비에 실패했습니다. 다시 실행해 주세요.", "Startup failed. Please reopen NDR.");
        let theme = settings.map_or(ndr::styles::DEFAULT_THEME.into(), |settings| settings.text("Theme", ndr::styles::DEFAULT_THEME));
        let palette = system_commands::theme_palette(&theme);
        ctx.set_theme(palette.egui_theme());
        ctx.set_visuals(system_commands::palette_visuals(&palette));
        // Style the hidden native window before eframe reveals the loading frame.
        if let Some(hwnd) = hwnd {
            // egui-winit sets ICON_SMALL from the viewport PNG, but not
            // ICON_BIG on Windows. Use the embedded executable icon for
            // the taskbar and Alt+Tab before the first visible frame.
            use windows_sys::Win32::UI::WindowsAndMessaging::{LoadIconW, SendMessageW, SetClassLongPtrW, WM_GETICON, WM_SETICON, ICON_BIG, ICON_SMALL, GCLP_HICON, GCLP_HICONSM};
            if let Ok(module) = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::PCWSTR::null()) } {
                // LoadIconW returns a shared resource handle, not owned by us.
                let icon = unsafe { LoadIconW(module.0, 101usize as *const u16) };
                if !icon.is_null() {
                    unsafe {
                        // Supply a class-icon fallback as well as the window
                        // icon, using shared handles with process lifetime.
                        SetClassLongPtrW(hwnd as _, GCLP_HICON, icon as isize);
                        SendMessageW(hwnd as _, WM_SETICON, ICON_BIG as usize, icon as isize);
                        let small = SendMessageW(hwnd as _, WM_GETICON, ICON_SMALL as usize, 0);
                        if small != 0 {
                            SetClassLongPtrW(hwnd as _, GCLP_HICONSM, small);
                        }
                    }
                }
            }
            palette.apply_window_caption(ctx, hwnd);
        }
        // Install all font families before the first paint; the full app
        // can render immediately on handoff without a missing list family.
        set_app_fonts(ctx, None, false, 1.0, &startup_language(&options, settings));
        let (tx, prepared) = mpsc::channel();
        let worker_options = options.clone();
        let repaint = ctx.clone();
        let _ = thread::Builder::new().name("ndr-startup".into()).spawn(move || {
            let data = PreparedStartup::prepare(&worker_options, profile, loaded);
            let _ = tx.send(data);
            repaint.request_repaint();
        });
        Self { phase: StartupPhase::Loading(Box::new(Pending { options, navigation, guard, hwnd, prepared, painted: false, created,shell_ms:0, loading_text, failure_text })) }
    }

    fn draw_loading(ctx: &egui::Context, message: &str, failed: bool) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.36).max(8.0));
                ui.heading("NDR");
                ui.add_space(8.0);
                if !failed {
                    ui.spinner();
                }
                ui.label(message);
            });
        });
    }
}

impl eframe::App for StartupApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let StartupPhase::Ready(app) = &mut self.phase {
            app.raw_input_hook(ctx, raw);
        } else {
            raw.max_texture_side = Some(raw.max_texture_side.unwrap_or(2048).min(2048));
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        match &mut self.phase {
            StartupPhase::Ready(app) => return app.update(ctx, frame),
            StartupPhase::Failed { message } => return Self::draw_loading(ctx, message, true),
            StartupPhase::Loading(pending) if !pending.painted => {
                // eframe reveals the window after this paint, even if the
                // startup worker has not returned or already finished.
                pending.painted = true;
                Self::draw_loading(ctx, &pending.loading_text, false);
                pending.shell_ms=pending.created.elapsed().as_millis() as u64;
                ctx.request_repaint();
                return;
            }
            StartupPhase::Loading(_) => {}
        }
        let StartupPhase::Loading(pending) = &mut self.phase else { unreachable!() };
        match pending.prepared.try_recv() {
            Ok(prepared) => {
                let prepare_ms=prepared.prepare_ms;
                let mut app = Box::new(App::finish_startup(
                    ctx, std::mem::take(&mut pending.options), prepared,
                    pending.navigation.take(), pending.guard.take(), pending.hwnd,
                ));
                app.smoke_startup=serde_json::json!({"shellSubmittedMs":pending.shell_ms,
                    "workerPrepareMs":prepare_ms,"handoffMs":pending.created.elapsed().as_millis() as u64});
                app.update(ctx, frame);
                self.phase = StartupPhase::Ready(app);
            }
            Err(mpsc::TryRecvError::Empty) => Self::draw_loading(ctx, &pending.loading_text, false),
            Err(mpsc::TryRecvError::Disconnected) => {
                let message = std::mem::take(&mut pending.failure_text);
                Self::draw_loading(ctx, &message, true);
                self.phase = StartupPhase::Failed { message };
            }
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let StartupPhase::Ready(app) = &mut self.phase {
            app.on_exit(gl);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_preparation_keeps_explicit_folder_and_restores_valid_saved_tabs() {
        let root = std::env::temp_dir().join(format!(
            "ndr-startup-{}-{:?}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let explicit = root.join("explicit");
        let kept = root.join("kept");
        let removed = root.join("missing");
        for folder in [&explicit, &kept] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let mut settings = Settings::default();
        settings.set_strings("LeftTabs", vec![
            kept.to_string_lossy().into_owned(),
            removed.to_string_lossy().into_owned()
        ]);
        settings.values.insert("SelectedLeftTab".into(), serde_json::json!(1));
        settings.set_flag("LeftShowHiddenFiles", true);
        settings.set_text("Language", "en-US");
        let options = Options { left: Some(explicit.clone()), dual: Some(false), ..Default::default() };
        let prepared = PreparedStartup::prepare(&options, root.join("profile"), Ok(settings));
        let (tabs, selected, hidden) = &prepared.panes_init[0];
        assert_eq!(tabs[*selected], explicit, "explicit folder has priority");
        assert!(tabs.contains(&kept));
        assert!(!tabs.contains(&removed), "an unavailable saved tab is not restored");
        assert!(*hidden);
        assert_eq!(prepared.language, "en");
        assert_eq!(prepared.settings.text("Language", ""), "en-US", "opening a profile must not overwrite its preference");
        assert!(!prepared.dual);
        std::fs::remove_dir_all(root).unwrap();
    }
}
