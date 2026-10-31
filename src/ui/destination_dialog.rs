//! C# SelectDestinationFolder + DestinationPickerWindow ([formNDRDestination]): most used destinations (MostUsedDestinations, by count then recency,
//! existing folders only, MaxRecentFileCount entries, removable from the right-click menu), work folders 1-5 (number keys), browse for another folder.
//! RememberDestination (LastDestination + use count and last use time) and CopyMoveToFolderAsync.
//! Inside an archive, copy chooses the extraction destination (EXTRACT title) like original FUN_00439bac, and move does nothing.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{IDYES, MB_ICONASTERISK, MB_ICONQUESTION, MB_OK, MB_YESNO};

/// Copy, move or extract (title key).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DestinationPurpose {
    Copy,
    Move,
    /// Original miFTPGetClick (0x4412f8): FTP_GET + SELECTDEST.
    FtpGet,
    /// Used when porting the extraction flow (ExtractArchiveAsync).
    #[allow(dead_code)]
    Extract,
}

/// Window text and list.
#[derive(Clone, Debug)]
pub(super) struct DestinationForm {
    pub most_used: Vec<(String, u64)>,
    pub work_folders: [String; 5],
    pub browse_label: String,
    pub cancel_label: String,
    pub remove_label: String,
    pub clear_label: String,
    pub initial: PathBuf,
    /// Original TfSelectDest cbPath: editable destination path (initially the other pane folder). OK or Enter goes to that folder (a folder in the same pane works too).
    pub path: String,
    pub ok_label: String,
    /// Skin theme: the original TfSelectDest layout (list lbList + cbPath and buttons below). Otherwise the C# DestinationPickerWindow layout.
    pub original: bool,
    /// Selected row of the original lbList (ItemIndex). Selecting puts its path in cbPath (lbListClick 0x453b40).
    pub selected: usize,
    /// Original drive rows (kind 8): the drive list when the window opened.
    pub drives: Vec<DestinationDrive>,
    /// Free space text of drive rows (second text of the original "%10s %s", RES.DISKFREE).
    pub free_label: String,
    /// Original FormKeyDown (0x454b8c) Ctrl+D/G/Q/S/O = FUN_0040db88(0-4): Desktop (CSIDL 0x10), Documents (5), Pictures (0x27), Music (0xd),
    /// Videos (0xe); for 1-4 the configured special folder wins. Paths resolved when the window opened (None when missing).
    pub special_folders: [Option<String>; 5],
}

/// Original TfSelectDest list rows (FUN_004532d4): kind 1 = initial folder (+0x420, the other pane), 2 = recent (most used folders +0x434), 4 = work folder (+0x438),
/// 8 = drive (TNXDriveList, CD/DVD (5) excluded).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DestinationItem {
    Initial(String),
    Recent(String),
    Work(usize, String),
    Drive(DestinationDrive),
}

/// Drive row data of the destination window (TNXDriveInfo: root, GetDriveName +0x1c, type +0x18, for fixed disks (free +0x10, total +8)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DestinationDrive {
    pub root: String,
    pub name: String,
    pub kind: u32,
    pub sizes: Option<(u64, u64)>,
}

impl DestinationItem {
    pub(super) fn path(&self) -> &str {
        match self {
            DestinationItem::Initial(p) | DestinationItem::Recent(p) | DestinationItem::Work(_, p) => p,
            DestinationItem::Drive(drive) => &drive.root,
        }
    }
}

/// List of original FUN_004532d4: the initial folder (if any), recent folders other than it, non-empty existing work folders, drives other than CD/DVD.
/// Returns (row, whether it is the first row of a group).
pub(super) fn original_items(form: &DestinationForm) -> Vec<(DestinationItem, bool)> {
    let mut items = Vec::new();
    let initial = form.initial.to_string_lossy().into_owned();
    if !initial.is_empty() && form.initial.is_dir() {
        items.push((DestinationItem::Initial(initial.clone()), true));
    }
    let mut first = true;
    for (path, _) in &form.most_used {
        if ndr::names::equals_ignore_case(path, &initial) {
            continue;
        }
        items.push((DestinationItem::Recent(path.clone()), first));
        first = false;
    }
    let mut first = true;
    for (index, path) in form.work_folders.iter().enumerate() {
        if path.trim().is_empty() || !Path::new(path).is_dir() {
            continue;
        }
        items.push((DestinationItem::Work(index, path.clone()), first));
        first = false;
    }
    let mut first = true;
    for drive in form.drives.iter().filter(|d| d.kind != 5) {
        items.push((DestinationItem::Drive(drive.clone()), first));
        first = false;
    }
    items
}

