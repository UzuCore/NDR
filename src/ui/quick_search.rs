//! C# MainWindow.QuickSearch.cs의 파일 목록 빠른 찾기(패널마다 입력 버퍼)와 빠른 필터(QuickFilterAsync, ShowFilterAsync).
//! 폴더 트리 빠른 찾기는 폴더 트리 이식(보류)과 함께 남긴다.
use super::*;

impl App {
    fn quick_search_caption(&self) -> &'static str {
        if self.language == "ko" { "빠른 찾기" } else { "Quick search" }
    }

    fn quick_search_cleared(&mut self) {
        self.status = if self.language == "ko" { "빠른 찾기를 지웠습니다." } else { "Quick search cleared." }.into();
    }

    /// C# SelectQuickSearchMatch: 찾으면 커서를 옮기고 상태 줄에 입력을, 없으면 "찾을 수 없음"을 보인다.
    /// C#은 WPF 선택 모델 때문에 표시를 지우지만, Rust 목록은 원본처럼 표시와 커서가 따로라 표시는 그대로 둔다.
    fn select_quick_search_match(&mut self, pane: usize, query: &str, cycle_from_next: bool) -> bool {
        let p = &self.panes[pane];
        let names: Vec<String> = p.entries.iter().map(|e| e.name.clone()).collect();
        let cursor = (!names.is_empty()).then_some(p.cursor);
        let caption = self.quick_search_caption();
        match ndir::parity::quick_search_match(&names, cursor, query, cycle_from_next) {
            Some(row) => {
                self.panes[pane].cursor = row;
                self.status = format!("{caption}: {query}");
                true
            }
            None => {
                if query.is_empty() || names.is_empty() {
                    return false;
                }
                self.status = if self.language == "ko" { format!("{caption}: {query} · 찾을 수 없음") } else { format!("{caption}: {query} · no match") };
                false
            }
        }
    }

    /// C# Window_PreviewTextInput + TryHandleQuickSearchKeyDown. 처리했으면 true(그 프레임의 다른 키 처리는 하지 않는다).
    /// 글자: 1100ms 안에 이어 치면 붙이고, 아니면 새로 시작. 못 찾고 새 글자가 입력 전체보다 짧으면 그 글자만으로 다음 항목부터 다시 찾는다.
    /// Backspace/Esc(수식 키 없이)는 입력이 있을 때만 빠른 찾기가 먼저 받는다.
    pub(super) fn typed_search_input(&mut self, ctx: &egui::Context) -> bool {
        let pane = self.active;
        let (texts, back, escape) = ctx.input(|input| {
            let plain = !input.modifiers.ctrl && !input.modifiers.alt && !input.modifiers.command;
            let texts: Vec<String> = if plain {
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        // 스페이스는 표시 전환 키(SelectCurrAndMove)가 먼저 받는다.
                        egui::Event::Text(text) if !text.is_empty() && text != " " && !text.chars().any(char::is_control) => Some(text.clone()),
                        _ => None,
                    })
                    .collect()
            } else {
                vec![]
            };
            let none = input.modifiers.is_none();
            (texts, none && input.key_pressed(egui::Key::Backspace), none && input.key_pressed(egui::Key::Escape))
        });
        if !self.typed_search[pane].query.is_empty() && (back || escape) {
            if escape {
                self.typed_search[pane] = TypedSearch::default();
                self.quick_search_cleared();
            } else {
                let search = &mut self.typed_search[pane];
                search.query.pop();
                search.last_input = Some(Instant::now());
                if search.query.is_empty() {
                    self.typed_search[pane] = TypedSearch::default();
                    self.quick_search_cleared();
                } else {
                    let query = search.query.clone();
                    self.select_quick_search_match(pane, &query, false);
                }
            }
            return true;
        }
        if texts.is_empty() || self.panes[pane].loading {
            return false;
        }
        for text in texts {
            let search = &mut self.typed_search[pane];
            let expired = search.query.is_empty()
                || search.last_input.is_none_or(|at| at.elapsed().as_millis() > ndir::parity::QUICK_SEARCH_TIMEOUT_MS);
            if expired {
                search.query = text.clone();
            } else {
                search.query.push_str(&text);
            }
            search.last_input = Some(Instant::now());
            let query = search.query.clone();
            if !self.select_quick_search_match(pane, &query, false) && text.chars().count() < query.chars().count() {
                let search = &mut self.typed_search[pane];
                search.query = text.clone();
                search.last_input = Some(Instant::now());
                self.select_quick_search_match(pane, &text, true);
            }
        }
        true
    }

    /// C# QuickFilterAsync: 검색 칸이나 도구 모음 빠른 찾기 칸에서 Enter.
    pub(super) fn quick_filter(&mut self, pane: usize, text: &str) {
        self.panes[pane].filter = ndir::parity::quick_filter_pattern(text);
        self.refresh(pane);
    }

    /// C# ShowFilterAsync = 원본 miFilterCustomClick(0x441504): TfGetText(TITLE_FILTER, MSG_FILTER, 현재 필터),
    /// 앞뒤 공백을 떼어 적용하고 새로 읽는다. 상태 줄 알림은 C#.
    pub(super) fn show_filter_prompt(&mut self) {
        let pane = self.active;
        let title = self.language_pack.text("RES.TITLE_FILTER", "Filter");
        let message = self.language_pack.text("RES.MSG_FILTER", "Type a filter you want to use. You can put several filters separated with semi-colon(;)\nex) *.exe;*.txt;a??.dll");
        let current = self.panes[pane].filter.clone();
        self.prompt(&title, &message, &current, None, false, move |app, value, _| {
            let Some(pattern) = value else { return };
            app.panes[pane].filter = pattern.trim().to_owned();
            app.refresh(pane);
            let filter = app.panes[pane].filter.clone();
            app.status = if filter.is_empty() { app.tr("목록 필터를 해제했습니다.", "List filter cleared.").into_owned() } else { format!("{}: {filter}", app.tr("목록 필터", "List filter")) };
        });
    }

    /// 스킨 테마의 빠른 찾기(원본 TNXCustomFileListView). 처리했으면 true.
    /// - 찾는 글(+0x3e0)은 시간이 지나도 남고, 커서를 다른 길(화살표·Home/End·쪽 넘김·누르기)로 옮기거나 목록이 바뀌면 지운다
    ///   (원본 ArrowKey 0x7d69dc·MouseDown 0x7d63c8·FileListChanged 0x7d79c8).
    /// - 글자는 WMChar(0x7d5a18) 규칙으로 거르고, Backspace는 찾는 글이 있을 때만 받는다(그 밖에는 원본 FormShortCut 0x42b36c처럼 단축키).
    pub(super) fn original_typed_search(&mut self, ctx: &egui::Context) -> bool {
        for pane in 0..2 {
            let cursor = self.panes[pane].cursor;
            // 원본 MouseDown(0x7d63c8)은 항목을 누를 때마다(같은 항목이어도) 찾는 글을 지운다.
            let pressed = std::mem::take(&mut self.panes[pane].item_pressed);
            let search = &mut self.typed_search[pane];
            if !search.query.is_empty() && (pressed || search.cursor != Some(cursor)) {
                *search = TypedSearch::default();
            }
        }
        let pane = self.active;
        let has_text = !self.typed_search[pane].query.is_empty();
        // 원본 목록 창은 IME를 받는다: 목록에 글쇠가 갈 때 IME를 켜 두고(조합 창은 패널 왼쪽 위), 조합 글로 찾는다.
        let rect = self.pane_rects[pane];
        ctx.output_mut(|o| o.ime = Some(egui::output::IMEOutput { rect, cursor_rect: egui::Rect::from_min_size(rect.min, egui::vec2(1.0, 16.0)) }));
        let ime: Vec<egui::ImeEvent> = ctx.input(|input| input.events.iter().filter_map(|e| if let egui::Event::Ime(ev) = e { Some(ev.clone()) } else { None }).collect());
        let ime_handled = !self.panes[pane].loading && self.original_ime_search(pane, &ime);
        let (texts, back) = ctx.input(|input| {
            let plain = !input.modifiers.ctrl && !input.modifiers.alt && !input.modifiers.command;
            let texts: Vec<char> = if plain {
                input.events.iter().filter_map(|event| if let egui::Event::Text(text) = event { Some(text.chars().collect::<Vec<_>>()) } else { None }).flatten().collect()
            } else {
                vec![]
            };
            (texts, plain && has_text && input.key_pressed(egui::Key::Backspace))
        });
        if self.panes[pane].loading || (texts.is_empty() && !back) {
            return ime_handled;
        }
        let caps = native::caps_lock_state();
        let mut handled = back || ime_handled;
        let keys = back.then_some('\u{8}').into_iter().chain(texts);
        for c in keys {
            let empty = self.typed_search[pane].query.is_empty();
            let Some(key) = original_search_key(c, caps, empty) else { continue };
            handled = true;
            self.original_quick_search_step(pane, key);
        }
        handled
    }

    /// 원본 WMChar의 한 글자 처리와 QuickSearch(0x7d5f9c): 찾는 글에 붙여(Backspace는 끝 글자를 지우고) 지금 항목부터 찾고,
    /// 못 찾고 새 글자가 찾는 글보다 짧으면 그 글자만으로 다음 항목부터 다시 찾는다. 그래도 없으면 찾는 글을 비운다.
    fn original_quick_search_step(&mut self, pane: usize, key: OriginalSearchKey) {
        let mut text = std::mem::take(&mut self.typed_search[pane].query);
        let typed = match key {
            OriginalSearchKey::Clear => String::new(),
            OriginalSearchKey::Back => {
                text.pop();
                String::new()
            }
            OriginalSearchKey::Char(c) => {
                text.push(c);
                c.to_string()
            }
        };
        if key != OriginalSearchKey::Clear {
            self.original_search_text(pane, text, typed);
            return;
        }
        text.clear();
        let cursor = self.panes[pane].cursor;
        let search = &mut self.typed_search[pane];
        search.query = text;
        search.cursor = Some(cursor);
    }

    /// 원본 QuickSearch(0x7d5f9c): 찾는 글로 지금 항목부터 찾고, 못 찾고 친 글자가 찾는 글보다 짧으면 그 글자만으로
    /// 다음 항목부터 다시 찾는다. 그래도 없으면 찾는 글을 비운다.
    fn original_search_text(&mut self, pane: usize, mut text: String, typed: String) {
        let names: Vec<String> = self.panes[pane].entries.iter().map(|e| e.name.clone()).collect();
        let cursor = self.panes[pane].cursor;
        let mut found = original_quick_search(&names, cursor, &text, true);
        if found.is_none() && typed.chars().count() < text.chars().count() {
            text = typed;
            found = original_quick_search(&names, cursor, &text, false);
        }
        match found {
            Some(row) => {
                self.panes[pane].cursor = row;
                self.panes[pane].reveal = true;
            }
            None => text.clear(),
        }
        let cursor = self.panes[pane].cursor;
        let search = &mut self.typed_search[pane];
        search.query = text;
        search.cursor = Some(cursor);
    }

    /// 원본 WMIMEComposition(0x7d584c): 조합을 시작하면 찾는 글 끝에 조합 글자를 붙이고, 조합이 바뀌면 끝 글자를 바꾸고,
    /// 조합 글이 비면 끝 글자를 지운다. 결과 글(GCS_RESULTSTR)이 오면 조합을 끝낸다. 메시지마다 QuickSearch(조합 글자).
    /// egui: Preedit = 조합 중, Commit = 결과, Disabled(결과 없이) = 조합 글이 빈 것.
    pub(super) fn original_ime_search(&mut self, pane: usize, events: &[egui::ImeEvent]) -> bool {
        let committed = events.iter().any(|e| matches!(e, egui::ImeEvent::Commit(_)));
        let mut handled = false;
        for event in events {
            let (composition, result) = match event {
                egui::ImeEvent::Preedit(text) => (text.clone(), false),
                egui::ImeEvent::Commit(text) => (text.clone(), true),
                egui::ImeEvent::Disabled if !committed && self.typed_search[pane].ime_base.is_some() => (String::new(), true),
                _ => continue,
            };
            let base = match self.typed_search[pane].ime_base.take() {
                Some(base) => base,
                None if composition.is_empty() => continue,
                None => std::mem::take(&mut self.typed_search[pane].query),
            };
            let text = format!("{base}{composition}");
            if !result {
                self.typed_search[pane].ime_base = Some(base.clone());
            }
            handled = true;
            let typed: String = composition.chars().last().map(String::from).unwrap_or_default();
            if !result {
                // 고친 것: 원본은 조합 중간 글자(예: "방"을 치는 동안의 "바")로 찾지 못하면 찾는 글을 비워 한글 찾기가 끊긴다.
                // NDIR은 조합 중에는 찾지 못해도 글을 남기고 커서를 그대로 두며, 결과 글이 오면 원본 규칙대로 찾는다.
                let names: Vec<String> = self.panes[pane].entries.iter().map(|e| e.name.clone()).collect();
                if let Some(row) = original_quick_search(&names, self.panes[pane].cursor, &text, true) {
                    self.panes[pane].cursor = row;
                    self.panes[pane].reveal = true;
                }
                let cursor = self.panes[pane].cursor;
                let search = &mut self.typed_search[pane];
                search.query = text;
                search.cursor = Some(cursor);
                continue;
            }
            self.original_search_text(pane, text, typed);
        }
        handled
    }

    /// 원본 tbMainMenuAdvancedCustomDraw(0x430e74): 지금 패널의 찾는 글이 있으면 메뉴 줄 오른쪽 끝(오른쪽 − 글 너비 − 10)에
    /// 대문자(FUN_00514870, a~z만)로, 세로 가운데, 스킨 [Menu] mtext 색으로 그린다.
    pub(super) fn paint_original_search_text(&mut self, ui: &egui::Ui) {
        let Some(skin) = self.current_skin() else { return };
        let text = &self.typed_search[self.active].query;
        if text.is_empty() {
            return;
        }
        let rect = ui.max_rect();
        let color = skin.color("Menu", "mtext").map(|c| Color32::from_rgb(c[0], c[1], c[2])).unwrap_or(Color32::from_gray(0xB4));
        let font = egui::FontId::proportional(super::BASE_TEXT_SIZE);
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_ascii_uppercase(), font, color));
        let at = egui::pos2(rect.right() - galley.size().x - 10.0, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, color);
    }
}

