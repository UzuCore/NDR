//! Panel parts of the original skin themes (black, classic, gray).
//! - Address bar: original FUN_00481978 (a TNXPaintPanel of height 19 holding a TNXPathLabel plus two addressbtns.png buttons, width 20) and
//!   TNXPathLabel.Paint(0x7c9714)·DrawDivider(0x7c9694)·UpdateDisplayInfo(0x7c8c20, segment width = text width + 4).
//! - Panel border: skin [List] lframelocal·lframenet·lframearc·lframeftp·lframeinactive, lframewidth.
use super::pane_chrome::CrumbTarget;
use super::*;
use ndr::styles::Skin;

/// One address-bar segment: the visible text, where clicking it goes, and the local folder whose subfolders the chevron (>) shows (None if there is none).
pub(super) struct CrumbSegment {
    pub label: String,
    pub target: CrumbTarget,
    pub folder: Option<PathBuf>,
}

/// Original address-bar height (FUN_00481978 SetHeight 0x13) and end-button width (0x14).
pub(super) const ADDRESS_BAR_HEIGHT: f32 = 19.0;
const ADDRESS_BUTTON_WIDTH: f32 = 20.0;

/// Skin conf.ini [List] colors. The fallback used when a key is missing follows the default TColor the original passes to FUN_00463798 (e.g. lback·laback 141414,
/// latext 646464, lfolderdivider 3C3C3C, ldrivesframe 1A1A1A, lframetree 3C3C3C). Text/background keys whose original default is 0 (black)
/// (lfileback, lfiletext, lfolderback, menu colors) would make black text on a black background, so they are set to readable values (fixed). The bundled skin set has every key.
pub(super) fn skin_color(skin: &Skin, key: &str, fallback: Color32) -> Color32 {
    skin.color("List", key).map(|c| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])).unwrap_or(fallback)
}

/// Skin [Menu] colors (the order original FUN_00463914 reads: mmainback +0x20, mmaintext +0x24, mmainover +0x28, mmainoverback +0x2c,
/// mback +0x30, mtext +0x34, mborder +0x38, mdisabled +0x3c, mhighlight +0x40, mover +0x44, mrback +0x48, mrtext +0x4c,
/// mrover +0x50, mdivider +0x54).
fn menu_color(skin: &Skin, key: &str, fallback: Color32) -> Color32 {
    skin.color("Menu", key).map(|c| Color32::from_rgb(c[0], c[1], c[2])).unwrap_or(fallback)
}

/// Original dropdown menu drawing (MenuDrawItem 0x42e518): background mback, text mtext, the selected row uses background mhighlight and text mover, disabled text mdisabled,
/// divider mdivider, shortcut text mrtext (selected row mrover), border mborder. Corners are square.
pub(super) fn skin_menu_style(skin: &Skin) -> egui::style::StyleModifier {
    let back = menu_color(skin, "mback", Color32::from_gray(0x14));
    let text = menu_color(skin, "mtext", Color32::from_gray(0xB4));
    let high = menu_color(skin, "mhighlight", Color32::from_gray(0xE6));
    let over = menu_color(skin, "mover", Color32::BLACK);
    let disabled = menu_color(skin, "mdisabled", Color32::from_gray(0x64));
    let divider = menu_color(skin, "mdivider", Color32::from_gray(0x30));
    let border = menu_color(skin, "mborder", Color32::from_gray(0x5F));
    let shortcut = menu_color(skin, "mrtext", Color32::from_gray(0xC8));
    egui::style::StyleModifier::new(move |style| {
        egui::containers::menu::menu_style(style);
        // Original MenuDrawItem: text starts at the row's left + 0x10 (the 16 pixels before it are the checkmark slot). MenuMeasureItem's width is also text + 0x20,
        // so there is 16 on the right too. Fix: previously the egui menu padding was 2 pixels, so text hugged the left and only check items were pushed by the box width.
        style.spacing.button_padding.x = MENU_GUTTER;
        let v = &mut style.visuals;
        v.override_text_color = None;
        v.window_fill = back;
        v.panel_fill = back;
        v.window_stroke = egui::Stroke::new(1.0, border);
        v.menu_corner_radius = egui::CornerRadius::ZERO;
        v.weak_text_color = Some(shortcut);
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, divider);
        v.widgets.noninteractive.fg_stroke.color = disabled;
        v.widgets.inactive.fg_stroke.color = text;
        for state in [&mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            state.bg_fill = high;
            state.weak_bg_fill = high;
            state.fg_stroke.color = over;
            state.corner_radius = egui::CornerRadius::ZERO;
        }
        v.widgets.inactive.corner_radius = egui::CornerRadius::ZERO;
        // The NDR menu (PopupMenu) row chosen with the keyboard (Button::selected) also uses the original selected-row color.
        v.selection.bg_fill = high;
        v.selection.stroke.color = over;
    })
}

const MENU_STYLE_ID: &str = "ndr-skin-menu-style";

/// Left padding of the original dropdown menu row (MenuDrawItem: text = row left + 0x10).
pub(super) const MENU_GUTTER: f32 = 16.0;

/// In the original bitmap CHECK (16×16, check item), the (x, y) of the white dots (the dots recolored to the text color).
const MENU_CHECK_DOTS: [(u8, u8); 21] = [
    (10, 4), (9, 5), (10, 5), (4, 6), (8, 6), (9, 6), (10, 6), (4, 7), (5, 7), (7, 7), (8, 7), (9, 7),
    (4, 8), (5, 8), (6, 8), (7, 8), (8, 8), (5, 9), (6, 9), (7, 9), (6, 10),
];
/// White dots of the original bitmap RCHECK (16×16, radio item).
const MENU_RADIO_DOTS: [(u8, u8); 12] = [(7, 6), (8, 6), (6, 7), (7, 7), (8, 7), (9, 7), (6, 8), (7, 8), (8, 8), (9, 8), (7, 9), (8, 9)];

/// Original MenuDrawItem(0x42e518): a checked item (TMenuItem +0x44) draws CHECK (a radio item +0x49 draws RCHECK) at the row's left, vertically centered in the row.
/// The white dots take the text color (selected row mover, otherwise mtext; a disabled item is the same color), the rest is transparent. An unchecked row leaves that slot empty.
pub(super) fn paint_menu_check(ui: &egui::Ui, item: egui::Rect, radio: bool, selected: bool) {
    let color = if selected { ui.visuals().widgets.hovered.fg_stroke.color } else { ui.visuals().widgets.inactive.fg_stroke.color };
    let origin = egui::pos2(item.left().round(), (item.top() + ((item.height() - 16.0) / 2.0).trunc()).round());
    let dots: &[(u8, u8)] = if radio { &MENU_RADIO_DOTS } else { &MENU_CHECK_DOTS };
    // One pixel at a time: drawn as mesh rectangles with no feathering (a 1×1 rect_filled turned into a line and was invisible).
    let mut mesh = egui::Mesh::default();
    for &(x, y) in dots {
        mesh.add_colored_rect(egui::Rect::from_min_size(origin + egui::vec2(x as f32, y as f32), egui::vec2(1.0, 1.0)), color);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
}

/// Check/radio menu item (original TMenuItem Checked·RadioItem): the same button as a normal item, just with the check graphic added.
pub(super) fn menu_check_button(ui: &mut egui::Ui, enabled: bool, checked: bool, radio: bool, button: egui::Button<'_>) -> egui::Response {
    let response = ui.add_enabled(enabled, button);
    if checked {
        paint_menu_check(ui, response.rect, radio, response.hovered() || response.has_focus());
    }
    response
}

/// The shortcut cell of the original MenuDrawItem: background mrback (+0x48, width = shortcut text width), text mrtext (+0x4c), selected row mrover (+0x50).
pub(super) fn skin_menu_shortcut_colors(skin: &Skin) -> (Color32, Color32, Color32) {
    (menu_color(skin, "mrback", Color32::BLACK), menu_color(skin, "mrtext", Color32::from_gray(0xC8)), menu_color(skin, "mrover", Color32::from_rgb(0x55, 0x77, 0xEE)))
}

/// Shortcut text of a skin menu item (original DrawTextW DT_RIGHT, cell right − 4): normal mrtext, selected row mrover. Returns the text width.
pub(super) fn paint_menu_shortcut(ui: &egui::Ui, item: egui::Rect, shortcut: &str, colors: (Color32, Color32, Color32), selected: bool) -> f32 {
    if shortcut.is_empty() {
        return 0.0;
    }
    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = if selected { colors.2 } else { colors.1 };
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(shortcut.to_owned(), font, color));
    let width = galley.size().x;
    ui.painter().galley(egui::pos2(item.right() - 4.0 - width, item.center().y - galley.size().y / 2.0), galley, color);
    width
}

/// Original MenuMeasureItem(0x42df68)·MenuDrawItem: shortcut cell width = the widest shortcut text in that menu (first 8) + 0xe, and every row (dividers included)
/// is filled with mrback on the right. `slot` is the space reserved before drawing the menu (so it is drawn behind the items).
pub(super) fn fill_menu_shortcut_column(ui: &egui::Ui, slot: egui::layers::ShapeIdx, widest: f32, colors: (Color32, Color32, Color32)) {
    let area = ui.min_rect();
    let column = egui::Rect::from_min_max(egui::pos2(area.right() - (widest.max(8.0) + 14.0), area.top()), area.max);
    ui.painter().set(slot, egui::Shape::rect_filled(column, 0.0, colors.0));
}

/// Store this scene's dropdown menu appearance (skin_menu_style for a skin theme, otherwise none) in ctx. The right-click menu and NDR menu read it.
pub(super) fn set_menu_style(ctx: &egui::Context, style: Option<egui::style::StyleModifier>) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(MENU_STYLE_ID), style));
}

/// Apply this scene's dropdown menu appearance to `ui` (leaves it as-is if none).
pub(super) fn apply_menu_style(ui: &mut egui::Ui) {
    if let Some(style) = ui.ctx().data(|d| d.get_temp::<Option<egui::style::StyleModifier>>(egui::Id::new(MENU_STYLE_ID))).flatten() {
        style.apply(ui.style_mut());
    }
}

/// Same as `Response::context_menu`, but uses the original menu colors (skin_menu_style) for a skin theme.
pub(super) fn context_menu(response: &egui::Response, add: impl FnOnce(&mut egui::Ui)) -> Option<egui::InnerResponse<()>> {
    let style = response.ctx.data(|d| d.get_temp::<Option<egui::style::StyleModifier>>(egui::Id::new(MENU_STYLE_ID))).flatten();
    let popup = egui::Popup::context_menu(response);
    match style {
        Some(style) => popup.style(style),
        None => popup,
    }
    .show(add)
}

/// Open the list right-click menu (original pmMain, TPopupMenu.Popup) on this frame. `pane_menu` does the opening.
pub(super) fn request_pane_menu(ctx: &egui::Context, index: usize) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("ndr-list-menu-open", index)), true));
}

/// Whether this frame requested the list right-click menu to open (reading it once clears it).
pub(super) fn take_pane_menu_request(ctx: &egui::Context, index: usize) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new(("ndr-list-menu-open", index)))).unwrap_or(false)
}

