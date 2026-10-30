//! C# MainWindow.Toolbar: BuildMainToolbar(왼쪽 묶음 + 오른쪽 묶음, 구분선, 빠른 찾기 칸, 명령 단추), GetToolbarLabel,
//! RefreshMainToolbarIcons(스킨 그림 18×18, 없으면 글), ApplyMainToolbarButtonVisibility, ExecuteToolbarCommandAsync.
use super::*;
use ndir::commands::{ToolbarItem, ToolbarKind};

/// C# LanguageService.MenuKeys 가운데 도구 모음 명령이 쓰는 것(createZip은 C# 표에 없다).
fn menu_key(action: &str) -> Option<&'static str> {
    Some(match action {
        "historyBack" => "Menu.miHistoryBack",
        "historyForward" => "Menu.miHistoryForward",
        "goParent" => "Menu.miToParent",
        "newFolder" => "Menu.miNewFolder",
        "rename" => "Menu.miRename",
        "copyToOther" => "Menu.miCopyTo",
        "moveToOther" => "Menu.miMoveTo",
        "deleteRecycle" => "Menu.miDelete",
        "extractSelected" => "Menu.miArcExtract",
        "advancedRename" => "Menu.miAdvRename",
        "saveList" => "Menu.miSaveList",
        "split" => "Menu.miSplitFile",
        "join" => "Menu.miJoinFile",
        "desktop" => "Menu.miToDesktop",
        "documents" => "Menu.miToDocuments",
        "pictures" => "Menu.miToPictures",
        "music" => "Menu.miToMusic",
        "videos" => "Menu.miToVideos",
        "favorites" => "Menu.miFavorites",
        "openRecycle" => "Menu.miRecycleBin",
        "search" => "Menu.miSearchFile",
        "browseFolder" => "Menu.miFolderTree",
        "toggleHidden" => "Menu.miShowHidden",
        "newTab" => "Menu.miNewTab",
        _ => return None,
    })
}

/// C# LanguageService.ToAccessText: "&&" → '&', '&' → '_'(접근 키), '_' → "__".
pub(super) fn to_access_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' if chars.peek() == Some(&'&') => {
                chars.next();
                out.push('&');
            }
            '&' => out.push('_'),
            '_' => out.push_str("__"),
            other => out.push(other),
        }
    }
    out
}

/// WPF 접근 키 글이 보이는 모양: 처음 나오는 '_' 하나는 표시만 하고, "__"는 '_' 하나.
pub(super) fn access_display(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut marker_used = false;
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
                continue;
            }
            if !marker_used {
                marker_used = true;
                continue;
            }
        }
        out.push(c);
    }
    out
}

impl App {
    /// C# LanguageService.MenuText(section 생략): 언어 값(없으면 대체 글)을 ToAccessText로.
    fn menu_access_text(&self, key: &str, fallback: &str) -> String {
        to_access_text(&self.language_pack.raw(key).unwrap_or_else(|| fallback.to_owned()))
    }

