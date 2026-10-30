//! WPF DataGrid 자리의 egui 표: 머리글(누르면 정렬), 줄 선택(Ctrl/Shift, 위/아래/Home/End, Ctrl+A), Ctrl+C(선택 줄을 탭 글로),
//! 확인란 열. 폴더 비교 결과 창과 컴퓨터 청소 창이 같이 쓴다. 줄 데이터는 부르는 쪽이 갖고, 보이는 순서와 선택만 여기 둔다.
use super::*;
use std::{cmp::Ordering as Order, collections::BTreeSet, ops::Range};

/// 열 하나: 머리글, 너비, 오른쪽 맞춤, 확인란 열.
pub(super) struct Column {
    pub title: String,
    pub width: f32,
    pub right: bool,
    pub check: bool,
    /// 가운데 맞춤(TListColumn Alignment = taCenter).
    pub center: bool,
}

impl Column {
    pub(super) fn text(title: impl Into<String>, width: f32) -> Self {
        Self { title: title.into(), width, right: false, check: false, center: false }
    }
    pub(super) fn center(title: impl Into<String>, width: f32) -> Self {
        Self { title: title.into(), width, right: false, check: false, center: true }
    }
    pub(super) fn right(title: impl Into<String>, width: f32) -> Self {
        Self { title: title.into(), width, right: true, check: false, center: false }
    }
    pub(super) fn check(title: impl Into<String>, width: f32) -> Self {
        Self { title: title.into(), width, right: false, check: true, center: false }
    }
}

/// 보이는 순서(정렬), 선택한 줄(rows 번호), Shift 기준 줄, 글쇠로 움직이는 줄.
#[derive(Clone, Debug, Default)]
pub(super) struct GridState {
    pub order: Vec<usize>,
    pub sort: Option<(usize, bool)>,
    pub selected: BTreeSet<usize>,
    pub anchor: Option<usize>,
    pub cursor: Option<usize>,
    visible: Range<usize>,
    scroll_to: Option<usize>,
}

/// 한 장면에서 일어난 일. 확인란을 누른 줄은 부르는 쪽이 값을 바꾼다.
#[derive(Default)]
pub(super) struct GridEvents {
    pub sort: Option<usize>,
    pub toggled: Vec<usize>,
    pub copy: bool,
    /// 두 번 누른 줄(MouseDoubleClick).
    pub double_clicked: Option<usize>,
    /// 줄 오른쪽 단추 메뉴에서 고른 항목(context_menu가 돌려준 값).
    pub menu: Option<usize>,
    /// 표를 누름(초점이 이 표로 온다).
    pub clicked: bool,
}

/// 그리기 설정. multi_select = DataGridSelectionMode.Extended, sortable = CanUserSortColumns, enabled = IsEnabled.
pub(super) struct GridView<'a> {
    pub id: &'a str,
    pub columns: &'a [Column],
    pub height: f32,
    pub enabled: bool,
    pub sortable: bool,
    pub multi_select: bool,
    /// 글쇠(위아래·Home/End·Ctrl+A/C)를 받는가(한 창에 표가 둘이면 초점이 있는 표만).
    pub keyboard: bool,
    /// 줄 오른쪽 단추 메뉴(ContextMenu). 고르면 그 값을 GridEvents::menu로 돌려준다.
    pub context_menu: Option<&'a dyn Fn(&mut egui::Ui) -> Option<usize>>,
    /// 줄 글자 색(원본 OnAdvancedCustomDrawItem처럼 줄마다 Font.Color). None이면 보통 글자 색. 고른 줄은 선택 색.
    pub row_color: Option<&'a dyn Fn(usize) -> Option<Color32>>,
    /// 줄 바탕(원본 TListView Color). None이면 테마 바탕과 번갈이 줄.
    pub background: Option<Color32>,
}

pub(super) const ROW_HEIGHT: f32 = 22.0;

impl GridState {
    pub(super) fn new(count: usize) -> Self {
        Self { order: (0..count).collect(), ..Default::default() }
    }

    /// DataGrid 머리글: 다른 열이면 오름차순, 같은 열이면 방향을 바꾼다.
    pub(super) fn toggle_sort(&mut self, column: usize, compare: impl Fn(usize, usize) -> Order) {
        let descending = matches!(self.sort, Some((current, false)) if current == column);
        self.sort = Some((column, descending));
        let mut order: Vec<usize> = (0..self.order.len()).collect();
        order.sort_by(|&a, &b| if descending { compare(a, b).reverse() } else { compare(a, b) });
        self.order = order;
    }

