//! Field resembling VCL TDateTimePicker (original TfChangeAttr dtpDate: dtkDate with short date, dtpTime: dtkTime).
//! Parts inside the field (year/month/day, hour/minute/second) are chosen by clicking or Left/Right and changed with Up/Down or digits. The date field opens a
//! calendar with the right ▼ (DTM_GETMONTHCAL style: month ◀ ▶, weekday header, 6 weeks); the time field changes with the right up/down buttons (UpDown).
//! The display format is fixed to yyyy-MM-dd and HH:mm:ss (the original uses the user locale format).
use chrono::{Datelike, NaiveDate, NaiveTime, Timelike};
use eframe::egui::{self, Color32};

/// Chosen part and digits being typed (digit accumulation of the original control).
#[derive(Clone, Copy, Default)]
struct PickerState {
    segment: usize,
    typed: i64,
    typed_digits: u8,
    calendar: Option<(i32, u32)>,
}

const BUTTON_WIDTH: f32 = 17.0;

fn days_in_month(year: i32, month: u32) -> u32 {
    let next = if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { NaiveDate::from_ymd_opt(year, month + 1, 1) };
    next.and_then(|n| n.pred_opt()).map_or(31, |d| d.day())
}

/// Draws three parts (value, minimum, maximum, digits) and handles keys and clicks. True when changed.
fn segments_ui(ui: &mut egui::Ui, rect: egui::Rect, enabled: bool, id: egui::Id, values: &mut [i64; 3], ranges: [(i64, i64); 3], digits: [usize; 3], separator: &str) -> (egui::Response, bool) {
    let response = ui.interact(rect, id, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let mut state = ui.ctx().data(|d| d.get_temp::<PickerState>(id)).unwrap_or_default();
    let visuals = ui.visuals().clone();
    let painter = ui.painter_at(rect);
    let focused = enabled && response.has_focus();
    painter.rect(rect, 0.0, visuals.extreme_bg_color, if focused { visuals.selection.stroke } else { visuals.widgets.inactive.bg_stroke }, egui::StrokeKind::Inside);
    let font = egui::FontId::proportional(13.0);
    let text_color = if enabled { visuals.text_color() } else { visuals.weak_text_color() };
    // Part positions.
    let mut x = rect.left() + 4.0;
    let mut cells = [egui::Rect::NOTHING; 3];
    for (index, value) in values.iter().enumerate() {
        let text = format!("{value:0width$}", width = digits[index]);
        let galley = painter.layout_no_wrap(text, font.clone(), text_color);
        let cell = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley.size());
        if focused && state.segment == index {
            painter.rect_filled(cell.expand2(egui::vec2(1.0, 0.0)), 0.0, visuals.selection.bg_fill);
            painter.galley(cell.min, galley, visuals.selection.stroke.color);
        } else {
            painter.galley(cell.min, galley, text_color);
        }
        cells[index] = cell;
        x = cell.right();
        if index < 2 {
            let galley = painter.layout_no_wrap(separator.to_owned(), font.clone(), text_color);
            painter.galley(egui::pos2(x, cell.top()), galley.clone(), text_color);
            x += galley.size().x;
        }
    }
    if response.clicked() {
        response.request_focus();
        if let Some(pos) = response.interact_pointer_pos() {
            state.segment = cells.iter().position(|c| pos.x <= c.right() + 2.0).unwrap_or(2);
        }
        state.typed_digits = 0;
    }
    let mut changed = false;
    if focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
        let events = ui.input(|i| i.events.clone());
        for event in events {
            match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.alt && !modifiers.ctrl => {
                    let (min, max) = ranges[state.segment];
                    match key {
                        egui::Key::ArrowLeft => state.segment = state.segment.saturating_sub(1),
                        egui::Key::ArrowRight => state.segment = (state.segment + 1).min(2),
                        egui::Key::ArrowUp => values[state.segment] = if values[state.segment] >= max { min } else { values[state.segment] + 1 },
                        egui::Key::ArrowDown => values[state.segment] = if values[state.segment] <= min { max } else { values[state.segment] - 1 },
                        egui::Key::Home => values[state.segment] = min,
                        egui::Key::End => values[state.segment] = max,
                        _ => continue,
                    }
                    state.typed_digits = 0;
                    changed |= matches!(key, egui::Key::ArrowUp | egui::Key::ArrowDown | egui::Key::Home | egui::Key::End);
                }
                egui::Event::Text(text) => {
                    for digit in text.chars().filter_map(|c| c.to_digit(10)) {
                        let (min, max) = ranges[state.segment];
                        state.typed = if state.typed_digits == 0 { digit as i64 } else { state.typed * 10 + digit as i64 };
                        state.typed_digits += 1;
                        if state.typed >= min && state.typed <= max {
                            values[state.segment] = state.typed;
                            changed = true;
                        }
                        // When all digits are filled (or another digit would overflow), move to the next part.
                        if state.typed_digits as usize >= digits[state.segment] || state.typed * 10 > max {
                            state.typed_digits = 0;
                            state.segment = (state.segment + 1).min(2);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    (response, changed)
}

/// dtkDate field (short date + ▼ calendar).
pub(super) fn date_picker(ui: &mut egui::Ui, rect: egui::Rect, enabled: bool, id_salt: &str, value: &mut NaiveDate) -> egui::Response {
    let id = egui::Id::new(("ndr-date-picker", id_salt));
    let field = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right() - BUTTON_WIDTH, rect.bottom()));
    let mut values = [value.year() as i64, value.month() as i64, value.day() as i64];
    let days = days_in_month(value.year(), value.month()) as i64;
    let (response, changed) = segments_ui(ui, field, enabled, id, &mut values, [(1601, 9999), (1, 12), (1, days)], [4, 2, 2], "-");
    if changed {
        let (year, month) = (values[0] as i32, values[1] as u32);
        let day = (values[2] as u32).min(days_in_month(year, month));
        if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
            *value = date;
        }
    }
    // ▼ button and calendar.
    let button = egui::Rect::from_min_max(egui::pos2(field.right(), rect.top()), rect.max);
    let arrow = ui.put(button, egui::Button::new(egui::RichText::new("▼").size(8.0)).corner_radius(0.0));
    let arrow = if enabled { arrow } else { arrow.on_disabled_hover_text("") };
    let mut state = ui.ctx().data(|d| d.get_temp::<PickerState>(id)).unwrap_or_default();
    if enabled && arrow.clicked() {
        state.calendar = if state.calendar.is_some() { None } else { Some((value.year(), value.month())) };
        response.request_focus();
    }
    if !enabled {
        state.calendar = None;
    }
    if let Some((year, month)) = state.calendar {
        let mut shown = (year, month);
        let mut picked = None;
        let area = egui::Area::new(id.with("calendar")).order(egui::Order::Foreground).fixed_pos(egui::pos2(rect.left(), rect.bottom() + 1.0)).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).corner_radius(0.0).show(ui, |ui| {
                picked = calendar_ui(ui, &mut shown, *value);
            });
        });
        state.calendar = Some(shown);
        let outside = ui.input(|i| i.pointer.any_pressed()) && ui.ctx().pointer_interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !button.contains(p));
        if let Some(date) = picked {
            *value = date;
            state.calendar = None;
        } else if outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.calendar = None;
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    response
}