/// List right-click menu: one per panel (original menu colors for a skin theme). If open_at is set, it opens at that position on this frame
/// (the click position, or the bottom-left of the cursor item's cell for the keyboard). The position is remembered while open.
/// Fix: when the menu was attached to the response of the clicked row, on a frame that did not draw that row (layout fit, list reload, etc.) egui
/// treated it as a discarded menu and closed it, so it vanished at once. Now the panel draws the menu every frame.
pub(super) fn pane_menu(ui: &egui::Ui, index: usize, open_at: Option<egui::Pos2>, add: impl FnOnce(&mut egui::Ui)) -> Option<egui::InnerResponse<()>> {
    let ctx = ui.ctx().clone();
    let at_id = egui::Id::new(("ndr-list-menu-at", index));
    if let Some(at) = open_at {
        ctx.data_mut(|d| d.insert_temp(at_id, at));
    }
    let at = ctx.data(|d| d.get_temp::<egui::Pos2>(at_id)).unwrap_or_default();
    let style = ctx.data(|d| d.get_temp::<Option<egui::style::StyleModifier>>(egui::Id::new(MENU_STYLE_ID))).flatten();
    let popup = egui::Popup::new(egui::Id::new(("ndr-list-menu", index)), ctx, egui::PopupAnchor::Position(at), ui.layer_id())
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .gap(0.0)
        .open_memory(open_at.map(|_| egui::SetOpenCommand::Bool(true)));
    match style {
        Some(style) => popup.style(style),
        None => popup.style(egui::containers::menu::menu_style),
    }
    .show(add)
}

/// Same as `Popup::menu` (dropdown below a button), but uses the original menu colors for a skin theme.
pub(super) fn menu_popup(response: &egui::Response) -> egui::Popup<'_> {
    let style = response.ctx.data(|d| d.get_temp::<Option<egui::style::StyleModifier>>(egui::Id::new(MENU_STYLE_ID))).flatten();
    let popup = egui::Popup::menu(response);
    match style {
        Some(style) => popup.style(style),
        None => popup,
    }
}

/// Original menu bar row (tbMainMenuAdvancedCustomDraw·…DrawButton 0x430e74·0x430cb4): background mmainback, text mmaintext,
/// the hovered/opened button uses background mmainoverback and text mmainover. Returns (background color, button style).
pub(super) fn skin_menu_bar_style(skin: &Skin) -> (Color32, egui::style::StyleModifier) {
    let back = menu_color(skin, "mmainback", Color32::from_gray(0x2A));
    let text = menu_color(skin, "mmaintext", Color32::from_gray(0x9F));
    let over_back = menu_color(skin, "mmainoverback", Color32::from_gray(0x5A));
    let over = menu_color(skin, "mmainover", Color32::WHITE);
    let style = egui::style::StyleModifier::new(move |style| {
        egui::containers::menu::menu_style(style);
        let v = &mut style.visuals;
        v.override_text_color = None;
        v.widgets.inactive.fg_stroke.color = text;
        for state in [&mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            state.bg_fill = over_back;
            state.weak_bg_fill = over_back;
            state.fg_stroke.color = over;
            state.corner_radius = egui::CornerRadius::ZERO;
        }
    });
    (back, style)
}

/// Original tab bar height (FUN_00481fb8 SetHeight 0x18) and tab button (TNXPngButton) width 12.
pub(super) const TAB_SET_HEIGHT: f32 = 24.0;
const TAB_BUTTON_WIDTH: f32 = 12.0;
/// Original folder info bar height (FUN_00482700 SetHeight 0x14).
pub(super) const FOLDER_INFO_HEIGHT: f32 = 20.0;
/// Original drive button bar height (FUN_004825d8 SetHeight 0x16).
pub(super) const DRIVE_BAR_HEIGHT: f32 = 22.0;

/// Original FUN_007d7e6c(a, b, t): per channel Round(a × t + b × (1 − t)). t is the original's single constant (0.3·0.4·0.6).
pub(super) fn mix_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = f64::from(t);
    let channel = |x: u8, y: u8| (f64::from(x) * t + f64::from(y) * (1.0 - t)).round_ties_even().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(channel(a.r(), b.r()), channel(a.g(), b.g()), channel(a.b(), b.b()))
}

const HLSMAX: f64 = 240.0;
const RGBMAX: f64 = 255.0;

/// VCL GraphUtil.ColorRGBToHLS (original FUN_00652150, HLSMAX 240): (hue, luminance, saturation). An achromatic color is hue 160, saturation 0.
pub(super) fn rgb_to_hls(color: Color32) -> (u16, u16, u16) {
    let (r, g, b) = (f64::from(color.r()), f64::from(color.g()), f64::from(color.b()));
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let luminance = (((max + min) * HLSMAX + RGBMAX) / (2.0 * RGBMAX)).trunc() as u16;
    if max == min {
        return (160, luminance, 0);
    }
    let saturation = if f64::from(luminance) <= HLSMAX / 2.0 {
        ((max - min) * HLSMAX + (max + min) / 2.0) / (max + min)
    } else {
        ((max - min) * HLSMAX + (2.0 * RGBMAX - max - min) / 2.0) / (2.0 * RGBMAX - max - min)
    };
    let delta = |x: f64| (((max - x) * (HLSMAX / 6.0) + (max - min) / 2.0) / (max - min)).trunc();
    let (rd, gd, bd) = (delta(r), delta(g), delta(b));
    let mut hue = if r == max {
        bd - gd
    } else if g == max {
        HLSMAX / 3.0 + rd - bd
    } else {
        2.0 * HLSMAX / 3.0 + gd - rd
    };
    if hue < 0.0 {
        hue += HLSMAX;
    }
    if hue > HLSMAX {
        hue -= HLSMAX;
    }
    (hue.round_ties_even() as u16, luminance, saturation.trunc() as u16)
}

/// VCL GraphUtil.ColorHLSToRGB (original FUN_006525c4).
pub(super) fn hls_to_rgb(hue: u16, luminance: u16, saturation: u16) -> Color32 {
    let round = |v: f64| if v > 255.0 { 255 } else { v.round_ties_even().max(0.0) as u8 };
    let (h, l, s) = (f64::from(hue), f64::from(luminance), f64::from(saturation));
    if saturation == 0 {
        let v = round(l * RGBMAX / HLSMAX);
        return Color32::from_rgb(v, v, v);
    }
    let magic2 = if l <= HLSMAX / 2.0 { (l * (HLSMAX + s) + HLSMAX / 2.0) / HLSMAX } else { l + s - (l * s + HLSMAX / 2.0) / HLSMAX };
    let magic1 = 2.0 * l - magic2;
    let hue_to_rgb = |mut hue: f64| {
        if hue < 0.0 {
            hue += HLSMAX;
        }
        if hue > HLSMAX {
            hue -= HLSMAX;
        }
        let value = if hue < HLSMAX / 6.0 {
            magic1 + ((magic2 - magic1) * hue + HLSMAX / 12.0) / (HLSMAX / 6.0)
        } else if hue < HLSMAX / 2.0 {
            magic2
        } else if hue < HLSMAX * 2.0 / 3.0 {
            magic1 + ((magic2 - magic1) * (HLSMAX * 2.0 / 3.0 - hue) + HLSMAX / 12.0) / (HLSMAX / 6.0)
        } else {
            magic1
        };
        value.round_ties_even()
    };
    let channel = |offset: f64| round((hue_to_rgb(h + offset) * RGBMAX + HLSMAX / 2.0) / HLSMAX);
    Color32::from_rgb(channel(HLSMAX / 3.0), channel(0.0), channel(-HLSMAX / 3.0))
}

/// Original cursor/selection bar colors (DrawFocus): a top-to-bottom vertical gradient, plus a 1-pixel line at the top and bottom edges for the cursor row.
#[derive(Debug, PartialEq)]
pub(super) struct FocusBar {
    pub top: Color32,
    pub bottom: Color32,
    pub lines: Option<Color32>,
}

/// Original TNXFileListView.DrawFocus(0x7d7f54, Details/List view) and SetCanvasColor(0x7d8e80).
/// Background color: a selected non-cursor item is NXSELECT, the cursor item is its own item color. If luminance (HLS 0~240) is below 20, raise the top by +20, otherwise lower the bottom by −20;
/// the line is +30 below 30, otherwise −30. If the list has focus, the cursor row stays as-is (text NXBACK) and a selected row is mixed with NXBACK at 0.15·0.15·line 0.2.
/// Without focus, the cursor row is a 0.06 flat fill with the line at item color 0.25, and a selected row is 0.1·0.1·0.15.
pub(super) fn focus_bar(item: Color32, back: Color32, select: Color32, cursor: bool, selected: bool, list_focused: bool) -> FocusBar {
    let base = if !cursor && selected { select } else { item };
    let (h, l, s) = rgb_to_hls(base);
    let (mut top, mut bottom) = (base, base);
    if l < 20 {
        top = hls_to_rgb(h, l + 20, s);
    } else {
        bottom = hls_to_rgb(h, l - 20, s);
    }
    let line = if l < 30 { hls_to_rgb(h, l + 30, s) } else { hls_to_rgb(h, l - 30, s) };
    let (top, bottom, line) = match (list_focused, cursor) {
        (true, true) => (top, bottom, line),
        (true, false) => (mix_color(top, back, 0.15), mix_color(bottom, back, 0.15), mix_color(line, back, 0.2)),
        (false, true) => {
            let flat = mix_color(top, back, 0.06);
            (flat, flat, mix_color(item, back, 0.25))
        }
        (false, false) => (mix_color(top, back, 0.1), mix_color(bottom, back, 0.1), mix_color(line, back, 0.15)),
    };
    FocusBar { top, bottom, lines: cursor.then_some(line) }
}

/// Bar shape: a GDI+ vertical gradient fill (top = top, bottom = bottom) plus the top-edge/bottom-edge lines of the cursor row (left ~ right − 1).
pub(super) fn focus_bar_shapes(rect: egui::Rect, bar: &FocusBar) -> Vec<egui::Shape> {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), bar.top);
    mesh.colored_vertex(rect.right_top(), bar.top);
    mesh.colored_vertex(rect.right_bottom(), bar.bottom);
    mesh.colored_vertex(rect.left_bottom(), bar.bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    let mut shapes = vec![egui::Shape::mesh(mesh)];
    if let Some(line) = bar.lines {
        let stroke = egui::Stroke::new(1.0, line);
        shapes.push(egui::Shape::hline(rect.left()..=rect.right() - 1.0, rect.top() + 0.5, stroke));
        shapes.push(egui::Shape::hline(rect.left()..=rect.right() - 1.0, rect.bottom() - 0.5, stroke));
    }
    shapes
}

/// Original FUN_007cdfc0 (clockwise)·FUN_007ce0e4 (counterclockwise) 90°-rotated image: rotates the `uv` region and draws it into `dest` (dest width = original height).
pub(super) fn rotated_image(painter: &egui::Painter, texture: egui::TextureId, dest: egui::Rect, uv: egui::Rect, clockwise: bool) {
    let (l, t, r, b) = (uv.min.x, uv.min.y, uv.max.x, uv.max.y);
    // Corner order: top-left, top-right, bottom-right, bottom-left.
    let corners = if clockwise { [(l, b), (l, t), (r, t), (r, b)] } else { [(r, t), (r, b), (l, b), (l, t)] };
    let mut mesh = egui::Mesh::with_texture(texture);
    for (pos, (u, v)) in [dest.left_top(), dest.right_top(), dest.right_bottom(), dest.left_bottom()].into_iter().zip(corners) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv: egui::pos2(u, v), color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Original DrawFocus style 3 (thumbnail preview, 0x7d87a1). The background/line color is the NXSELECT that DrawThumbnailItem put in the pen/brush, and
/// if luminance (HLS 0~240) is below 50 the top is raised by +50, otherwise the bottom is lowered by −50. Then it is mixed with NXBACK (top·bottom, line):
/// drop target 0.5·1.0; with focus, cursor 0.4·(0.9 if also marked, otherwise 0.6), marked only 0.2·0.4;
/// without focus, cursor 0.2·(0.7 if marked, otherwise 0.4), marked only 0.1·0.3. Returns (top, bottom, line).
pub(super) fn thumbnail_focus(select: Color32, back: Color32, cursor: bool, marked: bool, drop: bool, list_focused: bool) -> (Color32, Color32, Color32) {
    let (h, l, s) = rgb_to_hls(select);
    let (mut top, mut bottom) = (select, select);
    if l < 50 {
        top = hls_to_rgb(h, l + 50, s);
    } else {
        bottom = hls_to_rgb(h, l - 50, s);
    }
    let (fill, line) = if drop {
        (0.5, 1.0)
    } else {
        match (list_focused, cursor) {
            (true, true) => (0.4, if marked { 0.9 } else { 0.6 }),
            (true, false) => (0.2, 0.4),
            (false, true) => (0.2, if marked { 0.7 } else { 0.4 }),
            (false, false) => (0.1, 0.3),
        }
    };
    (mix_color(top, back, fill), mix_color(bottom, back, fill), mix_color(select, back, line))
}

/// The bar position of the original DrawThumbnailItem(0x7da1c4): left = cell left + (cell width − size − 12) div 2 + 1, top = cell top + 1,
/// width = size + 10, height = size + 12 (item height +0x340) + that item's text height (GetThumbnailTextHeight) − 2.
pub(super) fn thumbnail_focus_rect(cell: egui::Rect, thumb: f32, text_height: f32) -> egui::Rect {
    let left = cell.left() + ((cell.width() - thumb - 12.0) / 2.0).trunc() + 1.0;
    let top = cell.top() + 1.0;
    egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(thumb + 10.0, thumb + 12.0 + text_height - 2.0))
}

