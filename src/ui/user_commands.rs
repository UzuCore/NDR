use super::*;


/// Original FUN_00479970 (function key cell name from one [FuncKey] value): an empty value gives an empty name. "[name]command" gives the text inside [ ] (everything after [ when empty),
/// "#internal command" gives RES FUNC_<upper-case name> when it is in the internal command list (FUN_004586ac, otherwise the text after #), and anything else (a program)
/// the file name after PathStripPath and PathRemoveExtension (an extension containing a space is not treated as an extension).
pub(super) fn original_function_key_caption(command: &str, resource: &dyn Fn(&str) -> Option<String>) -> String {
    let command = command.trim();
    if command.is_empty() {
        return String::new();
    }
    if let Some(rest) = command.strip_prefix('[') {
        let inside = rest.split_once(']').map_or("", |(name, _)| name);
        return if inside.is_empty() { rest.to_owned() } else { inside.to_owned() };
    }
    if let Some(name) = command.strip_prefix('#') {
        let known = super::function_key_editor::INTERNAL_COMMANDS.iter().any(|(id, _)| id.trim_start_matches('#').eq_ignore_ascii_case(name));
        return known.then(|| resource(&format!("FUNC_{}", name.to_uppercase()))).flatten().unwrap_or_else(|| name.to_owned());
    }
    let file = command.rsplit(['\\', '/']).next().unwrap_or(command);
    match file.rfind('.') {
        Some(dot) if !file[dot..].contains(' ') => file[..dot].to_owned(),
        _ => file.to_owned(),
    }
}

/// Original default function keys (default\nexusfile.ini [FuncKey]): internal commands used when nothing is assigned.
fn original_default_function_key(key: &str) -> &'static str {
    match key {
        "F2" => "#Rename",
        "F3" => "#CopyTo",
        "F4" => "#MoveTo",
        "F5" => "#Refresh",
        "F6" => "#AdvRename",
        "F7" => "#NewFolder",
        "F8" => "#RecycleBin",
        _ => "",
    }
}

/// Pressing an ALTFUNCTIONS/CTRLFUNCTIONS button is the same as pressing that key (with the same Alt/Ctrl).
fn modifier_function_key(key: &str) -> Option<egui::Key> {
    match key {
        "<-" => Some(egui::Key::ArrowLeft),
        "->" => Some(egui::Key::ArrowRight),
        other => egui::Key::from_name(other),
    }
}

impl App {
    /// Original ALTFUNCTIONS/CTRLFUNCTIONS ("key=name" rows): the first while only Alt is held, the second while only Ctrl is held. Otherwise None (F1-F12 buttons).
    pub(super) fn modifier_functions(&self, modifiers: egui::Modifiers) -> Option<Vec<(String, String)>> {
        let text = match (modifiers.alt, modifiers.ctrl, modifiers.shift) {
            (true, false, false) => self.language_pack.text("RES.ALTFUNCTIONS", "C=Copy\nM=Move\nD=Delete\nR=Rename\nK=New Folder\nU=Select All\nV=View Text\nB=Edit Text"),
            (false, true, false) => self.language_pack.text(
                "RES.CTRLFUNCTIONS",
                "Z=Change Attr\nA=Add To Archive\nE=Extract\nD=Desktop\nG=Documents\nB=RecycleBin\n<-=Copy To Left\n->=Copy To Right",
            ),
            _ => return None,
        };
        // '=' cannot be the key itself (as in "<-=..."), so split at the first '='.
        Some(text.lines().filter_map(|line| line.split_once('=')).map(|(key, name)| (key.to_owned(), name.to_owned())).collect())
    }