    /// C# GetToolbarLabel. 돌려주는 글은 C#의 단추 글(접근 키 '_' 포함) 그대로다.
    pub(super) fn toolbar_label(&self, item: &ToolbarItem) -> String {
        let language = &self.language_pack;
        if !item.caption.is_empty() {
            return language.translate_pair(&item.caption, &item.caption);
        }
        if let Some(command) = item.action.strip_prefix("user:") {
            // 이름(<name>)이 없는 UserCommand는 기능키 칸과 같은 규칙(FUN_00479970: "[이름]명령"의 이름, 프로그램은 확장자 뺀 파일 이름).
            let resource = |name: &str| language.raw(&format!("RES.{name}"));
            return super::user_commands::original_function_key_caption(command, &resource);
        }
        if let Some(name) = item.action.strip_prefix("function:") {
            let id = format!("#{name}");
            if let Some((_, command)) = function_key_editor::INTERNAL_COMMANDS.iter().find(|(candidate, _)| candidate.eq_ignore_ascii_case(&id)) {
                if !command.trim().is_empty() {
                    return language.translate_pair(command, command);
                }
            }
        }
        let action = item.action.as_str();
        let fallback = match action {
            "historyBack" => "뒤로",
            "historyForward" => "앞으로",
            "goParent" => "상위",
            "newFolder" => "새 폴더",
            "rename" => "이름 변경",
            "copyToOther" => "복사",
            "moveToOther" => "이동",
            "deleteToolbar" => "삭제",
            "extractToolbar" => "압축 풀기",
            "createZip" => "ZIP 만들기",
            "batchRename" => "일괄 변경",
            "saveList" => "목록 저장",
            "split" => "분할",
            "join" => "합치기",
            "desktop" => "바탕 화면",
            "documents" => "문서",
            "pictures" => "사진",
            "music" => "음악",
            "videos" => "동영상",
            "favorites" => "즐겨찾기",
            "openRecycle" => "휴지통",
            "search" => "찾기",
            "browseFolder" => "폴더 선택",
            "toggleHidden" => "숨김 파일",
            "newTab" => "새 탭",
            "closeTab" => "탭 닫기",
            other => other,
        };
        let menu_action = match action {
            "deleteToolbar" => "deleteRecycle",
            "extractToolbar" => "extractSelected",
            "batchRename" => "advancedRename",
            other => other,
        };
        let mut text = match menu_action {
            "newTab" => self.menu_access_text("Menu.pmiNewTab", fallback),
            "closeTab" => self.menu_access_text("Menu.pmiCloseTab", fallback),
            other => {
                let translated = language.translate_pair(fallback, fallback);
                // C# Menu(): MenuKeys에 없으면 대체 글 그대로(ToAccessText 없음).
                match menu_key(other) {
                    Some(key) => self.menu_access_text(key, &translated),
                    None => translated,
                }
            }
        };
        text = text.replace('&', "");
        let chars: Vec<char> = text.chars().collect();
        if chars.len() >= 3 && chars[1] == '.' && chars[2] == ' ' && chars[0].is_alphanumeric() {
            text = chars[3..].iter().collect();
        }
        text
    }

    /// C# ApplyMainToolbarButtonVisibility.
    pub(super) fn toolbar_button_visible(&self, action: &str) -> bool {
        match action {
            "deleteToolbar" => self.settings.flag("ShowDeleteToolbarButton", true),
            "newTab" | "closeTab" => self.settings.flag("UseTabs", true),
            _ => true,
        }
    }