/// Fills the original FUN_007d7b3c (a rounded rectangle path with radius 4) with a vertical gradient brush and strokes it with a 1-pixel pen (GDI+ anti-aliasing).
pub(super) fn rounded_gradient_shapes(rect: egui::Rect, radius: f32, top: Color32, bottom: Color32, line: Color32) -> Vec<egui::Shape> {
    let lerp = |y: f32| {
        let t = if rect.height() > 0.0 { ((y - rect.top()) / rect.height()).clamp(0.0, 1.0) } else { 0.0 };
        let channel = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Color32::from_rgb(channel(top.r(), bottom.r()), channel(top.g(), bottom.g()), channel(top.b(), bottom.b()))
    };
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let corners = [
        (egui::pos2(rect.left() + r, rect.top() + r), 180.0f32),
        (egui::pos2(rect.right() - r, rect.top() + r), 270.0),
        (egui::pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (egui::pos2(rect.left() + r, rect.bottom() - r), 90.0),
    ];
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), lerp(rect.center().y));
    for (center, start) in corners {
        for step in 0..=6 {
            let angle = (start + 90.0 * step as f32 / 6.0).to_radians();
            let p = center + egui::vec2(angle.cos(), angle.sin()) * r;
            mesh.colored_vertex(p, lerp(p.y));
        }
    }
    let n = mesh.vertices.len() as u32 - 1;
    for i in 1..=n {
        mesh.add_triangle(0, i, if i == n { 1 } else { i + 1 });
    }
    vec![egui::Shape::mesh(mesh), egui::Shape::rect_stroke(rect, egui::CornerRadius::same(radius as u8), egui::Stroke::new(1.0, line), egui::StrokeKind::Inside)]
}

/// The ▶ of the original InitSelMarkBitmap(0x7db878, 6×10, color NXSELECT): for each column x (1~4), rows x ~ 8 − x. Drawn at row left + 2, vertically centered.
pub(super) fn selection_mark_shapes(rect: egui::Rect, color: Color32) -> Vec<egui::Shape> {
    let origin = egui::pos2(rect.left() + 2.0, rect.top() + ((rect.height() - 10.0) / 2.0).floor());
    (1..=4)
        .map(|x| {
            let column = egui::Rect::from_min_max(origin + egui::vec2(x as f32, x as f32), origin + egui::vec2(x as f32 + 1.0, (9 - x) as f32));
            egui::Shape::rect_filled(column, 0.0, color)
        })
        .collect()
}

/// Original list row colors (skin theme, DrawListItem·DrawFocus·InitSelMarkBitmap): NX default colors.
#[derive(Clone, Copy)]
pub(super) struct ListColors {
    pub back: Color32,
    pub select: Color32,
    pub size: Color32,
    pub readonly: Color32,
    pub hidden: Color32,
    pub mb: Color32,
    pub gb: Color32,
    pub time: Color32,
    pub attr: Color32,
    pub desc: Color32,
}

impl ListColors {
    /// Cell text color of the original DrawListItem: for the cursor row, NXBACK if focused (SetCanvasColor inverts it), otherwise the item color.
    /// Otherwise size NXSIZE (read-only NXREADONLY, hidden NXHIDDEN), date NXTIME, attributes NXATTR, description NXDESC, and the rest the item color.
    pub(super) fn cell(&self, column: super::list_view::DetailColumn, item: Color32, attributes: u32, cursor: bool, focused: bool) -> Color32 {
        use super::list_view::DetailColumn as C;
        if cursor {
            return if focused { self.back } else { item };
        }
        match column {
            C::Size if attributes & 2 != 0 => self.hidden,
            C::Size if attributes & 1 != 0 => self.readonly,
            C::Size => self.size,
            C::Modified => self.time,
            C::Attributes => self.attr,
            C::Description => self.desc,
            _ => item,
        }
    }
}

/// Original size cell (DrawListItem cell 3): the leading number (0~9 . ,) and the rest (the unit). The rest is NXGB if the whole text has a G, or NXMB if it has an M.
pub(super) fn split_size_text(text: &str) -> (&str, &str) {
    let end = text.char_indices().find(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == ',')).map_or(text.len(), |(i, _)| i);
    text.split_at(end)
}

/// Original TNXScrollBar.UpdateThumbWidth·RecalcThumbPos(0x7cfdf0·0x7cfe90): usable length = track length − button × 2,
/// thumb length = usable length × visible amount / total (not below the track thickness or 20, and not above the usable length), position = button + (usable length − thumb) × current / (total − visible amount).
/// Returns (top of track to top of thumb, thumb length).
pub(super) fn scrollbar_thumb(track: f32, thickness: f32, button: f32, offset: f32, content: f32, viewport: f32) -> (f32, f32) {
    let usable = (track - 2.0 * button).max(0.0);
    let mut length = if content > 0.0 { (usable * viewport / content).floor() } else { usable };
    length = length.max(thickness).max(20.0).min(usable);
    let range = (content - viewport).max(1.0);
    let position = ((usable - length) * (offset / range).clamp(0.0, 1.0)).floor().min(usable - length).max(0.0);
    (button + position, length)
}

/// TNXLabel markup (skin theme): text with no color tag uses the label text color; &lt;b&gt; is a bold face of the same size (Segoe UI Bold·Malgun Gothic Bold).
fn skin_markup_job(text: &str, base: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for span in ndr::markup::render(text) {
        let family = if span.bold { egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into()) } else { egui::FontFamily::Proportional };
        let color = span.color.map_or(base, |(r, g, b)| Color32::from_rgb(r, g, b));
        job.append(&span.text, 0.0, egui::TextFormat { font_id: egui::FontId::new(super::BASE_TEXT_SIZE, family), color, ..Default::default() });
    }
    job
}

/// Result of the original TNXTabSet.CalcTabWidth(0x7d0858).
#[derive(Debug, PartialEq)]
pub(super) struct TabSetLayout {
    pub tab_width: i32,
    /// Visible tabs at once (+0x2cc).
    pub visible: usize,
    /// They do not all fit, so scroll buttons (《 》) are shown. Then tabs start at 32 (otherwise at 16).
    pub scroll: bool,
}

/// Original CalcTabWidth: usable width = width − 16, tab width = longest text + 28. If it overflows, use + 14; if it still overflows,
/// split evenly if that can be wider than the minimum width (SetMinTabWidth 60), otherwise subtract 32 for the scroll buttons and split into as many 60-wide tabs as fit.
pub(super) fn tab_set_layout(text_widths: &[i32], width: i32) -> TabSetLayout {
    const MIN_TAB_WIDTH: i32 = 60;
    let count = text_widths.len() as i32;
    let widest = text_widths.iter().copied().max().unwrap_or(0);
    let mut available = width - 16;
    let mut tab = widest + 28;
    if count > 0 && count * tab > available {
        tab = widest + 14;
        if count * tab > available {
            if MIN_TAB_WIDTH < available / count {
                tab = available / count;
            } else {
                available -= 32;
                tab = available / (available / MIN_TAB_WIDTH).max(1);
            }
        }
    }
    let tab = tab.max(1);
    TabSetLayout { tab_width: tab, visible: (available / tab).max(0) as usize, scroll: available < count * tab }
}

/// Original SetTabIndex·TabsChanged first-tab alignment: keep the current tab visible and leave no empty space at the end.
pub(super) fn tab_first_index(first: usize, active: usize, visible: usize, count: usize) -> usize {
    let visible = visible.max(1);
    let mut first = first.min(active);
    if first + visible <= active {
        first = active + 1 - visible;
    }
    first.min(count.saturating_sub(visible))
}

/// Original TNXPathLabel.UpdateDisplayInfo(0x7c8c20): segment width = text width + 4. Usable width = label width − 16, first-segment share = first text + 20.
/// 1 segment: if wider than the usable width, label width − 32. 2 segments: set the last to usable width − (first text + 36). 3 or more: keep the first and last, and
/// the middle segments share (usable width − last text − 20 − first share − (count × 16 − 32)). Segments narrower than their share stay, the rest shrink to the same share.
/// If shrinking all middle segments to "..." (text + 4) still does not fit, shrink the last to (usable width − first share − ("..." + 20) × middle count − 16). Division is Delphi div.
pub(super) fn path_label_widths(text: &[i32], dots: i32, width: i32) -> Vec<i32> {
    let mut w: Vec<i32> = text.iter().map(|t| t + 4).collect();
    let n = w.len();
    if n == 0 {
        return w;
    }
    let avail = width - 16;
    let first = text[0] + 20;
    match n {
        1 => {
            if avail < first {
                w[0] = width - 32;
            }
        }
        2 => {
            if avail < text[1] + 20 + first {
                w[1] = avail - (text[0] + 36);
            }
        }
        _ => {
            let middle_count = n as i32 - 2;
            let dividers = n as i32 * 16 - 32;
            let mut rest = avail - (text[n - 1] + 20 + first) - dividers;
            if (dots + 4) * middle_count > rest {
                let last = avail - first - (dots + 20) * middle_count - 16;
                w[n - 1] = last;
                rest = avail - (last + first) - dividers;
            }
            let share = rest / middle_count;
            let mut small = 0;
            for width in &w[1..n - 1] {
                if *width < share {
                    small += 1;
                    rest -= width;
                }
            }
            if small < middle_count {
                let each = rest / (middle_count - small);
                for width in &mut w[1..n - 1] {
                    if *width > each {
                        *width = each;
                    }
                }
            }
        }
    }
    w
}

impl App {
    /// Original panel border (FUN_00483300, drawing the frame panel +0x330): the color depends on the location kind (local lframelocal, lframenet if it starts with "\\", archive lframearc,
    /// FTP lframeftp). If the list has focus, use that color as-is (inner-ring ratio 0); otherwise mix it with the frame background (lback) (background 40% + border 60%, integer division).
    /// The width is lframewidth (every bundled skin set uses 1, so a single inner ring).
    pub(super) fn skin_frame_stroke(&self, skin: &Skin, index: usize) -> egui::Stroke {
        let pane = &self.panes[index];
        let key = if pane.remote.is_some() {
            "lframeftp"
        } else if pane.archive.is_some() {
            "lframearc"
        } else if pane.path.to_string_lossy().starts_with("\\\\") {
            "lframenet"
        } else {
            "lframelocal"
        };
        let width = skin.positive_integer("List", "lframewidth", 1) as f32;
        let frame = skin_color(skin, key, Color32::from_rgb(0x30, 0x5B, 0x91));
        let color = if self.active == index {
            frame
        } else {
            let back = skin_color(skin, "lback", Color32::from_gray(0x14));
            let ring = |b: u8, f: u8| ((u32::from(b) * 40 + 60 * u32::from(f)) / 100) as u8;
            Color32::from_rgb(ring(back.r(), frame.r()), ring(back.g(), frame.g()), ring(back.b(), frame.b()))
        };
        egui::Stroke::new(width, color)
    }

