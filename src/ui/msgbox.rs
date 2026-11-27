//! Original TfMsgBox screen (fMsgBox DFM: client 413x119, pTitle/lTitle, Image1, lSummary, lText, btn3/btn2/btn1).
//! egui has no OS dialog, so it is drawn inside a modal area at the DFM coordinates. The result comes back through a callback.
use super::*;
use eframe::egui::{FontId, Rect, Sense, Vec2, pos2, text::LayoutJob, vec2};
use ndr::msgbox as mb;

pub(super) type MsgBoxCallback = Box<dyn FnOnce(&mut App, i32, &egui::Context)>;

/// Drawn result (for tests and diagnostics).
#[derive(Clone, Debug, Default)]
pub(super) struct MsgBoxLayout {
    pub client: Vec2,
    pub origin: egui::Pos2,
    pub text_top: f32,
    pub image_top: f32,
    pub icon_rect: Option<Rect>,
    pub text_left: f32,
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
    /// USER32 MessageBoxW look: no title band, the caption is the window title bar.
    pub system: bool,
    /// Only About NDR uses the embedded application artwork.
    pub program_icon: bool,
    about_usage: Option<AboutUsage>,
    on_result: Option<MsgBoxCallback>,
}

/// A process-local sample; no system-wide utilization or other processes
/// are queried. CPU is normalized to the logical processor count, as in
/// Task Manager. The first sample has no delta yet.
struct AboutUsage {
    last: Instant,
    ticks: Option<u64>,
    ram: Option<usize>,
    cpu: Option<f64>,
    prefix: String,
    suffix: String,
}
impl AboutUsage {
    fn read() -> (Option<u64>, Option<usize>) {
        use windows_sys::Win32::{Foundation::FILETIME,
            System::{Threading::{GetCurrentProcess,GetProcessTimes},
            ProcessStatus::{K32GetProcessMemoryInfo,PROCESS_MEMORY_COUNTERS}}};
        unsafe {
            let process=GetCurrentProcess();
            let mut created:FILETIME=std::mem::zeroed();
            let mut exited:FILETIME=std::mem::zeroed();
            let mut kernel:FILETIME=std::mem::zeroed();
            let mut user:FILETIME=std::mem::zeroed();
            let filetime=|t:FILETIME| (u64::from(t.dwHighDateTime)<<32)|u64::from(t.dwLowDateTime);
            let ticks=(GetProcessTimes(process,&mut created,&mut exited,&mut kernel,&mut user)!=0)
                .then(||filetime(kernel).saturating_add(filetime(user)));
            let mut mem:PROCESS_MEMORY_COUNTERS=std::mem::zeroed();
            mem.cb=std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let ram=(K32GetProcessMemoryInfo(process,&mut mem,mem.cb)!=0).then_some(mem.WorkingSetSize);
            (ticks,ram)
        }
    }
    fn new(prefix:String,suffix:String)->Self {
        let (ticks,ram)=Self::read();
        Self{last:Instant::now(),ticks,ram,cpu:None,prefix,suffix}
    }
    fn sample(&mut self)->bool {
        let now=Instant::now();
        let elapsed=now.saturating_duration_since(self.last);
        if elapsed<Duration::from_secs(1){return false}
        let (ticks,ram)=Self::read();
        let cores=std::thread::available_parallelism().map_or(1,usize::from);
        self.cpu=match (self.ticks,ticks){
            (Some(old),Some(new)) if new>=old=>
                Some(((new-old) as f64*0.000_01/elapsed.as_secs_f64()/cores as f64).clamp(0.0,100.0)),
            _=>None,
        };
        self.ticks=ticks;self.ram=ram;self.last=now;true
    }
    fn text(&self)->String {
        let ram=self.ram.map(|n|format!("{:.1} MiB",n as f64/1048576.0)).unwrap_or_else(||"—".into());
        let cpu=self.cpu.map(|n|format!("{n:.2}%")).unwrap_or_else(||"—".into());
        format!("{}\nRAM: {ram}\nCPU: {cpu}\n{}",self.prefix,self.suffix)
    }
}

const FONT: f32 = 12.0;