    /// C# RefreshMainToolbarIcons: 지금 테마의 스킨에서 그림(NexusFileSkinService.LoadImage), 없거나 못 읽으면 None(글 단추).
    pub(super) fn toolbar_icon(&mut self, ctx: &egui::Context, icon: &str) -> Option<egui::TextureHandle> {
        if icon.is_empty() {
            return None;
        }
        let theme = self.settings.text("Theme", "Black");
        let key = format!("{theme}|{icon}");
        if let Some(cached) = self.toolbar_icons.get(&key) {
            return cached.clone();
        }
        let texture = ndir::styles::Skin::load(&self.resources.join("Skins"), &theme)
            .ok()
            .and_then(|skin| skin.image_path(icon))
            .and_then(|path| decode_png(&path).ok())
            .map(|(width, height, rgba)| ctx.load_texture(format!("toolbar:{key}"), egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba), egui::TextureOptions::LINEAR));
        self.toolbar_icons.insert(key, texture.clone());
        texture
    }

    /// 지금 테마가 원본 스킨인가(이미 읽어 둔 skin_cache로, &self에서). 테마가 바뀐 첫 장면은 current_skin이 다시 읽을 때까지 false일 수 있다.
    pub(super) fn skin_active(&self) -> bool {
        let theme = self.settings.text("Theme", "Black");
        self.skin_cache.as_ref().is_some_and(|(name, skin)| *name == theme && skin.is_some())
    }

    /// 지금 테마의 원본 스킨(검정·클래식·회색). 테마가 바뀔 때만 conf.ini를 다시 읽는다. C# 판 테마면 None.
    pub(super) fn current_skin(&mut self) -> Option<ndir::styles::Skin> {
        let theme = self.settings.text("Theme", "Black");
        if self.skin_cache.as_ref().map(|(name, _)| name.as_str()) != Some(theme.as_str()) {
            let skin = ndir::styles::Skin::load(&self.resources.join("Skins"), &theme).ok();
            self.skin_cache = Some((theme, skin));
        }
        self.skin_cache.as_ref().and_then(|(_, skin)| skin.clone())
    }

    /// 원본 스킨의 BMP 그림(tbbackimg, bbbackimg, bbdividerimg). 없거나 못 읽으면 None.
    pub(super) fn skin_bitmap(&mut self, ctx: &egui::Context, file: &str) -> Option<egui::TextureHandle> {
        let theme = self.settings.text("Theme", "Black");
        let key = format!("bmp|{theme}|{file}");
        if let Some(cached) = self.toolbar_icons.get(&key) {
            return cached.clone();
        }
        let texture = self
            .current_skin()
            .and_then(|skin| skin.image_path(file))
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| ndir::styles::decode_bmp(&bytes).ok())
            .map(|(width, height, rgba)| ctx.load_texture(format!("skin:{key}"), egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba), egui::TextureOptions::NEAREST));
        self.toolbar_icons.insert(key, texture.clone());
        texture
    }

    /// 원본 pToolbar(스킨 테마, FUN_00444710): 배경 tbbackimg를 가로로 이어 그리고, 단추(TNXImageButton, Stretch)는
    /// tbbtnwidth × tbbtnheight로 위 tbtopoffset(처음 값 4)에 놓는다. 구분선 칸은 15. 그림이 없는 항목은 스킨 tb_noimage.png.
    pub(super) fn skin_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, skin: &ndir::styles::Skin) {
        let rect = ui.max_rect();
        if let Some(back) = skin.values.get("toolbar.tbbackimg").cloned().and_then(|file| self.skin_bitmap(ctx, &file)) {
            let tile_width = back.size_vec2().x.max(1.0);
            let mut x = rect.left();
            while x < rect.right() {
                let tile = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(tile_width, rect.height()));
                ui.painter().image(back.id(), tile, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
                x += tile_width;
            }
        }
        let button = egui::vec2(skin.positive_integer("Toolbar", "tbbtnwidth", 35) as f32, skin.positive_integer("Toolbar", "tbbtnheight", 35) as f32);
        let top = skin.positive_integer("Toolbar", "tbtopoffset", 4) as f32;
        let items: Vec<ToolbarItem> = self.toolbar.items.iter().filter(|item| item.kind != ToolbarKind::Command || self.toolbar_button_visible(&item.action)).cloned().collect();
        let width_of = |item: &ToolbarItem| match item.kind {
            ToolbarKind::Separator => 15.0,
            ToolbarKind::QuickSearch => item.width as f32,
            ToolbarKind::Command => button.x,
        };
        let (left, right): (Vec<ToolbarItem>, Vec<ToolbarItem>) = items.into_iter().partition(|item| !item.align_right);
        let (left_x, right_x) = toolbar_slots(&left.iter().map(width_of).collect::<Vec<_>>(), &right.iter().map(width_of).collect::<Vec<_>>(), rect.width());
        let mut clicked = None;
        let mut favorites_at = None;
        let mut container_at = None;
        for (item, x) in left.iter().zip(left_x).chain(right.iter().zip(right_x)) {
            let slot = egui::Rect::from_min_size(egui::pos2(rect.left() + x, rect.top() + top), egui::vec2(width_of(item), button.y));
            let before = clicked.is_some();
            self.skin_toolbar_item(ui, ctx, item, slot, &mut clicked);
            if !before && clicked.as_deref() == Some("favorites") {
                favorites_at = Some(slot.left_bottom());
            }
            if !before {
                if let Some(index) = clicked.as_deref().and_then(|a| a.strip_prefix("container:")).and_then(|n| n.parse::<usize>().ok()) {
                    container_at = Some((index, slot.left_bottom()));
                }
            }
        }
        // 원본 FUN_00444710: 형식 3(Container)은 DropDownMenu = pmToolBar(+0x874). 단추 아래에 하위 항목 메뉴.
        if let Some((index, position)) = container_at {
            return self.show_toolbar_container(index, Some(position));
        }
        // 원본 FUN_00444710: command가 Favorites인 단추는 DropDownMenu = pmFavorites(+0x404). 누르면 단추 아래에 즐겨찾기 메뉴가 열린다.
        if let Some(position) = favorites_at {
            self.show_favorites_menu();
            if let Some(popup) = self.popup.as_mut() {
                popup.position = Some(position);
            }
            return;
        }
        if let Some(action) = clicked {
            self.toolbar_command(&action, ctx);
        }
    }

    fn skin_toolbar_item(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, item: &ToolbarItem, slot: egui::Rect, clicked: &mut Option<String>) {
        match item.kind {
            ToolbarKind::Separator => {}
            ToolbarKind::QuickSearch => {
                let field = egui::Rect::from_center_size(slot.center(), egui::vec2(slot.width(), 21.0));
                let response = ui.put(field, egui::TextEdit::singleline(&mut self.toolbar_query).id(egui::Id::new("ndir-toolbar-query")).char_limit(1024));
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let text = self.toolbar_query.clone();
                    self.quick_filter(self.active, &text);
                }
            }
            ToolbarKind::Command => {
                let label = access_display(&self.toolbar_label(item));
                let icon = self.toolbar_icon(ctx, &item.icon).or_else(|| self.toolbar_icon(ctx, "tb_noimage.png"));
                // 원본 FUN_00444710: Back·Forward 단추(+0xd70·+0xd74)는 기록이 없으면 Enabled = False. TNXImageButton.Paint는
                // 꺼진 단추를 상수 알파 0x82로 그리고(칸이 셋 이상이면 칸 2), 누를 수 없다.
                let enabled = match item.action.as_str() {
                    "historyBack" => !self.panes[self.active].back.is_empty(),
                    "historyForward" => !self.panes[self.active].forward.is_empty(),
                    _ => true,
                };
                let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
                let response = ui.interact(slot, egui::Id::new(("ndir-skin-toolbar", item.action.as_str(), slot.left() as i32)), sense);
                match icon {
                    Some(texture) => {
                        // 원본 TNXImageButton.Paint(0x7c3adc): 그림 칸이 하나(도구 모음 그림)면 포인터를 올려도 그대로이고,
                        // 누르면 FUN_007c39e4로 칸마다 색 × 6 / 10 + 검정 × 4 / 10(어둡게).
                        let tint = if !enabled {
                            Color32::WHITE.gamma_multiply(130.0 / 255.0)
                        } else if response.is_pointer_button_down_on() && response.hovered() {
                            Color32::from_gray(153)
                        } else {
                            Color32::WHITE
                        };
                        ui.painter().image(texture.id(), slot, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), tint);
                    }
                    None => {
                        ui.painter().text(slot.center(), egui::Align2::CENTER_CENTER, &label, egui::FontId::proportional(11.0), ui.visuals().text_color());
                    }
                }
                if response.on_hover_text(self.toolbar_hint(&item.action, &label)).clicked() {
                    *clicked = Some(item.action.clone());
                }
            }
        }
    }

    /// C# BuildMainToolbar: AlignRight 항목은 오른쪽 묶음에(그 안의 순서는 파일 순서). 도구 모음이 창보다 넓으면 가로로 굴린다
    /// (C# ToolbarScroller: Shift+휠, egui 기본 가로 굴리기 수식 키도 Shift).
    pub(super) fn user_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let items = self.toolbar.items.clone();
        let (left, right): (Vec<ToolbarItem>, Vec<ToolbarItem>) = items.into_iter().partition(|item| !item.align_right);
        let available = ui.available_width();
        let (left_width, right_width) = self.toolbar_widths;
        let fits = left_width + right_width + 16.0 <= available;
        let mut clicked = None;
        let mut measured = (0.0, 0.0);
        egui::ScrollArea::horizontal().id_salt("ndir-toolbar").auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal(|ui| {
                let start = ui.cursor().min.x;
                for item in &left {
                    self.toolbar_item(ui, ctx, item, &mut clicked);
                }
                measured.0 = ui.cursor().min.x - start;
                if fits {
                    let size = egui::vec2((available - measured.0).max(0.0), ui.available_height().max(24.0));
                    ui.allocate_ui_with_layout(size, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for item in right.iter().rev() {
                            self.toolbar_item(ui, ctx, item, &mut clicked);
                        }
                        measured.1 = ui.min_rect().width();
                    });
                } else {
                    let start = ui.cursor().min.x;
                    for item in &right {
                        self.toolbar_item(ui, ctx, item, &mut clicked);
                    }
                    measured.1 = ui.cursor().min.x - start;
                }
            });
        });
        self.toolbar_widths = measured;
        if let Some(action) = clicked {
            self.toolbar_command(&action, ctx);
        }
    }

    fn toolbar_item(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, item: &ToolbarItem, clicked: &mut Option<String>) {
        match item.kind {
            ToolbarKind::Separator => {
                ui.separator();
            }
            ToolbarKind::QuickSearch => {
                // C# 도구 모음 빠른 찾기 칸: 최대 1024자, Enter에서 활성 패널에 빠른 필터(QuickFilterAsync).
                let response = ui.add(egui::TextEdit::singleline(&mut self.toolbar_query).id(egui::Id::new("ndir-toolbar-query")).char_limit(1024).desired_width(item.width as f32));
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let text = self.toolbar_query.clone();
                    self.quick_filter(self.active, &text);
                }
                let tip = self.language_pack.translate_pair("현재 폴더 빠른 이름 필터 (Enter)", "Quick name filter for the current folder (Enter)");
                response.on_hover_text(tip);
            }
            ToolbarKind::Command => {
                if !self.toolbar_button_visible(&item.action) {
                    return;
                }
                let label = access_display(&self.toolbar_label(item));
                let response = match self.toolbar_icon(ctx, &item.icon) {
                    Some(texture) => ui.add(egui::Button::image(egui::Image::new(&texture).fit_to_exact_size(egui::vec2(18.0, 18.0)))),
                    None => ui.add(egui::Button::new(RichText::new(&label).size(11.0))),
                };
                if response.on_hover_text(self.toolbar_hint(&item.action, &label)).clicked() {
                    *clicked = Some(item.action.clone());
                }
            }
        }
    }

    /// 단추 도움말: 원본 FUN_00486694의 BACKHINT / FORWARDHINT("뒤로 (%s)", %s = 그 단추가 열 경로). 그 밖은 단추 글.
    pub(super) fn toolbar_hint(&self, action: &str, label: &str) -> String {
        let pane = &self.panes[self.active];
        let (key, fallback, target) = match action {
            "historyBack" => ("RES.BACKHINT", "Back (%s)", pane.back.last()),
            "historyForward" => ("RES.FORWARDHINT", "Foward (%s)", pane.forward.last()),
            "deleteToolbar" => return self.language_pack.text("RES.RECYCLEBIN_HINT", "Click: Send to RecycleBin\nShift+Click: Delete\nShift+Ctrl+Click:Shred\nCtrl+Alt+Click:Open RecycleBin"),
            _ => return label.to_owned(),
        };
        match target {
            Some(path) => self.language_pack.text(key, fallback).replace("%s", &path.to_string_lossy()),
            None => label.to_owned(),
        }
    }

    /// C# ExecuteToolbarCommandAsync: "function:이름"은 내부 명령(ExecuteFunctionKeyMacroAsync), 나머지는 메뉴 명령.
    pub(super) fn toolbar_command(&mut self, action: &str, ctx: &egui::Context) {
        if let Some(name) = action.strip_prefix("function:") {
            let id = format!("#{name}");
            if !function_key_editor::INTERNAL_COMMANDS.iter().any(|(candidate, _)| candidate.eq_ignore_ascii_case(&id)) || !self.original_internal(name, ctx) {
                self.warn("toolbar.xml refers to an unavailable internal command.", "toolbar.xml refers to an unavailable internal command.");
            }
            return;
        }
        if let Some(command) = action.strip_prefix("user:") {
            // 원본 FUN_0044381c(폼, 명령, 내부 명령 아님): 기능키의 사용자 명령("[이름]프로그램 인수", 매크로)과 같은 실행.
            let binding = ndir::commands::FunctionBinding { name: String::new(), command: command.to_owned(), parameters: String::new(), internal: false, legacy: true };
            self.run_binding(&binding, ctx);
            return;
        }
        if let Some(index) = action.strip_prefix("container:").and_then(|n| n.parse::<usize>().ok()) {
            return self.show_toolbar_container(index, None);
        }
        self.dispatch(action, ctx);
    }

    /// 원본 pmToolBarPopup(FUN_00444e60): Container 단추의 하위 항목마다 메뉴 줄(이름), Separator는 구분선.
    /// 누르면 FUN_00444e04 = FUN_0044381c(폼, 명령, 형식 == InternalCommand). 단추 아래(`position`)에 펼친다.
    pub(super) fn show_toolbar_container(&mut self, index: usize, position: Option<egui::Pos2>) {
        let Some(container) = self.toolbar.items.get(index) else { return };
        let mut items = Vec::new();
        for child in &container.children {
            if child.kind == ToolbarKind::Separator {
                items.push(super::favorites_dialog::PopupItem::Separator);
                continue;
            }
            let (label, access) = super::favorites_dialog::access_key(&self.toolbar_label(child));
            items.push(super::favorites_dialog::PopupItem::Entry { label, access, gesture: String::new(), enabled: true, action: super::favorites_dialog::PopupAction::Toolbar(child.action.clone()) });
        }
        if items.is_empty() {
            return;
        }
        self.popup = Some(super::favorites_dialog::PopupMenu { items, position, highlight: None, opened: true });
    }
}

