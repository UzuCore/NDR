use super::*;

/// 파일 목록 ScrollArea: 끌기·스크롤 막대는 그대로, 바퀴는 원본 규칙(App::wheel_scroll)으로 따로 처리한다.
/// 목록 스크롤: 막대만(바퀴는 원본 WMMouseWheel 처리, 끌어서 굴리기는 원본에 없다 — 빈 곳 끌기는 표시한 항목 끌기).
pub(super) const WHEEL_OFF: egui::scroll_area::ScrollSource = egui::scroll_area::ScrollSource { scroll_bar: true, drag: false, mouse_wheel: false };

/// C# "::drives::"(내 PC) 보기인가.
pub(super) fn is_drives(path: &Path) -> bool {
    path.as_os_str() == super::remote_window::DRIVES
}

/// 원본 GetSelAreaAt(0x7dae7c): 미리 보기가 아닌 보기에서 항목 칸 왼쪽 12화소(GetItemRect의 left ~ left + 0xc).
fn selection_area(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(rect.min, egui::pos2((rect.left() + 12.0).min(rect.right()), rect.bottom()))
}

/// 원본 DrawListItem(0x7d8f58)의 칸 글: 칸 안쪽(왼쪽 + 5, 오른쪽 − 5)에 한 줄(넘치면 …). 크기는 오른쪽 맞춤(단위 GB·MB 색),
/// 날짜 칸은 날짜를 왼쪽·시각을 오른쪽(DateToStr·TimeToStr), 나머지는 왼쪽 맞춤. 자세히·자동·이름 보기가 함께 쓴다.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_original_cell(ui: &mut egui::Ui, rect: egui::Rect, column: DetailColumn, text: &str, cell_color: Color32, cursor_row: bool, o: &super::skin_pane::ListColors, list_style: u8) {
    let inner = rect.shrink2(egui::vec2(5.0, 0.0));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let layout = |ui: &mut egui::Ui, parts: &[(&str, Color32)]| {
        let mut job = egui::text::LayoutJob::default();
        for (part, part_color) in parts.iter().filter(|(p, _)| !p.is_empty()) {
            super::list_text(*part, list_style).color(*part_color).append_to(&mut job, ui.style(), egui::FontSelection::Default, egui::Align::Center);
        }
        job.wrap = egui::text::TextWrapping { max_width: inner.width().max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let top = |height: f32| rect.top() + ((rect.height() - height) / 2.0).floor();
    match column {
        DetailColumn::Size => {
            let (number, unit) = if cursor_row { (text, "") } else { super::skin_pane::split_size_text(text) };
            let upper = text.to_uppercase();
            let unit_color = if upper.contains('G') { o.gb } else if upper.contains('M') { o.mb } else { cell_color };
            let galley = layout(ui, &[(number, cell_color), (unit, unit_color)]);
            painter.galley(egui::pos2(inner.right() - galley.size().x, top(galley.size().y)), galley, cell_color);
        }
        // 원본 DrawListItem 필드 4: DateToStr는 칸 왼쪽, TimeToStr는 오른쪽. 둘 사이는 listing::LIST_DATE_TIME_GAP.
        // 고친 것: 첫 빈칸에서 나눠 "5일 전 23:19"가 "5일"과 "전 23:19"로 갈라졌다.
        DetailColumn::Modified if text.contains(ndr::listing::LIST_DATE_TIME_GAP) => {
            let (date, time) = text.split_once(ndr::listing::LIST_DATE_TIME_GAP).unwrap_or((text, ""));
            let date = layout(ui, &[(date, cell_color)]);
            let time = layout(ui, &[(time, cell_color)]);
            // 칸이 좁아 날짜와 시각이 붙으면(사이 6 미만) 한 줄 글로 왼쪽에 쓴다.
            if date.size().x + time.size().x + 6.0 <= inner.width() {
                painter.galley(egui::pos2(inner.left(), top(date.size().y)), date, cell_color);
                painter.galley(egui::pos2(inner.right() - time.size().x, top(time.size().y)), time, cell_color);
            } else {
                let galley = layout(ui, &[(text, cell_color)]);
                painter.galley(egui::pos2(inner.left(), top(galley.size().y)), galley, cell_color);
            }
        }
        _ => {
            let galley = layout(ui, &[(text, cell_color)]);
            painter.galley(egui::pos2(inner.left(), top(galley.size().y)), galley, cell_color);
        }
    }
}

/// 원본 DrawListItem(0x7d8f58)은 이름을 DT_END_ELLIPSIS | DT_MODIFYSTRING으로 그려 줄였는지(+0x65)를 남긴다.
/// NDR은 커서 줄 이름 칸과 보인 이름·온 이름의 글 모양을 기억해 두고, 툴팁(original_name_tip)이 쓴다.
pub(super) type NameTip = std::cell::RefCell<Option<(usize, usize, egui::Rect, std::sync::Arc<egui::Galley>, std::sync::Arc<egui::Galley>)>>;

/// 원본 DrawThumbnailItem(0x7da1c4)은 DrawThumbnailText가 이름을 줄였는지(+0x65)를 남긴다. 이번 그림에서 커서 칸의 이름이 줄었으면
/// (패널, 줄, 항목 칸, 이름 글 첫 줄 위, 썸네일 크기, DT_WORDBREAK로 나눈 온 이름 줄들).
pub(super) type ThumbTip = std::cell::RefCell<Option<(usize, usize, egui::Rect, f32, f32, Vec<std::sync::Arc<egui::Galley>>)>>;

/// 드라이브 항목 정보(GetDriveName, GetDriveTypeW, 고정 디스크면 (여유, 전체)).
pub(super) type DriveInfo = (String, u32, Option<(u64, u64)>);

/// 패널 목록이 함께 쓰는 드라이브 목록 캐시(원본 TNXDriveList 자리): 읽은 때, (루트, 정보), 뒤에서 새로 읽는 중인지.
#[derive(Default)]
pub(super) struct DriveListCache {
    at: Option<Instant>,
    items: Vec<(PathBuf, Option<DriveInfo>)>,
    refreshing: bool,
    /// 채운 함수(SystemHooks ready_drives, drive_item). 바뀌면(시험의 가짜 함수 등) 새로 읽는다.
    source: Option<(usize, usize)>,
}

impl DriveListCache {
    /// 다음 목록은 드라이브를 새로 읽는다(F5, 드라이브 이름 바꾸기, 네트워크 드라이브 연결·끊기).
    pub(super) fn invalidate(cache: &std::sync::Arc<std::sync::Mutex<DriveListCache>>) {
        if let Ok(mut guard) = cache.lock() {
            guard.at = None;
        }
    }
}

/// 캐시의 드라이브 목록: 5초 안에 읽은 것이면 그대로, 지났으면 그대로 돌려주고 뒤에서 한 번 새로 읽는다. 처음이면 바로 읽는다.
pub(super) fn cached_drive_items(cache: &std::sync::Arc<std::sync::Mutex<DriveListCache>>, ready: fn() -> Vec<PathBuf>, item: fn(&Path) -> Option<DriveInfo>) -> Vec<(PathBuf, Option<DriveInfo>)> {
    let read = move || ready().into_iter().map(|root| { let info = item(&root); (root, info) }).collect::<Vec<_>>();
    let source = Some((ready as usize, item as usize));
    {
        let Ok(mut guard) = cache.lock() else { return read() };
        let at = guard.at.filter(|_| guard.source == source);
        match at {
            Some(at) if at.elapsed() < Duration::from_secs(5) => return guard.items.clone(),
            Some(_) => {
                let items = guard.items.clone();
                if !guard.refreshing {
                    guard.refreshing = true;
                    let cache = cache.clone();
                    thread::spawn(move || {
                        let fresh = read();
                        if let Ok(mut guard) = cache.lock() {
                            guard.items = fresh;
                            guard.at = Some(Instant::now());
                            guard.refreshing = false;
                            guard.source = source;
                        }
                    });
                }
                return items;
            }
            None => {}
        }
    }
    let fresh = read();
    if let Ok(mut guard) = cache.lock() {
        guard.items = fresh.clone();
        guard.at = Some(Instant::now());
        guard.source = source;
    }
    fresh
}

/// Win32 DrawTextW DT_WORDBREAK처럼 낱말(빈칸) 사이에서만 줄을 나눈다. 폭보다 긴 낱말은 그대로 한 줄(더 넓다).
pub(super) fn word_break_lines(text: &str, width: f32, measure: &mut dyn FnMut(&str) -> f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let candidate = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
        if line.is_empty() || measure(&candidate) <= width {
            line = candidate;
        } else {
            lines.push(std::mem::take(&mut line));
            line = word.to_owned();
        }
    }
    lines.push(line);
    lines
}
#[allow(clippy::too_many_arguments)]
pub(super) fn note_cursor_name(tip: &NameTip, ui: &egui::Ui, pane: usize, row: usize, cell: egui::Rect, shown: &str, full: &str, style: u8) {
    let galley = |text: &str| egui::WidgetText::from(super::list_text(text, style)).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    *tip.borrow_mut() = Some((pane, row, cell, galley(shown), galley(full)));
}

/// 원본 GetFieldMaxWidth(0x7df9d8)가 남기는 값: 칸마다 가장 넓은 글(DetailColumn 순서 7칸, 목록 글꼴), 이름 너비 합(+0x358),
/// 드라이브가 아닌 항목 수(목록 항목 수 − 종류 4, GetAdjustedNameWidth·GetAdjustedExtWidth의 나눗수), 확장자 평균(+0x350).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct FieldMax {
    pub max: [f32; 7],
    pub name_sum: f32,
    pub items: usize,
    pub ext_average: f32,
}

