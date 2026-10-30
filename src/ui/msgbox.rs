//! 원본 TfMsgBox 화면(fMsgBox DFM: 클라이언트 413x119, pTitle/lTitle, Image1, lSummary, lText, btn3/btn2/btn1).
//! egui에는 OS 대화 상자가 없으므로 모달 영역 안에 DFM 좌표대로 그린다. 결과는 콜백으로 돌려준다.
use super::*;
use eframe::egui::{FontId, Rect, Sense, Vec2, pos2, text::LayoutJob, vec2};
use ndr::msgbox as mb;

pub(super) type MsgBoxCallback = Box<dyn FnOnce(&mut App, i32, &egui::Context)>;

/// 그린 결과(테스트와 진단용).
#[derive(Clone, Debug, Default)]
pub(super) struct MsgBoxLayout {
    pub client: Vec2,
    pub origin: egui::Pos2,
    pub text_top: f32,
    pub image_top: f32,
    pub title_shown: bool,
    pub line_count: usize,
    pub slots: [Option<Rect>; 3],
    pub slot_ids: [Option<egui::Id>; 3],
    pub captions: [String; 3],
}

pub(super) struct MsgBox {
    pub title: String,
    pub summary: String,
    pub text: String,
    pub flags: u32,
    pub serial: u64,
    pub focused: bool,
    pub layout: Option<MsgBoxLayout>,
    /// USER32 MessageBoxW 모양: 제목 띠 없음, 캡션은 창 제목 줄.
    pub system: bool,
    on_result: Option<MsgBoxCallback>,
}

const FONT: f32 = 12.0;

impl App {
    /// FUN_0040bb38(title, summary, text, flags): 빈 제목은 제품 이름. 결과는 Win32 방식 ID로 콜백에 전달된다.
    pub(super) fn message_box(
        &mut self,
        title: &str,
        summary: &str,
        text: &str,
        flags: u32,
        on_result: impl FnOnce(&mut App, i32, &egui::Context) + 'static,
    ) {
        self.msgbox_serial += 1;
        self.msgboxes.push(MsgBox {
            title: if title.is_empty() { mb::DEFAULT_TITLE.into() } else { title.into() },
            summary: summary.into(),
            text: text.into(),
            flags,
            serial: self.msgbox_serial,
            focused: false,
            layout: None,
            system: false,
            on_result: Some(Box::new(on_result)),
        });
    }

    /// 원본이 MessageBoxW(caption, text, flags)를 부르는 곳. 빈 캡션은 그대로 둔다(원본 NULL 캡션).
    pub(super) fn system_box(
        &mut self,
        caption: &str,
        text: &str,
        flags: u32,
        on_result: impl FnOnce(&mut App, i32, &egui::Context) + 'static,
    ) {
        self.message_box(caption, "", text, flags, on_result);
        if let Some(last) = self.msgboxes.last_mut() {
            last.title = caption.into();
            last.system = true;
        }
    }

    /// 버튼의 ModalResult로 맨 앞 상자를 닫는다. ModalResult 0(원본이 처리하지 않는 종류의 DFM 버튼)은 닫지 않는다.
    pub(super) fn answer_message_box(&mut self, modal_result: i32, ctx: &egui::Context) {
        if modal_result == 0 || self.msgboxes.is_empty() {
            return;
        }
        let mut current = self.msgboxes.remove(0);
        if let Some(callback) = current.on_result.take() {
            callback(self, mb::map_result(modal_result), ctx);
        }
    }

    /// [RES] 캡션(단축 문자 '&' 포함). 언어 항목이 없으면 원본 리터럴.
    fn msgbox_caption(&self, spec: &mb::ButtonSpec) -> String {
        if spec.key.is_empty() {
            return String::new();
        }
        self.language_pack.raw(&format!("RES.{}", spec.key)).unwrap_or_else(|| spec.fallback.into())
    }