    /// Original bbFunctions (TNXButtonBar, skin theme): seven cells F2-F8 of FUN_00430714 ("F%d %s"), ALTFUNCTIONS/CTRLFUNCTIONS cells while Alt/Ctrl is held.
    /// Paint (0x7c1568): cell width = row width / cell count, background bbbackimg tiled horizontally. DrawItem (0x7c0ec0): key text (bbkeytext) at left + 5,
    /// the name after its width + 8 (bbtext, bbover when hovered), a pressed cell shifts by (1, 1), and bbdividerimg at the cell's right end - 2 (vertically centered).
    pub(super) fn skin_function_key_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, skin: &ndr::styles::Skin) {
        let rect = ui.max_rect();
        if let Some(back) = skin.values.get("buttonbar.bbbackimg").cloned().and_then(|file| self.skin_bitmap(ctx, &file)) {
            let size = back.size_vec2();
            let mut x = rect.left();
            while x < rect.right() {
                ui.painter().with_clip_rect(rect).image(back.id(), egui::Rect::from_min_size(egui::pos2(x, rect.top()), size), egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
                x += size.x.max(1.0);
            }
        }
        let modifiers = ctx.input(|i| i.modifiers);
        let cells: Vec<(String, String)> = match self.modifier_functions(modifiers) {
            Some(items) => items,
            None => (2..=8)
                .map(|number| {
                    let key = format!("F{number}");
                    // Original FUN_00479970: the name comes from the [FuncKey] value. A named C# assignment uses that name; with no assignment, the original default value.
                    let resource = |name: &str| self.language_pack.raw(&format!("RES.{name}"));
                    let name = match ndr::commands::resolve_function_key(&self.settings, &key) {
                        Some(b) if !b.legacy && !b.name.trim().is_empty() => b.name,
                        Some(b) => original_function_key_caption(&b.command, &resource),
                        None => original_function_key_caption(original_default_function_key(&key), &resource),
                    };
                    (key, name)
                })
                .collect(),
        };
        let color = |key: &str, fallback: Color32| skin.color("ButtonBar", key).map(rgba).unwrap_or(fallback);
        let (key_color, text_color, over_color) = (color("bbkeytext", Color32::from_rgb(0xE2, 0xA3, 0x1D)), color("bbtext", ui.visuals().text_color()), color("bbover", Color32::WHITE));
        let divider = skin.values.get("buttonbar.bbdividerimg").cloned().and_then(|file| self.skin_bitmap(ctx, &file));
        let font = egui::FontId::proportional(super::BASE_TEXT_SIZE);
        let width = rect.width() / cells.len().max(1) as f32;
        let mut clicked = None;
        for (index, (key, name)) in cells.iter().enumerate() {
            let cell = egui::Rect::from_min_size(egui::pos2(rect.left() + width * index as f32, rect.top()), egui::vec2(width, rect.height()));
            let response = ui.interact(cell, egui::Id::new(("ndr-skin-fkey", index)), egui::Sense::click());
            let shift = if response.is_pointer_button_down_on() { egui::vec2(1.0, 1.0) } else { egui::Vec2::ZERO };
            // Cell text runs up to the divider at the right end (- 2).
            let painter = ui.painter().with_clip_rect(cell.with_max_x(cell.right() - 3.0));
            let drawn = painter.text(egui::pos2(cell.left() + 5.0, cell.center().y) + shift, egui::Align2::LEFT_CENTER, key, font.clone(), key_color);
            painter.text(egui::pos2(drawn.right() + 8.0, cell.center().y) + shift, egui::Align2::LEFT_CENTER, name, font.clone(), if response.hovered() { over_color } else { text_color });
            if let Some(divider) = &divider {
                let size = divider.size_vec2();
                let at = egui::pos2(cell.right() - 2.0, cell.top() + ((cell.height() - size.y) / 2.0).floor());
                ui.painter().image(divider.id(), egui::Rect::from_min_size(at, size), egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            if response.clicked() {
                clicked = Some(key.clone());
            }
            // Original bbFunctionsContextPopup (0x4373ec): right-clicking a base cell (not the Alt/Ctrl cells) also runs that function key
            // (FUN_004308a0, no menu appears).
            if response.secondary_clicked() && !(modifiers.alt || modifiers.ctrl) {
                clicked = Some(key.clone());
            }
        }
        match clicked {
            Some(key) if modifiers.alt || modifiers.ctrl => {
                if let Some(key) = modifier_function_key(&key) {
                    self.shortcut_key(key, modifiers, ctx);
                }
            }
            Some(key) => self.run_function_key_button(&key, ctx),
            None => {}
        }
    }

    /// C# RunFunctionKeyButtonAsync: the macro when assigned, otherwise the default action, else "no action is set."
    pub(super) fn run_function_key_button(&mut self, key: &str, ctx: &egui::Context) {
        if self.invoke_function_key(key, ctx) {
            return;
        }
        let command = match key {
            "F2" => "rename",
            "F3" => "copyToOther",
            "F4" => "moveToOther",
            "F5" => "refresh",
            "F6" => "advancedRename",
            "F7" => "newFolder",
            "F8" => "deleteToolbar",
            "F10" => "folderTree",
            "F11" => "favoritesMenu",
            "F12" => "shortcuts",
            _ => {
                self.status = format!("{key}: {}", if self.language == "ko" { "설정된 동작이 없습니다." } else { "No action is configured." });
                return;
            }
        };
        self.dispatch(command, ctx);
    }

    pub(super) fn invoke_function_key(&mut self, key: &str, ctx: &egui::Context) -> bool {
        let binding = match ndr::commands::resolve_function_key(&self.settings, key) {
            Some(binding) => binding,
            // Without [FuncKey] the original uses the default\nexusfile.ini values (F2 #Rename ... F8 #RecycleBin). Fixed: this version used the defaults
            // only for the function key row names and never ran them, so in a new profile F3 (copy), F4 (move) and others did nothing.
            // When the profile has that key's slot (even with an empty value), its value is followed.
            None if !self.function_key_configured(key) && !original_default_function_key(key).is_empty() => {
                let command = original_default_function_key(key);
                ndr::commands::FunctionBinding { name: key.to_owned(), command: command.to_owned(), parameters: String::new(), internal: true, legacy: true }
            }
            None => return false,
        };
        self.run_binding(&binding, ctx);
        true
    }

    /// Whether the profile has that function key's slot (FunctionKeys or FunctionKeyBindings).
    fn function_key_configured(&self, key: &str) -> bool {
        ["FunctionKeys", "FunctionKeyBindings"].iter().any(|field| self.settings.values.get(*field).and_then(|v| v.as_object()).is_some_and(|map| map.keys().any(|k| k.eq_ignore_ascii_case(key))))
    }

    /// Runs one function key or toolbar UserCommand assignment (C# ExecuteFunctionKeyMacroAsync, original FUN_0044381c).
    pub(super) fn run_binding(&mut self, binding: &ndr::commands::FunctionBinding, ctx: &egui::Context) {
        let panes = self.panes.each_ref().map(|p| compat::MacroPane {
            path: p.path.clone(),
            focused: p.entries.get(p.cursor).map(|e| e.name.clone()),
            selected: p
                .selected
                .iter()
                .filter_map(|i| p.entries.get(*i).map(|e| e.name.clone()))
                .collect(),
        });
        let executable = std::env::current_exe().unwrap_or_default();
        let program_dir = executable.parent().unwrap_or(Path::new("."));
        match ndr::commands::prepare_function_key(
            binding,
            &panes[self.active],
            &panes[0],
            &panes[1],
            program_dir,
        ) {
            ndr::commands::PreparedCommand::Disabled => {}
            ndr::commands::PreparedCommand::Internal(command) => {
                if !self.original_internal(&command, ctx) {
                    self.dispatch(&command, ctx);
                }
            }
            ndr::commands::PreparedCommand::External { program, arguments, prompt } => {
                if prompt {
                    // C# ExecuteFunctionKeyMacroAsync: with TakeParameterPrompt, InputDialog(TITLE_EDITPARAM, MSG_EDITPARAM, arguments); cancel does not run.
                    let title = self.language_pack.text("TITLE_EDITPARAM", "Edit Parameters");
                    let message = self.language_pack.text("MSG_EDITPARAM", "Type Parameters.");
                    self.prompt(&title, &message, &arguments, None, false, move |app, text, _| {
                        if let Some(text) = text {
                            app.run_function_key_program(&program, &text);
                        }
                    });
                } else {
                    self.run_function_key_program(&program, &arguments);
                }
            }
        }
    }

    /// C# ExecuteFunctionKeyMacroAsync (after editing arguments): an empty program does nothing, an existing folder opens in the active pane,
    /// otherwise ShellExecuteEx(program, arguments, active pane folder (none when missing)). The shell reports failures itself, so nothing else is shown.
    pub(super) fn run_function_key_program(&mut self, program: &str, arguments: &str) {
        if program.is_empty() {
            return;
        }
        if Path::new(program).is_dir() {
            return self.navigate(self.active, PathBuf::from(program), true);
        }
        let path = self.panes[self.active].path.clone();
        let directory = if path.is_dir() { path } else { PathBuf::new() };
        let _ = (self.shell_runner)(Path::new(program), arguments, &directory);
    }
    pub(super) fn apply_preferences(&mut self, ctx: &egui::Context) {
        let theme = self.settings.text("Theme", "Black");
        // Skin base color table -> skin colors -> color table.
        let palette = super::system_commands::theme_palette(&theme);
        self.dark = palette.dark();
        let mut visuals = super::system_commands::palette_visuals(&palette);
        if let Ok(skin) = ndr::styles::Skin::load(&self.resources.join("Skins"), &theme) {
            if let Some(color) = skin
                .color("List", "lback")
                .or_else(|| skin.color("List", "lfileback"))
            {
                visuals.panel_fill = rgba(color);
                visuals.extreme_bg_color = rgba(color);
            }
            if let Some(color) = skin.color("List", "lfiletext") {
                visuals.override_text_color = Some(rgba(color));
            }
            if let Some(color) = skin.color("Menu", "mback") {
                visuals.window_fill = rgba(color);
            }
        }
        if self.settings.flag("UseNexusFileColorMap", false) {
            if let Some(map) = self
                .settings
                .values
                .get("NexusFileColorMapValues")
                .and_then(|v| v.as_object())
            {
                if let Some(color) = map
                    .get("NXBACK")
                    .and_then(|v| v.as_str())
                    .and_then(ndr::styles::parse_color)
                {
                    visuals.panel_fill = rgba(color);
                    visuals.extreme_bg_color = rgba(color);
                }
                if let Some(color) = map
                    .get("NXFORE")
                    .and_then(|v| v.as_str())
                    .and_then(ndr::styles::parse_color)
                {
                    visuals.override_text_color = Some(rgba(color));
                }
            }
        }
        ctx.set_visuals(visuals);
        // Original and C# ListFont (font name) plus the original style (ListFontStyle, TFontStyles: 1 bold, 2 italic): only when they change,
        // the font file of that face ("name Bold" etc., otherwise the nearest or regular face) is read and the "list" family rebuilt (the normal font when not found).
        let list_font = self.settings.text("ListFont", "Segoe UI");
        let style = (self.settings.values.get("ListFontStyle").and_then(|v| v.as_i64()).unwrap_or(0) & 0x0F) as u8;
        let key = format!("{list_font}\u{0}{}", style & 3);
        self.list_font_style = style;
        if self.list_font_name.as_deref() != Some(key.as_str()) {
            let data = native::styled_font_names(&list_font, style)
                .iter()
                .find_map(|name| (self.system.font_file)(name))
                .and_then(|(path, index)| native::mapped_file(&path).map(|bytes| (bytes, index)));
            super::set_app_fonts(ctx, data, style & 1 != 0);
            self.list_font_name = Some(key);
        }
        // C# version grid.FontSize = ListFontSize (pixels, default 14): the size applies only to the file list (panes.rs). Other text stays at the default 12 px.
        self.list_font_size = self
            .settings
            .values
            .get("ListFontSize")
            .and_then(|v| v.as_f64())
            .unwrap_or(ndr::settings::DEFAULT_LIST_FONT_PIXELS)
            .clamp(8.0, 32.0) as f32;
        ctx.style_mut(|style| {
            for text_style in [
                egui::TextStyle::Body,
                egui::TextStyle::Button,
                egui::TextStyle::Monospace,
            ] {
                style
                    .text_styles
                    .entry(text_style)
                    .or_insert_with(|| egui::FontId::proportional(super::BASE_TEXT_SIZE))
                    .size = super::BASE_TEXT_SIZE;
            }
        });
        self.dual = self.settings.flag("DualPaneMode", self.dual);
        // C# WindowOpacity: alpha applies at once (no restart). Window value = clamp(WindowAlpha/255, 0.35, 1.0).
        // During the startup check the window is off screen, so it is left alone.
        if self.options.smoke_seconds.is_none() {
            // C# ApplyWindowOpacity: alpha only when UseWindowAlpha is on, otherwise opaque (1.0).
            let raw = self.settings.values.get("WindowAlpha").and_then(|v| v.as_f64()).unwrap_or(210.0);
            let alpha = if self.settings.flag("UseWindowAlpha", false) { native::window_opacity(raw) } else { 1.0 };
            if (alpha - self.applied_alpha).abs() > f32::EPSILON {
                // When opaque from the start, the layered window (WS_EX_LAYERED) is not turned on.
                if !(self.applied_alpha < 0.0 && alpha >= 1.0) {
                    native::set_window_opacity(self.hwnd, alpha);
                }
                self.applied_alpha = alpha;
            }
        }
        if let Some(lifecycle) = &self.lifecycle {
            if self.options.smoke_seconds.is_none() {
                let _ = lifecycle.set_tray(self.settings.flag("HideWhenMinimized", false));
                let key = self.settings.text("ExplorerShortcutKey", "Q");
                let _ = lifecycle.set_hotkey(Some(&key));
            }
        }
    }
}
fn rgba(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}
