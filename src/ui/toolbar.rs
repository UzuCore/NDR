//! C# MainWindow.Toolbar: BuildMainToolbar (left group + right group, separators, quick search field, command buttons), GetToolbarLabel,
//! RefreshMainToolbarIcons (skin images 18x18, text when missing), ApplyMainToolbarButtonVisibility, ExecuteToolbarCommandAsync.
use super::*;
use ndr::commands::{ToolbarItem, ToolbarKind};

/// Entries of C# LanguageService.MenuKeys used by toolbar commands (createZip is not in the C# table).
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

/// C# LanguageService.ToAccessText: "&&" -> '&', '&' -> '_' (access key), '_' -> "__".
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

/// How WPF access key text looks: the first '_' is only a marker, and "__" is one '_'.
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
    /// C# LanguageService.MenuText (section omitted): the language value (otherwise the fallback text) through ToAccessText.
    fn menu_access_text(&self, key: &str, fallback: &str) -> String {
        to_access_text(&self.language_pack.raw(key).unwrap_or_else(|| fallback.to_owned()))
    }

    /// C# GetToolbarLabel. The returned text is the C# button text as is (including the '_' access key).
    pub(super) fn toolbar_label(&self, item: &ToolbarItem) -> String {
        let language = &self.language_pack;
        if !item.caption.is_empty() {
            return language.translate_pair(&item.caption, &item.caption);
        }
        if let Some(command) = item.action.strip_prefix("user:") {
            // A UserCommand without a name (<name>) follows the function key cell rule (FUN_00479970: the name of "[name]command", for a program the file name without extension).
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
                // C# Menu(): the fallback text as is when not in MenuKeys (no ToAccessText).
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

    /// C# RefreshMainToolbarIcons: images from the current theme's skin (NexusFileSkinService.LoadImage); None when missing or unreadable (text button).
    pub(super) fn toolbar_icon(&mut self, ctx: &egui::Context, icon: &str) -> Option<egui::TextureHandle> {
        if icon.is_empty() {
            return None;
        }
        let theme = self.settings.text("Theme", "Black");
        let key = format!("{theme}|{icon}");
        if let Some(cached) = self.toolbar_icons.get(&key) {
            return cached.clone();
        }
        let texture = ndr::styles::Skin::load(&self.resources.join("Skins"), &theme)
            .ok()
            .and_then(|skin| skin.image_path(icon))
            .and_then(|path| decode_png(&path).ok())
            .map(|(width, height, rgba)| ctx.load_texture(format!("toolbar:{key}"), egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba), egui::TextureOptions::LINEAR));
        self.toolbar_icons.insert(key, texture.clone());
        texture
    }

    /// Original skin of the current theme (black, classic, gray). conf.ini is reread only when the theme changes. For names that are not original skins or unreadable skins,
    /// the original default black (Black) is used.
    pub(super) fn current_skin(&mut self) -> Option<ndr::styles::Skin> {
        let theme = self.settings.text("Theme", "Black");
        if self.skin_cache.as_ref().map(|(name, _)| name.as_str()) != Some(theme.as_str()) {
            let folder = self.resources.join("Skins");
            let skin = ndr::styles::Skin::load(&folder, &theme).ok().or_else(|| ndr::styles::Skin::load(&folder, "Black").ok());
            self.skin_cache = Some((theme, skin));
        }
        self.skin_cache.as_ref().and_then(|(_, skin)| skin.clone())
    }

    /// BMP images of the original skin (tbbackimg, bbbackimg, bbdividerimg). None when missing or unreadable.
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
            .and_then(|bytes| ndr::styles::decode_bmp(&bytes).ok())
            .map(|(width, height, rgba)| ctx.load_texture(format!("skin:{key}"), egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba), egui::TextureOptions::NEAREST));
        self.toolbar_icons.insert(key, texture.clone());
        texture
    }

    /// Original pToolbar (skin theme, FUN_00444710): tiles the background tbbackimg horizontally, and places buttons (TNXImageButton, Stretch)
    /// as tbbtnwidth x tbbtnheight at top tbtopoffset (default 4). Separator cells are 15. Items without an image use the skin tb_noimage.png.
    pub(super) fn skin_toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, skin: &ndr::styles::Skin) {
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
        self.favorites_button = None;
        for (item, x) in left.iter().zip(left_x).chain(right.iter().zip(right_x)) {
            let slot = egui::Rect::from_min_size(egui::pos2(rect.left() + x, rect.top() + top), egui::vec2(width_of(item), button.y));
            if item.action == "favorites" {
                self.favorites_button = Some(slot.left_bottom());
            }
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
        // Original FUN_00444710: kind 3 (Container) has DropDownMenu = pmToolBar (+0x874). A sub-item menu below the button.
        if let Some((index, position)) = container_at {
            return self.show_toolbar_container(index, Some(position));
        }
        // Original FUN_00444710: a button whose command is Favorites has DropDownMenu = pmFavorites (+0x404). Clicking opens the favorites menu below the button.
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
                let response = ui.put(field, egui::TextEdit::singleline(&mut self.toolbar_query).id(egui::Id::new("ndr-toolbar-query")).char_limit(1024));
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let text = self.toolbar_query.clone();
                    self.quick_filter(self.active, &text);
                }
            }
            ToolbarKind::Command => {
                let label = access_display(&self.toolbar_label(item));
                let icon = self.toolbar_icon(ctx, &item.icon).or_else(|| self.toolbar_icon(ctx, "tb_noimage.png"));
                // Original FUN_00444710: Back and Forward buttons (+0xd70, +0xd74) have Enabled = False without history. TNXImageButton.Paint
                // draws a disabled button with constant alpha 0x82 (cell 2 when there are three or more cells), and it cannot be clicked.
                let enabled = match item.action.as_str() {
                    "historyBack" => !self.panes[self.active].back.is_empty(),
                    "historyForward" => !self.panes[self.active].forward.is_empty(),
                    _ => true,
                };
                let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
                let response = ui.interact(slot, egui::Id::new(("ndr-skin-toolbar", item.action.as_str(), slot.left() as i32)), sense);
                let drop_down = item.action == "favorites" || item.action.starts_with("container:");
                let pressed = enabled && (response.is_pointer_button_down_on() || response.clicked()) && ui.input(|i| i.pointer.primary_pressed());
                match icon {
                    Some(texture) => {
                        // Original TNXImageButton.Paint (0x7c3adc): with a single image cell (toolbar image) hovering changes nothing,
                        // and pressing darkens each cell color via FUN_007c39e4: color x 6 / 10 + black x 4 / 10.
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
                if response.on_hover_text(self.toolbar_hint(&item.action, &label)).clicked() && !drop_down {
                    *clicked = Some(item.action.clone());
                }
                // Original TNXImageButton with a DropDownMenu (Favorites, Container): MouseDown 0x7c45b8 → DropDown 0x7c49ac → Click 0x7c4860
                // pops the menu up at the button's bottom left on the press itself. While another menu is open that press only closes it.
                if drop_down && pressed && self.popup.is_none() {
                    *clicked = Some(item.action.clone());
                }
            }
        }
    }

    /// Button tooltips: BACKHINT / FORWARDHINT of original FUN_00486694 ("Back (%s)", %s = the path the button opens). Others use the button text.
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

    /// C# ExecuteToolbarCommandAsync: "function:name" is an internal command (ExecuteFunctionKeyMacroAsync), the rest are menu commands.
    pub(super) fn toolbar_command(&mut self, action: &str, ctx: &egui::Context) {
        if let Some(name) = action.strip_prefix("function:") {
            let id = format!("#{name}");
            if !function_key_editor::INTERNAL_COMMANDS.iter().any(|(candidate, _)| candidate.eq_ignore_ascii_case(&id)) || !self.original_internal(name, ctx) {
                self.warn("toolbar.xml refers to an unavailable internal command.", "toolbar.xml refers to an unavailable internal command.");
            }
            return;
        }
        if let Some(command) = action.strip_prefix("user:") {
            // Original FUN_0044381c (form, command, not internal): runs like a function key user command ("[name]program arguments", macros).
            let binding = ndr::commands::FunctionBinding { name: String::new(), command: command.to_owned(), parameters: String::new(), internal: false, legacy: true };
            self.run_binding(&binding, ctx);
            return;
        }
        if let Some(index) = action.strip_prefix("container:").and_then(|n| n.parse::<usize>().ok()) {
            return self.show_toolbar_container(index, None);
        }
        self.dispatch(action, ctx);
    }

    /// Original pmToolBarPopup (FUN_00444e60): one menu row (name) per sub-item of a Container button, Separator as a separator.
    /// Clicking runs FUN_00444e04 = FUN_0044381c(form, command, kind == InternalCommand). Opens below the button (`position`).
    pub(super) fn show_toolbar_container(&mut self, index: usize, position: Option<egui::Pos2>) {
        let Some(container) = self.toolbar.items.get(index) else { return };
        let mut items = Vec::new();
        for child in &container.children {
            if child.kind == ToolbarKind::Separator {
                items.push(super::favorites_dialog::PopupItem::Separator);
                continue;
            }
            let (label, access) = super::favorites_dialog::access_key(&self.toolbar_label(child));
            items.push(super::favorites_dialog::PopupItem::Entry { label, access, gesture: String::new(), enabled: true, checked: false, action: super::favorites_dialog::PopupAction::Toolbar(child.action.clone()) });
        }
        if items.is_empty() {
            return;
        }
        self.popup = Some(super::favorites_dialog::PopupMenu { items, position, highlight: None, opened: true });
    }
}

/// Original FUN_00444710 (constructor 0x47d7ac: left margin 4, right margin 6): left items start at 4, right (align=right) items at
/// (width - 6 - sum(width + 4)); in both, the next item comes width + 4 later. Returns each item's left x (relative to the toolbar).
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

/// Toolbar image (PNG) -> (width, height, RGBA).
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
        let skin = ndr::styles::Skin::load(&skins, "Black").expect("the Black skin ships");
        let path = skin.image_path("tb_back.png").expect("tb_back.png ships in the skin");
        let (width, height, rgba) = decode_png(&path).unwrap();
        assert!(width > 0 && height > 0 && rgba.len() == width * height * 4);
    }
}
