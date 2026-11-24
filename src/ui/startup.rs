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
}

fn startup_language(options: &Options, settings: Option<&Settings>) -> String {
    options.language.clone().unwrap_or_else(|| settings.map_or("ko".into(), |settings| settings.text("Language", "ko")))
}

impl PreparedStartup {
    pub(super) fn prepare(options: &Options, profile: PathBuf, loaded: Result<Settings, String>) -> Self {
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
        Self { profile, settings, error, language, dual, panes_init: [left, right], resources, language_pack, toolbar }
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
            palette.apply_window_caption(ctx, hwnd);
        }
        // Install all font families before the first paint; the full app
        // can render immediately on handoff without a missing list family.
        set_app_fonts(ctx, None, false, 1.0);
        let (tx, prepared) = mpsc::channel();
        let worker_options = options.clone();
        let repaint = ctx.clone();
        let _ = thread::Builder::new().name("ndr-startup".into()).spawn(move || {
            let data = PreparedStartup::prepare(&worker_options, profile, loaded);
            let _ = tx.send(data);
            repaint.request_repaint();
        });
        Self { phase: StartupPhase::Loading(Box::new(Pending { options, navigation, guard, hwnd, prepared, painted: false, loading_text, failure_text })) }
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
                ctx.request_repaint();
                return;
            }
            StartupPhase::Loading(_) => {}
        }
        let StartupPhase::Loading(pending) = &mut self.phase else { unreachable!() };
        match pending.prepared.try_recv() {
            Ok(prepared) => {
                let mut app = Box::new(App::finish_startup(
                    ctx, std::mem::take(&mut pending.options), prepared,
                    pending.navigation.take(), pending.guard.take(), pending.hwnd,
                ));
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