impl App {
    /// 목록 글꼴로 잰 글 너비(원본 GetTextWidth).
    pub(super) fn list_text_width(&self, ui: &egui::Ui, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let font = egui::FontId::new(self.list_font_size, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()));
        ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x).ceil()
    }

    /// 원본 GetFieldMaxWidth(0x7df9d8)가 남기는 값(FieldMax). 목록이 바뀔 때만 다시 잰다(원본 local_48[2] 캐시).
    /// 이름은 폴더·파일(종류 1·3), 확장자는 파일, 크기는 크기가 0이 아닌 파일, 날짜·설명은 폴더·파일에서 잰다.
    /// 크기 칸은 늘 머리글(FIELD_SIZE, 원본 GetString)보다 좁지 않다. 확장자 평균은 비어 있지 않은 확장자 너비의 정수 평균(+0x350).
    pub(super) fn original_field_max(&self, ui: &egui::Ui, index: usize) -> FieldMax {
        let pane = &self.panes[index];
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (pane.generation, pane.entries.len(), self.list_font_size.to_bits(), pane.view.cells.len(), pane.view.cells.first().map(|c| c[0].clone()), self.language_pack.text("RES.FIELD_SIZE", "Size")).hash(&mut h);
            h.finish()
        };
        let id = egui::Id::new(("ndr-original-field-max", index));
        if let Some((_, fm)) = ui.data(|d| d.get_temp::<(u64, FieldMax)>(id)).filter(|c| c.0 == key) {
            return fm;
        }
        // 고친 것(빠르기): 전에는 항목마다 글 배치(layout_no_wrap, 그림 조각 만들기)로 재서 파일 2만 개면 약 60ms였고, 폴더 감시로
        // 목록이 다시 읽힐 때마다 그만큼 멈췄다. 글자 폭의 합으로 잰다(원본 GetTextWidth도 글자 폭 합이다).
        let font = egui::FontId::new(self.list_font_size, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()));
        let mut fm = FieldMax::default();
        let (mut ext_sum, mut ext_count) = (0f32, 0usize);
        ui.fonts_mut(|fonts| {
            let mut measure = |text: &str| -> f32 { text.chars().map(|c| fonts.glyph_width(&font, c)).sum::<f32>().ceil() };
            for (entry, cells) in pane.entries.iter().zip(&pane.view.cells) {
                if entry.drive {
                    continue;
                }
                let name = measure(&cells[0]);
                fm.max[0] = fm.max[0].max(name);
                fm.name_sum += name;
                fm.items += 1;
                if !entry.is_dir {
                    let ext = measure(&cells[1]);
                    fm.max[1] = fm.max[1].max(ext);
                    if ext > 0.0 {
                        ext_sum += ext;
                        ext_count += 1;
                    }
                    if entry.size > 0 {
                        fm.max[3] = fm.max[3].max(measure(&cells[3]));
                    }
                }
                fm.max[4] = fm.max[4].max(measure(&cells[4]));
                fm.max[6] = fm.max[6].max(measure(&cells[6]));
            }
        });
        if ext_count > 0 {
            fm.ext_average = (ext_sum / ext_count as f32).floor();
        }
        fm.max[3] = fm.max[3].max(self.list_text_width(ui, &self.language_pack.text("RES.FIELD_SIZE", "Size")));
        ui.data_mut(|d| d.insert_temp(id, (key, fm)));
        fm
    }

    /// 원본 GetAdjustedNameWidth(0x7e00a8): 가장 긴 이름 + 10. 드라이브가 아닌 항목이 6개 이상이고 "WWWWW"가 그보다 좁으며
    /// 이름 평균(정수 나눗셈) × 2가 가장 긴 이름보다 짧으면 Trunc(평균 × 1.5 + 10)(0x7e012c의 1.5, 10.0; 0x8c3c8c는 버림 FISTP).
    pub(super) fn original_adjusted_name_width(&self, ui: &egui::Ui, fm: &FieldMax) -> f32 {
        let base = fm.max[0] + 10.0;
        if fm.items > 5 && self.list_text_width(ui, "WWWWW") < base {
            let average = (fm.name_sum / fm.items as f32).floor();
            if average * 2.0 < fm.max[0] {
                return (average * 1.5 + 10.0).trunc();
            }
        }
        base
    }

    /// 원본 GetAdjustedExtWidth(0x7e0134): 가장 긴 확장자 + 10. 드라이브가 아닌 항목이 6개 이상, 확장자 평균이 있고, "WAA"가 그보다
    /// 좁으며 평균 × 2가 가장 긴 확장자보다 짧으면 Trunc(평균 × 1.5 + 10)(0x7e01b0의 1.5, 10.0).
    pub(super) fn original_adjusted_ext_width(&self, ui: &egui::Ui, fm: &FieldMax) -> f32 {
        let base = fm.max[1] + 10.0;
        if fm.items > 5 && fm.ext_average > 0.0 && self.list_text_width(ui, "WAA") < base && fm.ext_average * 2.0 < fm.max[1] {
            return (fm.ext_average * 1.5 + 10.0).trunc();
        }
        base
    }

    /// 원본 FitFileNameFields(0x7e0298): 이름 = max(가장 긴 이름 + 10, "WWWWAAAA" + 10), 확장자 = max(가장 긴 확장자 + 10, "WWA" + 10).
    /// 둘의 합이 `room`보다 크면 `room` − GetAdjustedExtWidth가 21보다 작을 때 둘을 비율로(정수 나눗셈), 아니면 확장자 = GetAdjustedExtWidth,
    /// 이름 = 나머지. (이름, 확장자). 확장자를 이름에 붙이면 확장자는 0, 이름은 min(가장 긴 이름 + 10, room)이고 그것이 "WWWWAAAA" + 10보다
    /// 좁고 "WWWWAAAA" + 10이 room에 들어가면 "WWWWAAAA" + 10.
    pub(super) fn original_fit_name_fields(&self, ui: &egui::Ui, fm: &FieldMax, room: f32, attach: bool) -> (f32, f32) {
        let eight = self.list_text_width(ui, "WWWWAAAA") + 10.0;
        if attach {
            let natural = fm.max[0] + 10.0;
            let name = if natural <= room { natural } else { room };
            return (if name < eight && eight <= room { eight } else { name.max(0.0) }, 0.0);
        }
        let name = (fm.max[0] + 10.0).max(eight);
        let ext = (fm.max[1] + 10.0).max(self.list_text_width(ui, "WWA") + 10.0);
        if name + ext <= room {
            return (name, ext);
        }
        let adjusted_ext = self.original_adjusted_ext_width(ui, fm);
        if room - adjusted_ext < 21.0 {
            let sum = (name + ext).max(1.0);
            ((name * room / sum).trunc().max(0.0), (ext * room / sum).trunc().max(0.0))
        } else {
            (room - adjusted_ext, adjusted_ext)
        }
    }

    /// 원본 UpdateDisplayDetails(0x7e1374)의 칸 너비(스킨 테마 자세히 보기). 칸마다 GetFieldMaxWidth(가장 넓은 글) + 10(글이 없으면 0),
    /// 속성 칸은 "ARHS"(SFTP는 "drwxrwxrwx") + 10이다. 모두 합쳐 `avail`(세로 막대를 뺀 목록 너비)을 넘으면 이름을 GetAdjustedNameWidth,
    /// 확장자를 GetAdjustedExtWidth로 줄인다. 그러고 나서 남는 자리가 있으면 FitFileNameFields(나머지 칸을 뺀 너비)로 이름·확장자를
    /// 다시 맞추고("WWWWAAAA"·"WWA" 최소), 여전히 넘치면 그대로 둔다(가로로 넘긴다). 남는 너비는 채우지 않는다.
    /// NDR 이름 칸은 아이콘 머리(GetListHeaderWidth)를 함께 담는다.
    pub(super) fn original_detail_widths(&self, ui: &egui::Ui, index: usize, avail: f32, columns: &[(super::list_view::DetailColumn, String, f32)]) -> Vec<Option<f32>> {
        use super::list_view::DetailColumn as C;
        let fm = self.original_field_max(ui, index);
        let max = fm.max;
        let pane = &self.panes[index];
        let sftp = pane.remote.as_ref().is_some_and(|r| r.is_sftp());
        let header = if self.settings.flag("ShowIcons", true) { 30.0 } else { 14.0 };
        let attach = self.settings.flag("AttachExtensionToName", false);
        let natural = |w: f32| if w >= 1.0 { w + 10.0 } else { 0.0 };
        let attributes = if pane.entries.is_empty() { 0.0 } else { self.list_text_width(ui, if sftp { "drwxrwxrwx" } else { "ARHS" }) + 10.0 };
        let field = |column: C| -> f32 {
            match column {
                C::Name => natural(max[0]),
                C::Extension => natural(max[1]),
                C::Size => natural(max[3]),
                C::Modified => natural(max[4]),
                C::Attributes => attributes,
                C::Description => natural(max[6]),
                C::Kind => 0.0,
            }
        };
        let mut widths: Vec<f32> = columns.iter().map(|c| field(c.0)).collect();
        let position = |column: C| columns.iter().position(|c| c.0 == column);
        let (n, e) = (position(C::Name), position(C::Extension));
        let total = |widths: &[f32]| header + widths.iter().sum::<f32>();
        if total(&widths) > avail {
            if let Some(n) = n {
                widths[n] = self.original_adjusted_name_width(ui, &fm);
            }
            if let Some(e) = e {
                widths[e] = self.original_adjusted_ext_width(ui, &fm);
            }
        }
        let sum = total(&widths);
        if sum < avail {
            let (name_w, ext_w) = (n.map_or(0.0, |n| widths[n]), e.map_or(0.0, |e| widths[e]));
            // 원본 FitFileNameFields(목록 너비 − (0x344 − (이름 + 확장자))): 0x344는 머리를 담으므로 room = 너비 − 머리 − 나머지 칸.
            let room = avail - (sum - name_w - ext_w);
            let (name, ext) = self.original_fit_name_fields(ui, &fm, room, attach || e.is_none());
            if let Some(n) = n {
                widths[n] = name;
            }
            if let Some(e) = e {
                widths[e] = ext;
            }
        }
        if let Some(n) = n {
            widths[n] += header;
        }
        widths.into_iter().map(Some).collect()
    }

    /// 원본 SetItemIndex(0x7dbdec): 커서가 옮겨 간 항목(드라이브 제외)의 이름이 줄여 그려졌으면 600ms(+0x408) 뒤
    /// ShowToolTip(0x7d7454)이 TNXToolTipWnd(WS_POPUP | WS_BORDER, 바탕 clInfoBk, 글 clInfoText, 목록 글꼴)에 온 이름을
    /// 가운데로 그린다. 자리: 항목 칸 왼쪽 + GetListHeaderWidth(아이콘이면 30, 아니면 14) − 2, 위 − 2, 너비 = 글 너비 + 10,
    /// 높이 = 줄 높이 + 4, 화면 밖이면 안으로 민다. 3초(TIP_HIDE_MS) 뒤 숨는다. 목록에 초점이 있을 때만(활성 패널, 창 초점). 스킨 테마.
    pub(super) fn original_name_tip(&mut self, ctx: &egui::Context, index: usize) {
        let tip = self.name_tip.borrow_mut().take_if(|(pane, ..)| *pane == index);
        let thumb_tip = self.thumb_tip.borrow_mut().take_if(|(pane, ..)| *pane == index);
        self.name_tip_shown[index] = None;
        let cursor = self.panes[index].cursor;
        let now = Instant::now();
        let since = match self.tip_since[index] {
            Some((row, at)) if row == cursor => at,
            _ => {
                self.tip_since[index] = Some((cursor, now));
                now
            }
        };
        // 원본 IsHintMsg: 바퀴(WM_MOUSEWHEEL)면 숨긴다.
        if ctx.input(|i| i.raw_scroll_delta != egui::Vec2::ZERO) {
            self.tip_since[index] = Some((cursor, now - Duration::from_secs(60)));
            return;
        }
        let color = |index: i32| {
            let (r, g, b) = native::system_color(index);
            Color32::from_rgb(r, g, b)
        };
        // 원본 ShowToolTip 형식 3(미리 보기): 글 자리 왼쪽 = 항목 왼쪽 + (항목 너비 − 썸네일 크기) / 2, 너비 = 썸네일 크기,
        // DrawTextW(DT_CALCRECT | DT_WORDBREAK | DT_CENTER | DT_NOPREFIX)로 높이를 재고(더 좁으면 썸네일 크기), 사방 4를 넓혀
        // 이름 글 위에 겹친다(+0x2cc = 0: 여러 줄 가운데). NDR은 칸 안 이름 글 첫 줄에 맞춘다.
        if let Some((_, row, cell, text_top, thumb, lines)) = thumb_tip {
            let drive = self.panes[index].entries.get(row).is_none_or(|e| e.drive);
            if row != cursor || drive || self.active != index || !ctx.input(|i| i.focused) || lines.is_empty() {
                return;
            }
            let (delay, life) = (Duration::from_millis(600), Duration::from_millis(600 + TIP_HIDE_MS));
            let elapsed = now.saturating_duration_since(since);
            if elapsed < delay {
                ctx.request_repaint_after(delay - elapsed);
                return;
            }
            if elapsed >= life {
                return;
            }
            ctx.request_repaint_after(life - elapsed);
            let widest = lines.iter().map(|g| g.size().x).fold(thumb, f32::max);
            let line_height = lines.iter().map(|g| g.size().y).fold(0.0, f32::max);
            let left = (cell.left() + ((cell.width() - thumb) / 2.0).trunc()).round();
            let inner = egui::Rect::from_min_size(egui::pos2(left, text_top), egui::vec2(widest, line_height * lines.len() as f32));
            let mut rect = inner.expand(4.0);
            let (back, border, fore) = (color(24), color(6), color(23));
            let paint = |painter: &egui::Painter, rect: egui::Rect| {
                painter.rect(rect, 0.0, back, egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
                let inner = rect.shrink(4.0);
                for (i, galley) in lines.iter().enumerate() {
                    let at = egui::pos2(inner.center().x - galley.size().x / 2.0, inner.top() + i as f32 * line_height).round();
                    painter.galley(at, galley.clone(), fore);
                }
            };
            if show_tip_window(ctx, index, rect, &paint) {
                self.name_tip_shown[index] = Some(rect);
                return;
            }
            let screen = ctx.content_rect();
            if rect.right() > screen.right() {
                rect = rect.translate(egui::vec2(screen.right() - rect.right(), 0.0));
            }
            if rect.bottom() > screen.bottom() {
                rect = rect.translate(egui::vec2(0.0, screen.bottom() - rect.bottom()));
            }
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("ndr-name-tip", index))));
            paint(&painter, rect);
            self.name_tip_shown[index] = Some(rect);
            return;
        }
        let Some((_, row, cell, shown, full)) = tip else { return };
        let drive = self.panes[index].entries.get(row).is_none_or(|e| e.drive);
        if row != cursor || drive || self.active != index || !ctx.input(|i| i.focused) {
            return;
        }
        let header = if self.settings.flag("ShowIcons", true) { 30.0 } else { 14.0 };
        if header + shown.size().x + 5.0 <= cell.width() {
            return;
        }
        let (delay, life) = (Duration::from_millis(600), Duration::from_millis(600 + TIP_HIDE_MS));
        let elapsed = now.saturating_duration_since(since);
        if elapsed < delay {
            ctx.request_repaint_after(delay - elapsed);
            return;
        }
        if elapsed >= life {
            return;
        }
        ctx.request_repaint_after(life - elapsed);
        let mut rect = egui::Rect::from_min_size(egui::pos2(cell.left() + header - 2.0, cell.top() - 2.0), egui::vec2(full.size().x + 10.0, cell.height() + 4.0));
        let (back, border, fore) = (color(24), color(6), color(23));
        let paint = |painter: &egui::Painter, rect: egui::Rect| {
            painter.rect(rect, 0.0, back, egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
            let at = (rect.center() - full.size() / 2.0).round();
            painter.galley(at, full.clone(), fore);
        };
        if show_tip_window(ctx, index, rect, &paint) {
            self.name_tip_shown[index] = Some(rect);
            return;
        }
        let screen = ctx.content_rect();
        if rect.right() > screen.right() {
            rect = rect.translate(egui::vec2(screen.right() - rect.right(), 0.0));
        }
        if rect.bottom() > screen.bottom() {
            rect = rect.translate(egui::vec2(0.0, screen.bottom() - rect.bottom()));
        }
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("ndr-name-tip", index))));
        paint(&painter, rect);
        self.name_tip_shown[index] = Some(rect);
    }
}

/// 원본 TNXToolTipWnd 숨김 시간(+0x2c8): 처음 값은 5000(0x7efbd9)이지만 목록 생성자가 3000으로 바꾼다(0x7d4ef5).
/// 고친 것: 전에는 5초였다.
pub(super) const TIP_HIDE_MS: u64 = 3000;