/// One calendar month (6 weeks from Sunday). Returns the chosen day.
fn calendar_ui(ui: &mut egui::Ui, shown: &mut (i32, u32), selected: NaiveDate) -> Option<NaiveDate> {
    let mut picked = None;
    let cell = egui::vec2(26.0, 18.0);
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
    ui.horizontal(|ui| {
        if ui.add_sized([cell.x, cell.y], egui::Button::new("◀").frame(false)).clicked() {
            *shown = if shown.1 == 1 { (shown.0 - 1, 12) } else { (shown.0, shown.1 - 1) };
        }
        ui.add_sized([cell.x * 5.0, cell.y], egui::Label::new(egui::RichText::new(format!("{}-{:02}", shown.0, shown.1)).strong()));
        if ui.add_sized([cell.x, cell.y], egui::Button::new("▶").frame(false)).clicked() {
            *shown = if shown.1 == 12 { (shown.0 + 1, 1) } else { (shown.0, shown.1 + 1) };
        }
    });
    ui.horizontal(|ui| {
        for (index, name) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
            let color = if index == 0 { Color32::from_rgb(0xE0, 0x40, 0x40) } else { ui.visuals().weak_text_color() };
            ui.add_sized([cell.x, cell.y], egui::Label::new(egui::RichText::new(*name).color(color)));
        }
    });
    let Some(first) = NaiveDate::from_ymd_opt(shown.0, shown.1, 1) else { return None };
    let start = first - chrono::Duration::days(first.weekday().num_days_from_sunday() as i64);
    let today = chrono::Local::now().date_naive();
    for week in 0..6 {
        ui.horizontal(|ui| {
            for day in 0..7 {
                let date = start + chrono::Duration::days(week * 7 + day);
                let inside = date.month() == shown.1;
                let text = egui::RichText::new(date.day().to_string()).color(if inside { ui.visuals().text_color() } else { ui.visuals().weak_text_color() });
                let response = ui.add_sized([cell.x, cell.y], egui::Button::new(text).frame(false).selected(date == selected));
                if date == today {
                    ui.painter().rect_stroke(response.rect.shrink(1.0), 0.0, egui::Stroke::new(1.0, Color32::from_rgb(0xE0, 0x40, 0x40)), egui::StrokeKind::Inside);
                }
                if response.clicked() {
                    picked = Some(date);
                }
            }
        });
    }
    picked
}

