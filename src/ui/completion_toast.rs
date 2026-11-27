//! Small in-window completion notices. No native windows, textures, timers or
//! background threads. Memory use is bounded to five short strings.
use super::*;

const MAX_TOASTS: usize = 5;
const VISIBLE: Duration = Duration::from_secs(4);
const FADE: Duration = Duration::from_millis(250);
const WIDTH: f32 = 270.0;
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 9.0;
const TEXT_GAP: f32 = 3.0;
const ICON_SIZE: f32 = 19.0;
const GAP: f32 = 9.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NoticeKind { Success, Warning, Error }

pub(super) struct Notice {
    pub title: String,
    pub detail: String,
    pub kind: NoticeKind,
    pub since: Instant,
}

#[derive(Default)]
pub(super) struct Notices {
    pub entries: Vec<Notice>,
}

impl Notices {
    pub fn push_at(&mut self, title: impl Into<String>, detail: impl Into<String>, kind: NoticeKind, now: Instant) {
        let title: String = title.into();
        let detail: String = detail.into();
        let trimmed_title: String = title.chars().take(64).collect();
        let trimmed_detail: String = detail.chars().take(180).collect();
        if self.entries.len() == MAX_TOASTS {
            self.entries.remove(0);
        }
        self.entries.push(Notice { title: trimmed_title, detail: trimmed_detail, kind, since: now });
    }

    pub fn prune(&mut self, now: Instant) {
        self.entries.retain(|item| now.saturating_duration_since(item.since) < VISIBLE + FADE);
    }
}

fn alpha(elapsed: Duration) -> f32 {
    if elapsed < VISIBLE { 1.0 }
    else { 1.0 - (elapsed - VISIBLE).as_secs_f32() / FADE.as_secs_f32() }
}

impl App {
    pub(super) fn completion_notice(&mut self, ko: &str, en: &str, detail: impl Into<String>) {
        self.push_notice(ko, en, detail, NoticeKind::Success);
    }

    pub(super) fn warning_notice(&mut self, ko: &str, en: &str, detail: impl Into<String>) {
        self.push_notice(ko, en, detail, NoticeKind::Warning);
    }

    pub(super) fn completion_notice_operation(&mut self, title: String, detail: impl Into<String>) {
        self.push_operation_notice(title, detail, NoticeKind::Success);
    }

    pub(super) fn warning_notice_operation(&mut self, title: String, detail: impl Into<String>) {
        self.push_operation_notice(title, detail, NoticeKind::Warning);
    }

    pub(super) fn error_notice_operation(&mut self, title: String, detail: impl Into<String>) {
        self.push_operation_notice(title, detail, NoticeKind::Error);
    }

    fn push_notice(&mut self, ko: &str, en: &str, detail: impl Into<String>, kind: NoticeKind) {
        let title = self.tr(ko, en).into_owned();
        self.push_operation_notice(title, detail, kind);
    }

    fn push_operation_notice(&mut self, title: String, detail: impl Into<String>, kind: NoticeKind) {
        if !self.settings.flag("ShowCompletionToast", true) { return; }
        self.completion_toasts.push_at(title, detail, kind, Instant::now());
        self.egui_ctx.request_repaint();
    }

