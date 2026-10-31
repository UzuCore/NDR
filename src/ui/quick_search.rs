//! File list quick search (input buffer per pane) and quick filter (QuickFilterAsync, ShowFilterAsync) of C# MainWindow.QuickSearch.cs.
//! Folder tree quick search stays with the folder tree port (on hold).
use super::*;

impl App {
    /// C# QuickFilterAsync: Enter in the search field or the toolbar quick search field.
    pub(super) fn quick_filter(&mut self, pane: usize, text: &str) {
        self.panes[pane].filter = ndr::parity::quick_filter_pattern(text);
        self.refresh(pane);
    }

    /// C# ShowFilterAsync = original miFilterCustomClick (0x441504): TfGetText(TITLE_FILTER, MSG_FILTER, current filter),
    /// applied with leading and trailing spaces trimmed, then reread. The original shows no status line notice (the C# one was removed).
    pub(super) fn show_filter_prompt(&mut self) {
        let pane = self.active;
        let title = self.language_pack.text("RES.TITLE_FILTER", "Filter");
        let message = self.language_pack.text("RES.MSG_FILTER", "Type a filter you want to use. You can put several filters separated with semi-colon(;)\nex) *.exe;*.txt;a??.dll");
        // The original passes the list's current mask, which is "*.*" when no filter is set (seen in the running original); "*.*" means no filter.
        // Previously the box started empty.
        let current = match self.panes[pane].filter.as_str() {
            "" => "*.*".to_owned(),
            filter => filter.to_owned(),
        };
        self.prompt(&title, &message, &current, None, false, move |app, value, _| {
            let Some(pattern) = value else { return };
            let pattern = pattern.trim();
            app.panes[pane].filter = if pattern == "*.*" { String::new() } else { pattern.to_owned() };
            app.refresh(pane);
        });
    }

    /// Quick search of the skin theme (original TNXCustomFileListView). True when handled.
    /// - The search text (+0x3e0) stays over time and is cleared when the cursor moves another way (arrows, Home/End, paging, clicks) or the list changes
    ///   (original ArrowKey 0x7d69dc, MouseDown 0x7d63c8, FileListChanged 0x7d79c8).
    /// - Characters are filtered by the WMChar (0x7d5a18) rules, and Backspace is taken only while there is search text (otherwise a shortcut like original FormShortCut 0x42b36c).
    pub(super) fn original_typed_search(&mut self, ctx: &egui::Context) -> bool {
        for pane in 0..2 {
            let cursor = self.panes[pane].cursor;
            // Original MouseDown (0x7d63c8) clears the search text on every item click (even the same item).
            let pressed = std::mem::take(&mut self.panes[pane].item_pressed);
            let search = &mut self.typed_search[pane];
            if !search.query.is_empty() && (pressed || search.cursor != Some(cursor)) {
                *search = TypedSearch::default();
            }
        }
        let pane = self.active;
        let has_text = !self.typed_search[pane].query.is_empty();
        // The original list window accepts IME input: the IME stays on while keys go to the list (composition window at the pane top left) and the composition text searches.
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

    /// One character of original WMChar plus QuickSearch (0x7d5f9c): appends to the search text (Backspace removes the last character) and searches from the current item;
    /// if not found and the new character is shorter than the search text, searches again from the next item with that character alone. If still not found, the search text is cleared.
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

    /// Original QuickSearch (0x7d5f9c): searches from the current item with the search text; if not found and the typed character is shorter than the search text, searches
    /// again from the next item with that character alone. If still not found, the search text is cleared.
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

    /// Original WMIMEComposition (0x7d584c): starting a composition appends the composing character to the search text, a changed composition replaces the last character,
    /// and an empty composition removes it. The result text (GCS_RESULTSTR) ends the composition. Each message runs QuickSearch(composing character).
    /// egui: Preedit = composing, Commit = result, Disabled (without result) = empty composition.
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
                // Fixed: the original cleared the search text when an intermediate composition character (for example "바" while typing "방") found nothing, which broke Korean search.
                // NDR keeps the text and the cursor during composition even without a match, and searches by the original rules when the result arrives.
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

    /// Original tbMainMenuAdvancedCustomDraw (0x430e74): when the current pane has search text, draws it at the right end of the menu bar (right - text width - 10)
    /// in upper case (FUN_00514870, a-z only), vertically centered, in the skin [Menu] mtext color.
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

/// Result of the original WMChar character filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OriginalSearchKey {
    /// '!' (0x21): clears the search text (SetSearchText("")).
    Clear,
    Back,
    Char(char),
}

/// Original WMChar (0x7d5a18): with Caps Lock off, upper case A-Z is not taken; with empty search text, Backspace and '.' are not taken either.
/// Accepted characters are '.', Backspace, and 0x21-0x7e except \ / : * ? (FUN_007d59bc). Spaces, Korean and so on are not taken.
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

/// Original QuickSearch(text, include current item) (0x7d5ca8). 0 for empty text. From the current item (the next one when not included, the first after the last) to the end
/// names with the same prefix (StrCmpNIW), then from the first item up to the current one when it did not start at the first, then the same order for names containing the text (StrStrIW).
/// When it starts at the first item, the containment search is skipped (as in the original).
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