pub(super) enum DestinationAction {
    /// Original btnOk: the folder in the path field.
    Accept,
    MostUsed(String),
    Work(usize),
    Browse,
    Remove(String),
    Clear,
    Cancel,
}

fn documents() -> PathBuf {
    PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Documents")
}

/// Original lbListKeyDown (0x454130, list has key focus): 1-9 (or Alt+1-9) = work folder, Ctrl+1-9 = Nth recent folder (original +0x434 excludes the initial
/// folder), 0 = initial folder (+0x420), Shift+Delete = clear all history. When the path field has key focus (`typing`) keys are path characters.
fn list_keys(ui: &egui::Ui, form: &DestinationForm, typing: bool) -> Option<DestinationAction> {
    enum Pick {
        Work(usize),
        MostUsed(usize),
        Initial,
        Clear,
        Drive(char),
    }
    const DIGITS: [egui::Key; 10] = [egui::Key::Num0, egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8, egui::Key::Num9];
    if typing {
        return None;
    }
    let typed = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key: egui::Key::Delete, pressed: true, modifiers, .. } if modifiers.shift && !modifiers.ctrl && !modifiers.alt => Some(Pick::Clear),
            // Original lbListKeyDown: A-Z with no modifier or with Shift = confirm with that drive root.
            egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.ctrl && !modifiers.alt && key.name().len() == 1 && key.name().chars().all(|c| c.is_ascii_uppercase()) => key.name().chars().next().map(Pick::Drive),
            egui::Event::Key { key, pressed: true, modifiers, .. } => {
                let n = DIGITS.iter().position(|k| k == key)?;
                let plain = !modifiers.any();
                let alt = modifiers.alt && !modifiers.ctrl && !modifiers.shift;
                let ctrl = modifiers.ctrl && !modifiers.alt && !modifiers.shift;
                match n {
                    0 if plain => Some(Pick::Initial),
                    0 => None,
                    _ if plain || alt => Some(Pick::Work(n - 1)),
                    _ if ctrl => Some(Pick::MostUsed(n - 1)),
                    _ => None,
                }
            }
            _ => None,
        })
    });
    let initial = form.initial.to_string_lossy();
    match typed? {
        Pick::Work(index) if form.work_folders.get(index).is_some_and(|w| !w.trim().is_empty()) => Some(DestinationAction::Work(index)),
        Pick::MostUsed(index) => form
            .most_used
            .iter()
            .map(|(p, _)| p)
            .filter(|p| !form.original || !ndr::names::equals_ignore_case(p, &initial))
            .nth(index)
            .map(|p| DestinationAction::MostUsed(p.clone())),
        Pick::Initial if form.initial.is_dir() => Some(DestinationAction::MostUsed(initial.into_owned())),
        Pick::Clear => Some(DestinationAction::Clear),
        Pick::Drive(letter) => form.drives.iter().find(|d| d.root.chars().next().is_some_and(|c| c.eq_ignore_ascii_case(&letter))).map(|d| DestinationAction::MostUsed(d.root.clone())),
        _ => None,
    }
}

