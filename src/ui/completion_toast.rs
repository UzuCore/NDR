//! Small in-window completion notices. No native windows, textures, timers or
//! background threads. Memory use is bounded to five short strings.
use super::*;

const MAX_TOASTS: usize = 5;
const VISIBLE: Duration = Duration::from_secs(4);
const FADE: Duration = Duration::from_millis(250);
const WIDTH: f32 = 270.0;
const HEIGHT: f32 = 69.0;
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

    pub(super) fn error_notice(&mut self, ko: &str, en: &str, detail: impl Into<String>) {
        self.push_notice(ko, en, detail, NoticeKind::Error);
    }

    fn push_notice(&mut self, ko: &str, en: &str, detail: impl Into<String>, kind: NoticeKind) {
        if !self.settings.flag("ShowCompletionToast", true) { return; }
        let title = self.tr(ko, en).into_owned();
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
        for (position, notice) in self.completion_toasts.entries.iter().enumerate() {
            let elapsed = now.saturating_duration_since(notice.since);
            let opacity = alpha(elapsed).clamp(0.0, 1.0);
            if elapsed >= VISIBLE {
                next_repaint = next_repaint.min(Duration::from_millis(33));
            } else {
                next_repaint = next_repaint.min(VISIBLE - elapsed);
            }
            // First notification stays at the top; new notifications appear
            // at bottom. Older notifications individually expire and slide out.
            let above = len - 1 - position;
            let bottom = 15.0 + above as f32 * (HEIGHT + GAP);
            let (symbol, accent) = match notice.kind {
                NoticeKind::Success => ("✓", Color32::from_rgb(75, 190, 128)),
                NoticeKind::Warning => ("!", Color32::from_rgb(240, 181, 76)),
                NoticeKind::Error => ("×", Color32::from_rgb(231, 100, 100)),
            };
            let foreground = Color32::from_white_alpha((248.0 * opacity) as u8);
            let secondary = Color32::from_white_alpha((190.0 * opacity) as u8);
            let fill = Color32::from_rgba_unmultiplied(33, 37, 44, (240.0 * opacity) as u8);
            let stroke = Color32::from_rgba_unmultiplied(126, 136, 152, (135.0 * opacity) as u8);
            let layer_id = egui::Id::new(("ndr_completion_toast", position));
            super::keep_opaque(ctx, layer_id);
            let area = egui::Area::new(layer_id)
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-15.0, -bottom))
                .order(egui::Order::Foreground)
                .interactable(true);
            let response = area.show(ctx, |ui| {
                egui::Frame::new()
                    .fill(fill)
                    .stroke(egui::Stroke::new(1.0_f32, stroke))
                    .corner_radius(egui::CornerRadius::same(9))
                    .inner_margin(egui::Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - 24.0);
                        ui.set_height(HEIGHT - 18.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(symbol).color(accent.gamma_multiply(opacity)).size(19.0).strong());
                            ui.vertical(|ui| {
                                ui.add(egui::Label::new(
                                    egui::RichText::new(&notice.title).color(foreground).size(13.0).strong()
                                ).truncate());
                                ui.add(egui::Label::new(
                                    egui::RichText::new(&notice.detail).color(secondary).size(11.0)
                                ).truncate());
                            });
                        });
                    });
            });
            if response.response.interact(egui::Sense::click()).clicked() {
                dismiss = Some(position);
            }
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
