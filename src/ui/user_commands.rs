use super::*;


/// 원본 FUN_00479970(기능키 칸 이름, [FuncKey] 값 하나): 빈 값은 빈 이름. "[이름]명령"이면 [ ] 안 글(비었으면 [ 뒤 전부),
/// "#내부 명령"이면 내부 명령 목록에 있을 때 RES FUNC_<이름 대문자>(FUN_004586ac, 없으면 # 뒤 글), 그 밖(프로그램)은
/// PathStripPath·PathRemoveExtension한 파일 이름(확장자에 빈칸이 있으면 확장자로 보지 않음).
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

/// 원본 기본 기능키(default\nexusfile.ini [FuncKey]): 지정이 없을 때의 내부 명령.
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

/// ALTFUNCTIONS/CTRLFUNCTIONS 단추를 누르면 그 글쇠(같은 Alt/Ctrl과 함께)를 누른 것과 같다.
fn modifier_function_key(key: &str) -> Option<egui::Key> {
    match key {
        "<-" => Some(egui::Key::ArrowLeft),
        "->" => Some(egui::Key::ArrowRight),
        other => egui::Key::from_name(other),
    }
}

impl App {
    /// 원본 ALTFUNCTIONS/CTRLFUNCTIONS("글쇠=이름" 줄): Alt만 누르면 앞의 것, Ctrl만 누르면 뒤의 것. 그 밖은 None(F1~F12 단추).
    pub(super) fn modifier_functions(&self, modifiers: egui::Modifiers) -> Option<Vec<(String, String)>> {
        let text = match (modifiers.alt, modifiers.ctrl, modifiers.shift) {
            (true, false, false) => self.language_pack.text("RES.ALTFUNCTIONS", "C=Copy\nM=Move\nD=Delete\nR=Rename\nK=New Folder\nU=Select All\nV=View Text\nB=Edit Text"),
            (false, true, false) => self.language_pack.text(
                "RES.CTRLFUNCTIONS",
                "Z=Change Attr\nA=Add To Archive\nE=Extract\nD=Desktop\nG=Documents\nB=RecycleBin\n<-=Copy To Left\n->=Copy To Right",
            ),
            _ => return None,
        };
        // "<-=..."처럼 글쇠 자리에 '='가 올 수 없으므로 첫 '='에서 나눈다.
        Some(text.lines().filter_map(|line| line.split_once('=')).map(|(key, name)| (key.to_owned(), name.to_owned())).collect())
    }

    /// 원본 bbFunctions(TNXButtonBar, 스킨 테마): FUN_00430714의 F2~F8 일곱 칸("F%d %s"), Alt·Ctrl을 누르면 ALTFUNCTIONS·CTRLFUNCTIONS 칸.
    /// Paint(0x7c1568): 칸 너비 = 줄 너비 / 칸 수, 배경 bbbackimg 가로 반복. DrawItem(0x7c0ec0): 왼쪽 + 5에 글쇠 글(bbkeytext),
    /// 그 너비 + 8 뒤에 이름(bbtext, 포인터가 있으면 bbover), 누른 칸은 (1, 1) 밀림, 칸 오른쪽 끝 − 2에 bbdividerimg(세로 가운데).
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
                    // 원본 FUN_00479970: [FuncKey] 값으로 이름을 정한다. C# 판 지정에 이름이 있으면 그 이름, 지정이 없으면 원본 기본 값.
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
            // 칸 글은 오른쪽 끝 구분선(− 2) 앞까지.
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
            // 원본 bbFunctionsContextPopup(0x4373ec): 기본 칸(Alt·Ctrl 칸이 아닐 때)을 오른쪽 단추로 눌러도 그 기능키를 실행한다
            // (FUN_004308a0, 메뉴는 띄우지 않는다).
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