    pub(super) fn msgbox_icon(&mut self, ctx: &egui::Context, id: u16) -> Option<egui::TextureHandle> {
        if id == 0 {
            return None;
        }
        if let Some(texture) = self.msgbox_icons.get(&id) {
            return Some(texture.clone());
        }
        let (width, height, rgba) = native::system_icon_rgba(id).ok()?;
        let texture = ctx.load_texture(
            format!("ndr-system-icon-{id:x}"),
            egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba),
            egui::TextureOptions::NEAREST,
        );
        self.msgbox_icons.insert(id, texture.clone());
        Some(texture)
    }

    pub(super) fn msgbox_ui(&mut self, ctx: &egui::Context) {
        let Some(current) = self.msgboxes.first() else {
            return;
        };
        let (title, summary, text, flags, serial, focused, system) = (
            current.title.clone(),
            current.summary.clone(),
            current.text.clone(),
            current.flags,
            current.serial,
            current.focused,
            current.system,
        );
        let caption = if system { title.clone() } else { mb::FORM_CAPTION.to_owned() };
        let specs = mb::buttons(flags);
        let captions: [String; 3] = std::array::from_fn(|i| specs[i].map(|s| self.msgbox_caption(&s)).unwrap_or_default());
        let icon = self.msgbox_icon(ctx, mb::icon_id(flags));
        let dark = self.dark;
        let text_color = if dark { Color32::from_rgb(241, 245, 249) } else { Color32::from_rgb(17, 24, 39) };
        let title_color = if dark { Color32::from_rgb(203, 213, 225) } else { Color32::from_rgb(70, 70, 70) };
        let strip = if dark { Color32::from_rgb(31, 41, 55) } else { Color32::from_rgb(233, 233, 233) };
        let summary_color = if dark { Color32::from_rgb(96, 165, 250) } else { Color32::from_rgb(28, 99, 174) };
        let mut pressed: Option<i32> = None;
        let mut layout = MsgBoxLayout::default();

        let modal_id = egui::Id::new(("ndr-msgbox", serial));
        super::keep_opaque(ctx, modal_id);
        egui::Modal::new(modal_id).show(ctx, |ui| {
            let font = FontId::proportional(FONT);
            let (title_galley, summary_galley, line_galleys, line_height) = ui.fonts_mut(|fonts| {
                let line_height = fonts.row_height(&font);
                (
                    fonts.layout_no_wrap(title.clone(), font.clone(), title_color),
                    fonts.layout_no_wrap(summary.clone(), font.clone(), summary_color),
                    mb::lines(&text).into_iter().map(|line| fonts.layout_no_wrap(line, font.clone(), text_color)).collect::<Vec<_>>(),
                    line_height,
                )
            });
            let text_width = line_galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
            let text_height = mb::text_height(line_galleys.len(), line_height);
            let has_title = !system && !title.is_empty();
            let has_summary = !summary.is_empty();
            let geometry = mb::layout(has_title, has_summary, title_galley.size().x, summary_galley.size().x, text_width, text_height);
            let visible = specs.iter().filter(|s| s.is_some()).count();
            let client = vec2(mb::client_width(&geometry, visible), geometry.client_height);

            // 창 제목 줄(폼 Caption과 닫기 단추: 닫으면 mrCancel).
            ui.horizontal(|ui| {
                ui.set_width(client.x);
                ui.label(RichText::new(&caption).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("Close").clicked() {
                        pressed = Some(2);
                    }
                });
            });
            let (rect, _) = ui.allocate_exact_size(client, Sense::hover());
            let painter = ui.painter_at(rect);
            let origin = rect.min;
            if has_title {
                painter.rect_filled(Rect::from_min_size(origin, vec2(client.x, 24.0)), 0.0, strip);
                painter.galley(origin + vec2(10.0, 4.0), title_galley, title_color);
            }
            if let Some(texture) = &icon {
                painter.image(
                    texture.id(),
                    Rect::from_min_size(origin + vec2(8.0, geometry.image_top), vec2(32.0, 32.0)),
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            if has_summary {
                painter.galley(origin + vec2(64.0, geometry.summary_top), summary_galley, summary_color);
            }
            let line_count = line_galleys.len();
            for (index, galley) in line_galleys.into_iter().enumerate() {
                painter.galley(origin + vec2(64.0, geometry.text_top + index as f32 * (line_height + 4.0)), galley, text_color);
            }

            for slot in 0..3 {
                let Some(spec) = specs[slot] else { continue };
                let button_rect = Rect::from_min_size(origin + vec2(mb::button_left(client.x, slot), mb::button_top(client.y)), vec2(78.0, 25.0));
                let (display, underline) = mb::display_caption(&captions[slot]);
                let mut job = LayoutJob::default();
                for (index, character) in display.chars().enumerate() {
                    let mut format = egui::TextFormat { font_id: font.clone(), color: text_color, ..Default::default() };
                    if underline == Some(index) {
                        format.underline = egui::Stroke::new(1.0, text_color);
                    }
                    job.append(&character.to_string(), 0.0, format);
                }
                let response = ui.put(button_rect, egui::Button::new(job).min_size(vec2(78.0, 25.0)));
                // TfMsgBox: TabOrder의 첫 버튼. MessageBoxW 자리(system): MB_DEFBUTTON1..3.
                let focus_slot = if system { mb::system_default_slot(&specs, flags) } else { mb::first_visible(&specs) };
                if Some(slot) == focus_slot && !focused {
                    response.request_focus();
                }
                if response.clicked() {
                    pressed = Some(spec.modal_result);
                }
                layout.slots[slot] = Some(button_rect);
                layout.slot_ids[slot] = Some(response.id);
            }
            layout.client = client;
            layout.origin = origin;
            layout.text_top = geometry.text_top;
            layout.image_top = geometry.image_top;
            layout.title_shown = has_title;
            layout.line_count = line_count;
            layout.captions = captions.clone();
        });

        // 키보드: Esc는 Cancel 버튼, 포커스 없는 Enter는 Default 버튼, 단축 문자는 Alt 없이도(CM_DIALOGCHAR).
        if pressed.is_none() {
            let any_focused = layout.slot_ids.iter().flatten().any(|id| ctx.memory(|m| m.has_focus(*id)));
            ctx.input(|input| {
                for event in &input.events {
                    let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { continue };
                    match key {
                        egui::Key::Escape => {
                            if let Some(slot) = mb::cancel_slot(&specs) {
                                pressed = specs[slot].map(|s| s.modal_result);
                            }
                        }
                        egui::Key::Enter if !any_focused && focused => {
                            let slot = if system { mb::system_default_slot(&specs, flags) } else { mb::default_slot(&specs) };
                            if let Some(slot) = slot {
                                pressed = specs[slot].map(|s| s.modal_result);
                            }
                        }
                        _ if !(modifiers.ctrl || modifiers.shift || modifiers.command) => {
                            let name = key.name();
                            let character = name.chars().next().filter(|_| name.chars().count() == 1);
                            if let Some(character) = character {
                                if let Some(slot) = (0..3).find(|&slot| specs[slot].is_some() && mb::access_key(&captions[slot]) == Some(character.to_ascii_uppercase())) {
                                    pressed = specs[slot].map(|s| s.modal_result);
                                }
                            }
                        }
                        _ => {}
                    }
                    if pressed.is_some() {
                        break;
                    }
                }
            });
        }
        if let Some(current) = self.msgboxes.first_mut() {
            if current.serial == serial {
                current.focused = true;
                current.layout = Some(layout);
            }
        }
        if let Some(result) = pressed {
            self.answer_message_box(result, ctx);
        }
    }
}
