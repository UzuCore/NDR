//! C# SelectDestinationFolder + DestinationPickerWindow([formNDRDestination]): 자주 쓴 대상(MostUsedDestinations, 많이·최근 순,
//! 있는 폴더만, MaxRecentFileCount개, 오른쪽 단추 메뉴로 지우기), 작업 폴더 1~5(숫자 글쇠), 다른 폴더 찾아보기.
//! RememberDestination(LastDestination + 사용 횟수·마지막 사용 시각)과 CopyMoveToFolderAsync.
//! 압축 파일 안에서 복사는 원본 FUN_00439bac처럼 압축 풀기 대상 선택(EXTRACT 제목)이고, 이동은 아무것도 하지 않는다.
use super::original_dialogs::{DialogKind, DialogResult};
use super::*;
use ndr::msgbox::{IDYES, MB_ICONASTERISK, MB_ICONQUESTION, MB_OK, MB_YESNO};

/// 복사·이동·압축 풀기(제목 글쇠).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DestinationPurpose {
    Copy,
    Move,
    /// 원본 miFTPGetClick(0x4412f8): FTP_GET + SELECTDEST.
    FtpGet,
    /// 압축 풀기 흐름(ExtractArchiveAsync) 이식 때 쓴다.
    #[allow(dead_code)]
    Extract,
}

/// 창의 글과 목록.
#[derive(Clone, Debug)]
pub(super) struct DestinationForm {
    pub most_used: Vec<(String, u64)>,
    pub work_folders: [String; 5],
    pub browse_label: String,
    pub cancel_label: String,
    pub remove_label: String,
    pub clear_label: String,
    pub initial: PathBuf,
    /// 원본 TfSelectDest cbPath: 고칠 수 있는 대상 경로(처음 값 = 다른 패널 폴더). 확인·Enter로 그 폴더(같은 패널 안 폴더도 됨).
    pub path: String,
    pub ok_label: String,
    /// 스킨 테마: 원본 TfSelectDest 배치(목록 lbList + 아래 cbPath·단추). 아니면 C# DestinationPickerWindow 배치.
    pub original: bool,
    /// 원본 lbList의 고른 줄(ItemIndex). 고르면 cbPath에 그 경로(lbListClick 0x453b40).
    pub selected: usize,
}

/// 원본 TfSelectDest 목록 줄(FUN_004532d4): 종류 1 = 처음 폴더(+0x420, 다른 패널), 2 = 최근(자주 쓴 폴더 +0x434), 4 = 작업 폴더(+0x438).
/// 종류 8(드라이브)은 드라이브 목록과 함께 옮기지 않았다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DestinationItem {
    Initial(String),
    Recent(String),
    Work(usize, String),
}

impl DestinationItem {
    pub(super) fn path(&self) -> &str {
        match self {
            DestinationItem::Initial(p) | DestinationItem::Recent(p) | DestinationItem::Work(_, p) => p,
        }
    }
}

/// 원본 FUN_004532d4의 목록: 처음 폴더(있으면), 그것과 다른 최근 폴더, 비어 있지 않고 있는 작업 폴더. (줄, 묶음의 첫 줄인지).
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
    items
}

pub(super) enum DestinationAction {
    /// 원본 btnOk: 경로 칸의 폴더.
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

/// 원본 lbListKeyDown(0x454130, 목록에 글쇠 초점): 1~9(또는 Alt+1~9) = 작업 폴더, Ctrl+1~9 = 최근 폴더 N번째(원본 +0x434는 처음 폴더를
/// 뺀 목록), 0 = 처음 폴더(+0x420), Shift+Delete = 기록 모두 지우기. 경로 칸에 글쇠 초점이 있으면(`typing`) 글쇠는 경로 글자다.
fn list_keys(ui: &egui::Ui, form: &DestinationForm, typing: bool) -> Option<DestinationAction> {
    enum Pick {
        Work(usize),
        MostUsed(usize),
        Initial,
        Clear,
    }
    const DIGITS: [egui::Key; 10] = [egui::Key::Num0, egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8, egui::Key::Num9];
    if typing {
        return None;
    }
    let typed = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Key { key: egui::Key::Delete, pressed: true, modifiers, .. } if modifiers.shift && !modifiers.ctrl && !modifiers.alt => Some(Pick::Clear),
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
        _ => None,
    }
}