/// 원본 TNXToolTipWnd는 따로 뜨는 팝업 창(WS_POPUP | WS_BORDER)이라 메인 창 밖까지 나가고, 화면(Screen.Width/Height) 밖이면 안으로 민다.
/// 툴팁이 창 안에 들어가면 false(창 안에 그린다). 넘치면 테두리 없는 egui 창(활성화하지 않음, 포인터 통과, 늘 위, 작업 표시줄 없음)을
/// 화면 좌표에 띄우고 true. 여러 창을 그릴 수 없는 환경(시험 등)이면 false.
pub(super) fn show_tip_window(ctx: &egui::Context, index: usize, rect: egui::Rect, paint: &dyn Fn(&egui::Painter, egui::Rect)) -> bool {
    if ctx.content_rect().contains_rect(rect) || ctx.embed_viewports() {
        return false;
    }
    let (inner, monitor) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size));
    let Some(inner) = inner else { return false };
    let mut at = inner.min + rect.min.to_vec2();
    // 원본은 Screen.Width/Height 안으로 민다. egui는 모니터 크기만 알고 원점은 모르므로, 메인 창이 (0, 0)부터 그 크기 안에 있을 때만 민다.
    if let Some(monitor) = monitor.filter(|m| inner.min.x >= 0.0 && inner.min.y >= 0.0 && inner.max.x <= m.x + 1.0 && inner.max.y <= m.y + 1.0) {
        at.x = at.x.min(monitor.x - rect.width()).max(0.0);
        at.y = at.y.min(monitor.y - rect.height()).max(0.0);
    }
    let builder = egui::ViewportBuilder::default()
        .with_title("NDR")
        .with_decorations(false)
        .with_resizable(false)
        .with_taskbar(false)
        .with_active(false)
        .with_always_on_top()
        .with_mouse_passthrough(true)
        .with_position(at)
        .with_inner_size(rect.size());
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("ndr-name-tip-window", index)), builder, |ctx, _| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let local = egui::Rect::from_min_size(ui.max_rect().min, rect.size());
            paint(ui.painter(), local);
        });
    });
    true
}

/// 포인터가 이 줄의 표시 칸 위에 있는가(누르기·두 번 누르기를 표시 칸 몫으로 돌릴 때).
pub(super) fn pointer_in_selection_area(rect: egui::Rect, ui: &egui::Ui) -> bool {
    ui.input(|i| i.pointer.interact_pos()).is_some_and(|pos| selection_area(rect).contains(pos) && ui.clip_rect().contains(pos))
}

/// 원본 MouseDown(0x7d63c8)·MouseMove(0x7d65f0)·MouseUp(0x7d6740)(스킨 테마). 왼쪽 단추를 이 줄에서 누른 순간에 처리하고 true.
/// - Shift(+Ctrl): 지금 커서 줄~누른 줄을 더해 고르고, 끌기 기준(+0x3d4) = 전 커서 줄, 지난 끝(+0x3d8) = 누른 줄.
/// - Ctrl만: 누른 줄을 뒤집는다(고른 줄이 없고 전 커서 줄이 다르면 그 줄도 고른다). 그냥 누르기는 커서만 옮긴다.
/// - 표시 칸(왼쪽 12화소)이면 그다음 그 줄을 한 번 더 뒤집고 기준 = 끝 = 그 줄.
/// 누른 채 다른 줄 위로 움직이면, 지난 끝 줄이 기준과 다를 때 Ctrl이 없으면 표시를 모두 지우고(Ctrl이면 기준~지난 끝 줄만 풀고)
/// 기준~포인터 아래 줄을 고른다. 단추를 놓으면 끝(pane_ui 앞에서 range_drag를 비운다).
/// 항목 끌기를 시작하는 그림인가. C# 테마는 egui drag_started_by. 스킨 테마는 원본 FUN_004856d8(목록 OnMouseMove):
/// 왼쪽·오른쪽 단추를 누른 채 누른 자리에서 가로나 세로로 8화소를 넘게 움직였고, 누른 자리가 표시 칸(GetSelAreaAt)이 아니면 시작한다.
/// 시작할지는 끌고 있지 않은 동안 매 그림 본다(App이 file_drag로 한 번만 받는다).
pub(super) fn drag_begins(response: &egui::Response, button: egui::PointerButton, skin: bool) -> bool {
    if !skin {
        return response.drag_started_by(button);
    }
    if !response.dragged_by(button) {
        return false;
    }
    let (origin, pos) = response.ctx.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos()));
    let (Some(origin), Some(pos)) = (origin, pos) else { return false };
    !selection_area(response.rect).contains(origin) && ((pos.x - origin.x).abs() > 8.0 || (pos.y - origin.y).abs() > 8.0)
}

/// 왼쪽 단추 누름 기록(windows_double_press): 이 그림 번호, 지난 누름(시각, 자리), 지난 누름이 두 번째 누름이었는가.
#[derive(Clone, Copy, Default)]
struct PressTrack {
    frame: u64,
    last: Option<(f64, egui::Pos2)>,
    last_double: bool,
}

/// 지난(또는 이번 그림의) 왼쪽 단추 누름이 Windows 기준의 두 번째 누름(WM_LBUTTONDBLCLK)인가. 원본 컨트롤은 Windows가 정하는 대로
/// 받는다: 앞 누름에서 GetDoubleClickTime 안이고, 앞 누름 자리를 가운데로 한 SM_CXDOUBLECLK × SM_CYDOUBLECLK 사각 안이면 두 번째
/// 누름이고, 그다음 누름은 다시 보통 누름이다. egui는 시간만 보므로(다른 줄을 빨리 두 번 눌러도 두 번 누르기) 이것을 함께 본다.
/// 그림마다 한 번 셈한다(App::update 첫머리와 목록 줄이 부른다).
pub(super) fn windows_double_press(ctx: &egui::Context) -> bool {
    let id = egui::Id::new("ndr-windows-double-press");
    let frame = ctx.cumulative_frame_nr() + 1;
    let (press, time, scale) = ctx.input(|i| {
        let press = i.events.iter().rev().find_map(|e| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some(*pos),
            _ => None,
        });
        (press, i.time, i.pixels_per_point)
    });
    ctx.data_mut(|d| {
        let mut track = d.get_temp::<PressTrack>(id).unwrap_or_default();
        if track.frame != frame {
            track.frame = frame;
            if let Some(pos) = press {
                let (ms, cx, cy) = native::double_click_metrics();
                let (half_x, half_y) = (cx as f32 / scale / 2.0, cy as f32 / scale / 2.0);
                let double = !track.last_double
                    && track.last.is_some_and(|(at, from)| (time - at) * 1000.0 <= ms as f64 && (pos.x - from.x).abs() <= half_x && (pos.y - from.y).abs() <= half_y);
                track.last = Some((time, pos));
                track.last_double = double;
            }
            d.insert_temp(id, track);
        }
        track.last_double
    })
}

/// Windows 기준(windows_double_press)의 두 번째 누름으로 된 누르기. egui double_clicked는 쓰지 않는다: egui는 2 × 간격 안의
/// 세 번째 누르기를 세 번 누르기로 돌려 두 번 누르기를 숨기는데(한 항목을 누르고 1초 안에 다른 항목을 두 번 누르면 열리지 않음),
/// Windows에는 세 번 누르기가 없다(두 번째 누름 다음은 다시 보통 누름).
pub(super) fn windows_double_clicked(response: &egui::Response) -> bool {
    let double = windows_double_press(&response.ctx);
    response.clicked() && double
}

pub(super) fn original_press_input(pane: &mut Pane, row: usize, rect: egui::Rect, ui: &egui::Ui) -> bool {
    original_press(pane, row, rect, ui, false)
}

/// original_press_input의 미리 보기(형식 3)판: 원본 MouseDown은 형식 3이면 표시 칸(GetSelAreaAt)을 보지 않고,
/// MouseMove도 형식 3이면 끌어 고르지 않는다.
pub(super) fn original_thumbnail_press(pane: &mut Pane, row: usize, rect: egui::Rect, ui: &egui::Ui) -> bool {
    original_press(pane, row, rect, ui, true)
}

fn original_press(pane: &mut Pane, row: usize, rect: egui::Rect, ui: &egui::Ui, thumbnails: bool) -> bool {
    let (pressed, down, pos, moved, modifiers) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.interact_pos(), i.pointer.delta() != egui::Vec2::ZERO, i.modifiers));
    let double_press = windows_double_press(ui.ctx());
    let Some(pos) = pos.filter(|p| ui.clip_rect().contains(*p)) else { return false };
    // 고친 것: 날 입력을 읽으므로, 위에 다른 층(대화 상자·메시지 상자·메뉴)이 있으면 그 누름은 목록 것이 아니다. 전에는 압축하기 창의
    // 확인 단추가 다른 패널 줄 위에 있으면 그 패널이 지금 패널이 되어 압축 파일이 다른 패널 폴더에 생겼다.
    if ui.ctx().layer_id_at(pos).is_some_and(|layer| layer != ui.layer_id()) {
        return false;
    }
    // 원본 MouseDown(0x7d63c8): 표시를 바꾸는 갈래만 왼쪽 단추(ssLeft 8)이고 SetItemFocused·찾는 글 지우기는 어느 단추든 한다.
    // 고친 것: 전에는 가운데 단추로 누르면 아무 일도 없었다(오른쪽 단추는 따로 처리).
    if ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Middle)) && rect.contains(pos) {
        pane.item_pressed = true;
        pane.cursor = row;
        return true;
    }
    if pressed && rect.contains(pos) {
        // 원본 MouseDown(0x7d63c8)은 두 번 누르기의 두 번째 누름(ssDouble, WM_LBUTTONDBLCLK)이면 표시·커서를 건드리지 않는다.
        // Windows 기준(windows_double_press) 안의 두 번째 누름이 그것이다. 세 번째 누름은 다시 보통 누름.
        if double_press {
            return true;
        }
        pane.item_pressed = true;
        let before = pane.cursor.min(pane.entries.len().saturating_sub(1));
        if modifiers.shift {
            for j in before.min(row)..=before.max(row) {
                pane.selected.insert(j);
            }
            pane.range_drag = Some((before, row));
        } else if modifiers.ctrl {
            if pane.selected.is_empty() && before != row && before < pane.entries.len() {
                pane.selected.insert(before);
            }
            if !pane.selected.insert(row) {
                pane.selected.remove(&row);
            }
        }
        pane.cursor = row;
        pane.anchor = row;
        if thumbnails {
            pane.range_drag = None;
        } else if selection_area(rect).contains(pos) {
            if !pane.selected.insert(row) {
                pane.selected.remove(&row);
            }
            pane.range_drag = Some((row, row));
        }
        return true;
    }
    if thumbnails {
        return false;
    }
    if let (true, true, true, Some((anchor, last))) = (down, moved, rect.contains(pos), pane.range_drag) {
        if last != anchor {
            if modifiers.ctrl {
                for j in anchor.min(last)..=anchor.max(last) {
                    pane.selected.remove(&j);
                }
            } else {
                pane.selected.clear();
            }
        }
        for j in anchor.min(row)..=anchor.max(row) {
            pane.selected.insert(j);
        }
        pane.range_drag = Some((anchor, row));
    }
    false
}

/// 목록 줄을 눌렀을 때의 표시와 커서.
/// 스킨 테마는 원본 TNXCustomFileListView.MouseDown(0x7d63c8): 그냥 누르기와 오른쪽 단추는 커서만 옮기고 표시는 그대로,
/// Shift(+Ctrl)는 지금 커서부터 누른 줄까지 더해 고르고, Ctrl은 누른 줄을 뒤집는다(고른 줄이 없으면 전 커서 줄도 고른다).
/// C# 테마는 C# 판(WPF 목록): 그냥 누르기는 표시를 지우고, Shift는 기준 줄부터, 오른쪽 단추는 고르지 않은 줄이면 표시를 지운다.
/// 목록 줄을 오른쪽 단추로 눌렀을 때 띄울 것(원본 FUN_00484e8c).
pub(super) enum ListMenu {
    /// 띄우지 않는다(압축·FTP 패널, 커서 항목 없음).
    None,
    /// 원본 pmMain(`list_context_menu`).
    Main,
    /// 원본 FUN_00486104: 탐색기(셸) 메뉴.
    Shell(Vec<PathBuf>),
}

impl App {
    /// 원본 목록 OnContextPopup(FUN_00484e8c): 로컬 패널(FUN_00481948 = 0)에서만 띄운다. "오른쪽버튼 클릭: 탐색기 메뉴 사용하기"
    /// (chkExplorerMenu, 설정 +0xd1)가 켜져 있거나, 표시가 없고 커서 항목이 드라이브(종류 4)면 탐색기 메뉴(FUN_00486104:
    /// 드라이브면 그 드라이브, 아니면 표시한 항목 또는 커서 항목), 그 밖은 pmMain이다.
    pub(super) fn list_menu_kind(&self, index: usize) -> ListMenu {
        let pane = &self.panes[index];
        if pane.archive.is_some() || pane.remote.is_some() || pane.loading {
            return ListMenu::None;
        }
        let Some(focused) = pane.entries.get(pane.cursor) else { return ListMenu::None };
        if pane.selected.is_empty() && focused.drive {
            return ListMenu::Shell(vec![focused.path.clone()]);
        }
        if self.settings.flag("UseExplorerMenu", false) {
            let targets = pane.targets();
            return if targets.is_empty() { ListMenu::None } else { ListMenu::Shell(targets) };
        }
        ListMenu::Main
    }