/// Original TfSelectDest (DFM ClientWidth 426 x ClientHeight 428, minimum width 350): lbList on top (white, row height 18, a group's first row + 3 with
/// a 0xC0C0C0 line above), below in a 70-high band cbPath (4,7 width - 8 x 21), btnTree (4,37 138x25), btnOk (width - 188,37 89x25, default),
/// btnCancel (width - 93,37 89x25, cancel). Height like FUN_004532d4: rows x 18 + groups x 3 + 82 (300 to screen - 50); width = longest path + 28
/// (350 to 800). Rows are drawn like lbListDrawItem (0x454de8): icon 16 (left + 4), text at left + 24 with the parent path (gray 0x808080, 0xC0C0C0 when selected) and
/// the name (black, white when selected); the selected row has a 0x5A5A5A->0x323232 background with top lines 0x323232 and 0x787878. Work folder rows add a number to the icon.
/// A click selects and puts that path in the path field (lbListClick), a double click confirms (OnDblClick = btnOkClick), the right button on a recent row opens pmMUFolder.
/// With key focus in the list, Up/Down select and Delete removes the selected recent row (pmiMUFDeleteClick). btnTree (folder tree) is not ported, so
/// the C# version's browse for another folder sits in its place.
pub(super) fn destination_ui_original(ui: &mut egui::Ui, form: &mut DestinationForm, enter: bool, icon: Option<&egui::TextureHandle>, drive_icons: &[Option<egui::TextureHandle>], screen_height: f32) -> Option<DestinationAction> {
    let mut action = None;
    let items = original_items(form);
    if form.selected >= items.len() {
        form.selected = 0;
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let measure = |ui: &egui::Ui, text: &str| ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::BLACK).size().x);
    // Original: the window width uses only the folder row text (+0x414); the drive row "%C: %s" text width (+0x418) positions the free space text.
    let drive_text = |drive: &DestinationDrive| format!("{}: {}", drive.root.chars().next().unwrap_or(' ').to_ascii_uppercase(), drive.name);
    let widest = items.iter().filter(|(item, _)| !matches!(item, DestinationItem::Drive(_))).map(|(item, _)| measure(ui, item.path())).fold(0.0f32, f32::max);
    let drive_width = items.iter().filter_map(|(item, _)| if let DestinationItem::Drive(d) = item { Some(measure(ui, &drive_text(d))) } else { None }).fold(0.0f32, f32::max);
    let sections = items.iter().enumerate().filter(|(i, (_, first))| *i > 0 && *first).count();
    let width = (widest.ceil() + 28.0).clamp(350.0, 800.0);
    let height = (items.len() as f32 * 18.0 + sections as f32 * 3.0 + 82.0).clamp(300.0, (screen_height - 50.0).max(300.0));
    let (area, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let list_rect = egui::Rect::from_min_size(area.min, egui::vec2(width, height - 70.0));
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, height - 70.0 + y), egui::vec2(w, h));
    let list_id = egui::Id::new("ndr-destination-list");
    let path_id = egui::Id::new("ndr-destination-path");
    ui.painter().rect_filled(list_rect, 0.0, Color32::WHITE);
    // The list itself (including empty space) is the widget that takes key focus. It comes before the rows so it does not hide row clicks.
    if ui.interact(list_rect, list_id, egui::Sense::click()).clicked() {
        ui.memory_mut(|m| m.request_focus(list_id));
    }
    let typing = ui.memory(|m| m.has_focus(path_id));
    let list_focused = ui.memory(|m| m.has_focus(list_id));
    if list_focused {
        // Keep egui from moving focus with Up/Down (the original list box selects rows with the arrows).
        ui.memory_mut(|m| m.set_focus_lock_filter(list_id, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
    }
    // List keys (Up/Down, Delete).
    if list_focused && !items.is_empty() {
        let (up, down, delete) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Delete) && !i.modifiers.any()));
        let before = form.selected;
        if up {
            form.selected = form.selected.saturating_sub(1);
        }
        if down {
            form.selected = (form.selected + 1).min(items.len() - 1);
        }
        if form.selected != before {
            form.path = items[form.selected].0.path().to_owned();
        }
        if delete {
            if let (DestinationItem::Recent(path), _) = &items[form.selected] {
                action = Some(DestinationAction::Remove(path.clone()));
            }
        }
    }
    let mut row_top = list_rect.top();
    ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
        ui.set_clip_rect(list_rect);
        for (index, (item, first)) in items.iter().enumerate() {
            let extra = if index > 0 && *first { 3.0 } else { 0.0 };
            let row = egui::Rect::from_min_size(egui::pos2(list_rect.left(), row_top), egui::vec2(width, 18.0 + extra));
            row_top += row.height();
            if row.top() > list_rect.bottom() {
                break;
            }
            let content = egui::Rect::from_min_max(egui::pos2(row.left(), row.top() + extra), row.max);
            let selected = index == form.selected;
            let painter = ui.painter();
            if selected {
                let mut mesh = egui::Mesh::default();
                let (top, bottom) = (Color32::from_rgb(0x5a, 0x5a, 0x5a), Color32::from_rgb(0x32, 0x32, 0x32));
                mesh.colored_vertex(content.left_top(), top);
                mesh.colored_vertex(content.right_top(), top);
                mesh.colored_vertex(content.right_bottom(), bottom);
                mesh.colored_vertex(content.left_bottom(), bottom);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(egui::Shape::mesh(mesh));
                painter.hline(content.x_range(), content.top() + 0.5, egui::Stroke::new(1.0, Color32::from_rgb(0x32, 0x32, 0x32)));
                painter.hline(content.x_range(), content.top() + 1.5, egui::Stroke::new(1.0, Color32::from_rgb(0x78, 0x78, 0x78)));
            }
            if extra > 0.0 {
                painter.hline(row.x_range(), row.top() + 1.5, egui::Stroke::new(1.0, Color32::from_gray(0xc0)));
            }
            let icon_rect = egui::Rect::from_min_size(egui::pos2(content.left() + 4.0, content.center().y - 8.0), egui::vec2(16.0, 16.0));
            let row_icon = match item {
                DestinationItem::Drive(drive) => form.drives.iter().position(|d| d == drive).and_then(|i| drive_icons.get(i)).and_then(|t| t.as_ref()),
                _ => icon,
            };
            if let Some(icon) = row_icon {
                painter.image(icon.id(), icon_rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            if let DestinationItem::Drive(drive) = item {
                // Original lbListDrawItem kind 8: "%C: %s" (drive letter, GetDriveName) at left + 24; for a fixed disk (3) then
                // (at +0x418 widest drive text + 4) "%10s %s" (free space StrFormatByteSizeW, DISKFREE), and from text end + 6 a bar 80 wide (to the end when less than 80
                // remains) x (row - 8): GRAYTEXT frame, inside (inset 1) filled by the used ratio (above 51% sky blue, above 71% orange, above 91% red, each
                // blended with the window color (FUN_004539ec, 150/255); 50% or less is GRAYTEXT).
                let text_color = if selected { Color32::WHITE } else { Color32::BLACK };
                let x = content.left() + 24.0;
                let galley = ui.fonts_mut(|f| f.layout_no_wrap(drive_text(drive), font.clone(), text_color));
                painter.galley(egui::pos2(x, content.center().y - galley.size().y / 2.0), galley, text_color);
                if let (3, Some((free, total))) = (drive.kind, drive.sizes) {
                    let x = x + drive_width + 4.0;
                    let galley = ui.fonts_mut(|f| f.layout_no_wrap(format!("{:>10} {}", ndr::native::format_byte_size(free), form.free_label), font.clone(), text_color));
                    let text_width = galley.size().x;
                    painter.galley(egui::pos2(x, content.center().y - galley.size().y / 2.0), galley, text_color);
                    let left = x + text_width + 6.0;
                    let right = if content.right() - left > 80.0 { left + 80.0 } else { content.right() };
                    if right > left + 2.0 {
                        let bar = egui::Rect::from_min_max(egui::pos2(left, content.top() + 4.0), egui::pos2(right, content.bottom() - 4.0));
                        let gray = { let (r, g, b) = ndr::native::system_color(17); Color32::from_rgb(r, g, b) };
                        painter.rect_stroke(bar, 0.0, egui::Stroke::new(1.0, gray), egui::StrokeKind::Inside);
                        let used = if total == 0 { 100 } else { (total.saturating_sub(free) as u128 * 100 / total as u128) as u32 };
                        let window = { let (r, g, b) = ndr::native::system_color(5); Color32::from_rgb(r, g, b) };
                        let tint = |c: Color32| {
                            let t = (255.0 - 150.0) / 255.0;
                            let mix = |a: u8, b: u8| (a as f32 * (1.0 - t) + b as f32 * t).round() as u8;
                            Color32::from_rgb(mix(c.r(), window.r()), mix(c.g(), window.g()), mix(c.b(), window.b()))
                        };
                        let fill = match used {
                            91.. => tint(Color32::from_rgb(0xff, 0, 0)),
                            71.. => tint(Color32::from_rgb(0xff, 0x80, 0)),
                            51.. => tint(Color32::from_rgb(0xa6, 0xca, 0xf0)),
                            _ => gray,
                        };
                        let inner = bar.shrink(1.0);
                        let filled = egui::Rect::from_min_max(inner.min, egui::pos2(inner.left() + ((inner.width() - 2.0).max(0.0) * used as f32 / 100.0).floor(), inner.bottom()));
                        painter.rect_filled(filled, 0.0, fill);
                    }
                }
                let response = ui.interact(row, list_id.with(index), egui::Sense::click());
                if response.clicked() || super::panes::windows_double_clicked(&response) || response.secondary_clicked() {
                    form.selected = index;
                    if !response.secondary_clicked() {
                        form.path = drive.root.clone();
                    }
                    ui.memory_mut(|m| m.request_focus(list_id));
                }
                if super::panes::windows_double_clicked(&response) {
                    action = Some(DestinationAction::Accept);
                }
                continue;
            }
            if let DestinationItem::Work(n, _) = item {
                // A small number box instead of the original +0x424 number image.
                let badge = egui::Rect::from_min_size(icon_rect.right_bottom() - egui::vec2(8.0, 9.0), egui::vec2(9.0, 10.0));
                painter.rect_filled(badge, 1.0, Color32::from_rgb(0x20, 0x50, 0xa0));
                painter.text(badge.center(), egui::Align2::CENTER_CENTER, (n + 1).to_string(), egui::FontId::proportional(9.0), Color32::WHITE);
            }
            let path = item.path();
            // Original: text shortened to the row width (left + 24 to right) with DT_PATH_ELLIPSIS | DT_MODIFYSTRING, split into parent path and name.
            let shown = super::original_dialogs::path_ellipsis(ui, path, (content.right() - content.left() - 24.0).max(0.0), &font);
            let trimmed = shown.trim_end_matches('\\');
            let (parent, name) = match trimmed.rfind('\\') {
                Some(cut) => (&shown[..=cut], &trimmed[cut + 1..]),
                None => ("", shown.as_str()),
            };
            let (parent_color, name_color) = if selected { (Color32::from_gray(0xc0), Color32::WHITE) } else { (Color32::from_gray(0x80), Color32::BLACK) };
            let mut x = content.left() + 24.0;
            if !parent.is_empty() {
                let galley = ui.fonts_mut(|f| f.layout_no_wrap(parent.to_owned(), font.clone(), parent_color));
                let y = content.center().y - galley.size().y / 2.0;
                let w = galley.size().x;
                painter.galley(egui::pos2(x, y), galley, parent_color);
                x += w + 2.0;
            }
            let galley = ui.fonts_mut(|f| f.layout_no_wrap(name.to_owned(), font.clone(), name_color));
            let y = content.center().y - galley.size().y / 2.0;
            painter.galley(egui::pos2(x, y), galley, name_color);
            let response = ui.interact(row, list_id.with(index), egui::Sense::click());
            if response.clicked() || super::panes::windows_double_clicked(&response) {
                form.selected = index;
                form.path = path.to_owned();
                ui.memory_mut(|m| m.request_focus(list_id));
            }
            // Original lbListMouseDown (0x4546ac): the right button only changes ItemIndex (not OnClick, so the path field stays).
            if response.secondary_clicked() {
                form.selected = index;
                ui.memory_mut(|m| m.request_focus(list_id));
            }
            if super::panes::windows_double_clicked(&response) {
                action = Some(DestinationAction::Accept);
            }
            if let DestinationItem::Recent(recent) = item {
                super::skin_pane::context_menu(&response, |ui| {
                    if ui.button(&form.remove_label).clicked() {
                        action = Some(DestinationAction::Remove(recent.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(&form.clear_label).clicked() {
                        action = Some(DestinationAction::Clear);
                        ui.close();
                    }
                });
            }
        }
    });
    // Bottom band: cbPath (key focus when the window opens, original FormShow), browse in btnTree's place, btnOk, btnCancel.
    let edit = ui.put(at(4.0, 7.0, width - 8.0, 21.0), egui::TextEdit::singleline(&mut form.path).id(path_id));
    // Original cbPath Hint "Alt+D": Alt+D gives the path field key focus.
    let alt_d = ui.input(|i| i.modifiers.alt && !i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::D));
    if alt_d || (!ui.memory(|m| m.had_focus_last_frame(path_id)) && ui.memory(|m| m.focused().is_none())) {
        edit.request_focus();
    }
    // Original FormKeyDown (0x454b8c, KeyPreview): Ctrl with D/G/Q/S/O puts the Desktop/Documents/Pictures/Music/Videos path in cbPath, focuses it and selects all.
    let special = [egui::Key::D, egui::Key::G, egui::Key::Q, egui::Key::S, egui::Key::O];
    if let Some(index) = ui.input(|i| if i.modifiers.ctrl { special.iter().position(|k| i.key_pressed(*k)) } else { None }) {
        if let Some(path) = form.special_folders[index].clone().filter(|p| !p.is_empty()) {
            let length = path.chars().count();
            form.path = path;
            edit.request_focus();
            let mut state = egui::TextEdit::load_state(ui.ctx(), path_id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(length))));
            state.store(ui.ctx(), path_id);
        }
    }
    // Original btnTree (4,37 138x25, "&Folder Tree") -> btnTreeClick (FUN_00453de4).
    if super::original_dialogs::access_button(ui, at(4.0, 37.0, 138.0, 25.0), &form.browse_label) {
        action = Some(DestinationAction::Browse);
    }
    if ui.put(at(width - 188.0, 37.0, 89.0, 25.0), egui::Button::new(&form.ok_label)).clicked() {
        action = Some(DestinationAction::Accept);
    }
    if ui.put(at(width - 93.0, 37.0, 89.0, 25.0), egui::Button::new(&form.cancel_label)).clicked() {
        action = Some(DestinationAction::Cancel);
    }
    if action.is_none() {
        action = list_keys(ui, form, typing);
    }
    if enter && action.is_none() {
        action = Some(DestinationAction::Accept);
    }
    action
}