/// 원본 TfSelectDest(DFM ClientWidth 426 × ClientHeight 428, 최소 너비 350): 위는 lbList(흰 바탕, 줄 높이 18, 묶음의 첫 줄은 + 3과
/// 위 0xC0C0C0 선), 아래 70 높이 칸에 cbPath(4,7 너비 − 8 × 21), btnTree(4,37 138×25), btnOk(너비 − 188,37 89×25, 기본),
/// btnCancel(너비 − 93,37 89×25, 취소). 크기는 FUN_004532d4처럼 줄 수 × 18 + 묶음 수 × 3 + 82(300 ~ 화면 − 50), 너비는 가장 긴 경로 + 28
/// (350 ~ 800). 줄 그리기는 lbListDrawItem(0x454de8): 아이콘 16(왼쪽 + 4), 글은 왼쪽 + 24에 상위 경로(회색 0x808080, 고르면 0xC0C0C0)와
/// 이름(검정, 고르면 흰색), 고른 줄은 0x5A5A5A→0x323232 바탕에 위 선 0x323232·0x787878. 작업 폴더 줄은 아이콘에 번호를 붙인다.
/// 누르면 고르고 경로 칸에 그 경로(lbListClick), 두 번 누르면 확인(OnDblClick = btnOkClick), 오른쪽 단추는 최근 줄이면 pmMUFolder.
/// 목록에 글쇠 초점이 있으면 위/아래로 고르고 Delete는 고른 최근 줄을 지운다(pmiMUFDeleteClick). btnTree(폴더 트리)는 옮기지 않아
/// 그 자리에 C# 판의 다른 폴더 찾아보기를 둔다.
pub(super) fn destination_ui_original(ui: &mut egui::Ui, form: &mut DestinationForm, enter: bool, icon: Option<&egui::TextureHandle>, screen_height: f32) -> Option<DestinationAction> {
    let mut action = None;
    let items = original_items(form);
    if form.selected >= items.len() {
        form.selected = 0;
    }
    let font = egui::TextStyle::Body.resolve(ui.style());
    let measure = |ui: &egui::Ui, text: &str| ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), Color32::BLACK).size().x);
    let widest = items.iter().map(|(item, _)| measure(ui, item.path())).fold(0.0f32, f32::max);
    let sections = items.iter().enumerate().filter(|(i, (_, first))| *i > 0 && *first).count();
    let width = (widest.ceil() + 28.0).clamp(350.0, 800.0);
    let height = (items.len() as f32 * 18.0 + sections as f32 * 3.0 + 82.0).clamp(300.0, (screen_height - 50.0).max(300.0));
    let (area, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let list_rect = egui::Rect::from_min_size(area.min, egui::vec2(width, height - 70.0));
    let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(area.min + egui::vec2(x, height - 70.0 + y), egui::vec2(w, h));
    let list_id = egui::Id::new("ndr-destination-list");
    let path_id = egui::Id::new("ndr-destination-path");
    ui.painter().rect_filled(list_rect, 0.0, Color32::WHITE);
    // 목록 자체(빈 자리 포함)가 글쇠 초점을 받는 위젯이다. 줄보다 먼저 두어 줄 누르기를 가리지 않게 한다.
    if ui.interact(list_rect, list_id, egui::Sense::click()).clicked() {
        ui.memory_mut(|m| m.request_focus(list_id));
    }
    let typing = ui.memory(|m| m.has_focus(path_id));
    let list_focused = ui.memory(|m| m.has_focus(list_id));
    if list_focused {
        // egui가 위/아래 화살표로 초점을 옮기지 않게 한다(원본 목록 상자는 화살표로 줄을 고른다).
        ui.memory_mut(|m| m.set_focus_lock_filter(list_id, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
    }
    // 목록 글쇠(위/아래, Delete).
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
            if let Some(icon) = icon {
                painter.image(icon.id(), icon_rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            if let DestinationItem::Work(n, _) = item {
                // 원본 +0x424 번호 그림 대신 작은 번호 칸.
                let badge = egui::Rect::from_min_size(icon_rect.right_bottom() - egui::vec2(8.0, 9.0), egui::vec2(9.0, 10.0));
                painter.rect_filled(badge, 1.0, Color32::from_rgb(0x20, 0x50, 0xa0));
                painter.text(badge.center(), egui::Align2::CENTER_CENTER, (n + 1).to_string(), egui::FontId::proportional(9.0), Color32::WHITE);
            }
            let path = item.path();
            // 원본: DT_PATH_ELLIPSIS | DT_MODIFYSTRING으로 줄 너비(왼쪽 + 24 ~ 오른쪽)에 맞춰 줄인 글을 상위 경로와 이름으로 나눈다.
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
            // 원본 lbListMouseDown(0x4546ac): 오른쪽 단추는 ItemIndex만 바꾼다(OnClick이 아니라 경로 칸은 그대로).
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
    // 아래 칸: cbPath(창이 열리면 글쇠 초점, 원본 FormShow), btnTree 자리의 찾아보기, btnOk, btnCancel.
    let edit = ui.put(at(4.0, 7.0, width - 8.0, 21.0), egui::TextEdit::singleline(&mut form.path).id(path_id));
    // 원본 cbPath Hint "Alt+D": Alt+D면 경로 칸으로 글쇠 초점.
    let alt_d = ui.input(|i| i.modifiers.alt && !i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::D));
    if alt_d || (!ui.memory(|m| m.had_focus_last_frame(path_id)) && ui.memory(|m| m.focused().is_none())) {
        edit.request_focus();
    }
    if ui.put(at(4.0, 37.0, 138.0, 25.0), egui::Button::new(&form.browse_label)).clicked() {
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
    /// MostUsedDestinations: UseCount 내림차순, LastUsedUtc 내림차순, 있는 폴더만, MaxRecentFileCount개.
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

    /// C# SelectDestinationFolder: 고른 폴더(전체 경로)를 콜백으로.
    pub(super) fn select_destination(&mut self, purpose: DestinationPurpose, initial_directory: PathBuf, current_pane: Option<PathBuf>, on_result: impl FnOnce(&mut App, PathBuf) + 'static) {
        let text = |app: &App, section: &str, key: &str, fallback: &str| app.language_pack.text(&format!("{section}.{key}"), fallback);
        let ko = self.language == "ko";
        // 원본(0x8ebad2·0x8ebb1e·0x8ed2b4 근처): 대상 선택 창 제목 = MOVE / COPY / EXTRACT + SELECTDEST(" - Select a destination").
        let (title_key, title_fallback) = match purpose {
            DestinationPurpose::Copy => ("RES.COPY", "Copy"),
            DestinationPurpose::Move => ("RES.MOVE", "Move"),
            DestinationPurpose::Extract => ("RES.EXTRACT", "Extract"),
            DestinationPurpose::FtpGet => ("RES.FTP_GET", "FTP Download"),
        };
        // GetDestinationInitialDirectory: DefaultDestinationMode 1이면 현재 패널 드라이브 루트.
        let fallback = if initial_directory.is_dir() { std::path::absolute(&initial_directory).unwrap_or(initial_directory) } else { documents() };
        let mode = self.settings.values.get("DefaultDestinationMode").and_then(|v| v.as_i64()).unwrap_or(0);
        let initial = match current_pane.filter(|_| mode == 1).and_then(|p| std::path::absolute(p).ok()).and_then(|p| p.ancestors().last().map(Path::to_path_buf)) {
            Some(root) if root.is_dir() => root,
            _ => fallback,
        };
        let work = self.settings.strings("WorkingFolders");
        let form = DestinationForm {
            most_used: self.most_used_destinations(),
            work_folders: std::array::from_fn(|i| work.get(i).map(|w| w.trim().to_owned()).unwrap_or_default()),
            browse_label: text(self, "formNDRDestination", "Button.Browse", if ko { "다른 폴더 찾아보기…" } else { "Browse for another folder…" }),
            cancel_label: text(self, "formNDRDestination", "Button.Cancel", "Cancel"),
            remove_label: text(self, "formSelectDest", "Menu.pmiMUFDelete", if ko { "기록에서 삭제" } else { "Remove from history" }),
            clear_label: text(self, "formSelectDest", "Menu.pmiClearMUFHistory", if ko { "기록 지우기" } else { "Clear history" }),
            path: initial.to_string_lossy().into_owned(),
            ok_label: text(self, "formSelectDest", "Caption.btnOk", "OK"),
            original: true,
            selected: 0,
            initial,
        };
        let title = format!("{}{}", self.language_pack.text(title_key, title_fallback), self.language_pack.text("RES.SELECTDEST", " - Select a destination"));
        self.push_original_dialog(title, DialogKind::Destination(Box::new(form)), move |app, result, _| {
            if let DialogResult::Destination(Some(path)) = result {
                on_result(app, path);
            }
        });
    }

    /// 창 단추.
    pub(super) fn destination_action(&mut self, action: DestinationAction, ctx: &egui::Context) {
        let title = self.original_dialogs.last().map(|d| d.title.clone()).unwrap_or_default();
        let folder_picker = self.folder_picker;
        let browse_title = self.language_pack.text("formNDRDestination.Browse.Title", "Choose destination folder");
        let Some(form) = self.destination_form() else { return };
        match action {
            DestinationAction::Cancel => self.answer_dialog(DialogResult::Destination(None), ctx),
            DestinationAction::Accept => {
                let typed = form.path.trim().trim_matches('"').to_owned();
                if typed.is_empty() {
                    return;
                }
                // 상대 경로는 지금 패널 폴더 기준(원본 PathCombine).
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
                let initial = form.initial.clone();
                if let Some(folder) = folder_picker(&browse_title, Some(&initial)) {
                    let full = std::path::absolute(&folder).unwrap_or(folder);
                    self.answer_dialog(DialogResult::Destination(Some(full)), ctx);
                }
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

    /// C# RememberDestination: 있는 폴더면 LastDestination과 MostUsedDestinations(사용 횟수 +1, 마지막 사용 UTC)를 저장.
    pub(super) fn remember_destination(&mut self, destination: &Path) {
        if !destination.is_dir() {
            return;
        }
        let full = std::path::absolute(destination).unwrap_or_else(|_| destination.to_path_buf()).to_string_lossy().into_owned();
        let full = if full.len() > 3 { full.trim_end_matches(['\\', '/']).to_owned() } else { full };
        self.settings.set_text("LastDestination", &full);
        // System.Text.Json DateTime(UTC): 소수 7자리.
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

    /// 원본 miCopyToClick(0x432e68)·miMoveToClick(0x432f40): 로컬 위치면 FUN_0042bdc4 = TfSelectDest(경로 칸 처음 값은 FUN_0043cb24).
    /// 압축 파일 안이면 복사는 FUN_00439bac(형식 0) = 고른 항목을 TfSelectDest(EXTRACT 제목)로 고른 폴더에 풀고,
    /// 이동은 FUN_0042bdc4가 로컬이 아니면 그만이므로 아무것도 하지 않는다(C# 판의 "이동할 수 없습니다" 알림은 없다).
    pub(super) fn copy_move_to_folder(&mut self, moving: bool) {
        let i = self.active;
        if self.panes[i].archive.is_some() {
            if !moving {
                self.extract_command(super::archive_dialog::ExtractMode::Selected);
            }
            return;
        }
        let targets = self.panes[i].targets();
        if targets.is_empty() {
            return;
        }
        let other = &self.panes[1 - i];
        // 원본 FUN_0043cb24: 두 패널이 보이고 다른 패널이 로컬이면 그 폴더, 아니면 지금 패널 폴더.
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