/// 원본 WMChar 글자 거르기의 결과.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OriginalSearchKey {
    /// '!'(0x21): 찾는 글을 비운다(SetSearchText("")).
    Clear,
    Back,
    Char(char),
}

/// 원본 WMChar(0x7d5a18): Caps Lock이 꺼져 있으면 대문자 A~Z는 받지 않고, 찾는 글이 비었으면 Backspace·'.'도 받지 않는다.
/// 받는 글자는 '.'·Backspace와 0x21~0x7e 가운데 \ / : * ? 가 아닌 것(FUN_007d59bc). 공백과 한글 등은 받지 않는다.
pub(super) fn original_search_key(c: char, caps_lock: bool, text_empty: bool) -> Option<OriginalSearchKey> {
    let code = c as u32;
    if !caps_lock && c.is_ascii_uppercase() {
        return None;
    }
    if text_empty && (c == '\u{8}' || c == '.') {
        return None;
    }
    let allowed = (0x21..0x7f).contains(&code) && !"\\/:*?".contains(c);
    if c != '.' && c != '\u{8}' && !allowed {
        return None;
    }
    if code < 0x22 && c != '\u{8}' {
        return Some(OriginalSearchKey::Clear);
    }
    Some(if c == '\u{8}' { OriginalSearchKey::Back } else { OriginalSearchKey::Char(c) })
}