impl App {
    /// MostUsedDestinations: UseCount descending, LastUsedUtc descending, existing folders only, MaxRecentFileCount entries.
    fn most_used_destinations(&self) -> Vec<(String, u64)> {
        let mut entries: Vec<(String, u64, String)> = self
            .settings
            .values
            .get("MostUsedDestinations")
            .and_then(|v| v.as_array())
            .map(|items| items.iter().filter_map(|item| Some((item.get("Path")?.as_str()?.to_owned(), item.get("UseCount").and_then(|c| c.as_u64()).unwrap_or(0), item.get("LastUsedUtc").and_then(|t| t.as_str()).unwrap_or("").to_owned()))).collect())
            .unwrap_or_default();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));
        let limit = self.settings.values.get("MaxRecentFileCount").and_then(|v| v.as_i64()).unwrap_or(10).clamp(0, 1000) as usize;
        entries.into_iter().filter(|(path, _, _)| !path.trim().is_empty() && Path::new(path).is_dir()).take(limit).map(|(p, c, _)| (p, c)).collect()
    }

    fn destination_form(&mut self) -> Option<&mut DestinationForm> {
        match self.original_dialogs.last_mut().map(|d| &mut d.kind) {
            Some(DialogKind::Destination(form)) => Some(form),
            _ => None,
        }
    }

    /// C# SelectDestinationFolder: passes the chosen folder (full path) to the callback.
    pub(super) fn select_destination(&mut self, purpose: DestinationPurpose, initial_directory: PathBuf, current_pane: Option<PathBuf>, on_result: impl FnOnce(&mut App, PathBuf) + 'static) {
        let text = |app: &App, section: &str, key: &str, fallback: &str| app.language_pack.text(&format!("{section}.{key}"), fallback);
        let ko = self.language == "ko";
        // Original (near 0x8ebad2, 0x8ebb1e, 0x8ed2b4): destination window title = MOVE / COPY / EXTRACT + SELECTDEST (" - Select a destination").
        let (title_key, title_fallback) = match purpose {
            DestinationPurpose::Copy => ("RES.COPY", "Copy"),
            DestinationPurpose::Move => ("RES.MOVE", "Move"),
            DestinationPurpose::Extract => ("RES.EXTRACT", "Extract"),
            DestinationPurpose::FtpGet => ("RES.FTP_GET", "FTP Download"),
        };
        // GetDestinationInitialDirectory: DefaultDestinationMode 1 means the current pane's drive root.
        let fallback = if initial_directory.is_dir() { std::path::absolute(&initial_directory).unwrap_or(initial_directory) } else { documents() };
        let mode = self.settings.values.get("DefaultDestinationMode").and_then(|v| v.as_i64()).unwrap_or(0);
        let initial = match current_pane.filter(|_| mode == 1).and_then(|p| std::path::absolute(p).ok()).and_then(|p| p.ancestors().last().map(Path::to_path_buf)) {
            Some(root) if root.is_dir() => root,
            _ => fallback,
        };
        let work = self.settings.strings("WorkingFolders");
        // Original FUN_004532d4 kind 8: the global drive list (TNXDriveList, the same cache as the list pane).
        let drive_items = if cfg!(test) {
            (self.system.ready_drives)().into_iter().map(|root| { let info = (self.system.drive_item)(&root); (root, info) }).collect()
        } else {
            super::panes::cached_drive_items(&self.drive_list, self.system.ready_drives, self.system.drive_item)
        };
        let drives = drive_items
            .into_iter()
            .filter_map(|(root, info)| {
                let (name, kind, sizes) = info?;
                Some(DestinationDrive { root: root.to_string_lossy().into_owned(), name, kind, sizes })
            })
            .collect();
        let special_folders = ["desktop", "documents", "pictures", "music", "videos"].map(|id| self.special_folder_path(id).map(|p| p.to_string_lossy().into_owned()));
        let form = DestinationForm {
            most_used: self.most_used_destinations(),
            work_folders: std::array::from_fn(|i| work.get(i).map(|w| w.trim().to_owned()).unwrap_or_default()),
            browse_label: text(self, "formSelectDest", "Caption.btnTree", "&Folder Tree"),
            cancel_label: text(self, "formNDRDestination", "Button.Cancel", "Cancel"),
            remove_label: text(self, "formSelectDest", "Menu.pmiMUFDelete", if ko { "기록에서 삭제" } else { "Remove from history" }),
            clear_label: text(self, "formSelectDest", "Menu.pmiClearMUFHistory", if ko { "기록 지우기" } else { "Clear history" }),
            path: initial.to_string_lossy().into_owned(),
            ok_label: text(self, "formSelectDest", "Caption.btnOk", "OK"),
            original: true,
            selected: 0,
            drives,
            free_label: self.language_pack.text("RES.DISKFREE", "Free"),
            special_folders,
            initial,
        };
        let title = format!("{}{}", self.language_pack.text(title_key, title_fallback), self.language_pack.text("RES.SELECTDEST", " - Select a destination"));
        self.push_original_dialog(title, DialogKind::Destination(Box::new(form)), move |app, result, _| {
            if let DialogResult::Destination(Some(path)) = result {
                on_result(app, path);
            }
        });
    }

    /// Window buttons.
    pub(super) fn destination_action(&mut self, action: DestinationAction, ctx: &egui::Context) {
        let title = self.original_dialogs.last().map(|d| d.title.clone()).unwrap_or_default();
        let Some(form) = self.destination_form() else { return };
        match action {
            DestinationAction::Cancel => self.answer_dialog(DialogResult::Destination(None), ctx),
            DestinationAction::Accept => {
                let typed = form.path.trim().trim_matches('"').to_owned();
                if typed.is_empty() {
                    return;
                }
                // Original btnOkClick (0x453c4c): when the selected row is a drive (kind 8) and cbPath equals that drive root (lstrcmpiW), instead of confirming
                // it runs btnTreeClick (FUN_00453de4) = pick a folder in that drive with the folder tree.
                let drive_row = original_items(form).get(form.selected).is_some_and(|(item, _)| matches!(item, DestinationItem::Drive(d) if ndr::names::equals_ignore_case(&d.root, form.path.trim())));
                if drive_row {
                    return self.destination_action(DestinationAction::Browse, ctx);
                }
                // Relative paths are based on the current pane folder (original PathCombine).
                let base = self.panes[self.active].path.to_string_lossy().into_owned();
                let full = PathBuf::from(ndr::names::combine_typed_name(&base, &typed));
                if full.is_dir() {
                    self.answer_dialog(DialogResult::Destination(Some(full)), ctx);
                } else {
                    self.status = format!("{}: {}", self.tr("폴더가 없습니다", "Folder not found"), full.display());
                }
            }
            DestinationAction::MostUsed(path) => {
                if Path::new(&path).is_dir() {
                    let full = std::path::absolute(&path).unwrap_or_else(|_| PathBuf::from(&path));
                    self.answer_dialog(DialogResult::Destination(Some(full)), ctx);
                }
            }
            DestinationAction::Work(index) => {
                let path = form.work_folders[index].clone();
                if !Path::new(&path).is_dir() {
                    let message = self.language_pack.text("formNDRDestination.Message.InvalidWorkFolder", "Work folder {0} is not set or is unavailable.").replace("{0}", &(index + 1).to_string());
                    return self.system_box(&title, &message, MB_OK | MB_ICONASTERISK, |_, _, _| {});
                }
                let full = std::path::absolute(&path).unwrap_or_else(|_| PathBuf::from(&path));
                self.answer_dialog(DialogResult::Destination(Some(full)), ctx);
            }
            DestinationAction::Browse => {
                // FUN_00453de4: FUN_0040ca90 with the path field if it is an existing folder, otherwise empty text (title = this window's title). Closes with the chosen path as the destination.
                let typed = form.path.trim().trim_matches('"').to_owned();
                let initial = if !typed.is_empty() && Path::new(&typed).is_dir() { PathBuf::from(typed) } else { PathBuf::new() };
                self.folder_tree_dialog(&initial, Some(title), |app, folder, ctx| {
                    if let Some(folder) = folder {
                        app.answer_dialog(DialogResult::Destination(Some(folder)), ctx);
                    }
                });
            }
            DestinationAction::Remove(path) => {
                let message = self.language_pack.text("RES.SELDEST_DELMUFOLDER", "Remove this folder from history?\n{0}").replace("{0}", &path);
                self.system_box(&title, &message, MB_YESNO | MB_ICONQUESTION, move |app, answer, _| {
                    if answer != IDYES {
                        return;
                    }
                    if let Some(items) = app.settings.values.get_mut("MostUsedDestinations").and_then(|v| v.as_array_mut()) {
                        let before = items.len();
                        items.retain(|item| !item.get("Path").and_then(|p| p.as_str()).is_some_and(|p| ndr::names::equals_ignore_case(p, &path)));
                        if items.len() == before {
                            return;
                        }
                    }
                    app.save();
                    let list = app.most_used_destinations();
                    if let Some(form) = app.destination_form() {
                        form.most_used = list;
                    }
                });
            }
            DestinationAction::Clear => {
                let empty = self.settings.values.get("MostUsedDestinations").and_then(|v| v.as_array()).is_none_or(|a| a.is_empty());
                if empty {
                    return;
                }
                let message = self.language_pack.text("RES.SELDEST_CLEARMUFOLDER", "Clear destination history?");
                self.system_box(&title, &message, MB_YESNO | MB_ICONQUESTION, |app, answer, _| {
                    if answer != IDYES {
                        return;
                    }
                    app.settings.values.insert("MostUsedDestinations".into(), serde_json::json!([]));
                    app.save();
                    if let Some(form) = app.destination_form() {
                        form.most_used.clear();
                    }
                });
            }
        }
    }

    /// C# RememberDestination: for an existing folder saves LastDestination and MostUsedDestinations (use count + 1, last use UTC).
    /// Original FUN_0046e6ec: ignores an empty path, drops a trailing '\' except at a root, then raises the count and sets the date of the
    /// entry with the same path (case-insensitive) or adds one with count 1. Folders that no longer exist are dropped when saving (FUN_0046e4dc).
    pub(super) fn remember_destination(&mut self, destination: &Path) {
        if !destination.is_dir() {
            return;
        }
        let full = std::path::absolute(destination).unwrap_or_else(|_| destination.to_path_buf()).to_string_lossy().into_owned();
        let full = if full.len() > 3 { full.trim_end_matches(['\\', '/']).to_owned() } else { full };
        self.settings.set_text("LastDestination", &full);
        // System.Text.Json DateTime (UTC): 7 fractional digits.
        let utc = chrono::Utc::now();
        let now = format!("{}.{:07}Z", utc.format("%Y-%m-%dT%H:%M:%S"), chrono::Timelike::nanosecond(&utc) / 100);
        let list = self.settings.values.entry("MostUsedDestinations").or_insert_with(|| serde_json::json!([]));
        if !list.is_array() {
            *list = serde_json::json!([]);
        }
        let items = list.as_array_mut().expect("an array");
        let index = match items.iter().position(|item| item.get("Path").and_then(|p| p.as_str()).is_some_and(|p| ndr::names::equals_ignore_case(p, &full))) {
            Some(index) => index,
            None => {
                items.push(serde_json::json!({"Path": full, "UseCount": 0, "LastUsedUtc": now}));
                items.len() - 1
            }
        };
        let count = items[index].get("UseCount").and_then(|c| c.as_i64()).unwrap_or(0);
        items[index]["UseCount"] = serde_json::json!(if count < i32::MAX as i64 { count + 1 } else { count });
        items[index]["LastUsedUtc"] = serde_json::json!(now);
        self.save();
    }

    /// Original miCopyToClick (0x432e68) / miMoveToClick (0x432f40): at a local location FUN_0042bdc4 = TfSelectDest (path field initial value from FUN_0043cb24).
    /// Inside an archive, copy is FUN_00439bac (mode 0) = extract the chosen items to the folder picked with TfSelectDest (EXTRACT title);
    /// move did nothing in the original (FUN_0042bdc4 stops when not local). Per user request NDR, for a modifiable archive (zip, 7z, tar, wim),
    /// extracts to the folder chosen in the same destination window and deletes from the archive.
    pub(super) fn copy_move_to_folder(&mut self, moving: bool) {
        let i = self.active;
        if self.panes[i].archive.is_some() {
            if !moving {
                return self.extract_command(super::archive_dialog::ExtractMode::Selected);
            }
            if self.editable_archive(i).is_none() {
                return;
            }
        }
        let targets = self.panes[i].targets();
        if targets.is_empty() {
            return;
        }
        let other = &self.panes[1 - i];
        // Original FUN_0043cb24: the other pane's folder when both panes show and the other pane is local, otherwise the current pane folder.
        let preferred = if !self.dual && self.panes[i].archive.is_none() {
            self.panes[i].path.clone()
        } else if other.archive.is_none() {
            other.path.clone()
        } else {
            PathBuf::from(self.favorites_current())
        };
        let initial = if preferred.is_dir() { preferred } else { documents() };
        let source_path = self.panes[i].path.clone();
        let purpose = if moving { DestinationPurpose::Move } else { DestinationPurpose::Copy };
        self.select_destination(purpose, initial, Some(source_path), move |app, destination| {
            app.active = i;
            app.transfer_from(i, targets, destination, moving, super::file_operation::TransferKind::Folder);
        });
    }
}