impl App {
    /// FUN_0040bb38(title, summary, text, flags): an empty title means the product name. The result is passed to the callback as a Win32-style ID.
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
            program_icon: false,
            about_usage: None,
            on_result: Some(Box::new(on_result)),
        });
    }

    /// Where the original calls MessageBoxW(caption, text, flags). An empty caption stays empty (original NULL caption).
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

    pub(super) fn program_info_box(&mut self, caption: &str, prefix:String, suffix:String) {
        let usage=AboutUsage::new(prefix,suffix);
        self.system_box(caption, &usage.text(), mb::MB_OK, |_, _, _| {});
        if let Some(last) = self.msgboxes.last_mut() {
            last.program_icon = true;
            last.about_usage=Some(usage);
        }
    }

    /// Closes the front box with the button's ModalResult. ModalResult 0 (DFM buttons of a kind the original does not handle) does not close.
    pub(super) fn answer_message_box(&mut self, modal_result: i32, ctx: &egui::Context) {
        if modal_result == 0 || self.msgboxes.is_empty() {
            return;
        }
        let mut current = self.msgboxes.remove(0);
        if let Some(callback) = current.on_result.take() {
            callback(self, mb::map_result(modal_result), ctx);
        }
    }

    /// [RES] caption (with the '&' access key). The original literal when the language entry is missing.
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
        let (image, options) = if id == 101 {
            let data = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/ndr-icon.png")).ok()?;
            (egui::ColorImage::from_rgba_unmultiplied([data.width as usize, data.height as usize], &data.rgba), egui::TextureOptions::LINEAR)
        } else {
            let (width, height, rgba) = native::system_icon_rgba(id).ok()?;
            (egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba), egui::TextureOptions::NEAREST)
        };
        let texture = ctx.load_texture(
            format!("ndr-system-icon-{id:x}"),
            image,
            options,
        );
        self.msgbox_icons.insert(id, texture.clone());
        Some(texture)
    }

    pub(super) fn msgbox_ui(&mut self, ctx: &egui::Context) {
        let Some(current) = self.msgboxes.first_mut() else {
            return;
        };
        if let Some(usage)=current.about_usage.as_mut(){
            if usage.sample(){current.text=usage.text();}
            ctx.request_repaint_after(Duration::from_secs(1).saturating_sub(usage.last.elapsed()));
        }
        let (title, summary, text, flags, serial, focused, system, program_icon) = (
            current.title.clone(),
            current.summary.clone(),
            current.text.clone(),
            current.flags,
            current.serial,
            current.focused,
            current.system,
            current.program_icon,
        );
        let caption = if system { title.clone() } else { mb::FORM_CAPTION.to_owned() };
        let specs = mb::buttons(flags);
        let captions: [String; 3] = std::array::from_fn(|i| specs[i].map(|s| self.msgbox_caption(&s)).unwrap_or_default());
        let icon = self.msgbox_icon(ctx, if program_icon { 101 } else { mb::icon_id(flags) });
        let screen = ctx.content_rect();
        let icon_size = if program_icon {
            if screen.width() < 480.0 || screen.height() < 350.0 { 64.0 } else { 96.0 }
        } else { 32.0 };
        let text_left = if program_icon { icon_size + 24.0 } else { 64.0 };
        let extra_width = text_left - 64.0;
        let dark = self.dark;
        let text_color = if dark { Color32::from_rgb(241, 245, 249) } else { Color32::from_rgb(17, 24, 39) };
        let title_color = if dark { Color32::from_rgb(203, 213, 225) } else { Color32::from_rgb(70, 70, 70) };
        let strip = if dark { Color32::from_rgb(31, 41, 55) } else { Color32::from_rgb(233, 233, 233) };
        let summary_color = if dark { Color32::from_rgb(96, 165, 250) } else { Color32::from_rgb(28, 99, 174) };
        let mut pressed: Option<i32> = None;
        let mut layout = MsgBoxLayout::default();

        let modal_id = egui::Id::new(("ndr-msgbox", serial));
        super::keep_opaque(ctx, modal_id);
        super::movable_modal(ctx, modal_id).show(ctx, |ui| {
            let font = FontId::proportional(FONT);
            let max_text_width = (screen.width() - 124.0 - extra_width).max(if program_icon { 96.0 } else { 180.0 }).min(840.0);
            let (title_galley, summary_galley, line_galleys, line_height) = ui.fonts_mut(|fonts| {
                let line_height = fonts.row_height(&font);
                (
                    fonts.layout_no_wrap(title.clone(), font.clone(), title_color),
                    fonts.layout_no_wrap(summary.clone(), font.clone(), summary_color),
                    mb::lines(&text).into_iter().map(|line| {
                        let mut job = LayoutJob::single_section(line, egui::TextFormat::simple(font.clone(), text_color));
                        job.wrap.max_width = max_text_width;
                        fonts.layout_job(job)
                    }).collect::<Vec<_>>(),
                    line_height,
                )
            });
            let text_width = line_galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
            // Preserve the original DFM line height for short messages.
            // Wrapped rows need the actual galley height.
            let galley_height = |g: &egui::Galley| if g.rows.len() <= 1 { line_height } else { g.size().y.max(line_height * g.rows.len() as f32) };
            let text_height = line_galleys.iter().map(|g| galley_height(g)).sum::<f32>() + 4.0 * line_galleys.len().saturating_sub(1) as f32;
            let has_title = !system && !title.is_empty();
            let has_summary = !summary.is_empty();
            let body_limit = (ctx.content_rect().height() - 190.0).max(80.0);
            let body_height = text_height.min(body_limit);
            let scroll_body = text_height > body_height + 0.5;
            let reserved_height = if program_icon { body_height.max(icon_size) } else { body_height };
            let geometry = mb::layout(has_title, has_summary, title_galley.size().x, summary_galley.size().x, text_width + extra_width + if scroll_body { 14.0 } else { 0.0 }, reserved_height);
            let visible = specs.iter().filter(|s| s.is_some()).count();
            let button_width = ui.fonts_mut(|fonts| captions.iter().map(|caption| {
                fonts.layout_no_wrap(mb::display_caption(caption).0, font.clone(), text_color).size().x.ceil() + 16.0
            }).fold(78.0_f32, f32::max));
            let button_min = 8.0 + visible as f32 * (button_width + 3.0) - 3.0 + 15.0;
            let caption_width = ui.fonts_mut(|fonts| fonts.layout_no_wrap(caption.clone(), font.clone(), title_color).size().x + 44.0);
            let client = vec2(mb::client_width(&geometry, visible).max(button_min).max(caption_width), geometry.client_height);

            // Window title bar (form Caption and close button: closing gives mrCancel). Dragging the bar moves the window.
            if super::modal_title_bar(ui, modal_id, &caption, Some(client.x)) {
                pressed = Some(2);
            }
            let (rect, _) = ui.allocate_exact_size(client, Sense::hover());
            let painter = ui.painter_at(rect);
            let origin = rect.min;
            if has_title {
                painter.rect_filled(Rect::from_min_size(origin, vec2(client.x, 24.0)), 0.0, strip);
                painter.galley(origin + vec2(10.0, 4.0), title_galley, title_color);
            }
            if let Some(texture) = &icon {
                let icon_top = geometry.image_top + if program_icon { (reserved_height - icon_size) * 0.5 } else { 0.0 };
                let icon_rect = Rect::from_min_size(origin + vec2(if program_icon { 12.0 } else { 8.0 }, icon_top), vec2(icon_size, icon_size));
                painter.image(
                    texture.id(),
                    icon_rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                layout.icon_rect = Some(icon_rect);
            }
            if has_summary {
                painter.galley(origin + vec2(text_left, geometry.summary_top), summary_galley, summary_color);
            }
            let line_count = line_galleys.iter().map(|g| g.rows.len().max(1)).sum();
            if scroll_body {
                let body = Rect::from_min_size(origin + vec2(text_left, geometry.text_top), vec2(client.x - text_left - 10.0, body_height));
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                    ui.set_clip_rect(body.intersect(ui.clip_rect()));
                    egui::ScrollArea::vertical().id_salt(modal_id.with("message-body")).max_height(body_height).auto_shrink([false, false]).show(ui, |ui| {
                        let (content, _) = ui.allocate_exact_size(vec2(text_width, text_height), Sense::hover());
                        let mut y = content.min.y;
                        for galley in line_galleys {
                            let height = galley_height(&galley);
                            ui.painter().galley(egui::pos2(content.min.x, y), galley, text_color);
                            y += height + 4.0;
                        }
                    });
                });
            } else {
                let mut y = geometry.text_top;
                for galley in line_galleys {
                    let height = galley_height(&galley);
                    painter.galley(origin + vec2(text_left, y), galley, text_color);
                    y += height + 4.0;
                }
            }

            for slot in 0..3 {
                let Some(spec) = specs[slot] else { continue };
                let left = client.x - 15.0 - button_width - (button_width + 3.0) * (2 - slot) as f32;
                let button_rect = Rect::from_min_size(origin + vec2(left, mb::button_top(client.y)), vec2(button_width, 25.0));
                let (display, underline) = mb::display_caption(&captions[slot]);
                let mut job = LayoutJob::default();
                for (index, character) in display.chars().enumerate() {
                    let mut format = egui::TextFormat { font_id: font.clone(), color: text_color, ..Default::default() };
                    if underline == Some(index) {
                        format.underline = egui::Stroke::new(1.0_f32, text_color);
                    }
                    job.append(&character.to_string(), 0.0, format);
                }
                let response = ui.put(button_rect, egui::Button::new(job).min_size(vec2(button_width, 25.0)));
                // TfMsgBox: the first button in TabOrder. In place of MessageBoxW (system): MB_DEFBUTTON1..3.
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
            layout.text_left = text_left;
            layout.title_shown = has_title;
            layout.line_count = line_count;
            layout.captions = captions.clone();
        });

        // Keyboard: Esc is the Cancel button, Enter without focus the Default button, access keys work without Alt (CM_DIALOGCHAR).
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