    /// Address-bar segments (original TNXPathLabel.SetPath): FTP = "ftp://host" + folders, archive = archive file folders + archive name + inner folders,
    /// My Computer = a single MYCOMPUTER, local = from the root down to the current folder.
    pub(super) fn crumb_segments(&self, index: usize) -> Vec<CrumbSegment> {
        let pane = &self.panes[index];
        let mut segments = Vec::new();
        if let Some(remote) = &pane.remote {
            segments.push(CrumbSegment { label: format!("ftp://{}", remote.host), target: CrumbTarget::Remote("/".into()), folder: None });
            let mut path = String::new();
            for part in remote.path.split('/').filter(|p| !p.is_empty()) {
                path.push('/');
                path.push_str(part);
                segments.push(CrumbSegment { label: part.to_owned(), target: CrumbTarget::Remote(path.clone()), folder: None });
            }
            return segments;
        }
        // The first segment (root) drops the trailing '\' like the original TNXPathLabel ("C:\" → "C:").
        let local = |path: &Path, first: bool| CrumbSegment {
            label: if first {
                let root = path.to_string_lossy();
                let trimmed = root.trim_end_matches('\\');
                if trimmed.is_empty() { root.into_owned() } else { trimmed.to_owned() }
            } else {
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            },
            target: CrumbTarget::Local(path.to_path_buf()),
            folder: Some(path.to_path_buf()),
        };
        if let Some(location) = &pane.archive {
            let full = std::path::absolute(&location.file).unwrap_or_else(|_| location.file.clone());
            let mut folders: Vec<PathBuf> = full.parent().map(|p| p.ancestors().map(Path::to_path_buf).collect()).unwrap_or_default();
            folders.reverse();
            segments.extend(folders.iter().enumerate().map(|(i, folder)| local(folder, i == 0)));
            let name = location.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            segments.push(CrumbSegment { label: name, target: CrumbTarget::Archive(String::new()), folder: None });
            let mut prefix = String::new();
            for part in location.prefix.split('/').filter(|p| !p.is_empty()) {
                prefix += part;
                prefix.push('/');
                segments.push(CrumbSegment { label: part.to_owned(), target: CrumbTarget::Archive(prefix.clone()), folder: None });
            }
            return segments;
        }
        if super::panes::is_drives(&pane.path) {
            segments.push(CrumbSegment { label: self.language_pack.text("RES.MYCOMPUTER", "My Computer"), target: CrumbTarget::Local(PathBuf::from(super::remote_window::DRIVES)), folder: None });
            return segments;
        }
        let full = std::path::absolute(&pane.path).unwrap_or_else(|_| pane.path.clone());
        let mut components: Vec<PathBuf> = full.ancestors().map(Path::to_path_buf).collect();
        components.reverse();
        segments.extend(components.iter().enumerate().map(|(i, path)| local(path, i == 0)));
        segments
    }