/// 원본 FUN_00444710(생성자 0x47d7ac: 왼쪽 여백 4, 오른쪽 여백 6): 왼쪽 항목은 4부터, 오른쪽(align=right) 항목은
/// (너비 − 6 − Σ(폭 + 4))부터, 둘 다 다음 항목은 폭 + 4 뒤. 돌려주는 값은 각 항목의 왼쪽 x(도구 모음 기준).
pub(super) fn toolbar_slots(left: &[f32], right: &[f32], width: f32) -> (Vec<f32>, Vec<f32>) {
    let place = |widths: &[f32], start: f32| {
        let mut x = start;
        widths
            .iter()
            .map(|w| {
                let at = x;
                x += w + 4.0;
                at
            })
            .collect::<Vec<_>>()
    };
    (place(left, 4.0), place(right, width - 6.0 - right.iter().map(|w| w + 4.0).sum::<f32>()))
}

/// 도구 모음 그림(PNG) → (너비, 높이, RGBA).
fn decode_png(path: &Path) -> Result<(usize, usize, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let size = reader.output_buffer_size().ok_or("PNG is too large")?;
    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let (width, height) = (info.width as usize, info.height as usize);
    if width == 0 || height == 0 || width > 512 || height > 512 {
        return Err("Unsupported toolbar image size".into());
    }
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return Err("Indexed PNG was not expanded".into()),
    };
    Ok((width, height, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_text_follows_wpf() {
        assert_eq!(to_access_text("&C. Copy && paste_x"), "_C. Copy & paste__x");
        assert_eq!(access_display("_C. Copy & paste__x"), "C. Copy & paste_x");
        assert_eq!(access_display("plain"), "plain");
    }

    #[test]
    fn toolbar_icons_from_the_shipped_skins_decode() {
        let skins = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Resources").join("Skins");
        let skin = ndir::styles::Skin::load(&skins, "Black").expect("the Black skin ships");
        let path = skin.image_path("tb_back.png").expect("tb_back.png ships in the skin");
        let (width, height, rgba) = decode_png(&path).unwrap();
        assert!(width > 0 && height > 0 && rgba.len() == width * height * 4);
    }
}