    pub(super) fn completion_toasts_ui(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        self.completion_toasts.prune(now);
        if !self.settings.flag("ShowCompletionToast", true) {
            self.completion_toasts.entries.clear();
            return;
        }
        let len = self.completion_toasts.entries.len();
        let mut dismiss = None;
        let mut next_repaint = Duration::from_secs(5);
        // Anchor above the function-key row and leave a little breathing
        // room. New notices stack upward using their actual painted height.
        let mut bottom = 27.0 + if self.settings.flag("ShowFunctionBar", true) { 20.0 } else { 0.0 };
        let width = WIDTH.min((ctx.content_rect().width() - 30.0).max(80.0));
        for position in (0..len).rev() {
            let notice = &self.completion_toasts.entries[position];
            let elapsed = now.saturating_duration_since(notice.since);
            let opacity = alpha(elapsed).clamp(0.0, 1.0);
            if elapsed >= VISIBLE {
                next_repaint = next_repaint.min(Duration::from_millis(33));
            } else {
                next_repaint = next_repaint.min(VISIBLE - elapsed);
            }
            let (symbol, accent) = match notice.kind {
                NoticeKind::Success => ("✓", Color32::from_rgb(75, 190, 128)),
                NoticeKind::Warning => ("!", Color32::from_rgb(240, 181, 76)),
                NoticeKind::Error => ("×", Color32::from_rgb(231, 100, 100)),
            };
            let foreground = Color32::from_white_alpha((248.0 * opacity) as u8);
            let secondary = Color32::from_white_alpha((205.0 * opacity) as u8);
            let fill = Color32::from_rgba_unmultiplied(33, 37, 44, (222.0 * opacity) as u8);
            let stroke = Color32::from_rgba_unmultiplied(126, 136, 152, (135.0 * opacity) as u8);
            let layer_id = egui::Id::new(("ndr_completion_toast", position));
            super::keep_opaque(ctx, layer_id);
            let text_x = PAD_X + ICON_SIZE + 9.0;
            let text_width = (width - text_x - PAD_X).max(1.0);
            let (symbol, title, detail) = ctx.fonts_mut(|fonts| {
                let one_line = |text: &str, font: egui::FontId, color| {
                    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, text_width);
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    job
                };
                let symbol = fonts.layout_no_wrap(symbol.to_owned(), egui::FontId::proportional(ICON_SIZE), accent.gamma_multiply(opacity));
                let title = fonts.layout_job(one_line(&notice.title, egui::FontId::new(13.0, egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())), foreground));
                let detail = (!notice.detail.is_empty()).then(|| fonts.layout_job(one_line(&notice.detail, egui::FontId::proportional(11.0), secondary)));
                (symbol, title, detail)
            });
            let text_height = title.size().y + detail.as_ref().map_or(0.0, |g| TEXT_GAP + g.size().y);
            let height = (text_height.max(symbol.size().y) + 2.0 * PAD_Y).ceil();
            let area = egui::Area::new(layer_id)
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-15.0, -bottom))
                .order(egui::Order::Foreground)
                .interactable(true);
            let response = area.show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect(rect, 9.0, fill, egui::Stroke::new(1.0_f32, stroke), egui::StrokeKind::Inside);
                painter.galley(egui::pos2(rect.left() + PAD_X + (ICON_SIZE - symbol.size().x) * 0.5, rect.center().y - symbol.size().y * 0.5), symbol, accent);
                let top = rect.center().y - text_height * 0.5;
                let y = top + title.size().y + TEXT_GAP;
                painter.galley(egui::pos2(rect.left() + text_x, top), title, foreground);
                if let Some(detail) = detail {
                    painter.galley(egui::pos2(rect.left() + text_x, y), detail, secondary);
                }
            });
            if response.response.interact(egui::Sense::click()).clicked() {
                dismiss = Some(position);
            }
            bottom += height + GAP;
        }
        if let Some(index) = dismiss {
            self.completion_toasts.entries.remove(index);
            ctx.request_repaint();
        }
        if !self.completion_toasts.entries.is_empty() {
            ctx.request_repaint_after(next_repaint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_stack_newest_at_bottom_and_bound_memory() {
        let now = Instant::now();
        let mut queue = Notices::default();
        for i in 0..9 { queue.push_at(format!("job {i}"), "done", NoticeKind::Success, now); }
        assert_eq!(queue.entries.len(), 5);
        assert_eq!(queue.entries[0].title, "job 4");
        assert_eq!(queue.entries[4].title, "job 8");
    }
    #[test]
    fn notices_expire_independently_oldest_first() {
        let now = Instant::now();
        let mut queue = Notices::default();
        queue.push_at("old", "finished", NoticeKind::Success, now);
        queue.push_at("new", "finished", NoticeKind::Success, now + Duration::from_secs(2));
        queue.prune(now + Duration::from_secs(5));
        assert_eq!(queue.entries.len(), 1);
        assert_eq!(queue.entries[0].title, "new");
        queue.prune(now + Duration::from_secs(7));
        assert!(queue.entries.is_empty());
        assert_eq!(alpha(VISIBLE + FADE), 0.0);
    }
    #[test]
    fn long_text_is_bounded_without_allocating_unbounded_items() {
        let mut queue = Notices::default();
        queue.push_at("이".repeat(500), "x".repeat(1000), NoticeKind::Warning, Instant::now());
        assert_eq!(queue.entries[0].title.chars().count(), 64);
        assert_eq!(queue.entries[0].detail.chars().count(), 180);
    }
}