/// 원본 QuickSearch(글, 지금 항목 포함)(0x7d5ca8). 글이 비었으면 0. 지금 항목(포함하지 않으면 다음, 마지막이면 처음)부터 끝까지
/// 앞부분이 같은 이름(StrCmpNIW), 시작이 처음이 아니면 처음부터 지금 항목 앞까지, 그다음 같은 차례로 이름 안에 든 것(StrStrIW).
/// 시작이 처음이면 이름 안 찾기는 하지 않는다(원본 그대로).
pub(super) fn original_quick_search(names: &[String], cursor: usize, text: &str, inclusive: bool) -> Option<usize> {
    if names.is_empty() {
        return None;
    }
    if text.is_empty() {
        return Some(0);
    }
    let count = names.len();
    let needle: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let lower = |name: &str| name.chars().flat_map(char::to_lowercase).collect::<Vec<char>>();
    let prefix = |i: usize| lower(&names[i]).starts_with(&needle);
    let contains = |i: usize| {
        let name = lower(&names[i]);
        name.windows(needle.len()).any(|w| w == needle.as_slice())
    };
    let cursor = cursor.min(count - 1);
    let start = if cursor == count - 1 { 0 } else if inclusive { cursor } else { cursor + 1 };
    let stop = start == 0 || (!inclusive && cursor == count - 1);
    if let Some(i) = (start..count).find(|&i| prefix(i)) {
        return Some(i);
    }
    if stop {
        return None;
    }
    (0..cursor).find(|&i| prefix(i)).or_else(|| (start..count).find(|&i| contains(i))).or_else(|| (0..cursor).find(|&i| contains(i)))
}