    /// Original address bar (skin theme). Left button = path history dropdown (FUN_0048ced4), right button = refresh (FUN_0048ce78),
    /// center TNXPathLabel: background laback, a leading ▼ (DrawDivider, click for the drive list), segment text (latext, current folder lacurr,
    /// a hovered segment is laover text + lafocus background), a ">" line after each segment (ladivider, click for that folder's subfolders).
    pub(super) fn skin_address_bar(&mut self, ui: &mut egui::Ui, index: usize, ctx: &egui::Context, skin: &Skin) {
        if !self.settings.flag("ShowAddressBar", true) {
            return;
        }
        let (rect, background) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ADDRESS_BAR_HEIGHT), egui::Sense::click());
        let painter = ui.painter().with_clip_rect(rect);
        // Original FUN_004896d8: laback (0x141414 if missing) is used for the frame (TNXPaintPanel) and the label background.
        painter.rect_filled(rect, 0.0, skin_color(skin, "laback", Color32::from_gray(0x14)));
        if self.panes[index].address_editing {
            let id = egui::Id::new(("ndr-address", index));
            let response = ui.put(rect.shrink2(egui::vec2(ADDRESS_BUTTON_WIDTH, 0.0)), egui::TextEdit::singleline(&mut self.panes[index].address).id(id).desired_width(f32::INFINITY));
            if response.lost_focus() {
                let go = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let text = self.panes[index].address.clone();
                self.end_address_edit(index);
                if go {
                    self.original_address_enter(index, &text);
                }
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.end_address_edit(index);
            }
            return;
        }
        // The two end buttons (TNXImageButton, addressbtns.png: columns = buttons, rows = normal·hovered).
        // Original FUN_00481978: both buttons have Align = alRight(4), width 0x14. Refresh has Left = frame width, path history has Left = 0, so
        // from the right edge it is [path history ▼][refresh] and the TNXPathLabel (alClient) fills the rest.
        // Fix (user-reported): previously the path history ▼ was placed at the left edge, so it appeared side by side with the label's drive ▼ (two ▼).
        let buttons = skin.values.get("list.labtnimgs").cloned().and_then(|file| self.toolbar_icon(ctx, &file));
        let right_button = egui::Rect::from_min_size(egui::pos2(rect.right() - ADDRESS_BUTTON_WIDTH, rect.top()), egui::vec2(ADDRESS_BUTTON_WIDTH, rect.height()));
        let left_button = egui::Rect::from_min_size(egui::pos2(right_button.left() - ADDRESS_BUTTON_WIDTH, rect.top()), egui::vec2(ADDRESS_BUTTON_WIDTH, rect.height()));
        let mut clicked_button = None;
        for (cell, slot) in [(0usize, left_button), (1, right_button)] {
            let response = ui.interact(slot, egui::Id::new(("ndr-skin-address-button", index, cell)), egui::Sense::click());
            if let Some(texture) = &buttons {
                let row = usize::from(response.hovered());
                let uv = egui::Rect::from_min_max(egui::pos2(cell as f32 / 2.0, row as f32 / 2.0), egui::pos2((cell + 1) as f32 / 2.0, (row + 1) as f32 / 2.0));
                let size = texture.size_vec2() / 2.0;
                // Original TNXImageButton: if there are fewer than 4 columns, color × 0.6 while held (FUN_007c39e4).
                let tint = if response.is_pointer_button_down_on() && response.hovered() { Color32::from_gray(153) } else { Color32::WHITE };
                painter.image(texture.id(), egui::Rect::from_center_size(slot.center(), size), uv, tint);
            }
            let tip = if cell == 0 { self.tr("경로 기록", "Path history") } else { self.tr("현재 폴더 새로 고침", "Refresh the current folder") }.into_owned();
            if response.on_hover_text(tip).clicked() {
                clicked_button = Some((cell, slot.left_bottom()));
            }
        }
        // The center path text (TNXPathLabel).
        let label = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top()), egui::pos2(left_button.left(), rect.bottom()));
        let painter = ui.painter().with_clip_rect(label);
        let font = egui::FontId::proportional(super::BASE_TEXT_SIZE);
        // Original FUN_004896d8 (the values used when a skin conf.ini [List] key is missing): latext → Font.Color 0x646464, lacurr → SetLastFolderColor
        // TColor 0x9696B4 (= RGB B4 96 96), laover → SetHighlightColor 0x969696, lafocus → SetFocusBackColor 0x323232,
        // ladivider → SetDividerColor 0x141414. Fix: previously the black-skin values were used as defaults, so a skin without these keys got different colors.
        let (text, current, over, focus, divider) = (
            skin_color(skin, "latext", Color32::from_gray(0x64)),
            skin_color(skin, "lacurr", Color32::from_rgb(0xB4, 0x96, 0x96)),
            skin_color(skin, "laover", Color32::from_gray(0x96)),
            skin_color(skin, "lafocus", Color32::from_gray(0x32)),
            skin_color(skin, "ladivider", Color32::from_gray(0x14)),
        );
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| label.contains(*p));
        let clicked = ui.input(|i| i.pointer.primary_clicked());
        let (top, height) = (label.top(), label.height());
        let mut action: Option<(Option<CrumbTarget>, Option<(PathBuf, bool, egui::Pos2)>)> = None;
        // DrawDivider: a ▼ made of 4 rows (8, 6, 4, 2 pixels) starting at x + 4. Click for the drive list.
        let mut x = label.left();
        let lead = egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(16.0, height));
        let lead_hot = pointer.is_some_and(|p| lead.contains(p));
        let (glyph_x, glyph_y) = (x + 4.0, (top + (height - 4.0) / 2.0).floor());
        for i in 0..4 {
            let y = glyph_y + i as f32 + 0.5;
            painter.line_segment([egui::pos2(glyph_x + i as f32, y), egui::pos2(glyph_x + 8.0 - i as f32, y)], egui::Stroke::new(1.0, if lead_hot { over } else { divider }));
        }
        if lead_hot && clicked {
            action = Some((None, Some((PathBuf::new(), true, lead.left_bottom()))));
        }
        x += 16.0;
        let segments = self.crumb_segments(index);
        let count = segments.len();
        // Original UpdateDisplayInfo: segment widths are set to fit the label width, and shrunken segments are drawn with DrawTextW(DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS).
        let (naturals, dots) = ui.fonts_mut(|f| {
            let mut measure = |s: &str| f.layout_no_wrap(s.to_owned(), font.clone(), text).size().x.round() as i32;
            let naturals = segments.iter().map(|s| measure(&s.label)).collect::<Vec<_>>();
            (naturals, measure("..."))
        });
        let widths = path_label_widths(&naturals, dots, label.width() as i32);
        for (i, segment) in segments.into_iter().enumerate() {
            let width = widths[i].max(0) as f32;
            let galley = if widths[i] < naturals[i] + 4 {
                let mut job = egui::text::LayoutJob::simple_singleline(segment.label.clone(), font.clone(), text);
                job.wrap = egui::text::TextWrapping { max_width: (width - 2.0).max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                ui.fonts_mut(|f| f.layout_job(job))
            } else {
                ui.fonts_mut(|f| f.layout_no_wrap(segment.label.clone(), font.clone(), text))
            };
            let cell = egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(width, height));
            let hot = pointer.is_some_and(|p| cell.contains(p));
            if hot {
                painter.rect_filled(cell, 0.0, focus);
                // Original MouseMove(0x7c9e30): hovering a segment shows a tooltip with Hint = PathOfLevel (the path joined by separators up to that segment).
                let path_of_level = match &segment.target {
                    CrumbTarget::Local(path) => {
                        let text = path.to_string_lossy().into_owned();
                        if text.len() == 3 && text.ends_with('\\') { text.trim_end_matches('\\').to_owned() } else { text }
                    }
                    CrumbTarget::Archive(inner) | CrumbTarget::Remote(inner) => inner.clone(),
                };
                ui.interact(cell, egui::Id::new(("ndr-skin-crumb-tip", index, i)), egui::Sense::hover()).on_hover_text(path_of_level);
            }
            let color = if i + 1 == count { current } else if hot { over } else { text };
            let pressed = hot && ui.input(|i| i.pointer.primary_down());
            let offset = if pressed && i + 1 != count { 1.0 } else { 0.0 };
            painter.galley_with_override_text_color(egui::pos2(x + 2.0 + offset, top + ((height - galley.size().y) / 2.0).floor() + offset), galley, color);
            // The ">" after a segment: lines converging from the top and bottom edges to the center (right + 10).
            let right = cell.right();
            let middle = top + (height / 2.0).floor();
            let arrow = egui::Rect::from_min_max(egui::pos2(right, top), egui::pos2(right + 16.0, top + height));
            let arrow_hot = segment.folder.is_some() && pointer.is_some_and(|p| arrow.contains(p));
            let stroke = egui::Stroke::new(1.0, if arrow_hot { over } else { divider });
            if i + 1 != count || segment.folder.is_some() {
                painter.line_segment([egui::pos2(right + 2.0, top), egui::pos2(right + 10.0, middle + 1.0)], stroke);
                painter.line_segment([egui::pos2(right + 2.0, top + height), egui::pos2(right + 10.0, middle)], stroke);
            }
            if clicked && hot {
                action = Some((Some(segment.target), None));
            } else if clicked && arrow_hot {
                let drive_root = i == 0 && count == 1;
                action = Some((None, segment.folder.map(|folder| (folder, drive_root, arrow.left_bottom()))));
            }
            x = right + 16.0;
        }
        // Original pmAddressBar: right-clicking shows the copy/edit address and favorites menu. Double-clicking empty space enters the path directly.
        if let Some(position) = ui.input(|i| i.pointer.secondary_clicked().then(|| i.pointer.interact_pos()).flatten()).filter(|p| label.contains(*p)) {
            return self.show_address_bar_menu(index, position);
        }
        // 원본 pathclick(ini [Operation], 설정 +0xf8, 환경 설정 창에는 없다): 2면 조각·">"·▼를 눌러도(FUN_0048bfa8·FUN_0048c548),
        // 1·2면 빈 곳을 눌러도(FUN_0048c9b8) 누른 자리에 주소 표시줄 메뉴(FUN_0048d10c). 0이면 아래 동작.
        let path_click = self.settings.values.get("PathClickMode").and_then(|v| v.as_i64()).unwrap_or(0);
        let menu_at = pointer.unwrap_or(label.left_bottom());
        if path_click == 2 && action.is_some() {
            return self.show_address_bar_menu(index, menu_at);
        }
        // 경로 조각 옆 빈 곳을 누르면(한 번) 경로 직접 입력. 조각·">"·▼·양끝 단추를 누른 것은 그 동작.
        if (background.clicked() || super::panes::windows_double_clicked(&background)) && action.is_none() && clicked_button.is_none() {
            self.active = index;
            if matches!(path_click, 1 | 2) {
                return self.show_address_bar_menu(index, menu_at);
            }
            return self.begin_address_edit(index, ctx);
        }
        let mut just_opened = false;
        match (clicked_button, action) {
            (Some((0, position)), _) => {
                self.active = index;
                self.show_history_menu(index, Some(position));
            }
            (Some((_, _)), _) => {
                self.active = index;
                self.refresh(index);
            }
            (None, Some((Some(target), _))) => {
                self.active = index;
                self.go_crumb(index, target);
            }
            (None, Some((None, Some((folder, drives, position))))) => {
                self.active = index;
                self.skin_crumb_menu = Some((index, folder, drives, position));
                just_opened = true;
            }
            _ => {}
        }
        self.skin_crumb_menu_ui(ctx, index, just_opened);
    }

    /// 원본 탭 줄(스킨 테마, FUN_00481fb8 + TNXTabSet). 탭이 둘 이상일 때만 보인다(Visible = 탭 수 > 1).
    /// 색은 FUN_00489048: 배경 tabback, 글 tabtext, 포인터 tabhot, 지금 탭 tabactive. 탭 그림 tabimgs(세 줄: 지금·보통·포인터),
    /// 단추 그림 tabbtnimgs(12×12 칸: ≡ 《 》 ×, 아래 줄은 포인터).
    /// Paint(0x7d0c28): 탭마다 왼쪽 5 + 가운데 10씩 + 오른쪽 5 그림, 글은 왼쪽 + 7부터 오른쪽 − 7까지(지금 탭은 위 0, 오른쪽 − 17, 아니면 위 + 3).
    /// 왼쪽 단추로 누르면 그 탭으로(MouseDown), 포인터를 올리면 전체 경로 풍선 도움말, 오른쪽 단추는 pmTab(FUN_0048a534).
    pub(super) fn skin_tab_set(&mut self, ui: &mut egui::Ui, index: usize, ctx: &egui::Context, skin: &Skin) {
        let count = self.panes[index].tabs.len();
        if !self.settings.flag("UseTabs", true) || count < 2 {
            return;
        }
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), TAB_SET_HEIGHT), egui::Sense::click());
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, skin_color(skin, "tabback", Color32::BLACK));
        let my_computer = self.language_pack.text("RES.MYCOMPUTER", "My Computer");
        let names: Vec<String> = self.panes[index].tabs.iter().map(|t| super::pane_chrome::tab_name(t, &my_computer)).collect();
        let font = egui::FontId::proportional(super::BASE_TEXT_SIZE);
        let widths: Vec<i32> = names.iter().map(|n| ui.fonts_mut(|f| f.layout_no_wrap(n.clone(), font.clone(), Color32::WHITE)).size().x.ceil() as i32).collect();
        let layout = tab_set_layout(&widths, rect.width() as i32);
        let active = self.panes[index].tab.min(count - 1);
        let first_id = egui::Id::new(("ndr-tab-first", index));
        let first = tab_first_index(ui.data(|d| d.get_temp::<usize>(first_id)).unwrap_or(0), active, layout.visible, count);
        ui.data_mut(|d| d.insert_temp(first_id, first));
        let lead = if layout.scroll { 32.0 } else { 16.0 };
        let tab_width = layout.tab_width as f32;
        let shown = layout.visible.min(count - first);
        // 단추(나중에 등록해 탭 줄보다 먼저 포인터를 받는다).
        let buttons = skin.values.get("list.tabbtnimgs").cloned().and_then(|file| self.toolbar_icon(ctx, &file));
        // 고친 것: 단추 그림은 탭 그림 뒤에 그린다(전에는 지금 탭 그림이 닫기 단추 ×를 덮어 보이지 않았다). 원본 단추는 탭 위의 자식 창이다.
        let button_paints = std::cell::RefCell::new(Vec::<(egui::TextureId, egui::Rect, egui::Rect, Color32)>::new());
        let button = |ui: &mut egui::Ui, column: usize, slot: egui::Rect, enabled: bool, salt: &str| {
            let response = ui.interact(slot, egui::Id::new(("ndr-skin-tab-button", index, salt)), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
            if let Some(texture) = &buttons {
                let row = usize::from(enabled && response.hovered());
                let uv = egui::Rect::from_min_max(egui::pos2(column as f32 / 4.0, row as f32 / 2.0), egui::pos2((column + 1) as f32 / 4.0, (row + 1) as f32 / 2.0));
                // 원본 TNXPngButton.Paint(0x7cb700): 꺼진 단추는 그림 알파를 반으로(FUN_007cb3ec).
                let tint = if enabled { Color32::WHITE } else { Color32::WHITE.gamma_multiply(0.5) };
                button_paints.borrow_mut().push((texture.id(), egui::Rect::from_center_size(slot.center(), egui::vec2(12.0, 12.0)), uv, tint));
            }
            response
        };
        let slot = |x: f32, top: f32| egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(TAB_BUTTON_WIDTH, TAB_SET_HEIGHT));
        let mut command = None;
        let mut new_first = None;
        let menu = button(ui, 0, slot(rect.left(), rect.top()), true, "menu");
        let mut over_button = menu.hovered();
        // 원본 UpdateMenu: "&%d. %s"(1~9), 그다음 "&%c. %s"(A~), 지금 탭에 표시. 메뉴가 열린 동안 그 글자를 치면 그 탭으로(Windows 메뉴 접근 글자).
        menu_popup(&menu).show(|ui| {
            for (t, name) in names.iter().enumerate() {
                let key = if t < 9 { char::from(b'1' + t as u8) } else { char::from(b'A' + (t - 9) as u8) };
                let item = ui.add(egui::Button::selectable(t == active, egui::RichText::new(format!("{key}. {name}"))));
                if item.clicked() || self.note_menu_target(Some(key.to_ascii_lowercase()), &item) {
                    command = Some(format!("tab:{t}"));
                    ui.close();
                }
            }
        });
        if layout.scroll {
            // 원본 LeftScroll/RightScroll: 첫 탭을 하나 옮기고 그 끝 탭으로 바꾼다.
            let left = button(ui, 1, slot(rect.left() + 16.0, rect.top()), first > 0, "left");
            let right = button(ui, 2, slot(rect.right() - TAB_BUTTON_WIDTH - 2.0, rect.top()), first + layout.visible < count, "right");
            over_button |= left.hovered() || right.hovered();
            if left.clicked() {
                new_first = Some(first - 1);
                command = Some(format!("tab:{}", first - 1));
            } else if right.clicked() {
                new_first = Some(first + 1);
                command = Some(format!("tab:{}", (first + layout.visible).min(count - 1)));
            }
        }
        // 지금 탭의 닫기 단추(SetTabIndex: 왼쪽 = 끝 − 12 − 4, 위 = 1).
        if active >= first && active < first + shown {
            let x = rect.left() + lead + ((active - first + 1) as f32) * tab_width - TAB_BUTTON_WIDTH - 4.0;
            let close_tip = self.language_pack.text("formMain.Menu.pmiCloseTab", if self.language == "ko" { "탭 닫기" } else { "Close Tab" });
            let close = button(ui, 3, slot(x, rect.top() + 1.0), true, "close");
            over_button |= close.hovered();
            if close.on_hover_text(close_tip).clicked() {
                command = Some(format!("closeTab:{active}"));
            }
        }
        // IndexOfTabAt: 앞자리(16 또는 32) 밖에서 (x − 앞자리) / 탭 너비 + 첫 탭.
        let tab_at = |p: egui::Pos2| -> Option<usize> {
            let x = p.x - rect.left();
            if !rect.contains(p) || x < lead {
                return None;
            }
            let offset = ((x - lead) / tab_width) as usize;
            (offset < layout.visible && first + offset < count).then_some(first + offset)
        };
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let hot = pointer.filter(|_| !over_button && response.hovered()).and_then(tab_at);
        let texture = skin.values.get("list.tabimgs").cloned().and_then(|file| self.toolbar_icon(ctx, &file));
        let (text, hot_text, active_text) = (
            skin_color(skin, "tabtext", Color32::from_gray(0x66)),
            skin_color(skin, "tabhot", Color32::from_gray(0xCC)),
            skin_color(skin, "tabactive", Color32::WHITE),
        );
        for offset in 0..shown {
            let t = first + offset;
            let x = rect.left() + lead + offset as f32 * tab_width;
            let right = x + tab_width;
            let (row, color) = if t == active { (0.0, active_text) } else if Some(t) == hot { (2.0, hot_text) } else { (1.0, text) };
            if let Some(texture) = &texture {
                let size = texture.size_vec2();
                let h = size.y / 3.0;
                let blit = |src: f32, width: f32, at: f32| {
                    let uv = egui::Rect::from_min_max(egui::pos2(src / size.x, row * h / size.y), egui::pos2((src + width) / size.x, (row + 1.0) * h / size.y));
                    painter.image(texture.id(), egui::Rect::from_min_size(egui::pos2(at, rect.top()), egui::vec2(width, h)), uv, Color32::WHITE);
                };
                blit(0.0, 5.0, x);
                let mut middle = x + 5.0;
                while middle < right - 10.0 {
                    blit(5.0, 10.0, middle);
                    middle += 10.0;
                }
                blit(5.0, 10.0, right - 15.0);
                blit(size.x - 5.0, 5.0, right - 5.0);
            }
            let (top, text_right) = if t == active { (rect.top(), right - 17.0) } else { (rect.top() + 3.0, right - 7.0) };
            let mut job = egui::text::LayoutJob::simple_singleline(names[t].clone(), font.clone(), color);
            job.wrap = egui::text::TextWrapping { max_width: (text_right - x - 7.0).max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let y = top + ((rect.bottom() - top - galley.size().y) / 2.0).floor();
            painter.galley(egui::pos2(x + 7.0, y), galley, color);
        }
        for (texture, at, uv, tint) in button_paints.take() {
            painter.image(texture, at, uv, tint);
        }
        if response.hovered() && !over_button && ui.input(|i| i.pointer.primary_pressed()) {
            if let Some(t) = hot {
                command = Some(format!("tab:{t}"));
            }
        }
        let menu_tab = egui::Id::new(("ndr-tab-menu-tab", index));
        if response.secondary_clicked() {
            let under = pointer.and_then(tab_at);
            ui.data_mut(|d| d.insert_temp(menu_tab, under));
        }
        context_menu(&response, |ui| {
            let under = ui.data(|d| d.get_temp::<Option<usize>>(menu_tab)).flatten();
            if let Some(action) = self.tab_actions(ui, index, under) {
                command = Some(action);
            }
        });
        if let Some(t) = hot {
            let path = self.panes[index].tabs[t].to_string_lossy().into_owned();
            response.on_hover_text(path);
        }
        if let Some(first) = new_first {
            ui.data_mut(|d| d.insert_temp(first_id, first));
        }
        if let Some(command) = command {
            self.active = index;
            self.tab_command(index, &command, ctx);
        }
    }

    /// 원본 폴더 정보 표시줄(스킨 테마, FUN_00482700 + 색 FUN_00489b24 + 그리기 FUN_0048bb74). 높이 20, 주소 줄 아래(목록 바로 위),
    /// 배경 lfolderback, 아래끝 선 lfolderdivider, 안쪽 여백 좌우 6. 왼쪽 FOLDERINFO(글 lfoldertext, &lt;c000000&gt; 자리는 lfoldersubtext1),
    /// 오른쪽 끝 디스크 여유(&lt;c000000&gt; = ldisksubtext1) 또는 선택 정보(lselinfotext), 그 앞 드라이브 글(모자라면 비운다, FUN_0048b500).
    /// FOLDERINFO를 누르면 지금 폴더, 드라이브 글·여유를 누르면 드라이브 루트의 셸 속성 창(FUN_0048bc24·FUN_0048bca8, 로컬만).
    pub(super) fn skin_folder_info_bar(&mut self, ui: &mut egui::Ui, index: usize, skin: &Skin) {
        if !self.settings.flag("ShowFolderDiskInfoBar", true) {
            return;
        }
        // info_texts가 요약(폴더·파일 수)을 채운다.
        let _ = self.info_texts(index);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), FOLDER_INFO_HEIGHT), egui::Sense::hover());
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, skin_color(skin, "lfolderback", Color32::BLACK));
        painter.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, skin_color(skin, "lfolderdivider", Color32::from_gray(0x3C))));
        let base = skin_color(skin, "lfoldertext", Color32::from_gray(0xC0));
        let hex = |key: &str| skin.color("List", key).map(|c| format!("<c{:02X}{:02X}{:02X}>", c[0], c[1], c[2])).unwrap_or_else(|| "<c000000>".into());
        let drives = super::panes::is_drives(&self.panes[index].path);
        // 원본 FUN_0048aa30: 내 PC도 FOLDERINFO(폴더 = 종류 1, 파일 = 종류 3 개수와 크기 합)이고, 드라이브 항목(종류 4)은 세지 않으며
        // FUN_0048b500은 루트를 못 구해 디스크 글을 비운다. 고친 것: 전에는 C# 판 글 "::drives:: · 드라이브 N개"가 보였다.
        let (counts, drive, right, hint) = if drives { (self.folder_info(index).0, String::new(), String::new(), String::new()) } else { self.folder_info(index) };
        let selected = !self.panes[index].selected.is_empty();
        let counts = counts.replace("<c000000>", &hex("lfoldersubtext1"));
        let (right, right_color) = if selected { (right, skin_color(skin, "lselinfotext", base)) } else { (right.replace("<c000000>", &hex("ldisksubtext1")), base) };
        let inner = rect.shrink2(egui::vec2(6.0, 0.0));
        let (left, mut drive_galley, right) = ui.fonts_mut(|fonts| {
            (
                fonts.layout_job(skin_markup_job(&counts, base)),
                fonts.layout_no_wrap(drive.clone(), egui::FontId::proportional(super::BASE_TEXT_SIZE), base),
                fonts.layout_job(skin_markup_job(&right, right_color)),
            )
        });
        if inner.width() - left.size().x - right.size().x - 18.0 <= drive_galley.size().x {
            drive_galley = ui.fonts_mut(|fonts| fonts.layout_no_wrap(String::new(), egui::FontId::proportional(super::BASE_TEXT_SIZE), base));
        }
        let middle = |height: f32| rect.top() + ((rect.height() - 1.0 - height) / 2.0).floor();
        let right_rect = egui::Rect::from_min_size(egui::pos2(inner.right() - 3.0 - right.size().x, middle(right.size().y)), right.size());
        let drive_rect = egui::Rect::from_min_size(egui::pos2(right_rect.left() - 6.0 - drive_galley.size().x, middle(drive_galley.size().y)), drive_galley.size());
        let left_rect = egui::Rect::from_min_size(egui::pos2(inner.left(), middle(left.size().y)), left.size());
        // 고친 것: 패널이 좁으면 FOLDERINFO가 오른쪽 여유 글 위로 겹쳐 그려졌다. 오른쪽 글(드라이브 글이 있으면 그 앞) 앞에서 자른다.
        let left_limit = if drive_galley.size().x > 0.0 { drive_rect.left() } else { right_rect.left() } - 6.0;
        let left_clip = egui::Rect::from_min_max(rect.min, egui::pos2(left_limit.max(rect.left()), rect.bottom())).intersect(rect);
        let left_rect = left_rect.intersect(left_clip);
        painter.with_clip_rect(left_clip).galley(egui::pos2(inner.left(), middle(left.size().y)), left, base);
        // 원본 +0x344(FUN_00482700 빨강 굵게, FUN_00487588): 이 패널이 숨김 파일을 보이면(설정 +0x130 + 쪽) "H", 아니면 빈 글.
        // 자리 = FOLDERINFO 왼쪽 + 너비 + 1. 굵은 꼴("bold" 묶음, Segoe UI Bold).
        if self.panes[index].show_hidden {
            let galley = ui.fonts_mut(|fonts| fonts.layout_no_wrap("H".into(), egui::FontId::new(super::BASE_TEXT_SIZE, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())), Color32::from_rgb(0xFF, 0, 0)));
            painter.galley(egui::pos2(left_rect.right() + 1.0, middle(galley.size().y)), galley, Color32::from_rgb(0xFF, 0, 0));
        }
        painter.galley(drive_rect.min, drive_galley, base);
        painter.galley(right_rect.min, right, right_color);
        let local = !drives && self.panes[index].archive.is_none() && self.panes[index].remote.is_none();
        let sense = if local { egui::Sense::click() } else { egui::Sense::hover() };
        let folder = ui.interact(left_rect, egui::Id::new(("ndr-skin-folder-info", index)), sense).on_hover_cursor(egui::CursorIcon::PointingHand);
        let disk = ui.interact(drive_rect.union(right_rect), egui::Id::new(("ndr-skin-disk-info", index)), sense).on_hover_cursor(egui::CursorIcon::PointingHand);
        let disk = if hint.is_empty() { disk } else { disk.on_hover_text(hint) };
        let target = if folder.clicked() {
            Some(self.panes[index].path.clone())
        } else if disk.clicked() {
            self.panes[index].path.ancestors().last().map(Path::to_path_buf)
        } else {
            None
        };
        if let Some(target) = target {
            self.active = index;
            if let Err(error) = (self.system.run_verb)(&target, "properties") {
                self.status = error;
            }
        }
    }

    /// 스킨 세로 막대 너비(원본 FUN_00486eec: 막대 너비 = scrollbg 그림 너비). 세 그림(scrollbg·scrollbtn·scrollthumb)이 다 있어야 쓴다.
    pub(super) fn skin_scrollbar_width(&mut self, ctx: &egui::Context, skin: &Skin) -> Option<f32> {
        let file = |key: &str| skin.values.get(&format!("list.{key}")).cloned();
        let background = self.toolbar_icon(ctx, &file("scrollbg")?)?;
        let _ = self.toolbar_icon(ctx, &file("scrollbtn")?)?;
        let _ = self.toolbar_icon(ctx, &file("scrollthumb")?)?;
        Some(background.size_vec2().x)
    }

    /// 원본 TNXScrollBar.Paint(0x7cec0c, 세로): 바탕 scrollbg를 막대 전체에 늘려 그리고, 위·아래 단추는 scrollbtn(왼쪽 반 보통, 오른쪽 반 누름,
    /// 아래 단추는 뒤집음, 길이 = 그림 높이), 엄지는 scrollthumb의 위 8·가운데 8씩·아래 8. 단추는 한 줄씩, 엄지 밖 바탕은 한 쪽씩 옮기고
    /// 누르고 있으면 80ms마다 되풀이(DoScrollTimer), 엄지는 끌어 옮긴다. 바뀐 위치를 돌려준다.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn skin_vertical_scrollbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, skin: &Skin, id: egui::Id, rect: egui::Rect, offset: f32, content: f32, viewport: f32, line: f32) -> Option<f32> {
        let file = |key: &str| skin.values.get(&format!("list.{key}")).cloned();
        let background = self.toolbar_icon(ctx, &file("scrollbg")?)?;
        let buttons = self.toolbar_icon(ctx, &file("scrollbtn")?)?;
        let thumb = self.toolbar_icon(ctx, &file("scrollthumb")?)?;
        let painter = ui.painter().with_clip_rect(rect);
        let whole = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
        painter.image(background.id(), rect, whole, Color32::WHITE);
        let button = buttons.size_vec2().y;
        let half = buttons.size_vec2().x / 2.0;
        let max = (content - viewport).max(0.0);
        let (thumb_top, length) = scrollbar_thumb(rect.height(), rect.width(), button, offset, content, viewport);
        let thumb_rect = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + thumb_top), egui::vec2(rect.width(), length));
        // 바탕을 먼저 등록하고 단추·엄지를 나중에 등록해 그쪽이 먼저 포인터를 받는다.
        let track = ui.interact(rect, id.with("track"), egui::Sense::click_and_drag());
        let up = ui.interact(egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), button)), id.with("up"), egui::Sense::click_and_drag());
        let down = ui.interact(egui::Rect::from_min_size(egui::pos2(rect.left(), rect.bottom() - button), egui::vec2(rect.width(), button)), id.with("down"), egui::Sense::click_and_drag());
        let grip = ui.interact(thumb_rect, id.with("thumb"), egui::Sense::drag());
        for (response, flip) in [(&up, false), (&down, true)] {
            let pressed = response.is_pointer_button_down_on() && response.hovered();
            let (u0, u1) = if pressed { (0.5, 1.0) } else { (0.0, 0.5) };
            let uv = if flip { egui::Rect::from_min_max(egui::pos2(u0, 1.0), egui::pos2(u1, 0.0)) } else { egui::Rect::from_min_max(egui::pos2(u0, 0.0), egui::pos2(u1, 1.0)) };
            painter.image(buttons.id(), egui::Rect::from_min_size(response.rect.min, egui::vec2(half, button)), uv, Color32::WHITE);
        }
        let size = thumb.size_vec2();
        let piece = |source: f32, height: f32, at: f32| {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, source / size.y), egui::pos2(1.0, (source + height) / size.y));
            painter.image(thumb.id(), egui::Rect::from_min_size(egui::pos2(rect.left(), thumb_rect.top() + at), egui::vec2(size.x, height)), uv, Color32::WHITE);
        };
        if length > 16.0 {
            if length < 24.0 {
                piece(8.0, length - 16.0, 8.0);
            } else {
                let mut at = 8.0;
                while at <= length - 16.0 {
                    piece(8.0, 8.0, at);
                    at += 8.0;
                }
                piece(8.0, 8.0, length - 16.0);
            }
        }
        piece(0.0, 8.0, 0.0);
        piece(size.y - 8.0, 8.0, length - 8.0);
        // 끌기: 잡은 자리(엄지 위에서 포인터까지)를 두고 엄지 위치를 옮긴다.
        let grab_id = id.with("grab");
        let pointer = ui.input(|i| i.pointer.interact_pos());
        if grip.drag_started() {
            if let Some(p) = pointer {
                ui.data_mut(|d| d.insert_temp(grab_id, p.y - thumb_rect.top()));
            }
        }
        if grip.dragged() {
            let grab = ui.data(|d| d.get_temp::<f32>(grab_id)).unwrap_or(length / 2.0);
            let usable = rect.height() - 2.0 * button - length;
            if let (Some(p), true) = (pointer, usable > 0.0) {
                let fraction = ((p.y - grab - rect.top() - button) / usable).clamp(0.0, 1.0);
                return Some((fraction * max).round());
            }
            return None;
        }
        // 단추·바탕: 누르는 순간 한 번, 누르고 있으면 300ms 뒤부터 80ms마다. 고친 것: 전에는 처음부터 80ms마다라 너무 빨리 넘어갔다.
        let repeat_id = id.with("repeat");
        let held = [(&up, -line), (&down, line)].into_iter().find(|(r, _)| r.is_pointer_button_down_on()).map(|(_, step)| step).or_else(|| {
            let p = pointer?;
            (track.is_pointer_button_down_on() && !grip.is_pointer_button_down_on()).then(|| if p.y < thumb_rect.top() { -viewport } else if p.y > thumb_rect.bottom() { viewport } else { 0.0 }).filter(|s| *s != 0.0)
        });
        let Some(step) = held else {
            ui.data_mut(|d| d.remove::<(f64, bool)>(repeat_id));
            return None;
        };
        // TNXScrollBar MouseDown(0x7ce8b8): one step at once, timer 300 ms; DoScrollTimer(0x7ce734) then 80 ms.
        let now = ui.input(|i| i.time);
        let state = ui.data(|d| d.get_temp::<(f64, bool)>(repeat_id));
        let wait = if state.is_some_and(|(_, repeated)| repeated) { 0.08 } else { 0.3 };
        let due = state.is_none_or(|(last, _)| now - last >= wait);
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
        if !due {
            return None;
        }
        ui.data_mut(|d| d.insert_temp(repeat_id, (now, state.is_some())));
        let next = (offset + step).clamp(0.0, max);
        (next != offset).then_some(next)
    }

    /// 원본 TNXScrollBar.Paint(0x7cec0c, 가로): 바탕은 시계 방향으로 돌린 scrollbg를 가로로 이어 붙이고(FUN_007cdfc0),
    /// 왼쪽 단추는 scrollbtn 반을 반시계 방향(FUN_007ce0e4), 오른쪽 단추는 시계 방향으로 돌린다(누르면 오른쪽 반).
    /// 엄지도 시계 방향이라 그림의 위 8은 오른쪽 끝, 아래 8은 왼쪽 끝에 온다. 단추 한 번에 `line`, 바탕은 `viewport`씩, 엄지는 끌기.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn skin_horizontal_scrollbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, skin: &Skin, id: egui::Id, rect: egui::Rect, offset: f32, content: f32, viewport: f32, line: f32) -> Option<f32> {
        let file = |key: &str| skin.values.get(&format!("list.{key}")).cloned();
        let background = self.toolbar_icon(ctx, &file("scrollbg")?)?;
        let buttons = self.toolbar_icon(ctx, &file("scrollbtn")?)?;
        let thumb = self.toolbar_icon(ctx, &file("scrollthumb")?)?;
        let painter = ui.painter().with_clip_rect(rect);
        let whole = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
        let bg = background.size_vec2();
        let mut x = rect.left();
        while x < rect.right() && bg.y > 0.0 {
            rotated_image(&painter, background.id(), egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(bg.y, bg.x)), whole, true);
            x += bg.y;
        }
        let button = buttons.size_vec2().y;
        let half = buttons.size_vec2().x / 2.0;
        let max = (content - viewport).max(0.0);
        let (thumb_left, length) = scrollbar_thumb(rect.width(), rect.height(), button, offset, content, viewport);
        let thumb_rect = egui::Rect::from_min_size(egui::pos2(rect.left() + thumb_left, rect.top()), egui::vec2(length, rect.height()));
        let track = ui.interact(rect, id.with("track"), egui::Sense::click_and_drag());
        let left = ui.interact(egui::Rect::from_min_size(rect.min, egui::vec2(button, rect.height())), id.with("left"), egui::Sense::click_and_drag());
        let right = ui.interact(egui::Rect::from_min_size(egui::pos2(rect.right() - button, rect.top()), egui::vec2(button, rect.height())), id.with("right"), egui::Sense::click_and_drag());
        let grip = ui.interact(thumb_rect, id.with("thumb"), egui::Sense::drag());
        for (response, clockwise) in [(&left, false), (&right, true)] {
            let pressed = response.is_pointer_button_down_on() && response.hovered();
            let uv = if pressed { egui::Rect::from_min_max(egui::pos2(0.5, 0.0), egui::pos2(1.0, 1.0)) } else { egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(0.5, 1.0)) };
            rotated_image(&painter, buttons.id(), egui::Rect::from_min_size(response.rect.min, egui::vec2(button, half)), uv, clockwise);
        }
        let size = thumb.size_vec2();
        let piece = |source: f32, width: f32, at: f32| {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, source / size.y), egui::pos2(1.0, (source + width) / size.y));
            rotated_image(&painter, thumb.id(), egui::Rect::from_min_size(egui::pos2(thumb_rect.left() + at, rect.top()), egui::vec2(width, size.x)), uv, true);
        };
        if length > 16.0 {
            if length < 24.0 {
                piece(8.0, length - 16.0, 8.0);
            } else {
                let mut at = 8.0;
                while at <= length - 16.0 {
                    piece(8.0, 8.0, at);
                    at += 8.0;
                }
                piece(8.0, 8.0, length - 16.0);
            }
        }
        piece(size.y - 8.0, 8.0, 0.0);
        piece(0.0, 8.0, length - 8.0);
        let grab_id = id.with("grab");
        let pointer = ui.input(|i| i.pointer.interact_pos());
        if grip.drag_started() {
            if let Some(p) = pointer {
                ui.data_mut(|d| d.insert_temp(grab_id, p.x - thumb_rect.left()));
            }
        }
        if grip.dragged() {
            let grab = ui.data(|d| d.get_temp::<f32>(grab_id)).unwrap_or(length / 2.0);
            let usable = rect.width() - 2.0 * button - length;
            if let (Some(p), true) = (pointer, usable > 0.0) {
                let fraction = ((p.x - grab - rect.left() - button) / usable).clamp(0.0, 1.0);
                return Some((fraction * max).round());
            }
            return None;
        }
        let repeat_id = id.with("repeat");
        let held = [(&left, -line), (&right, line)].into_iter().find(|(r, _)| r.is_pointer_button_down_on()).map(|(_, step)| step).or_else(|| {
            let p = pointer?;
            (track.is_pointer_button_down_on() && !grip.is_pointer_button_down_on()).then(|| if p.x < thumb_rect.left() { -viewport } else if p.x > thumb_rect.right() { viewport } else { 0.0 }).filter(|s| *s != 0.0)
        });
        let Some(step) = held else {
            ui.data_mut(|d| d.remove::<(f64, bool)>(repeat_id));
            return None;
        };
        // TNXScrollBar MouseDown(0x7ce8b8): one step at once, timer 300 ms; DoScrollTimer(0x7ce734) then 80 ms.
        let now = ui.input(|i| i.time);
        let state = ui.data(|d| d.get_temp::<(f64, bool)>(repeat_id));
        let wait = if state.is_some_and(|(_, repeated)| repeated) { 0.08 } else { 0.3 };
        let due = state.is_none_or(|(last, _)| now - last >= wait);
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(wait));
        if !due {
            return None;
        }
        ui.data_mut(|d| d.insert_temp(repeat_id, (now, state.is_some())));
        let next = (offset + step).clamp(0.0, max);
        (next != offset).then_some(next)
    }

    /// 원본 목록 줄 색 묶음(스킨 테마).
    pub(super) fn original_list_colors(&self) -> ListColors {
        let c = |key: &str| self.basic_list_color(key);
        ListColors {
            back: c("NXBACK"),
            select: c("NXSELECT"),
            size: c("NXSIZE"),
            readonly: c("NXREADONLY"),
            hidden: c("NXHIDDEN"),
            mb: c("NXMB"),
            gb: c("NXGB"),
            time: c("NXTIME"),
            attr: c("NXATTR"),
            desc: c("NXDESC"),
        }
    }

    /// 원본 색상표 기본 색(TNXFileColorMap.GetBasic): 색상표 값이 있으면 그 값, 없으면 원본 기본값(styles::DEFAULT_ELEMENTS).
    pub(super) fn basic_list_color(&self, key: &str) -> Color32 {
        let (_, nexus) = self.color_maps();
        nexus
            .and_then(|map| map.get(key))
            .and_then(|v| v.as_str())
            .and_then(ndr::styles::parse_color)
            .or_else(|| ndr::styles::DEFAULT_ELEMENTS.iter().find(|(k, _)| *k == key).and_then(|(_, v)| ndr::styles::parse_color(v)))
            .map(|[r, g, b, _]| Color32::from_rgb(r, g, b))
            .unwrap_or(Color32::BLACK)
    }

    /// 원본 TNXFileListView.DrawListHeader(0x7e2274, 자세히 보기): 높이 = 목록 글꼴 높이 + 8(CMFontChanged), 바탕은 목록 색,
    /// 칸 이름은 왼쪽 + 5부터(한 줄, 넘치면 …) NXFORE × 0.4 + NXBACK × 0.6, 정렬 칸은 × 0.6에 맨 위 가운데 5×3 삼각형
    /// (내림차순 ▼, 오름차순 ▲, × 0.3). 칸 오른쪽 끝에 세로선 NXLINE × 0.3 + NXBACK × 0.7. 누른 칸의 정렬 방식을 돌려준다.
    pub(super) fn skin_list_header(&self, ui: &mut egui::Ui, index: usize, columns: &[(super::list_view::DetailColumn, String, f32)], widths: &[Option<f32>], font: egui::FontId, scroll_x: f32) -> Option<&'static str> {
        let (back, fore, line) = (self.basic_list_color("NXBACK"), self.basic_list_color("NXFORE"), self.basic_list_color("NXLINE"));
        let height = ui.fonts_mut(|f| f.row_height(&font)).ceil() + 8.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, back);
        let current = self.side_text(index, "SortMode", "Name");
        let descending = self.side_flag(index, "SortDescending", false);
        // 원본 DrawListHeader(0x7e2274): 칸은 GetListHeaderWidth − 가로 위치(0x378)부터.
        let mut x = rect.left() - scroll_x;
        let mut clicked = None;
        // 원본은 머리글을 GetListHeaderWidth부터 그린다. NDR 이름 칸은 그 머리를 담으므로 이름 칸 글은 머리 너비 뒤 + 5(이름 글과 같은 자리).
        let lead = Self::list_header_width(self.settings.flag("ShowIcons", true));
        for (i, ((column, title, _), width)) in columns.iter().zip(widths).enumerate() {
            let Some(width) = *width else { continue };
            let cell = egui::Rect::from_min_max(egui::pos2(x, rect.top()), egui::pos2(x + width, rect.bottom()));
            let start = if *column == super::list_view::DetailColumn::Name { lead } else { 0.0 };
            let text = egui::Rect::from_min_max(egui::pos2(x + start + 5.0, rect.top()), cell.max);
            let mode = column.sort_mode();
            let sorted = mode.is_some() && mode == Some(current.as_str());
            let color = mix_color(fore, back, if sorted { 0.6 } else { 0.4 });
            if sorted {
                let mark = mix_color(fore, back, 0.3);
                let x0 = (text.left() + ((text.width() as i32) >> 1) as f32 - 2.0).floor();
                for row in 0..3 {
                    let (start, count) = (row as f32, 5 - 2 * row);
                    let y = if descending { rect.top() + row as f32 } else { rect.top() + 2.0 - row as f32 };
                    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(x0 + start, y), egui::vec2(count as f32, 1.0)), 0.0, mark);
                }
            }
            let mut job = egui::text::LayoutJob::simple_singleline(title.clone(), font.clone(), color);
            job.wrap = egui::text::TextWrapping { max_width: text.width().max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(egui::pos2(text.left(), rect.top() + ((rect.height() - galley.size().y) / 2.0).floor()), galley, color);
            painter.vline(cell.right() - 0.5, rect.y_range(), egui::Stroke::new(1.0, mix_color(line, back, 0.3)));
            let hit = cell.intersect(rect);
            if hit.width() > 0.0 && ui.interact(hit, egui::Id::new(("ndr-skin-header", index, i)), egui::Sense::click()).clicked() {
                clicked = mode;
            }
            x = cell.right();
        }
        clicked
    }

    /// 원본 드라이브 단추 줄(스킨 테마, sdrivebtns: FUN_004825d8 높이 22, 단추 FUN_00483108, 색 FUN_004895fc, 그리기 FUN_0048bbcc).
    /// 바탕 ldrivesback, 아래끝 선 ldrivesframe, 드라이브마다 TNXImageButton(왼쪽 = 순서 × 36, 너비 36·높이 20)에 셸 작은 아이콘과 "C:",
    /// 글 ldrivestext, 포인터 ldriveshot. 누르면 이 패널을 그 드라이브로(FUN_0048bdb4).
    pub(super) fn skin_drive_bar(&mut self, ui: &mut egui::Ui, index: usize, ctx: &egui::Context, skin: &Skin) {
        if !self.settings.flag("ShowDriveButtons", false) {
            return;
        }
        let drives = self.drive_button_list();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), DRIVE_BAR_HEIGHT), egui::Sense::hover());
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, skin_color(skin, "ldrivesback", Color32::from_gray(0x20)));
        painter.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, skin_color(skin, "ldrivesframe", Color32::from_gray(0x1A))));
        let (text, hot) = (skin_color(skin, "ldrivestext", Color32::from_gray(0xCC)), skin_color(skin, "ldriveshot", Color32::WHITE));
        let mut chosen = None;
        for (i, (root, tip)) in drives.iter().enumerate() {
            let button = egui::Rect::from_min_size(rect.min + egui::vec2(i as f32 * 36.0, 0.0), egui::vec2(36.0, 20.0));
            let response = ui.interact(button, egui::Id::new(("ndr-skin-drive", index, i)), egui::Sense::click());
            let label = root.to_string_lossy().trim_end_matches(['\\', '/']).to_owned();
            let font = egui::FontId::proportional(super::BASE_TEXT_SIZE);
            let label_color = if response.hovered() { hot } else { text };
            let caption = ui.fonts_mut(|f| f.layout_no_wrap(label.clone(), font.clone(), label_color));
            // 원본 TNXImageButton.Paint(글 배치 1, 간격 +0x1f4 = 2, 그림 16×16): 그림 x = (너비 − (간격 + 글 너비 + 그림 너비)) div 2,
            // 글 x = 그림 x + 그림 너비 + 간격, 둘 다 세로 가운데((높이 − 높이) div 2). 그림이 없으면 그림 너비 0.
            let icon = self.drive_icon(ctx, root);
            let icon_width = if icon.is_some() { 16.0 } else { 0.0 };
            let image_x = button.left() + ((button.width() - (2.0 + caption.size().x.round() + icon_width)) / 2.0).trunc();
            if let Some(icon) = icon {
                let whole = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                // 원본 TNXImageButton: 누른 동안 그림만 × 0.6(글은 그 뒤에 그린다).
                let tint = if response.is_pointer_button_down_on() && response.hovered() { Color32::from_gray(153) } else { Color32::WHITE };
                painter.image(icon.id(), egui::Rect::from_min_size(egui::pos2(image_x, button.top() + ((button.height() - 16.0) / 2.0).trunc()), egui::vec2(16.0, 16.0)), whole, tint);
            }
            let text_pos = egui::pos2(image_x + icon_width + 2.0, button.top() + ((button.height() - caption.size().y) / 2.0).trunc());
            painter.galley(text_pos, caption, label_color);
            if response.on_hover_text(tip).clicked() {
                chosen = Some(root.clone());
            }
        }
        if let Some(root) = chosen {
            self.active = index;
            self.navigate_drive(index, root);
        }
    }

    /// 원본 파일 정보 표시줄 높이(FUN_00482d44): 보이지 않으면 0, 확장자 정보 줄(sextinfobar)까지면 40, 아니면 20.
    pub(super) fn skin_file_info_height(&self) -> f32 {
        if !self.settings.flag("ShowFileInfoBar", true) {
            0.0
        } else if self.settings.flag("ShowExtensionInfoBar", false) {
            40.0
        } else {
            20.0
        }
    }

    /// 원본 파일 정보 표시줄(스킨 테마, FUN_00482d44 + 색 FUN_00489c54): 목록 아래, 배경 lfileback, 글 lfiletext,
    /// 첫 줄 라벨(0x34c)은 왼쪽 6·위 3·높이 14, 둘째 줄(0x350)은 위 22.
    pub(super) fn skin_file_info_bar(&mut self, ui: &mut egui::Ui, index: usize, skin: &Skin) {
        let height = self.skin_file_info_height();
        if height == 0.0 {
            return;
        }
        let texts = self.info_texts(index);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::hover());
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, skin_color(skin, "lfileback", Color32::from_gray(0x3C)));
        let base = skin_color(skin, "lfiletext", Color32::from_gray(0xA2));
        let width = (rect.width() - 12.0).max(0.0);
        // 원본 둘째 줄(라벨 0x350, 확장자 정보)은 글이 비어 있고, 누르면 FUN_0048bd50: 초점 항목이 파일(종류 3)이면
        // ShellExecute "open" https://ext.xiles.app/?ext=<항목 +8 = 확장자>.
        let empty = String::new();
        let lines: &[(&String, f32)] = if height > 20.0 { &[(&texts[1], 3.0), (&empty, 22.0)] } else { &[(&texts[1], 3.0)] };
        let mut open_extension = false;
        for (text, top) in lines {
            let mut job = skin_markup_job(text, base);
            job.wrap = egui::text::TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            let at = egui::pos2(rect.left() + 6.0, rect.top() + top + ((14.0 - galley.size().y) / 2.0).floor());
            let line = egui::Rect::from_min_size(at, egui::vec2(width, 14.0));
            painter.galley(at, galley, base);
            if *top > 20.0 {
                open_extension |= ui.interact(line, egui::Id::new(("ndr-skin-file-info", index, *top as i32)), egui::Sense::click()).clicked();
            } else {
                ui.interact(line, egui::Id::new(("ndr-skin-file-info", index, *top as i32)), egui::Sense::hover()).on_hover_text(ndr::markup::plain_text(text));
            }
        }
        if open_extension {
            self.open_extension_info(index);
        }
    }

    /// 원본 FUN_0048bd50(확장자 정보 줄 누르기): 초점 항목이 파일이면 https://ext.xiles.app/?ext=%s(0x913242)를 연다.
    /// %s = 항목 +8(색상표 GetExtension이 찾는 확장자, 점 없이).
    pub(super) fn open_extension_info(&mut self, index: usize) {
        let pane = &self.panes[index];
        let Some(entry) = pane.entries.get(pane.cursor).filter(|e| !e.is_dir && !e.drive) else { return };
        let extension = ndr::parity::list_extension(&entry.name).trim_start_matches('.').to_owned();
        let url = format!("https://ext.xiles.app/?ext={extension}");
        if let Err(error) = (self.shell_runner)(std::path::Path::new(&url), "", std::path::Path::new("")) {
            self.status = error;
        }
    }

    /// 주소 줄 ">"·맨 앞 ▼를 누른 뒤의 하위 폴더(또는 드라이브) 목록. 연 뒤 밖을 누르거나 Esc면 닫는다.
    fn skin_crumb_menu_ui(&mut self, ctx: &egui::Context, index: usize, just_opened: bool) {
        let Some((pane, folder, drives, position)) = self.skin_crumb_menu.clone() else { return };
        if pane != index {
            return;
        }
        let children = self.crumb_children(index, &folder, drives);
        // 원본 DropDownMenu(0x7c91d8): 항목이 없으면 메뉴를 띄우지 않는다(빈 TPopupMenu). 고친 것: 전에는 "하위 폴더 없음"을 보였다.
        if children.is_empty() {
            self.skin_crumb_menu = None;
            return;
        }
        let icons: Vec<Option<egui::TextureHandle>> = children.iter().map(|(child, _)| if drives { self.drive_icon(ctx, child) } else { self.path_icon(ctx, child, true) }).collect();
        let mut chosen = None;
        // 원본 경로 라벨 메뉴는 TfMain 메뉴의 반투명(MenuDrawItem 0x42e518의 WS_EX_LAYERED)을 쓰지 않는다.
        super::keep_opaque(ctx, egui::Id::new(("ndr-skin-crumb-menu", index)));
        let area = egui::Area::new(egui::Id::new(("ndr-skin-crumb-menu", index))).order(egui::Order::Foreground).fixed_pos(position).show(ctx, |ui| {
            // 원본 MenuMeasureItem(0x7ca4dc)·MenuDrawItem(0x7ca540): 스킨과 상관없이 흰 바탕(0xFFFFFF)에 글 0x0A0A0A, 고른 줄은
            // 회색(0x808080) 바탕에 흰 글. 아이콘(작은 셸 아이콘)은 왼쪽 + 4, 글은 왼쪽 + 4 + 0x14(DT_SINGLELINE | DT_VCENTER),
            // 너비 = 글 너비 + 0x1C. 테두리는 Windows 메뉴 테두리.
            let font = egui::TextStyle::Button.resolve(ui.style());
            let widest = ui.fonts_mut(|f| children.iter().map(|(_, h)| f.layout_no_wrap(h.clone(), font.clone(), Color32::BLACK).size().x).fold(0.0f32, f32::max));
            let item_width = (widest + 28.0).ceil();
            let item_height = (ui.fonts_mut(|f| f.row_height(&font)) + 6.0).ceil().max(20.0);
            egui::Frame::new().fill(Color32::WHITE).stroke(egui::Stroke::new(1.0, Color32::from_gray(0xA0))).inner_margin(egui::Margin::same(2)).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for ((child, header), icon) in children.iter().zip(&icons) {
                        let (rect, response) = ui.allocate_exact_size(egui::vec2(item_width, item_height), egui::Sense::click());
                        let selected = response.hovered();
                        let (back, fore) = if selected { (Color32::from_gray(0x80), Color32::WHITE) } else { (Color32::WHITE, Color32::from_gray(0x0A)) };
                        ui.painter().rect_filled(rect, 0.0, back);
                        if let Some(icon) = icon {
                            let top = rect.top() + ((rect.height() - 16.0) / 2.0).floor();
                            egui::Image::new((icon.id(), egui::vec2(16.0, 16.0))).paint_at(ui, egui::Rect::from_min_size(egui::pos2(rect.left() + 4.0, top), egui::vec2(16.0, 16.0)));
                        }
                        let galley = ui.fonts_mut(|f| f.layout_no_wrap(header.clone(), font.clone(), fore));
                        ui.painter().galley(egui::pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0), galley, fore);
                        if response.clicked() {
                            chosen = Some(child.clone());
                        }
                    }
                });
            });
        });
        let outside = !just_opened && ctx.input(|i| i.pointer.any_click()) && !area.response.hovered() && !area.response.rect.contains(ctx.input(|i| i.pointer.interact_pos()).unwrap_or(position));
        if let Some(child) = chosen {
            self.skin_crumb_menu = None;
            self.go_crumb(index, CrumbTarget::Local(child));
        } else if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.skin_crumb_menu = None;
        }
    }
}