    /// C# RunFunctionKeyButtonAsync: 지정이 있으면 그 매크로, 없으면 기본 동작, 그 밖은 "설정된 동작이 없습니다."
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
            // 원본은 [FuncKey]가 없으면 default\nexusfile.ini 값(F2 #Rename … F8 #RecycleBin)을 쓴다. 고친 것: 이 판은 기본 값을
            // 기능키 줄 이름에만 쓰고 실행하지 않아, 새 프로필에서 F3(복사)·F4(이동) 등이 아무 일도 하지 않았다.
            // 프로필에 그 글쇠 칸이 있으면(빈 값이어도) 그 값을 따른다.
            None if !self.function_key_configured(key) && !original_default_function_key(key).is_empty() => {
                let command = original_default_function_key(key);
                ndr::commands::FunctionBinding { name: key.to_owned(), command: command.to_owned(), parameters: String::new(), internal: true, legacy: true }
            }
            None => return false,
        };
        self.run_binding(&binding, ctx);
        true
    }

    /// 프로필에 그 기능키 칸(FunctionKeys 또는 FunctionKeyBindings)이 있는가.
    fn function_key_configured(&self, key: &str) -> bool {
        ["FunctionKeys", "FunctionKeyBindings"].iter().any(|field| self.settings.values.get(*field).and_then(|v| v.as_object()).is_some_and(|map| map.keys().any(|k| k.eq_ignore_ascii_case(key))))
    }

    /// 기능키·도구 모음 UserCommand의 지정 하나를 실행한다(C# ExecuteFunctionKeyMacroAsync, 원본 FUN_0044381c).
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
                    // C# ExecuteFunctionKeyMacroAsync: TakeParameterPrompt면 InputDialog(TITLE_EDITPARAM, MSG_EDITPARAM, 인수), 취소면 실행 안 함.
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

    /// C# ExecuteFunctionKeyMacroAsync(인수 편집 뒤): 빈 프로그램은 아무것도 안 함, 있는 폴더면 활성 패널에서 열기,
    /// 아니면 ShellExecuteEx(프로그램, 인수, 활성 패널 폴더(없으면 지정 없음)). 실패는 셸이 직접 알리므로 따로 알리지 않는다.
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
        // 스킨의 바탕 색 표 → 스킨 색 → 색상표.
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
        // 원본·C# 판 ListFont(글꼴 이름)와 원본 모양(ListFontStyle, TFontStyles: 1 굵게·2 기울임): 바뀌었을 때만
        // 그 꼴의 글꼴 파일("이름 Bold" 등, 없으면 가까운 꼴·보통 꼴)을 읽어 "list" 묶음을 다시 만든다(못 찾으면 보통 글꼴).
        let list_font = self.settings.text("ListFont", "Segoe UI");
        let style = (self.settings.values.get("ListFontStyle").and_then(|v| v.as_i64()).unwrap_or(0) & 0x0F) as u8;
        let key = format!("{list_font}\u{0}{}", style & 3);
        self.list_font_style = style;
        if self.list_font_name.as_deref() != Some(key.as_str()) {
            let data = native::styled_font_names(&list_font, style)
                .iter()
                .find_map(|name| (self.system.font_file)(name))
                .and_then(|(path, index)| std::fs::read(path).ok().map(|bytes| (bytes, index)));
            super::set_app_fonts(ctx, data, style & 1 != 0);
            self.list_font_name = Some(key);
        }
        // C# 판 grid.FontSize = ListFontSize(화소, 처음 값 14): 크기는 파일 목록에만 쓴다(panes.rs). 나머지 글은 처음 값 12화소 그대로.
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
        self.show_tree = self.settings.flag("ShowFolderTree", false);
        // C# WindowOpacity: 알파를 바로 적용(다시 시작 필요 없음). 창 글 = clamp(WindowAlpha/255, 0.35, 1.0).
        // 시작 확인 때는 화면 밖 창이라 건드리지 않는다.
        if self.options.smoke_seconds.is_none() {
            // C# ApplyWindowOpacity: UseWindowAlpha가 켜져 있을 때만 알파, 꺼져 있으면 불투명(1.0).
            let raw = self.settings.values.get("WindowAlpha").and_then(|v| v.as_f64()).unwrap_or(210.0);
            let alpha = if self.settings.flag("UseWindowAlpha", false) { native::window_opacity(raw) } else { 1.0 };
            if (alpha - self.applied_alpha).abs() > f32::EPSILON {
                // 처음부터 불투명이면 층 창(WS_EX_LAYERED)을 켜지 않는다.
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