    pub(super) fn select_all(&mut self) {
        self.selected = self.order.iter().copied().collect();
    }

    pub(super) fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// 선택한 줄(보이는 순서).
    pub(super) fn selected_in_order(&self) -> Vec<usize> {
        self.order.iter().copied().filter(|i| self.selected.contains(i)).collect()
    }

    /// 보이는 줄 하나를 고른다(Shift면 기준 줄부터, Ctrl이면 더하거나 빼기). 한 줄 선택이면 수식 키는 무시.
    pub(super) fn click(&mut self, display: usize, modifiers: egui::Modifiers, multi_select: bool) {
        let index = self.order[display];
        let anchor = self.anchor.and_then(|a| self.order.iter().position(|&r| r == a));
        if multi_select && modifiers.shift {
            if let Some(anchor) = anchor {
                if !modifiers.command {
                    self.selected.clear();
                }
                let (from, to) = (anchor.min(display), anchor.max(display));
                self.selected.extend(self.order[from..=to].iter().copied());
                self.cursor = Some(index);
                return;
            }
        }
        if multi_select && modifiers.command {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
        } else {
            self.selected.clear();
            self.selected.insert(index);
        }
        self.anchor = Some(index);
        self.cursor = Some(index);
    }

    /// DataGrid 복사(머리글 없음): 선택한 줄을 보이는 순서로, 칸은 탭, 줄은 CRLF, 확인란은 True/False.
    pub(super) fn copy_text(&self, columns: &[Column], text: &dyn Fn(usize, usize) -> String, checked: &dyn Fn(usize) -> bool) -> String {
        let mut out = String::new();
        for row in self.selected_in_order() {
            let cells: Vec<String> = columns.iter().enumerate().map(|(c, column)| if column.check { if checked(row) { "True".into() } else { "False".into() } } else { text(row, c) }).collect();
            out.push_str(&cells.join("\t"));
            out.push_str("\r\n");
        }
        out
    }
}

/// 표 그리기. text(row, column)은 글 열의 값, checked(row)는 확인란 열의 값.
pub(super) fn show(ui: &mut egui::Ui, view: &GridView, state: &mut GridState, text: &dyn Fn(usize, usize) -> String, checked: &dyn Fn(usize) -> bool) -> GridEvents {
    // 머리글과 줄 목록은 늘 위에서 아래로(부르는 쪽이 가로 배치 안에서 allocate_ui로 불러도 옆으로 늘어서지 않게).
    ui.vertical(|ui| show_rows_top_down(ui, view, state, text, checked)).inner
}