/// dtkTime field (time + up/down buttons).
pub(super) fn time_picker(ui: &mut egui::Ui, rect: egui::Rect, enabled: bool, id_salt: &str, value: &mut NaiveTime) -> egui::Response {
    let id = egui::Id::new(("ndr-time-picker", id_salt));
    let field = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right() - BUTTON_WIDTH, rect.bottom()));
    let mut values = [value.hour() as i64, value.minute() as i64, value.second() as i64];
    let (response, mut changed) = segments_ui(ui, field, enabled, id, &mut values, [(0, 23), (0, 59), (0, 59)], [2, 2, 2], ":");
    // UpDown: raises or lowers the chosen part by one.
    let half = rect.height() / 2.0;
    let up = egui::Rect::from_min_size(egui::pos2(field.right(), rect.top()), egui::vec2(BUTTON_WIDTH, half));
    let down = egui::Rect::from_min_size(egui::pos2(field.right(), rect.top() + half), egui::vec2(BUTTON_WIDTH, half));
    let segment = ui.ctx().data(|d| d.get_temp::<PickerState>(id)).unwrap_or_default().segment;
    let ranges = [23, 59, 59];
    let spin = |ui: &mut egui::Ui, rect: egui::Rect, text: &str| ui.add_enabled_ui(enabled, |ui| ui.put(rect, egui::Button::new(egui::RichText::new(text).size(7.0)).corner_radius(0.0))).inner.clicked();
    if spin(ui, up, "▲") {
        values[segment] = if values[segment] >= ranges[segment] { 0 } else { values[segment] + 1 };
        changed = true;
        response.request_focus();
    }
    if spin(ui, down, "▼") {
        values[segment] = if values[segment] <= 0 { ranges[segment] } else { values[segment] - 1 };
        changed = true;
        response.request_focus();
    }
    if changed {
        if let Some(time) = NaiveTime::from_hms_opt(values[0] as u32, values[1] as u32, values[2] as u32) {
            *value = time;
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ctx: &egui::Context, events: Vec<egui::Event>, draw: impl FnOnce(&mut egui::Ui)) {
        let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..Default::default() };
        let mut draw = Some(draw);
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some(draw) = draw.take() {
                    draw(ui)
                }
            });
        });
    }
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }
    fn click(pos: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        vec![egui::Event::PointerMoved(pos), button(true), button(false)]
    }

    #[test]
    fn date_and_time_pickers_change_segments_like_a_datetimepicker() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(145.0, 21.0));
        let mut date = NaiveDate::from_ymd_opt(2024, 1, 31).unwrap();
        let frame = |events: Vec<egui::Event>, date: &mut NaiveDate| run(&ctx, events, |ui| {
            date_picker(ui, rect, true, "t", date);
        });
        frame(vec![], &mut date);
        // Click the month part and press Up: January 31 -> February (only 29 days, so the 29th).
        frame(click(egui::pos2(rect.left() + 44.0, rect.center().y)), &mut date);
        frame(vec![key(egui::Key::ArrowUp)], &mut date);
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap());
        // Right to the day part, digits 0 and 5 -> the 5th.
        frame(vec![key(egui::Key::ArrowRight), egui::Event::Text("05".into())], &mut date);
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 2, 5).unwrap());
        // Down wraps from the minimum to the maximum (the year part goes down one year).
        frame(vec![key(egui::Key::ArrowLeft), key(egui::Key::ArrowLeft), key(egui::Key::ArrowDown)], &mut date);
        assert_eq!(date, NaiveDate::from_ymd_opt(2023, 2, 5).unwrap());

        let mut time = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 80.0), egui::vec2(145.0, 21.0));
        let frame = |events: Vec<egui::Event>, time: &mut NaiveTime| run(&ctx, events, |ui| {
            time_picker(ui, rect, true, "t", time);
        });
        frame(vec![], &mut time);
        frame(click(egui::pos2(rect.left() + 8.0, rect.center().y)), &mut time);
        frame(vec![key(egui::Key::ArrowUp)], &mut time);
        assert_eq!(time, NaiveTime::from_hms_opt(0, 59, 0).unwrap(), "23 → 0");
        // The up button (UpDown) raises the chosen part.
        frame(click(egui::pos2(rect.right() - 5.0, rect.top() + 4.0)), &mut time);
        assert_eq!(time, NaiveTime::from_hms_opt(1, 59, 0).unwrap());
        frame(vec![key(egui::Key::ArrowRight), egui::Event::Text("7".into())], &mut time);
        assert_eq!(time, NaiveTime::from_hms_opt(1, 7, 0).unwrap(), "7 can't take a second digit below 60, so it is done");
    }
}