    /// 원본 목록 OnMouseDown(FUN_00485078): 오른쪽 단추를 누른 채(Shift 상태 ssRight) 왼쪽 단추를 누르면, "탐색기 메뉴 사용하기"가
    /// 꺼져 있거나 표시가 없고 커서 항목이 드라이브면 탐색기(셸) 메뉴(FUN_00486104: 로컬 패널만, 드라이브면 그 드라이브, 아니면
    /// 표시한 항목 또는 커서 항목), 켜져 있으면 pmMain을 누른 자리에 띄운다(오른쪽 단추 메뉴와 반대).
    pub(super) fn rocker_menu_kind(&self, index: usize) -> ListMenu {
        let pane = &self.panes[index];
        let Some(focused) = pane.entries.get(pane.cursor) else { return ListMenu::None };
        let drive_cursor = pane.selected.is_empty() && focused.drive;
        if !self.settings.flag("UseExplorerMenu", false) || drive_cursor {
            if pane.archive.is_some() || pane.remote.is_some() || pane.loading {
                return ListMenu::None;
            }
            if drive_cursor {
                return ListMenu::Shell(vec![focused.path.clone()]);
            }
            let targets = pane.targets();
            return if targets.is_empty() { ListMenu::None } else { ListMenu::Shell(targets) };
        }
        ListMenu::Main
    }
}

pub(super) fn row_pressed(pane: &mut Pane, row: usize, modifiers: egui::Modifiers, secondary: bool, original: bool) {
    if original {
        if !secondary {
            if modifiers.shift {
                let from = pane.cursor.min(pane.entries.len().saturating_sub(1));
                for j in from.min(row)..=from.max(row) {
                    pane.selected.insert(j);
                }
            } else if modifiers.ctrl {
                if pane.selected.is_empty() && pane.cursor != row && pane.cursor < pane.entries.len() {
                    pane.selected.insert(pane.cursor);
                }
                if !pane.selected.insert(row) {
                    pane.selected.remove(&row);
                }
            }
        }
        pane.anchor = row;
    } else if secondary {
        if !pane.selected.contains(&row) {
            pane.selected.clear();
        }
    } else if modifiers.shift {
        for j in pane.anchor.min(row)..=pane.anchor.max(row) {
            pane.selected.insert(j);
        }
    } else if modifiers.ctrl {
        if !pane.selected.insert(row) {
            pane.selected.remove(&row);
        }
    } else {
        pane.selected.clear();
        pane.anchor = row;
    }
    pane.cursor = row;
}