fn show_rows_top_down(ui: &mut egui::Ui, view: &GridView, state: &mut GridState, text: &dyn Fn(usize, usize) -> String, checked: &dyn Fn(usize) -> bool) -> GridEvents {
    let mut events = GridEvents::default();
    let total: f32 = view.columns.iter().map(|c| c.width).sum();
    let visuals = ui.visuals().clone();
    let font = egui::FontId::proportional(13.0);
    let weak = |color: Color32| if view.enabled { color } else { color.gamma_multiply(0.45) };

    let (header, _) = ui.allocate_exact_size(egui::vec2(total, 24.0), egui::Sense::hover());
    let mut x = header.left();
    for (c, column) in view.columns.iter().enumerate() {
        let cell = egui::Rect::from_min_size(egui::pos2(x, header.top()), egui::vec2(column.width, header.height()));
        let sense = if view.enabled && view.sortable { egui::Sense::click() } else { egui::Sense::hover() };
        let response = ui.interact(cell, ui.id().with((view.id, "header", c)), sense);
        let fill = if response.hovered() && view.sortable && view.enabled { visuals.widgets.hovered.weak_bg_fill } else { visuals.widgets.inactive.weak_bg_fill };
        ui.painter().rect_filled(cell.shrink2(egui::vec2(0.5, 0.0)), 0.0, fill);
        let arrow = match state.sort {
            Some((sorted, descending)) if sorted == c => if descending { " ▼" } else { " ▲" },
            _ => "",
        };
        ui.painter().with_clip_rect(cell.shrink2(egui::vec2(4.0, 0.0))).text(cell.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, format!("{}{arrow}", column.title), font.clone(), weak(visuals.strong_text_color()));
        if response.clicked() {
            events.sort = Some(c);
        }
        x += column.width;
    }

    if view.enabled && view.keyboard {
        let (modifiers, up, down, home, end, all, copy) = ui.input(|i| {
            (
                i.modifiers,
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::End),
                i.modifiers.command && i.key_pressed(egui::Key::A),
                i.events.iter().any(|e| matches!(e, egui::Event::Copy)) || (i.modifiers.command && i.key_pressed(egui::Key::C)),
            )
        });
        if !state.order.is_empty() && (up || down || home || end) {
            let current = state.cursor.and_then(|a| state.order.iter().position(|&r| r == a));
            let last = state.order.len() - 1;
            let next = if home {
                0
            } else if end {
                last
            } else {
                match (current, up) {
                    (None, _) => 0,
                    (Some(c), true) => c.saturating_sub(1),
                    (Some(c), false) => (c + 1).min(last),
                }
            };
            state.click(next, egui::Modifiers { shift: modifiers.shift, ..Default::default() }, view.multi_select);
            state.scroll_to = Some(next);
        }
        if all && view.multi_select {
            state.select_all();
        }
        events.copy = copy && !state.selected.is_empty();
    }

    let mut scroll = egui::ScrollArea::vertical().id_salt((view.id, "rows")).auto_shrink([false, false]).max_height(view.height).min_scrolled_height(view.height);
    if let Some(display) = state.scroll_to.take() {
        let shown = (view.height / ROW_HEIGHT).floor().max(1.0) as usize;
        if display < state.visible.start || display + 1 >= state.visible.end {
            let top = if display < state.visible.start { display } else { (display + 1).saturating_sub(shown) };
            scroll = scroll.vertical_scroll_offset(top as f32 * ROW_HEIGHT);
        }
    }
    let modifiers = ui.input(|i| i.modifiers);
    if let Some(background) = view.background {
        ui.painter().rect_filled(egui::Rect::from_min_size(header.left_bottom(), egui::vec2(total, view.height)), 0.0, background);
    }
    let mut clicked = None;
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if !view.enabled {
            ui.disable();
        }
        scroll.show_rows(ui, ROW_HEIGHT, state.order.len(), |ui, range| {
            state.visible = range.clone();
            for display in range {
                let row = state.order[display];
                let (rect, response) = ui.allocate_exact_size(egui::vec2(total, ROW_HEIGHT), egui::Sense::click());
                let selected = state.selected.contains(&row);
                if selected && view.enabled {
                    ui.painter().rect_filled(rect, 0.0, visuals.selection.bg_fill);
                } else if let Some(background) = view.background {
                    ui.painter().rect_filled(rect, 0.0, background);
                } else if display % 2 == 1 {
                    ui.painter().rect_filled(rect, 0.0, visuals.faint_bg_color);
                }
                let normal = view.row_color.and_then(|color| color(row)).unwrap_or(visuals.text_color());
                let color = weak(if selected { visuals.selection.stroke.color } else { normal });
                let mut x = rect.left();
                for (c, column) in view.columns.iter().enumerate() {
                    let cell = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(column.width, ROW_HEIGHT));
                    if column.check {
                        let mut value = checked(row);
                        if ui.put(egui::Rect::from_center_size(cell.center(), egui::vec2(20.0, 18.0)), egui::Checkbox::without_text(&mut value)).changed() {
                            events.toggled.push(row);
                        }
                    } else {
                        let (anchor, align) = if column.center {
                            (cell.center(), egui::Align2::CENTER_CENTER)
                        } else if column.right {
                            (cell.right_center() - egui::vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER)
                        } else {
                            (cell.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER)
                        };
                        ui.painter().with_clip_rect(cell.shrink2(egui::vec2(4.0, 0.0))).text(anchor, align, text(row, c), font.clone(), color);
                    }
                    x += column.width;
                }
                if response.clicked() {
                    clicked = Some(display);
                }
                if super::panes::windows_double_clicked(&response) {
                    events.double_clicked = Some(row);
                }
                if response.secondary_clicked() && !selected {
                    clicked = Some(display);
                }
                if let Some(menu) = view.context_menu.filter(|_| view.enabled) {
                    super::skin_pane::context_menu(&response, |ui| {
                        if let Some(choice) = menu(ui) {
                            events.menu = Some(choice);
                            ui.close();
                        }
                    });
                }
            }
        });
    });
    if let Some(display) = clicked.filter(|_| view.enabled) {
        state.click(display, modifiers, view.multi_select);
        events.clicked = true;
    }
    events
}