impl App {
    pub(super) fn refresh(&mut self, index: usize) {
        // C# RefreshPaneAsync: 그 패널의 빠른 찾기 입력을 버린다.
        self.typed_search[index] = TypedSearch::default();
        // 원본은 자기 작업 동안 TNXShellChangeNotify를 끄고(SetActive(0) = FreeThread) 작업 뒤 다시 읽은 다음 다시 켠다(SetActive(1) =
        // ResetThread, 새 FindFirstChangeNotificationW). 그래서 자기 작업이 낸 바뀜은 1.5초 뒤 한 번 더 다시 읽게 하지 않는다.
        // NDR은 다시 읽을 때마다 감시를 새로 연다: 옛 감시의 알림은 버리고(folder_changed), 밀린 바뀜도 버린다(원본 Clear).
        // 고친 것: 전에는 새 폴더·이름 바꾸기 뒤 1.5초 지나 같은 폴더를 한 번 더 읽었다.
        self.panes[index].change_due = None;
        if let Some(wanted) = self.watch_wanted(index) {
            let ctx = self.egui_ctx.clone();
            self.arm_folder_watch(index, Some(wanted), &ctx);
        }
        // 패널 폴더 트리: 그 폴더의 하위 폴더 목록도 다시 읽는다(새로 만들거나 지운 폴더가 트리에 보이게).
        let current = self.panes[index].path.clone();
        self.tree_cache.retain(|(key, _), _| !ndr::shell_tree::same_key(key, &current));
        self.tree_requests.retain(|(key, _)| !ndr::shell_tree::same_key(key, &current));
        // 원본 새로 고침은 표시를 이름으로 되돌리므로(apply_listing) 새 목록이 올 때까지 표시를 둔다.
        let pane = &mut self.panes[index];
        pane.generation += 1;
        pane.loading = true;
        let generation = pane.generation;
        let path = pane.path.clone();
        let show_hidden = pane.show_hidden;
        let archive = pane.archive.clone();
        let remote = pane.remote.clone();
        let tx = self.listing_tx.clone();
        let include_legacy = self.settings.flag("UseDescriptionFile", true);
        let local = archive.is_none() && remote.is_none();
        // C# ListLocal("::drives::"): 준비된 드라이브마다 폴더 항목(이름 = "C:\"). ShowDriveList가 꺼져 있으면 비운다.
        let show_drive_list = self.settings.flag("ShowDriveList", true);
        let drives_view = local && is_drives(&path);
        // 원본 FUN_00486474: "목록에 디스크 드라이브 목록 보기"(ini showdrivelist, 처음 값 1 = 0x46eed9)가 켜져 있으면
        // 일반 폴더 목록 끝에 TNXDriveList의 드라이브마다 항목(속성 0x40000 → 종류 4)을 덧붙인다. 읽기는 목록 스레드에서 한다.
        let append_drives = local && !drives_view && show_drive_list;
        let ready_drives = self.system.ready_drives;
        let drive_item = self.system.drive_item;
        let drive_list = self.drive_list.clone();
        // 시험은 스레드마다 가짜 드라이브를 두므로 캐시를 거치지 않는다(캐시 자체는 cached_drive_items 시험).
        let drive_items = move |cache: &std::sync::Arc<std::sync::Mutex<DriveListCache>>| {
            if cfg!(test) { ready_drives().into_iter().map(|root| { let info = drive_item(&root); (root, info) }).collect() } else { cached_drive_items(cache, ready_drives, drive_item) }
        };
        // 내 PC 보기의 드라이브는 전처럼 부른 스레드에서 읽는다(캐시가 있으면 바로). 시험은 가짜 드라이브가 스레드마다 있으므로
        // 덧붙이는 드라이브도 부른 스레드에서 읽는다.
        let this_pc: Option<Vec<(PathBuf, Option<DriveInfo>)>> = (drives_view || (cfg!(test) && append_drives)).then(|| if show_drive_list { drive_items(&drive_list) } else { Vec::new() });
        thread::spawn(move || {
            // 원본 TNXDriveList는 시작할 때 한 번 만들어 들고 있다(목록마다 드라이브를 다시 묻지 않음). NDR은 패널 목록마다
            // 드라이브를 읽던 것을 캐시로 바꿨다(5초 안이면 그대로, 지나면 그대로 쓰고 뒤에서 새로 읽음). 끊긴 네트워크 드라이브처럼
            // 볼륨 정보가 늦게 오는 드라이브가 있어도 폴더를 옮길 때마다 기다리지 않는다.
            let listed = match this_pc {
                Some(items) => items,
                None if append_drives => drive_items(&drive_list),
                None => Vec::new(),
            };
            let roots: Vec<PathBuf> = listed.iter().map(|(root, _)| root.clone()).collect();
            let drives = drives_view.then(|| roots.clone());
            let appended: Vec<PathBuf> = if append_drives { roots } else { Vec::new() };
            // 원본 내 PC 드라이브 항목의 TNXDriveInfo(이름, 종류, 고정 디스크면 크기).
            let drive_infos = listed.into_iter().filter_map(|(root, info)| info.map(|info| (root, info))).collect();
            let result = if let Some(remote) = remote {
                super::ftp_pane::list_remote(&remote, show_hidden)
            } else if let Some(drives) = drives {
                Ok(drives
                    .into_iter()
                    .map(|root| Entry { name: root.to_string_lossy().into_owned(), path: root, is_dir: true, size: 0, modified: None, hidden: false, attributes: 0x10, drive: false })
                    .collect())
            } else if let Some(location) = archive {
                ndr::archive::list(
                    &location.file,
                    if location.password.is_empty() {
                        None
                    } else {
                        Some(&location.password)
                    },
                )
                .map(|items| {
                    let mut entries = std::collections::BTreeMap::new();
                    for e in items {
                        // 고친 것: 7-Zip은 Windows에서 "dir\sub\c.txt"처럼 \로 알려서, 압축 안 하위 폴더("dir/")에 들어가면 비어 보였다.
                        let key = e.key.replace('\\', "/");
                        if let Some(rest) = key.strip_prefix(&location.prefix) {
                            let mut pieces = rest.split('/');
                            if let Some(name) = pieces.next() {
                                if name.is_empty() {
                                    continue;
                                }
                                let folder = pieces.next().is_some() || e.is_dir;
                                // 그 항목 자신이면 그 시각. 아래 항목에서 나온 폴더는 폴더 항목이 나올 때 채운다.
                                let slot = entries.entry(name.to_owned()).or_insert(Entry {
                                    path: PathBuf::from(format!("{}{name}", location.prefix)),
                                    name: name.to_owned(),
                                    is_dir: folder,
                                    size: if folder { 0 } else { e.size },
                                    modified: None,
                                    hidden: false,
                                    attributes: if folder { 0x10 } else { 0 },
                                    drive: false,
                                });
                                if rest.trim_end_matches('/') == name {
                                    slot.modified = e.modified;
                                }
                            }
                        }
                    }
                    entries.into_values().collect()
                })
            } else {
                fsops::list_dir(&path, show_hidden).map(|mut entries| {
                    // 드라이브 항목 이름은 원본 DrawListItem처럼 "%s:"(글자 + ':').
                    entries.extend(appended.iter().map(|root| Entry {
                        name: root.to_string_lossy().trim_end_matches('\\').to_owned(),
                        path: root.clone(),
                        is_dir: true,
                        size: 0,
                        modified: None,
                        hidden: false,
                        attributes: 0x10,
                        drive: true,
                    }));
                    entries
                })
            };
            // C# ListLocal: 설명은 목록과 함께 읽는다(descript.dat는 늘, descript.ion은 UseDescriptionFile일 때).
            let descriptions = if local && result.is_ok() && !is_drives(&path) {
                compat::read_descriptions(&path, include_legacy).map(|map| map.into_iter().map(|(k, v)| (k.to_lowercase(), v)).collect()).unwrap_or_default()
            } else {
                HashMap::new()
            };
            let _ = tx.send(Listing {
                pane: index,
                generation,
                result,
                descriptions,
                drive_infos,
            });
        });
        // 원본 GetList(0x45a34c)는 목록을 그 자리에서 읽는다(읽는 동안 화면은 그대로, 다 읽으면 새 목록). 스킨 테마는 로컬·압축 목록을
        // 3초까지 기다려 "읽는 중" 그림 없이 바꾼다. 더 오래 걸리면(네트워크 등) 전처럼 따로 읽고 다 되면 바꾼다. FTP는 기다리지 않는다.
        if self.panes[index].remote.is_none() {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut auth = Vec::new();
            while self.panes[index].loading && self.panes[index].generation == generation {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                match self.listing_rx.recv_timeout(left) {
                    Ok(listing) => self.apply_listing(listing, &mut auth),
                    Err(_) => break,
                }
            }
            self.auth_pending.extend(auth);
        }
    }
    pub(super) fn navigate(&mut self, index: usize, path: PathBuf, remember: bool) {
        // 로컬 폴더로 가면 패널 FTP 모드는 끝난다(원본 FUN_0048d6e4: 끊은 뒤 로컬 폴더로).
        if let Some(remote) = self.panes[index].remote.as_ref() {
            if remote.busy.load(Ordering::Relaxed) {
                self.status = self.language_pack.text("RES.FTP_CANNOTDISCONNECT", "Cannot be disconnected while any operation is working.");
                return;
            }
            let active = self.active;
            self.active = index;
            self.ftp_disconnect();
            self.active = active;
        }
        let pane = &mut self.panes[index];
        // 같은 위치면 건너뛴다. 고친 것: 전에는 경로만 비교해, 압축 파일 맨 위에서 ".."로 나오면(압축은 지웠지만 패널 경로는 그
        // 압축 파일이 있는 폴더 그대로라) 다시 읽지 않아 압축 안 목록이 남았고, 그 항목을 로컬 파일로 다뤄 복사도 되지 않았다.
        // 지금 보이는 목록(listed)이 바로 그 로컬 폴더일 때만 건너뛴다.
        let same_listing = pane.archive.is_none() && pane.remote.is_none() && pane.listed.as_deref() == Some(format!("dir|{}", path.display()).to_lowercase().as_str());
        if pane.path == path && !pane.loading && same_listing {
            return;
        }
        // 원본 위치 바뀌기 전 사건(위치 +0x38 = FUN_0048631c): 그 패널의 알림 창(TfMessage, +0x30c)이 떠 있으면 닫고 표시를 지운다
        // (표시는 새 목록에서 지운다).
        if self.pane_message.as_ref().is_some_and(|(_, pane, _)| *pane == index) {
            self.pane_message = None;
        }
        let pane = &mut self.panes[index];
        if remember {
            pane.back.push(pane.path.clone());
            pane.forward.clear();
            // 원본 기록 목록 용량 20(FUN_00480d58 → FUN_0045806c(…, 0x14)): 꽉 차면 가장 오래된 것을 지운다(FUN_00458348).
            if pane.back.len() > 19 {
                let extra = pane.back.len() - 19;
                pane.back.drain(..extra);
            }
        }
        // 고친 것(사용자 요청): 뒤로·주소 줄 등으로 지금 폴더의 상위 폴더로 가면 떠나온 하위 폴더에 커서를 둔다(그 쪽이 보인다).
        // 원본 FUN_00486694는 ".."(1)·"\"(2)로 갈 때만 그렇게 하고, 뒤로(FUN_0048d58c)는 첫 항목으로 돌아갔다.
        if pane.archive.is_none() && self.pending_focus[index].is_none() {
            if let Ok(rest) = pane.path.strip_prefix(&path) {
                self.pending_focus[index] = rest.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned());
            }
        }
        let pane = &mut self.panes[index];
        pane.path = path.clone();
        pane.address = path.to_string_lossy().into_owned();
        let local = pane.archive.is_none();
        if local {
            pane.tabs[pane.tab] = path.clone();
            // C# NavigateLocalAsync: RememberDriveFolder.
            self.remember_drive_folder(&path);
        }
        self.refresh(index);
    }
    pub(super) fn parent(&mut self) {
        let i = self.active;
        if self.panes[i].remote.is_some() {
            return self.ftp_parent(i);
        }
        // 원본 FUN_00443f14 0x32 · FUN_00486694(올라감 = 1): 상위로 가면 떠나온 폴더(압축 밖으로 나오면 그 압축 파일)에 커서를 둔다.
        if let Some(location) = self.panes[i].archive.as_mut() {
            if location.prefix.is_empty() {
                let file = location.file.clone();
                self.panes[i].archive = None;
                self.pending_focus[i] = file.file_name().map(|n| n.to_string_lossy().into_owned());
                if let Some(parent) = file.parent() {
                    self.navigate(i, parent.to_path_buf(), false);
                } else {
                    self.refresh(i);
                }
            } else {
                let trimmed = location.prefix.trim_end_matches('/').to_owned();
                let (up, child) = trimmed.rsplit_once('/').map_or((String::new(), trimmed.clone()), |(p, c)| (format!("{p}/"), c.to_owned()));
                location.prefix = up;
                self.pending_focus[i] = Some(child);
                self.refresh(i);
            }
            return;
        }
        let current = self.panes[i].path.clone();
        if current.parent().is_some() && !is_drives(&current) {
            self.pending_focus[i] = current.file_name().map(|n| n.to_string_lossy().into_owned());
        }
        // 원본 FUN_00459ba8: 루트면(IsRoot) 아무것도 하지 않는다.
        if current.parent().is_none() || is_drives(&current) {
            return;
        }
        let Some(parent) = current.parent().map(Path::to_path_buf) else { return };
        {
            self.navigate(
                i,
                if parent.as_os_str().is_empty() {
                    PathBuf::from("/")
                } else {
                    parent
                },
                true,
            );
        }
    }
    pub(super) fn open_current(&mut self) {
        let i = self.active;
        let Some(entry) = self.panes[i].entries.get(self.panes[i].cursor).cloned() else {
            return;
        };
        // 원본 ".." 항목: 상위 위치로(로컬·압축·FTP).
        if entry.is_parent() {
            return self.parent();
        }
        if self.panes[i].remote.is_some() {
            // 패널 FTP 모드: 폴더면 그 안으로. 파일은 내려받기(F5/miFTPGet)로만 다룬다.
            if entry.is_dir {
                self.ftp_open_dir(i, &entry.name);
            }
            return;
        }
        if entry.is_dir {
            if let Some(location) = self.panes[i].archive.as_mut() {
                location.prefix = format!("{}/", entry.path.to_string_lossy().replace('\\', "/"));
                self.refresh(i);
            } else {
                self.navigate(i, entry.path, true);
            }
            return;
        }
        if self.panes[i].archive.is_some() {
            // C# OpenItemAsync: 압축 항목은 임시 폴더에 풀어 열고, 풀린 것이 압축 파일이면 그 안으로 들어간다.
            self.open_archive_entry(i, &entry);
            return;
        }
        // C# OpenItemAsync: IsArchive(형식 표) → GetArchiveFirstVolumePath로 첫 볼륨을 연다.
        // 동봉 7-Zip이 못 읽는 형식(.ace)은 C# TryGetExternalArchiveProgram처럼 외부 프로그램으로 풀고 그 폴더로.
        if ndr::archive::archive_format(&entry.path).is_some() && !ndr::archive::bundled_can_browse(&entry.path) {
            self.open_with_external_archive_program(i, ndr::archive::first_volume_path(&entry.path), true);
        } else if ndr::archive::archive_format(&entry.path).is_some() {
            self.panes[i].archive = Some(ArchiveLocation {
                file: ndr::archive::first_volume_path(&entry.path),
                prefix: String::new(),
                password: String::new(),
            });
            self.refresh(i);
        } else if self.settings.flag("PlayWaveInProgram", true) && entry.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")) {
            // 원본 0x483c79(playwav, 처음 값 켜짐 0x46f088): 확장자가 wav면 PlaySoundW(NULL)로 멈춘 뒤 PlaySoundW(파일, 0x22003). 셸로 열지 않는다.
            (self.system.play_wave)(None);
            (self.system.play_wave)(Some(&entry.path));
        } else if let Some(folder) = entry.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")).then(|| (self.system.shortcut_target)(&entry.path)).flatten().filter(|(_, folder)| *folder).map(|(target, _)| target) {
            // 원본 FUN_00483640(파일 열기): 바로 가기(.lnk)의 대상(GetShortcutTarget)이 폴더면 GetLongPathNameW 뒤 그 패널을 그 폴더로.
            // 고친 것: 전에는 셸로 열어 탐색기 창이 새로 떴다.
            let folder = std::path::absolute(&folder).unwrap_or(folder);
            self.navigate(i, native::long_path(&folder).unwrap_or(folder), true);
        } else {
            // C# OpenItemAsync: RememberOpenedExecutable(.exe/.lnk) 후 실행.
            let mut rows = self.settings.strings("ExecutableHistory");
            ndr::command_line::remember_opened_executable(&mut rows, &entry.path.to_string_lossy());
            self.settings.set_strings("ExecutableHistory", rows);
            if let Err(e) = native::shell_open(&entry.path, None, "open") {
                self.status = e;
            }
        }
    }
    /// 패널 설정 이름("Left…"/"Right…").
    pub(super) fn side_key(index: usize, name: &str) -> String {
        format!("{}{name}", if index == 0 { "Left" } else { "Right" })
    }
    /// 원본 TNXScrollStep(scrollstep1/2, 처음 값 2 = 0x46f20f): 1 = 줄 단위, 2 = 쪽 단위("페이지 단위 스크롤").
    pub(super) fn scroll_by_page(&self, index: usize) -> bool {
        self.settings.values.get(&Self::side_key(index, "ScrollStep")).and_then(|v| v.as_i64()).unwrap_or(2) != 1
    }
    /// 원본 WMMouseWheel(0x7d5ba0)·UpdateScrollBar(0x7db270): 쪽 단위면 한 칸에 보이는 줄 수만큼, 줄 단위면 Mouse.WheelScrollLines 줄.
    pub(super) fn wheel_step(by_page: bool, view_height: f32, row_height: f32, wheel_lines: u32) -> f32 {
        let rows = if by_page { (view_height / row_height).floor().max(1.0) } else { wheel_lines.max(1) as f32 };
        rows * row_height
    }
    /// 원본 WMMouseWheel(0x7d5ba0)은 WM_MOUSEWHEEL만 보고 Shift를 가리지 않는다(Shift + 바퀴도 같은 방향). 기울이는 바퀴(WM_MOUSEHWHEEL,
    /// WMMouseHWheel 0x7d5c94)는 받기만 하고 아무것도 옮기지 않는다. egui는 Shift + 바퀴를 가로(x)로 바꿔 주므로 Shift면 x가 바퀴다.
    pub(super) fn vertical_wheel(input: &egui::InputState) -> f32 {
        if input.modifiers.shift { input.raw_scroll_delta.x } else { input.raw_scroll_delta.y }
    }
    /// 목록(높이 height, 지금 커서 자리) 위에서 이번 틀에 굴린 바퀴를 원본 규칙으로 바꾼 새 세로 위치. 굴리지 않았으면 None.
    /// 목록 ScrollArea는 egui 바퀴를 끄고(WHEEL_OFF) 이 값을 vertical_scroll_offset으로 넣는다.
    pub(super) fn wheel_scroll(ui: &egui::Ui, salt: egui::Id, height: f32, row_height: f32, by_page: bool, wheel_lines: u32) -> Option<f32> {
        let area = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
        let (raw, zoom) = ui.input(|i| (Self::vertical_wheel(i), i.modifiers.ctrl || i.modifiers.command));
        if raw == 0.0 || zoom || !ui.rect_contains_pointer(area) {
            return None;
        }
        let notches = raw / ui.ctx().options(|o| o.input_options.line_scroll_speed).max(1.0);
        let current = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(salt)).map_or(0.0, |s| s.offset.y);
        Some((current - notches * Self::wheel_step(by_page, height, row_height, wheel_lines)).max(0.0))
    }
    /// 원본 SetScrollTop(형식 2, 쪽 단위 0xc5 == 2): 세로 위치 = 쪽 번호 × 한 쪽 줄 수 × 줄 높이(0..쪽 수 − 1).
    /// 바퀴로 옮긴 위치를 가장 가까운 쪽 경계로 맞춘다. 마지막 쪽도 쪽 경계에서 시작한다(아래가 비어 보일 수 있음).
    pub(super) fn page_snap(offset: f32, page_px: f32, pages: usize) -> f32 {
        if page_px <= 0.0 {
            return offset.max(0.0);
        }
        let page = (offset / page_px).round().max(0.0) as usize;
        page.min(pages.saturating_sub(1)) as f32 * page_px
    }
    pub(super) fn side_text(&self, index: usize, name: &str, default: &str) -> String {
        self.settings.text(&Self::side_key(index, name), default)
    }
    pub(super) fn side_flag(&self, index: usize, name: &str, default: bool) -> bool {
        self.settings.flag(&Self::side_key(index, name), default)
    }
    /// 확장자 색(ExtensionColors)과, 색상표를 쓸 때의 NexusFile 요소 색(NexusFileColorMapValues).
    pub(super) fn color_maps(&self) -> (Option<&serde_json::Map<String, serde_json::Value>>, Option<&serde_json::Map<String, serde_json::Value>>) {
        let extension = self.settings.values.get("ExtensionColors").and_then(|v| v.as_object());
        let nexus = self.settings.flag("UseNexusFileColorMap", false).then(|| self.settings.values.get("NexusFileColorMapValues").and_then(|v| v.as_object())).flatten();
        (extension, nexus)
    }
    /// 원본 SetLineSpace(+0x364, ini linespace 0~32): 줄과 줄 사이에 더하는 화소.
    pub(super) fn list_line_space(&self) -> f32 {
        self.settings.values.get("ListLineSpace").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 32) as f32
    }
    /// 원본 UpdateDisplayDetails·UpdateDisplayNames(+0x340): 항목 높이 = 목록 글꼴의 글 높이 + 2, 아이콘을 보이면(+0x385) 16 이상.
    pub(super) fn list_item_height(&self, ui: &egui::Ui) -> f32 {
        let text = ui.text_style_height(&egui::TextStyle::Body).ceil() + 2.0;
        if self.settings.flag("ShowIcons", true) { text.max(16.0) } else { text }
    }
    /// 원본 DrawListItem(+0x387): 항목 아래 (bottom + LineSpace / 2 − 1) 화소 줄에 NXLINE 가로선(정수 나눗셈, 굵기 1).
    /// 여기서는 그 화소 줄의 가운데(bottom + floor(LineSpace / 2) − 0.5)를 화면 화소 가운데에 맞춘다.
    /// 고친 것: 줄 위치가 화소 사이에 걸리면(목록 위치·화면 배율이 정수가 아닐 때) 1화소 선이 두 줄에 흐리게 번졌다.
    pub(super) fn row_line_y(painter: &egui::Painter, item_bottom: f32, line_space: f32) -> f32 {
        let y = item_bottom + (line_space / 2.0).floor() - 0.5;
        painter.round_to_pixel_center(y)
    }
    /// 원본 SetShowGridColumn(+0x386)·SetShowGridRow(+0x387) 선 색: GetBasic(13) NXLINE. 색상표를 쓰지 않으면 테마의 구분선 색.
    pub(super) fn grid_line_color(&self, ui: &egui::Ui) -> Color32 {
        let (_, nexus) = self.color_maps();
        nexus
            .and_then(|map| map.get("NXLINE"))
            .and_then(|v| v.as_str())
            .and_then(ndr::styles::parse_color)
            .map(|[r, g, b, _]| Color32::from_rgb(r, g, b))
            .unwrap_or(ui.visuals().widgets.noninteractive.bg_stroke.color)
    }
    /// C# SortPaneItems + ReorderPane: 패널 설정(정렬 방식·내림차순·폴더 정렬·논리 정렬·묶음)으로 다시 늘어놓고,
    /// 표시와 커서는 같은 항목에 둔다.
    pub(super) fn sort_pane(&mut self, index: usize) {
        let options = ndr::listing::SortOptions {
            folder_mode: self.side_text(index, "FolderSortMode", "Name"),
            folder_descending: self.side_flag(index, "FolderSortDescending", false),
            file_mode: self.side_text(index, "SortMode", "Name"),
            file_descending: self.side_flag(index, "SortDescending", false),
            logical: self.settings.flag("UseLogicalSort", true),
        };
        let group_mode = self.side_text(index, "GroupMode", "None");
        let category_mode = self.side_text(index, "FolderCategoryMode", "Auto");
        let group_descending = self.side_flag(index, "GroupDescending", false);
        let (extension_colors, nexus_colors) = self.color_maps();
        let (extension_colors, nexus_colors) = (extension_colors.cloned(), nexus_colors.cloned());
        let color = |entry: &Entry| ndr::listing::color_sort_key(entry, extension_colors.as_ref(), nexus_colors.as_ref());
        let pane = &mut self.panes[index];
        let cursor = pane.entries.get(pane.cursor).map(|e| e.path.clone());
        let marked: HashSet<PathBuf> = pane.selected.iter().filter_map(|i| pane.entries.get(*i).map(|e| e.path.clone())).collect();
        // 원본 ".." 항목은 정렬과 상관없이 맨 앞이다.
        let (mut parents, rest): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut pane.entries).into_iter().partition(|e| e.is_parent());
        pane.entries = rest;
        ndr::listing::sort_entries(&mut pane.entries, &options, &color);
        ndr::listing::sort_groups(&mut pane.entries, &group_mode, &category_mode, group_descending);
        // 원본 드라이브 항목(종류 4)은 정렬과 상관없이 목록 끝에 TNXDriveList 차례(글자 순)로 있다.
        let (mut items, mut drives): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut pane.entries).into_iter().partition(|e| !e.drive);
        drives.sort_by(|a, b| a.path.cmp(&b.path));
        items.append(&mut drives);
        parents.append(&mut items);
        pane.entries = parents;
        pane.selected = pane.entries.iter().enumerate().filter(|(_, e)| marked.contains(&e.path)).map(|(i, _)| i).collect();
        if let Some(cursor) = cursor {
            pane.cursor = pane.entries.iter().position(|e| e.path == cursor).unwrap_or(0);
        }
        self.compute_view(index);
    }
    /// 원본 SetSortType·SetSortDescend(0x7dbb28·0x7dba3c): 정렬을 바꾸면 목록의 FileListChanged → AdjustPage(옛 커서 번호)가 돌고,
    /// 그 뒤 커서 이름의 새 자리로 SetItemIndex(보이게)한다. 고친 것: 전에는 커서 항목이 정렬로 옮겨 가도 화면이 그대로라 안 보일 수 있었다.
    pub(super) fn resort_pane(&mut self, index: usize) {
        let old = self.panes[index].cursor;
        self.sort_pane(index);
        let pane = &mut self.panes[index];
        if !pane.entries.is_empty() {
            pane.adjust = Some((old.min(pane.entries.len() - 1), false));
            pane.reveal = true;
        }
    }
    pub(super) fn sort_command(&mut self, command: &str) -> bool {
        let i = self.active;
        let set = |app: &mut App, name: &str, value: &str| app.settings.set_text(&Self::side_key(i, name), value);
        let toggle = |app: &mut App, name: &str| {
            let key = Self::side_key(i, name);
            let old = app.settings.flag(&key, name == "ShowGroupName");
            app.settings.set_flag(&key, !old);
        };
        let sort_modes = ["None", "Name", "Extension", "Size", "Modified", "Attributes", "Color"];
        if let Some(mode) = command.strip_prefix("sort:") {
            if !sort_modes.contains(&mode) {
                return true;
            }
            set(self, "SortMode", mode);
        } else if let Some(mode) = command.strip_prefix("folderSort:") {
            if mode == "None" || !sort_modes.contains(&mode) {
                return true;
            }
            set(self, "FolderSortMode", mode);
        } else if let Some(mode) = command.strip_prefix("groupMode:") {
            if !["None", "Extension", "Size", "Time", "Category"].contains(&mode) {
                return true;
            }
            set(self, "GroupMode", mode);
        } else if let Some(mode) = command.strip_prefix("folderCategory:") {
            if !["Auto", "Default"].contains(&mode) {
                return true;
            }
            set(self, "FolderCategoryMode", mode);
        } else {
            match command {
                "sortDescending" => toggle(self, "SortDescending"),
                "folderSortDescending" => toggle(self, "FolderSortDescending"),
                "groupDescending" => toggle(self, "GroupDescending"),
                "groupShowName" => toggle(self, "ShowGroupName"),
                "sortNone" | "sortName" | "sortExtension" | "sortSize" | "sortTime" | "sortAttributes" | "sortColor" => {
                    let mode = match command {
                        "sortNone" => "None",
                        "sortExtension" => "Extension",
                        "sortSize" => "Size",
                        "sortTime" => "Modified",
                        "sortAttributes" => "Attributes",
                        "sortColor" => "Color",
                        _ => "Name",
                    };
                    // 원본 miSortByClick(0x431c88): 지금 정렬 종류를 다시 고르면(없음 제외) 내림차순을 뒤집는다(스킨 테마).
                    let current = self.side_text(i, "SortMode", "Name");
                    if mode != "None" && current == mode {
                        toggle(self, "SortDescending");
                    } else {
                        set(self, "SortMode", mode);
                    }
                }
                _ => return false,
            }
        }
        self.resort_pane(i);
        true
    }
    pub(super) fn pane_ui(&mut self, ui: &mut egui::Ui, index: usize, ctx: &egui::Context) {
        // 원본 패널(TNXFramePanel): 패널 안 어디를 눌러도(빈 곳·주소 줄·정보 표시줄 포함) 그 패널이 지금 패널이 된다.
        // 메뉴·대화 상자 같은 위 층을 누른 것은 빼고, 자리는 이번 그림의 패널 칸(ui.max_rect)으로 잰다.
        // 고친 것: 지난 그림의 pane_rects로 쟀는데, update가 패널을 그리기 전에 그것을 비워 빈 곳을 눌러도 패널이 바뀌지 않았다.
        let area = ui.max_rect();
        if let Some(pos) = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None }) {
            let on_background = ctx.layer_id_at(pos).is_none_or(|layer| layer.order == egui::Order::Background);
            if on_background && area.contains(pos) && self.original_dialogs.is_empty() && self.msgboxes.is_empty() {
                self.active = index;
            }
        }
        let active = self.active == index;
        // 원본 패널: 테두리 lframe*, 주소 줄(TNXPathLabel)은 목록 위(원본 FUN_00481978). 스킨을 읽지 못하면 그리지 않는다.
        let Some(skin_now) = self.current_skin() else { return };
        let skin = Some(skin_now);
        // 원본 MouseUp(0x7d6740): 왼쪽 단추를 놓으면 표시 칸 끌기를 끝낸다.
        if !ctx.input(|i| i.pointer.primary_down()) {
            self.panes[index].range_drag = None;
        }
        let stroke = match &skin {
            Some(skin) => self.skin_frame_stroke(skin, index),
            None => egui::Stroke::NONE,
        };
        // 원본 패널 틀: 테두리(lframewidth)만 있고 둥근 모서리·안쪽 여백이 없다.
        let frame = egui::Frame::new().inner_margin(egui::Margin::same(stroke.width.ceil() as i8));
        // 원본 탭 줄(TNXTabSet)은 틀 패널(+0x330) 밖, 패널 바로 위에 붙는다(FUN_00481fb8 SetParent(패널)).
        let outer_spacing = ui.spacing().item_spacing.y;
        if let Some(skin) = &skin {
            ui.spacing_mut().item_spacing.y = 0.0;
            self.skin_tab_set(ui, index, ctx, skin);
        }
        let shown = frame
            .stroke(stroke)
            .show(ui, |ui| {
                // 원본 틀 안 컨트롤(주소 줄·정보 줄·목록·파일 정보줄)은 Align으로 틈 없이 붙는다. 고친 것: 전에는 egui 기본 틈(3화소)이
                // 폴더 정보줄과 목록 사이, 목록과 파일 정보줄 사이에 끼었다.
                let _ = outer_spacing;
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut activate = false;
                let mut open = false;
                let mut command = None;
                let mut begin_drag = None;
                let mut folder_rows: Vec<(usize, egui::Rect, PathBuf)> = Vec::new();
                // 원본 틀 패널 안: 위에서부터 드라이브 단추 줄(켰을 때), 주소 줄, 폴더 정보 표시줄, 목록(사이 틈 없음).
                if let Some(skin) = &skin {
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        self.skin_drive_bar(ui, index, ctx, skin);
                        self.skin_address_bar(ui, index, ctx, skin);
                        self.skin_folder_info_bar(ui, index, skin);
                    });
                }
                // 원본 FUN_0048da38(miShowFoldersClick 0x445258, Alt+F3): 패널마다 목록 왼쪽(alLeft, 위 주소 줄·폴더 정보줄과 아래 파일 정보줄
                // 사이)에 폴더 트리(너비 = 설정 +0x48 + 패널 × 4, ini treewidth1/2), 그 오른쪽에 3화소 나눔 막대(MinSize 50, 끌어 놓으면
                // FUN_0048e2c0이 너비를 남긴다). 목록은 그 오른쪽 나머지.
                let outer: &mut egui::Ui = ui;
                let full = outer.available_rect_before_wrap();
                let tree_shown = self.settings.flag(if index == 0 { "ShowLeftFolderTree" } else { "ShowRightFolderTree" }, false);
                let tree_key = if index == 0 { "LeftFolderTreeWidth" } else { "RightFolderTreeWidth" };
                let tree_width = if tree_shown { (self.settings.values.get(tree_key).and_then(|v| v.as_f64()).unwrap_or(120.0) as f32).clamp(50.0, (full.width() - 53.0).max(50.0)) } else { 0.0 };
                let splitter = if tree_shown { 3.0 } else { 0.0 };
                if tree_shown {
                    let spacing = outer.spacing().item_spacing.y;
                    let below = self.skin_file_info_height() + spacing + self.ftp_log_reserve(index, spacing);
                    let height = (full.height() - below).max(80.0);
                    let tree_rect = egui::Rect::from_min_size(full.min, egui::vec2(tree_width, height));
                    self.pane_tree_ui(outer, index, tree_rect);
                    let split = egui::Rect::from_min_size(egui::pos2(tree_rect.right(), full.top()), egui::vec2(splitter, height));
                    let drag = outer.interact(split, egui::Id::new(("ndr-pane-tree-split", index)), egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                    if drag.dragged() {
                        if let Some(pointer) = drag.interact_pointer_pos() {
                            let width = (pointer.x - full.left()).clamp(50.0, (full.width() - 53.0).max(50.0)).round();
                            self.settings.values.insert(tree_key.into(), serde_json::json!(width as f64));
                        }
                    }
                    if drag.drag_stopped() {
                        self.save();
                    }
                }
                let mut list_ui = outer.new_child(egui::UiBuilder::new().id_salt("ndr-pane-list").max_rect(egui::Rect::from_min_max(egui::pos2(full.left() + tree_width + splitter, full.top()), full.max)).layout(egui::Layout::top_down(egui::Align::Min)));
                let ui = &mut list_ui;
                // C# ConfigureGrid 열: 이름, 확장자(이름에 붙이지 않을 때), 종류(ShowFileType), 크기, 수정한 날짜, 속성, 설명.
                let columns = self.detail_columns(self.language == "ko");
                // 원본 처음 값 viewstyle1/2=0(자동 보기).
                let view = self.settings.text(if index == 0 { "LeftViewStyle" } else { "RightViewStyle" }, "Auto");
                let tiles = matches!(view.as_str(), "Thumbnails" | "viewThumbnails");
                let names = matches!(view.as_str(), "Names" | "viewNames");
                let grouped = self.side_text(index, "GroupMode", "None") != "None";
                // 원본 UpdateDisplayAuto: 열이 둘 이상이거나 한 열에 다 들어가지 않으면(가로 막대) 열로 흐르는 목록(머리글 없음),
                // 다 들어가는 한 열이면 자세히처럼 한 열. 묶음 보기(C# 판에만 있음)는 C# RebuildAutoColumns(열마다 따로 스크롤)를 그대로 쓴다.
                let (auto_columns, auto_bar) = if matches!(view.as_str(), "Auto" | "viewAuto") {
                    if grouped {
                        (self.auto_column_count(ui, index, ui.available_width()), false)
                    } else {
                        let (_, columns, bar) = self.flow_shape(ui, index, ui.available_width(), (ui.available_height() - 60.0).max(80.0), true);
                        (columns, bar)
                    }
                } else {
                    (1, false)
                };
                // 원본 DrawItems(0x7da82c): 자동(0)·이름(1) 보기는 늘 열로 흐르는 목록이고 머리글(DrawListHeader)은 자세히(2)만 그린다.
                // 스킨 테마는 그래서 한 열에 다 들어가도 흐르는 목록(머리글 없음, UpdateDisplayAuto의 칸 너비)이다.
                let auto_view = matches!(view.as_str(), "Auto" | "viewAuto");
                let auto_flow = !grouped && (auto_columns > 1 || auto_bar || (auto_view && skin.is_some()));
                // 원본 스킨 세로 막대(TNXScrollBar): 넘칠 때만 보이고 그 너비만큼 목록이 좁아진다. 넘치는지는 지난 그림에서 잰다.
                let skin_bar = match &skin {
                    Some(s) => self.skin_scrollbar_width(ctx, s),
                    None => None,
                };
                let bar_flag = egui::Id::new(("ndr-skin-vbar", index));
                let bar_reserve = match skin_bar {
                    Some(w) if ui.data(|d| d.get_temp::<bool>(bar_flag)).unwrap_or(false) => w,
                    _ => 0.0,
                };
                // 칸 너비: 자세히 보기는 C# star 비율, 한 열 자동 보기는 C# ApplyAutoDetailColumns(이름 = 나머지, 칸은 내용 너비, 모자라면 숨김).
                let cell_widths: Vec<Option<f32>> = if tiles || names || auto_columns > 1 || auto_flow {
                    Vec::new()
                } else {
                    self.original_detail_widths(ui, index, ui.available_width() - bar_reserve, &columns)
                };
                // 원본 UpdateScrollBar(형식 2, 0x7db270): 칸 너비 합(0x370)이 목록 너비(GetRealClientWidth, 세로 막대를 뺀 너비)보다 크면
                // 가로 막대(0x334, 두께 12)를 보이고 가로 위치(0x378)는 0..합 − 너비, 아니면 가로 위치 = 0. AdjustPage(형식 2)도 0으로 둔다.
                // 고친 것: 전에는 가로로 넘기지 못해 오른쪽 칸이 잘린 채 볼 수 없었다.
                let detail_content: f32 = cell_widths.iter().flatten().sum();
                let detail_avail = ui.available_width() - bar_reserve;
                let detail_hbar = skin_bar.filter(|_| !cell_widths.is_empty() && detail_content > detail_avail + 0.5);
                let detail_x = {
                    let pane = &mut self.panes[index];
                    if pane.adjust.is_some() {
                        pane.detail_x = 0.0;
                    }
                    let max_x = if detail_hbar.is_some() { (detail_content - detail_avail).max(0.0) } else { 0.0 };
                    pane.detail_x = pane.detail_x.clamp(0.0, max_x);
                    pane.detail_x
                };
                if !tiles && !names && auto_columns == 1 && !auto_flow {
                    let current = self.side_text(index, "SortMode", "Name");
                    let font = egui::FontId::new(self.list_font_size, egui::FontFamily::Name(super::LIST_FONT_FAMILY.into()));
                    let clicked = self.skin_list_header(ui, index, &columns, &cell_widths, font, detail_x);
                    if let Some(mode) = clicked {
                        // 원본 머리글(0x484d88): 같은 방식이면 내림차순 전환, 다른 방식이면 SetSortType만(내림차순 그대로).
                        let descending = Self::side_key(index, "SortDescending");
                        let old = self.settings.flag(&descending, false);
                        let flip = if current == mode { !old } else { old };
                        self.settings.set_flag(&descending, flip);
                        self.settings.set_text(&Self::side_key(index, "SortMode"), mode);
                        self.resort_pane(index);
                    }
                }
                if let Some(error) = &self.panes[index].error {
                    ui.colored_label(Color32::LIGHT_RED, self.archive_error_text(error));
                }
                if self.panes[index].loading {
                    ui.spinner();
                }
                // C#처럼 필터는 목록을 받을 때 적용하므로 보이는 줄 = 목록 전체(선택·작업 대상과 일치).
                let visible: Vec<usize> = (0..self.panes[index].entries.len()).collect();
                let reserve = self.skin_file_info_height() + ui.spacing().item_spacing.y;
                // 원본 FTP 모드 패널: 목록 아래 나눔 막대와 기록 칸(FUN_0048d7ac).
                let reserve = reserve + self.ftp_log_reserve(index, ui.spacing().item_spacing.y);
                let height = (ui.available_height() - reserve).max(80.0);
                let ftp_log_room = height + self.panes[index].ftp_log_height;
                // 원본·C# 판 ListFont·ListFontSize: 목록(이름·칸 글, 줄 높이 셈)만 그 글꼴·크기. 머리글·주소 줄 등은 보통 글꼴.
                let saved_text_styles = ui.style().text_styles.clone();
                let list_font_size = self.list_font_size;
                let list_style = self.list_font_style;
                for style in [egui::TextStyle::Body, egui::TextStyle::Button] {
                    if let Some(font) = ui.style_mut().text_styles.get_mut(&style) {
                        font.family = egui::FontFamily::Name(super::LIST_FONT_FAMILY.into());
                        font.size = list_font_size;
                    }
                }
                let row_height = self.list_item_height(ui);
                let list_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), height));
                // 원본 WMSize(0x7d5814)·SetListStyle(0x7dc6d4) → AdjustPage: 목록 크기나 보기 형식이 바뀌면 커서에 맞춰 위치를 다시 잡는다.
                let shape = (list_rect.size(), view.clone());
                let pane = &mut self.panes[index];
                if pane.list_shape.as_ref().is_some_and(|old| (old.0 - shape.0).length() > 0.5 || old.1 != shape.1) && !pane.entries.is_empty() {
                    pane.adjust = Some((pane.cursor.min(pane.entries.len() - 1), false));
                }
                pane.list_shape = Some(shape);
                // 사용자 요청(2026-10-31 16:49): 바퀴는 원본 WMMouseWheel(0x7d5ba0)처럼 목록을 넘긴다(쪽 단위 설정이면 한 쪽씩).
                // r138~r160의 "바퀴 = 커서 위·아래"는 되돌렸다. 각 보기의 wheel_scroll이 처리한다.
                if tiles {
                    let(a,o,c)=self.tile_rows(ui,index,&visible,height,view.contains("humb"));activate|=a;open|=o;if c.is_some(){command=c;}
                } else if names || auto_flow {
                    let (a, o, c) = self.flow_rows(ui, index, height, !names);
                    activate |= a;
                    open |= o;
                    if c.is_some() {
                        command = c;
                    }
                } else if auto_columns > 1 {
                    let (a, o, c) = self.auto_rows(ui, index, auto_columns, height);
                    activate |= a;
                    open |= o;
                    if c.is_some() {
                        command = c;
                    }
                } else {
                if self.panes[index].view.cells.len() != self.panes[index].entries.len() {
                    self.compute_view(index);
                }
                // 원본 드라이브 항목(종류 4): 이름 칸 끝(가장 넓은 드라이브 이름)과 DISKFREE 글.
                let drive_extent = self.drive_text_extent(ui, index);
                let free_label = self.language_pack.text("RES.DISKFREE", "Free");
                let icons_shown = self.settings.flag("ShowIcons", true);
                let view_cache = std::mem::take(&mut self.panes[index].view);
                let (rows, texts) = (&view_cache.rows, &view_cache);
                let width = ui.available_width() - bar_reserve;
                // 원본 GetItemRect(형식 2): 줄은 −가로 위치에서 시작하고 너비는 칸 너비 합(목록보다 좁으면 목록 너비). 가로 막대가 보이면
                // 그 두께만큼 줄 자리가 낮아진다.
                let view_width = width;
                let width = width.max(detail_content);
                let height = match detail_hbar {
                    Some(bar) => (height - bar).max(row_height),
                    None => height,
                };
                // 원본 UpdateDisplayDetails: 줄 간격 = 항목 높이 + LineSpace(+0x364). egui는 줄마다 item_spacing.y를 더하므로 그것을 LineSpace로 둔다.
                // 고친 것: 전에는 egui 기본 틈(3화소)이 더해져 줄이 원본보다 성겼고 그 틈을 누르면 아무 항목도 아니었다.
                let line_space = self.list_line_space();
                let saved_spacing = ui.spacing().item_spacing.y;
                ui.spacing_mut().item_spacing.y = line_space;
                let (row_lines, line_color) = (self.settings.flag("ShowRowLines", false), self.grid_line_color(ui));
                // show_rows는 줄마다 item_spacing.y를 더해 쌓으므로 위치 계산도 그 간격(stride)으로 한다.
                let stride = row_height + ui.spacing().item_spacing.y;
                let by_page = self.scroll_by_page(index);
                let page_rows = ((height / stride).floor() as usize).max(1);
                let pages = rows.len().div_ceil(page_rows).max(1);
                let page_px = page_rows as f32 * stride;
                // 원본 SetScrollTop(쪽 단위): 바퀴로 옮긴 위치는 쪽 경계(쪽 번호 × 한 쪽)다.
                let wheel = Self::wheel_scroll(ui, egui::Id::new(("files", index)), height, stride, by_page, (self.system.wheel_lines)()).map(|o| if by_page { Self::page_snap(o, page_px, pages) } else { o });
                let mut area = egui::ScrollArea::both()
                    .id_salt(("files", index))
                    .scroll_source(WHEEL_OFF)
                    .auto_shrink([false, false])
                    .max_height(height)
                    .min_scrolled_height(height);
                if skin_bar.is_some() {
                    area = area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
                }
                // 원본 SetItemIndex(형식 2): 글쇠로 옮긴 커서 줄이 위로 벗어나면 그 줄을 맨 위로, 아래로 벗어나면 맨 아래로 넘긴다.
                // 쪽 단위면 커서가 든 쪽의 처음 줄을 맨 위로 둔다((커서 / 한 쪽 줄 수) × 한 쪽 줄 수).
                // 쪽 정보(다 보이는 줄 수, 맨 위·끝까지 보이는 항목)는 ArrowKey의 PageUp·PageDown이 쓴다.
                let current = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(egui::Id::new(("files", index)))).map_or(0.0, |s| s.offset.y);
                let mut offset = wheel.unwrap_or(current);
                // 원본 AdjustPage(형식 2): 가로 위치 = 0. 쪽 단위면 그 항목이 든 쪽의 처음 줄을 맨 위로, 아니면 위로 벗어날 때 맨 위,
                // 아래로 벗어날 때 맨 아래로. 다시 읽은 목록은 원본이 그 뒤 SetScrollTop(옛 위치)로 되돌리므로 세로는 하지 않는다.
                if let Some((item, reload)) = self.panes[index].adjust.take() {
                    if self.panes[index].detail_x != 0.0 {
                        self.panes[index].detail_x = 0.0;
                        ui.ctx().request_repaint();
                    }
                    if let Some(line) = rows.iter().position(|r| matches!(r, DetailRow::Item(i) if *i == item)).filter(|_| !reload) {
                        let (top, bottom) = (line as f32 * stride, line as f32 * stride + row_height);
                        if by_page {
                            offset = (line / page_rows) as f32 * page_px;
                        } else if top < offset {
                            offset = top;
                        } else if bottom > offset + height {
                            offset = bottom - height;
                        }
                    }
                }
                if std::mem::take(&mut self.panes[index].reveal) && wheel.is_none() {
                    let cursor = self.panes[index].cursor;
                    if let Some(line) = rows.iter().position(|r| matches!(r, DetailRow::Item(i) if *i == cursor)) {
                        let (top, bottom) = (line as f32 * stride, line as f32 * stride + row_height);
                        if top < offset || bottom > offset + height {
                            offset = if by_page {
                                (line / page_rows) as f32 * page_px
                            } else if top < offset {
                                top
                            } else {
                                bottom - height
                            };
                        }
                    }
                }
                let item = |r: &DetailRow| if let DetailRow::Item(i) = r { Some(*i) } else { None };
                let first_line = (offset / stride - 0.01).ceil().max(0.0) as usize;
                let last_line = (((offset + height - row_height).max(0.0) / stride).floor() as usize).max(first_line);
                let top_item = rows.iter().skip(first_line).find_map(item).unwrap_or(0);
                let last_item = rows.iter().take(last_line + 1).rev().find_map(item).unwrap_or(top_item).max(top_item);
                self.panes[index].page = list_keys::ListPage { style: list_keys::ListStyle::Details, rows: page_rows, columns: 1, top: top_item, last: last_item };
                if (offset - current).abs() > 0.5 {
                    area = area.vertical_scroll_offset(offset);
                }
                area = area.horizontal_scroll_offset(self.panes[index].detail_x);
                // 쪽 단위로 여러 쪽이면 마지막 쪽도 쪽 경계에서 시작하도록 빈 줄을 덧댄다(원본은 마지막 쪽 아래가 빈다).
                let drawn = if by_page && pages > 1 { rows.len().max(pages * page_rows + 2) } else { rows.len() };
                // 원본 스킨 테마: 커서·표시 막대(DrawFocus)와 칸 색(DrawListItem), 표시 ▶(InitSelMarkBitmap).
                let original = Some(self.original_list_colors());
                let output = area
                    .show_rows(ui, row_height, drawn, |ui, range| {
                        // 가로로 넘길 수 있게 내용 너비 = 줄 너비(칸 너비 합 또는 목록 너비). egui 안쪽 너비는 세로 막대 자리를 담으므로
                        // 그만큼 더해 마지막 칸이 세로 막대 앞까지 오게 한다.
                        ui.set_min_width(width + bar_reserve);
                        for visible_index in range {
                            let Some(detail_row) = rows.get(visible_index) else { continue };
                            let row = match detail_row {
                                DetailRow::Header(label) => {
                                    ui.add_sized([width, row_height], egui::Label::new(RichText::new(label).strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())).color(ui.visuals().weak_text_color())));
                                    continue;
                                }
                                DetailRow::Item(row) => *row,
                            };
                            // C# QueueFolderSizeCalculation: 보이는 폴더 줄이 생길 때.
                            self.queue_folder_size(index, row);
                            let icon = self.file_icon(ui.ctx(), index, row);
                            let icon = self.row_icon_for_skin(index, row, icon);
                            let pane = &mut self.panes[index];
                            let entry = &pane.entries[row];
                            let selected = pane.selected.contains(&row);
                            let default = if entry.is_dir {
                                Color32::from_rgb(228, 188, 85)
                            } else if entry.hidden {
                                Color32::GRAY
                            } else {
                                ui.visuals().text_color()
                            };
                            let color = texts.colors[row].map_or(default, |(r, g, b)| Color32::from_rgb(r, g, b));
                            let cells = &texts.cells[row];
                            let cursor_row = pane.cursor == row;
                            let bar_slot = original.map(|_| ui.painter().add(egui::Shape::Noop));
                            let framed = false;
                            let name_color = match original {
                                Some(o) if cursor_row && active => o.back,
                                _ => color,
                            };
                            let line = ui
                                .horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    // 원본 DrawListItem 종류 4: 드라이브 항목은 열과 상관없이 한 줄에 이름("C: 이름")과 남은 공간을 그린다.
                                    if entry.drive {
                                        let info = pane.drive_infos.get(&entry.path).map(|(_, kind, sizes)| (*kind, *sizes));
                                        return super::list_view::drive_row(ui, egui::vec2(width, row_height), icon.as_ref(), &cells[0], name_color, framed, pane.cursor == row, drive_extent, info, &free_label, list_style);
                                    }
                                    let mut name_response = None;
                                    for ((column, _, _), cell) in columns.iter().zip(&cell_widths) {
                                        let Some(cell) = *cell else { continue };
                                        let folder_size = (*column == DetailColumn::Size && entry.is_dir).then(|| pane.folder_sizes.get(&entry.path).map(|s| s.clone().unwrap_or_else(|| "…".into()))).flatten();
                                        let text = folder_size.as_ref().unwrap_or(&cells[column.index()]);
                                        let size = [cell, row_height];
                                        if *column == DetailColumn::Name {
                                            name_response = Some(App::name_cell(ui, egui::vec2(cell, row_height), icon.as_ref(), icons_shown, text, name_color, list_style));
                                            if cursor_row && original.is_some() {
                                                if let Some(response) = &name_response {
                                                    note_cursor_name(&self.name_tip, ui, index, row, response.rect, text, &entry.name, list_style);
                                                }
                                            }
                                        } else if let Some(o) = original {
                                            let cell_color = o.cell(*column, color, entry.attributes, cursor_row, active);
                                            let (rect, _) = ui.allocate_exact_size(egui::vec2(cell, row_height), egui::Sense::hover());
                                            paint_original_cell(ui, rect, *column, text, cell_color, cursor_row, &o, list_style);
                                        } else {
                                            ui.add_sized(size, egui::Label::new(super::list_text(text.as_str(), list_style).color(if *column == DetailColumn::Description { Color32::from_rgb(0x36, 0x82, 0xAF) } else { ui.visuals().weak_text_color() })).truncate());
                                        }
                                    }
                                    name_response.expect("the name column is always shown")
                                });
                            if let (Some(slot), Some(o)) = (bar_slot, original) {
                                let rect = egui::Rect::from_min_size(line.response.rect.left_top(), egui::vec2(width, row_height));
                                if cursor_row || selected {
                                    let bar = super::skin_pane::focus_bar(color, o.back, o.select, cursor_row, selected, active);
                                    ui.painter().set(slot, egui::Shape::Vec(super::skin_pane::focus_bar_shapes(rect, &bar)));
                                }
                                if selected {
                                    ui.painter().extend(super::skin_pane::selection_mark_shapes(rect, o.select));
                                }
                            }
                            // 원본 DrawListItem(+0x387): 항목 아래 (bottom + LineSpace / 2 − 1)에 NXLINE 가로선. 여기서는 줄 사이 틈 가운데.
                            if row_lines {
                                let y = Self::row_line_y(ui.painter(), line.response.rect.bottom(), ui.spacing().item_spacing.y);
                                let left = line.response.rect.left();
                                ui.painter().hline(left..=left + width, y, egui::Stroke::new(1.0, line_color));
                            }
                            // 줄 어디를 눌러도 그 항목(이름 칸만이 아니라 줄 전체). 원본 GetItemAt(형식 2): 번호 = (y + 위치 + LineSpace − 머리)
                            // / (높이 + LineSpace)라 줄 위의 틈(LineSpace)도 그 항목이다.
                            let gap = ui.spacing().item_spacing.y;
                            let row_rect = egui::Rect::from_min_size(line.response.rect.left_top() - egui::vec2(0.0, gap), egui::vec2(width, row_height + gap));
                            let response = ui.interact(row_rect, ui.id().with(("ndr-detail-row", index, row)), egui::Sense::click_and_drag());
                            // C# BeginFileDragTracking: 왼쪽 단추로 끌면 항목 끌기. 원본은 오른쪽 단추 끌기도 받아 놓을 때 pmDrop을 연다.
                            let folder_path = entry.is_dir.then(|| entry.path.clone());
                            // 원본 MouseDown: 스킨 테마는 누르는 순간 처리(줄 왼쪽 12화소는 표시 칸, Shift는 범위 끌기).
                            let in_press = skin.is_some() && original_press_input(pane, row, response.rect, ui);
                            let area_hit = skin.is_some() && pointer_in_selection_area(response.rect, ui);
                            // 원본(스킨): Shift로 누른 뒤의 범위 끌기 중에도 8화소를 넘으면 끌기(FUN_004856d8)가 시작된다(표시 칸 범위 끌기는 아님).
                            if drag_begins(&response, egui::PointerButton::Primary, skin.is_some()) && (skin.is_some() || pane.range_drag.is_none()) {
                                begin_drag = Some((row, false));
                            } else if drag_begins(&response, egui::PointerButton::Secondary, skin.is_some()) {
                                begin_drag = Some((row, true));
                            }
                            if let Some(path) = folder_path {
                                let visual = egui::Rect::from_min_size(line.response.rect.left_top(), egui::vec2(width, row_height));
                                folder_rows.push((index, visual.intersect(ui.clip_rect()), path));
                            }
                            if response.clicked() {
                                activate = true;
                                if skin.is_none() {
                                    let modifiers = ui.input(|i| i.modifiers);
                                    row_pressed(pane, row, modifiers, false, false);
                                }
                            }
                            if in_press {
                                activate = true;
                            }
                            // 원본 DblClick(0x7dd1ec): 표시 칸에서는 열지 않는다. 두 번 누르기는 Windows 기준(시간·자리).
                            if super::panes::windows_double_clicked(&response) && !area_hit {
                                pane.cursor = row;
                                activate = true;
                                open = true;
                            }
                            // WPF DataGrid: 오른쪽 단추는 그 줄을 고른다(이미 고른 줄이면 선택 유지). 스킨 테마는 원본처럼 커서만.
                            if response.secondary_clicked() {
                                row_pressed(pane, row, egui::Modifiers::NONE, true, skin.is_some());
                                activate = true;
                            }
                            // 줄 응답에 메뉴를 붙이지 않는다(패널 메뉴 skin_pane::pane_menu).
                            if response.secondary_clicked() {
                                super::skin_pane::request_pane_menu(ui.ctx(), index);
                            }
                        }
                    });
                // 원본 SetScrollTop(쪽 단위): 막대를 끌어 놓으면 위치는 가장 가까운 쪽 경계(쪽 번호)다.
                if by_page && pages > 1 && !ui.input(|i| i.pointer.any_down()) {
                    let snapped = Self::page_snap(output.state.offset.y, page_px, pages);
                    if (snapped - output.state.offset.y).abs() > 0.5 {
                        let mut state = output.state;
                        state.offset.y = snapped;
                        state.store(ui.ctx(), output.id);
                        ui.ctx().request_repaint();
                    }
                }
                if let (Some(bar_width), Some(skin)) = (skin_bar, &skin) {
                    let needs = output.content_size.y > output.inner_rect.height() + 0.5;
                    if ui.data(|d| d.get_temp::<bool>(bar_flag)) != Some(needs) {
                        ui.data_mut(|d| d.insert_temp(bar_flag, needs));
                        ui.ctx().request_repaint();
                    }
                    if needs {
                        let inner = output.inner_rect;
                        let bar = egui::Rect::from_min_max(egui::pos2(inner.right() - bar_width, inner.top()), inner.max);
                        if let Some(next) = self.skin_vertical_scrollbar(ui, ctx, skin, bar_flag.with("bar"), bar, output.state.offset.y, output.content_size.y, inner.height(), stride) {
                            let mut state = output.state;
                            state.offset.y = next;
                            state.store(ui.ctx(), output.id);
                            ui.ctx().request_repaint();
                        }
                    }
                }
                // 원본 가로 TNXScrollBar(형식 2): 목록 아래, 너비 = GetRealClientWidth, 한 칸 10, 한 쪽 = 목록 너비.
                self.panes[index].detail_x = output.state.offset.x.clamp(0.0, (detail_content - view_width).max(0.0));
                if let (Some(bar), Some(skin)) = (detail_hbar, &skin) {
                    let (strip, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), bar), egui::Sense::hover());
                    let rect = egui::Rect::from_min_size(strip.min, egui::vec2(view_width, bar));
                    if let Some(next) = self.skin_horizontal_scrollbar(ui, ctx, skin, egui::Id::new(("ndr-skin-detail-hbar", index)), rect, self.panes[index].detail_x, detail_content, view_width, 10.0) {
                        self.panes[index].detail_x = next.clamp(0.0, (detail_content - view_width).max(0.0));
                        ui.ctx().request_repaint();
                    }
                }
                self.panes[index].view = view_cache;
                ui.spacing_mut().item_spacing.y = saved_spacing;
                }
                ui.style_mut().text_styles = saved_text_styles;
                // 목록 오른쪽 단추 메뉴(줄을 그렸는지와 상관없이 그림마다). 원본 OnContextPopup(FUN_00484e8c)이 무엇을 띄울지 고른다.
                // 글쇠(Apps, MousePos -1, -1)는 지금 패널만: 표시가 없고 커서 항목이 ".."이면 띄우지 않고, 자리는 커서 항목 칸의 왼쪽 아래.
                let mouse_menu = super::skin_pane::take_pane_menu_request(ctx, index);
                let key_menu = index == self.active && std::mem::take(&mut self.pending_context_key);
                let parent_only = self.panes[index].selected.is_empty() && self.panes[index].entries.get(self.panes[index].cursor).is_some_and(|e| e.is_parent());
                // 원본 FUN_00485078: 오른쪽 단추를 누른 채 왼쪽 단추(누른 줄로 커서를 옮긴 뒤)면 rocker_menu_kind.
                let rocker = ui.rect_contains_pointer(list_rect) && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary) && i.pointer.button_down(egui::PointerButton::Secondary));
                let kind = if rocker {
                    self.rocker_menu_kind(index)
                } else if mouse_menu || (key_menu && !parent_only) {
                    self.list_menu_kind(index)
                } else {
                    ListMenu::None
                };
                let mouse_menu = mouse_menu || rocker;
                let open_menu = match kind {
                    ListMenu::Main if !mouse_menu => {
                        let cursor = self.panes[index].cursor;
                        let at = self.name_tip.borrow().as_ref().filter(|tip| tip.0 == index && tip.1 == cursor).map(|tip| tip.2.left_bottom());
                        Some(at.unwrap_or(list_rect.left_top()))
                    }
                    ListMenu::Main => Some(ctx.input(|i| i.pointer.interact_pos()).unwrap_or(list_rect.left_top())),
                    ListMenu::Shell(paths) => {
                        if let Err(error) = ndr::shell_menu::show(&paths) {
                            self.status = error;
                        }
                        None
                    }
                    ListMenu::None => None,
                };
                super::skin_pane::pane_menu(ui, index, open_menu, |ui| {
                    if let Some(chosen) = self.list_context_menu(ui, index) {
                        command = Some(chosen);
                    }
                });
                self.panes[index].drop_drive_marks();
                self.original_name_tip(ctx, index);
                // 목록 자리를 바깥 Ui에 반영하고 그 아래(FTP 기록 칸, 파일 정보줄)는 패널 너비 전체에.
                let used = ui.min_rect();
                let ui = outer;
                ui.advance_cursor_after_rect(egui::Rect::from_min_max(full.min, egui::pos2(full.right(), used.bottom().max(full.top()))));
                self.ftp_log_ui(ui, index, ftp_log_room);
                if let Some(skin) = &skin {
                    self.skin_file_info_bar(ui, index, skin);
                } else {
                    self.address_bar(ui, index, ctx);
                    self.info_bars(ui, index);
                }
                if activate {
                    self.active = index;
                }
                // 원본 FUN_004856d8: 빈 곳에서 누른 채 8화소 넘게 끌어도 표시한 항목이 있으면(GetSelCount > 0) 그 항목들을 끈다(스킨 테마).
                if begin_drag.is_none() && skin.is_some() && self.file_drag.is_none() && !self.panes[index].selected.is_empty() && ctx.dragged_id().is_none() {
                    let (origin, now, primary, secondary) = ui.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos(), i.pointer.primary_down(), i.pointer.secondary_down()));
                    if let (Some(origin), Some(now)) = (origin, now) {
                        let moved = (now - origin).abs();
                        if (primary || secondary) && list_rect.contains(origin) && (moved.x > 8.0 || moved.y > 8.0) {
                            begin_drag = Some((self.panes[index].cursor, secondary && !primary));
                        }
                    }
                }
                if let Some((row, secondary)) = begin_drag {
                    self.begin_file_drag(index, row, secondary, ui.input(|i| i.modifiers));
                }
                self.folder_rows.extend(folder_rows);
                if open {
                    self.open_current();
                }
                if let Some(command) = command {
                    if command == "openInNewTab" {
                        // C# OpenSelectedInNewTabAsync: 같은 위치의 새 탭에서 연다.
                        self.dispatch("newTab", ctx);
                        self.open_current();
                    } else {
                        self.dispatch(command, ctx);
                    }
                }
            });
        ui.spacing_mut().item_spacing.y = outer_spacing;
        self.pane_rects[index] = shown.response.rect;
    }
}
