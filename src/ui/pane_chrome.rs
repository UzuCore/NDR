//! C# 패널 틀(MainWindow.xaml LeftPane): 머리 줄("왼쪽 패널", 선택 N개, 새 탭 +, ☰ 탭 목록, 트리), 탭 줄(UseTabs, 탭마다 ×,
//! 오른쪽 단추 메뉴 = 새 탭/탭 닫기/다른 탭 모두 닫기), 아래 주소 표시줄(ShowAddressBar: 경로 조각 단추 + ⌄ 하위 폴더, … 직접 입력,
//! ▼ 이동 기록, ↻ 새로 고침, PathClickMode), NavigateAddressAsync("압축!/접두" 포함).
//! 차이(HANDOFF): 탭마다 따로 이동 기록을 두지 않는다(패널 하나의 기록). 압축 안 경로 조각에는 하위 폴더 ⌄가 없다.
//! "트리" 단추는 C#의 패널별 폴더 트리 창 대신 Rust의 폴더 트리 칸을 켜고 끈다(폴더 트리 창은 보류).
use super::*;
use std::fs;

/// Rust가 압축 패널로 여는 확장자(open_current와 같다).
fn is_archive(path: &Path) -> bool {
    let extension = path.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    ["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "iso", "cab", "lzh"].contains(&extension.as_str())
}

impl App {

    /// 원본 pmTab(DFM): pmiCloseTab, pmiCloseAllTab, N02("-"), pmiNewTab. 닫기 두 항목은 C#처럼 탭이 둘 이상일 때만 켠다.
    /// 원본 탭 줄(FUN_0048a534)은 포인터 아래에 탭이 없으면(None) 닫기 두 항목을 끈다.
    pub(super) fn tab_actions(&self, ui: &mut egui::Ui, index: usize, tab: Option<usize>) -> Option<String> {
        let menu = |key: &str, fallback: &str| self.language_pack.text(&format!("formMain.Menu.{key}"), fallback);
        let ko = self.language == "ko";
        let several = self.panes[index].tabs.len() > 1 && tab.is_some();
        let tab = tab.unwrap_or(self.panes[index].tab);
        let mut chosen = None;
        if ui.add_enabled(several, egui::Button::new(menu("pmiCloseTab", if ko { "탭 닫기" } else { "Close Tab" }))).clicked() {
            chosen = Some(format!("closeTab:{tab}"));
        }
        if ui.add_enabled(several, egui::Button::new(menu("pmiCloseAllTab", if ko { "다른 탭 모두 닫기" } else { "Close All Other Tabs" }))).clicked() {
            chosen = Some(format!("closeOthers:{tab}"));
        }
        ui.separator();
        if ui.button(menu("pmiNewTab", if ko { "새 탭" } else { "New Tab" })).clicked() {
            chosen = Some(format!("newTabFrom:{tab}"));
        }
        if chosen.is_some() {
            ui.close();
        }
        chosen
    }

    /// 탭 명령(C#의 탭마다 PaneState: 탭마다 따로 뒤로/앞으로 기록을 둔다).
    pub(super) fn tab_command(&mut self, index: usize, command: &str, ctx: &egui::Context) {
        let number = |prefix: &str| command.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok());
        if let Some(t) = number("tab:") {
            return self.select_tab(index, t);
        }
        if let Some(t) = number("newTabFrom:") {
            // C# AddTab(side, state): 그 탭의 위치를 가진 새 탭(기록은 비어 있다).
            let path = self.new_tab_location(index, t);
            let pane = &mut self.panes[index];
            pane.tab_history.resize(pane.tabs.len(), Default::default());
            pane.tabs.push(path);
            pane.tab_history.push(Default::default());
            let last = pane.tabs.len() - 1;
            return self.select_tab(index, last);
        }
        if let Some(t) = number("closeTab:") {
            let pane = &mut self.panes[index];
            if pane.tabs.len() <= 1 || t >= pane.tabs.len() {
                return;
            }
            pane.tab_history.resize(pane.tabs.len(), Default::default());
            let selected = pane.tab == t;
            if !selected {
                // 지금 탭의 기록은 그대로 두고 닫는 탭만 뺀다.
                pane.tabs.remove(t);
                pane.tab_history.remove(t);
                if pane.tab > t {
                    pane.tab -= 1;
                }
                return;
            }
            pane.tabs.remove(t);
            pane.tab_history.remove(t);
            let next = t.min(pane.tabs.len() - 1);
            let (back, forward) = std::mem::take(&mut pane.tab_history[next]);
            pane.back = back;
            pane.forward = forward;
            pane.tab = next;
            return self.show_tab(index);
        }
        if let Some(t) = number("closeOthers:") {
            let pane = &mut self.panes[index];
            if pane.tabs.len() <= 1 || t >= pane.tabs.len() {
                return;
            }
            pane.tab_history.resize(pane.tabs.len(), Default::default());
            let current = pane.tab;
            pane.tab_history[current] = (std::mem::take(&mut pane.back), std::mem::take(&mut pane.forward));
            let keep = pane.tabs[t].clone();
            let (back, forward) = std::mem::take(&mut pane.tab_history[t]);
            pane.tabs = vec![keep];
            pane.tab_history = vec![Default::default()];
            pane.back = back;
            pane.forward = forward;
            pane.tab = 0;
            return self.show_tab(index);
        }
        self.dispatch(command, ctx);
    }

    /// 새 탭 위치. 원본 miNewTabClick(0x4420f4)·pmiNewTabClick(0x44229c)은 FTP 모드(2)에서 "c:\\\tc:\\"(0x8ede2a) 탭을 만든다(그 탭으로 가면 FTP가 끊긴다).
    pub(super) fn new_tab_location(&self, index: usize, tab: usize) -> PathBuf {
        let pane = &self.panes[index];
        if pane.remote.is_some() {
            return PathBuf::from("c:\\");
        }
        pane.tabs.get(tab).cloned().unwrap_or_else(|| pane.path.clone())
    }

    fn select_tab(&mut self, index: usize, tab: usize) {
        let pane = &mut self.panes[index];
        if pane.tab == tab || tab >= pane.tabs.len() {
            return;
        }
        pane.tab_history.resize(pane.tabs.len(), Default::default());
        let old = pane.tab.min(pane.tabs.len() - 1);
        pane.tab_history[old] = (std::mem::take(&mut pane.back), std::mem::take(&mut pane.forward));
        let (back, forward) = std::mem::take(&mut pane.tab_history[tab]);
        pane.back = back;
        pane.forward = forward;
        pane.tab = tab;
        self.show_tab(index);
    }

    /// 고른 탭의 위치를 연다(기록에 넣지 않는다).
    fn show_tab(&mut self, index: usize) {
        let pane = &mut self.panes[index];
        let Some(path) = pane.tabs.get(pane.tab).cloned() else { return };
        pane.archive = None;
        self.active = index;
        self.navigate(index, path, false);
        self.refresh(index);
    }

    /// C# 주소 표시줄(ShowAddressBar).
    pub(super) fn address_bar(&mut self, ui: &mut egui::Ui, index: usize, ctx: &egui::Context) {
        if !self.settings.flag("ShowAddressBar", true) {
            return;
        }
        let ko = self.language == "ko";
        let mode = self.settings.values.get("PathClickMode").and_then(|v| v.as_i64()).unwrap_or(0);
        ui.add_space(5.0);
        let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 26.0));
        let background = ui.interact(rect, egui::Id::new(("ndr-address-bar", index)), egui::Sense::click());
        let mut action: Option<AddressAction> = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            if self.panes[index].address_editing {
                let id = egui::Id::new(("ndr-address", index));
                let response = ui.add(egui::TextEdit::singleline(&mut self.panes[index].address).id(id).font(egui::FontId::proportional(11.0)).desired_width(f32::INFINITY));
                if response.lost_focus() {
                    action = Some(if ui.input(|i| i.key_pressed(egui::Key::Enter)) { AddressAction::Go } else { AddressAction::EndEdit });
                } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    action = Some(AddressAction::EndEdit);
                }
                return;
            }
            egui::Frame::group(ui.style()).corner_radius(5.0).inner_margin(egui::Margin::symmetric(2, 1)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("↻").on_hover_text(if ko { "현재 폴더 새로 고침" } else { "Refresh the current folder" }).clicked() {
                            action = Some(AddressAction::Refresh);
                        }
                        let history = !self.panes[index].back.is_empty() || !self.panes[index].forward.is_empty();
                        let button = ui.add_enabled(history, egui::Button::new("▼").small()).on_hover_text(if ko { "경로 기록" } else { "Path history" });
                        if button.clicked() {
                            action = Some(AddressAction::History(button.rect.left_bottom()));
                        }
                        if ui.small_button("…").on_hover_text(if ko { "경로 직접 입력" } else { "Type a path" }).clicked() {
                            action = Some(AddressAction::BeginEdit);
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            egui::ScrollArea::horizontal().id_salt(("ndr-crumbs", index)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 1.0;
                                    if let Some(chosen) = self.crumbs(ui, index) {
                                        action = Some(chosen);
                                    }
                                });
                            });
                        });
                    });
                });
            });
        });
        ui.add_space(4.0);
        // 원본 pmAddressBar: 주소 표시줄(경로 조각 위 포함)을 오른쪽 단추로 누르면 주소 복사·편집과 즐겨찾기 메뉴.
        let right_click = ui.input(|i| i.pointer.secondary_clicked().then(|| i.pointer.interact_pos()).flatten()).filter(|p| rect.contains(*p));
        if let Some(position) = right_click.filter(|_| !self.panes[index].address_editing) {
            return self.show_address_bar_menu(index, position);
        }
        // PathClickMode 1: 단추 밖을 누르면, 2: 어디를 눌러도(조각 단추 포함) 즐겨찾기 메뉴.
        let favorites = match (mode, &action) {
            (2, Some(AddressAction::Navigate(_))) => true,
            (1 | 2, None) => background.clicked(),
            _ => false,
        };
        if favorites && !self.panes[index].address_editing {
            self.active = index;
            return self.show_favorites_menu();
        }
        let Some(action) = action else { return };
        self.active = index;
        match action {
            AddressAction::Refresh => self.refresh(index),
            AddressAction::History(position) => self.show_history_menu(index, Some(position)),
            AddressAction::BeginEdit => self.begin_address_edit(index, ctx),
            AddressAction::EndEdit => self.end_address_edit(index),
            AddressAction::Go => {
                let text = self.panes[index].address.clone();
                self.end_address_edit(index);
                self.navigate_address(index, &text);
            }
            AddressAction::Navigate(target) => self.go_crumb(index, target),
        }
    }

    pub(super) fn end_address_edit(&mut self, index: usize) {
        let pane = &mut self.panes[index];
        pane.address_editing = false;
        pane.address = pane.path.to_string_lossy().into_owned();
    }

    /// C# RenderPathBreadcrumb: 압축이면 "압축 이름 › 폴더…", 아니면 "루트 › 폴더…"(지금 폴더는 굵게). 로컬 조각마다 ⌄(하위 폴더, 드라이브 루트는 드라이브 목록).
    fn crumbs(&mut self, ui: &mut egui::Ui, index: usize) -> Option<AddressAction> {
        let mut chosen = None;
        let current = ui.visuals().strong_text_color();
        let normal = ui.visuals().text_color();
        let separator = |ui: &mut egui::Ui| {
            ui.label(RichText::new("›").weak());
        };
        let crumb = |ui: &mut egui::Ui, label: &str, is_current: bool| {
            let text = RichText::new(label).size(11.0).color(if is_current { current } else { normal });
            ui.add(egui::Button::new(if is_current { text.strong().family(egui::FontFamily::Name(super::BOLD_FONT_FAMILY.into())) } else { text }).frame(false)).clicked()
        };
        if let Some(remote) = self.panes[index].remote.clone() {
            // 원본 FUN_0048aa30(FTP 모드): "/%s%s" 경로. 호스트 조각 = 루트.
            let parts: Vec<&str> = remote.path.split('/').filter(|p| !p.is_empty()).collect();
            if crumb(ui, &format!("ftp://{}", remote.host), parts.is_empty()) {
                chosen = Some(AddressAction::Navigate(CrumbTarget::Remote("/".into())));
            }
            let mut path = String::new();
            for (i, part) in parts.iter().enumerate() {
                separator(ui);
                path.push('/');
                path.push_str(part);
                if crumb(ui, part, i + 1 == parts.len()) {
                    chosen = Some(AddressAction::Navigate(CrumbTarget::Remote(path.clone())));
                }
            }
            return chosen;
        }
        if let Some(location) = self.panes[index].archive.clone() {
            let parts: Vec<&str> = location.prefix.split('/').filter(|p| !p.is_empty()).collect();
            let name = location.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // 원본 FUN_0048aa30(압축 모드): TNXPathLabel.SetPath(PathCombine(압축 파일, 안 경로)) — 압축 파일이 있는 폴더부터 보인다.
            // 그 폴더 조각을 누르면 압축에서 나와 그 폴더로 간다.
            let full = std::path::absolute(&location.file).unwrap_or_else(|_| location.file.clone());
            let mut folders: Vec<PathBuf> = full.parent().map(|p| p.ancestors().map(Path::to_path_buf).collect()).unwrap_or_default();
            folders.reverse();
            for (i, folder) in folders.iter().enumerate() {
                let label = if i == 0 { folder.to_string_lossy().into_owned() } else { folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() };
                if crumb(ui, &label, false) {
                    chosen = Some(AddressAction::Navigate(CrumbTarget::Local(folder.clone())));
                }
                separator(ui);
            }
            if crumb(ui, &name, parts.is_empty()) {
                chosen = Some(AddressAction::Navigate(CrumbTarget::Archive(String::new())));
            }
            let mut prefix = String::new();
            for (i, part) in parts.iter().enumerate() {
                separator(ui);
                prefix += part;
                prefix.push('/');
                if crumb(ui, part, i + 1 == parts.len()) {
                    chosen = Some(AddressAction::Navigate(CrumbTarget::Archive(prefix.clone())));
                }
            }
            return chosen;
        }
        // C#: "::drives::"는 "내 PC" 조각 하나(누르면 다시 읽기).
        if super::panes::is_drives(&self.panes[index].path) {
            let label = self.language_pack.text("RES.MYCOMPUTER", "My Computer");
            if crumb(ui, &label, true) {
                chosen = Some(AddressAction::Navigate(CrumbTarget::Local(PathBuf::from(super::remote_window::DRIVES))));
            }
            return chosen;
        }
        let full = std::path::absolute(&self.panes[index].path).unwrap_or_else(|_| self.panes[index].path.clone());
        let mut components = full.ancestors().map(Path::to_path_buf).collect::<Vec<_>>();
        components.reverse();
        let count = components.len();
        for (i, path) in components.into_iter().enumerate() {
            if i > 0 {
                separator(ui);
            }
            let label = if i == 0 { path.to_string_lossy().into_owned() } else { path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() };
            if crumb(ui, &label, i + 1 == count) {
                chosen = Some(AddressAction::Navigate(CrumbTarget::Local(path.clone())));
            }
            let drive_root = i == 0 && count == 1 && label.len() == 3 && label.as_bytes()[1] == b':';
            let tip = self.tr("하위 폴더", "Subfolders").into_owned();
            ui.menu_button(RichText::new("▼").size(10.0).weak(), |ui| {
                let children = self.crumb_children(index, &path, drive_root);
                if children.is_empty() {
                    ui.add_enabled(false, egui::Button::new(self.tr("하위 폴더 없음", "No subfolders")));
                }
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for (child, header) in children {
                        if ui.button(header).clicked() {
                            chosen = Some(AddressAction::Navigate(CrumbTarget::Local(child)));
                            ui.close();
                        }
                    }
                });
            })
            .response
            .on_hover_text(tip);
        }
        chosen
    }

    /// C# ShowLocalPathMenu: 하위 폴더(숨김은 숨김 표시일 때만) 또는 드라이브 루트면 드라이브 목록("이름 (C:)"). 열려 있는 동안 한 번만 읽는다.
    pub(super) fn crumb_children(&mut self, index: usize, path: &Path, drives: bool) -> Vec<(PathBuf, String)> {
        let key = (path.to_path_buf(), drives);
        if let Some(cached) = self.crumb_cache.get(&key) {
            return cached.clone();
        }
        let children: Vec<(PathBuf, String)> = if drives {
            native::drives()
                .into_iter()
                .map(|root| {
                    let name = root.to_string_lossy().trim_end_matches(['\\', '/']).to_owned();
                    let header = match native::volume_label(&root).filter(|l| !l.trim().is_empty()) {
                        Some(label) => format!("{label} ({name})"),
                        None => name,
                    };
                    (root, header)
                })
                .collect()
        } else {
            let hidden = self.panes[index].show_hidden;
            fs::read_dir(path)
                .map(|items| {
                    items
                        .filter_map(Result::ok)
                        .filter(|item| item.file_type().map(|t| t.is_dir()).unwrap_or(false))
                        .filter(|item| {
                            use std::os::windows::fs::MetadataExt;
                            hidden || item.metadata().map(|m| m.file_attributes() & 0x2 == 0).unwrap_or(true)
                        })
                        .map(|item| (item.path(), item.file_name().to_string_lossy().into_owned()))
                        .collect()
                })
                .unwrap_or_default()
        };
        if self.crumb_cache.len() > 64 {
            self.crumb_cache.clear();
        }
        self.crumb_cache.insert(key, children.clone());
        children
    }

    pub(super) fn go_crumb(&mut self, index: usize, target: CrumbTarget) {
        match target {
            CrumbTarget::Local(path) => {
                self.panes[index].archive = None;
                self.navigate(index, path, true);
            }
            CrumbTarget::Archive(prefix) => {
                if let Some(location) = self.panes[index].archive.as_mut() {
                    location.prefix = prefix;
                }
                self.refresh(index);
            }
            CrumbTarget::Remote(path) => {
                if let Some(remote) = self.panes[index].remote.as_mut() {
                    remote.path = path;
                    let address = remote.address();
                    self.panes[index].address = address;
                }
                self.refresh(index);
            }
        }
    }

    /// 원본 주소 칸 Enter(FUN_0048cb30): 글이 "http://"로 시작하면 기본 프로그램으로 열고(ShellExecute "open"), "ftp://"면 그 패널의
    /// FTP 빠른 연결(FUN_0043e9f8 → TfFTPQuickConnect, 호스트 칸 = 글), 그 밖은 그 폴더로 간다(FUN_00459968). 대소문자는 가리지 않는다.
    /// 고친 것: 원본은 "https://"를 폴더 이름으로 보아 갈 수 없다고 했다. 이제 "http://"처럼 연다.
    pub(super) fn original_address_enter(&mut self, index: usize, text: &str) {
        let value = text.trim();
        let lower = value.to_lowercase();
        self.active = index;
        if lower.starts_with("http://") || lower.starts_with("https://") {
            if let Err(error) = (self.shell_runner)(Path::new(value), "", Path::new("")) {
                self.status = error;
            }
            return;
        }
        if lower.starts_with("ftp://") {
            return self.ftp_quick_connect(value.to_owned());
        }
        self.navigate_address(index, value);
    }

    /// C# NavigateAddressAsync + ResolveLocalNavigationPath: 환경 변수, "~", 상대 경로(지금 폴더 기준), "압축!/접두".
    /// 없는 폴더는 "폴더 이동" 경고.
    pub(super) fn navigate_address(&mut self, index: usize, address: &str) {
        let value = native::expand_environment(address.trim().trim_matches('"'));
        let base = match &self.panes[index].archive {
            Some(location) => location.file.parent().map(Path::to_path_buf).unwrap_or_default(),
            None => self.panes[index].path.clone(),
        };
        let resolve = |text: &str| -> PathBuf {
            let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default());
            let path = if text == "~" {
                home
            } else if let Some(rest) = text.strip_prefix("~\\").or_else(|| text.strip_prefix("~/")) {
                home.join(rest)
            } else {
                PathBuf::from(text)
            };
            let joined = if path.is_absolute() { path } else { base.join(path) };
            std::path::absolute(&joined).unwrap_or(joined)
        };
        if let Some(at) = value.find("!/") {
            let archive = resolve(value[..at].trim().trim_matches('"'));
            if archive.is_file() && is_archive(&archive) {
                let prefix = value[at + 2..].replace('\\', "/").trim_matches('/').to_owned();
                let pane = &mut self.panes[index];
                pane.archive = Some(ArchiveLocation { file: archive, prefix: if prefix.is_empty() { prefix } else { format!("{prefix}/") }, password: String::new() });
                self.active = index;
                return self.refresh(index);
            }
        }
        let path = resolve(&value);
        let ko = self.language == "ko";
        // C# NavigateLocalAsync: UNC 공유가 자격 증명을 요구하면 Windows 연결 창, 취소면 그만, 실패면 오류.
        if !path.is_dir() {
            let code = (self.system.network_path_error)(&path);
            if matches!(code, 5 | 65 | 86 | 1326) {
                let connection = (self.system.connect_share)(&path.to_string_lossy());
                if connection == 1223 {
                    return;
                }
                if connection != 0 {
                    let message = native::win32_message(connection);
                    let text = if ko { format!("네트워크 공유 인증에 실패했습니다. Windows 오류 {connection}: {message}") } else { format!("Network share authentication failed. Windows error {connection}: {message}") };
                    return self.system_box(if ko { "폴더 이동" } else { "Go to folder" }, &text, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
                }
            }
        }
        if !path.is_dir() {
            let (title, text) = if ko { ("폴더 이동", format!("폴더를 찾을 수 없습니다: {}", path.display())) } else { ("Go to folder", format!("The folder was not found: {}", path.display())) };
            return self.system_box(title, &text, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
        }
        self.panes[index].archive = None;
        self.active = index;
        self.navigate(index, path, true);
    }
}

/// C# TabName: 폴더 이름(없으면 경로). "::drives::"는 원본 MYCOMPUTER 글.
pub(super) fn tab_name(path: &Path, my_computer: &str) -> String {
    if super::panes::is_drives(path) {
        return my_computer.to_owned();
    }
    path.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub(super) enum CrumbTarget {
    Local(PathBuf),
    Archive(String),
    /// 패널 FTP 모드의 원격 폴더.
    Remote(String),
}

enum AddressAction {
    Refresh,
    History(egui::Pos2),
    BeginEdit,
    EndEdit,
    Go,
    Navigate(CrumbTarget),
}

impl App {
    /// C# BuildContextMenu(목록 오른쪽 단추 메뉴): 메뉴 글은 [formMain] Menu.mip…/mi… 키, 열릴 때 C#과 같은 조건으로 켜고 끈다.
    pub(super) fn list_context_menu(&self, ui: &mut egui::Ui, index: usize) -> Option<&'static str> {
        let items = self.list_menu_items(index);
        self.draw_list_menu(ui, items)
    }

    /// pmMain 항목(글, 명령, 켜짐). None은 구분선(맨 앞·맨 뒤·겹친 구분선은 뺐다).
    pub(super) fn list_menu_items(&self, index: usize) -> Vec<Option<(String, &'static str, bool)>> {
        let pane = &self.panes[index];
        let rows: Vec<usize> = if pane.loading {
            vec![]
        } else if pane.selected.is_empty() {
            pane.entries.get(pane.cursor).map(|_| vec![pane.cursor]).unwrap_or_default()
        } else {
            pane.selected.iter().copied().filter(|i| *i < pane.entries.len()).collect()
        };
        let local = pane.archive.is_none();
        let has = !rows.is_empty();
        // 원본 pmMainPopup(0x437f54): 초점 항목이 파일(+0x3c == 3)이면 mipOpenWith·mipView·mipEdit를 보이고, 로컬 패널(종류 0)에서
        // 초점 항목이 폴더(1)일 때만 mipOpenInNewTab을 보인다. mipArcExtractAuto는 로컬 패널이면 켠다.
        let focused = (!pane.loading).then(|| pane.entries.get(pane.cursor)).flatten();
        let focused_file = focused.is_some_and(|e| !e.is_dir);
        let new_tab = local && focused.is_some_and(|e| e.is_dir) && !super::panes::is_drives(&pane.path);
        // 원본 pmMainPopup(0x437f54): mipEditPaste.Enabled = IsClipboardFormatAvailable(CF_HDROP), mipArcExtractAuto.Enabled =
        // 로컬 패널. 다른 항목은 늘 켜져 있다(메뉴는 로컬 패널에서 항목을 눌렀을 때만 뜬다, `list_menu_kind`).
        // 고친 것: 전에는 메뉴가 열린 동안 그림마다 클립보드를 열어 파일 목록을 읽었다(다른 프로그램의 클립보드 쓰기를 막고,
        // 같은 때 클립보드를 바꾸면 읽던 자료가 풀렸다). 원본처럼 형식이 있는지만 본다.
        let paste = local && (self.clipboard.is_some() || native::clipboard_has_files());
        let auto_extract = local;
        // 원본 mipExplorerMenu(+0x770)는 pmMainPopup이 건드리지 않아 늘 보인다.
        let explorer = local && has;
        // 원래 글(& 포함): 그리기 때 &를 빼고, 메뉴가 열린 동안 친 글자(menu_letter)와 같은 접근 글자의 항목을 고른다.
        let text = |key: &str, fallback: &str| self.language_pack.raw(&format!("formMain.Menu.{key}")).unwrap_or_else(|| fallback.to_owned());
        type Item<'a> = Option<(String, &'static str, bool)>;
        // 원본 pmMain(docs/original-menus.txt) 차례: 탐색기 메뉴 | 새 탭에서 열기 | 다른 프로그램으로 열기·보기·편집 | 복사·잘라내기·붙여넣기 |
        // 복사 위치·이동 위치 | 삭제·이름 바꾸기·속성/시간·설명 | 압축 추가·자동 풀기 | 속성.
        let mut items: Vec<Item> = Vec::new();
        if explorer {
            items.push(Some((text("mipExplorerMenu", "탐색기 메뉴"), "explorerMenu", true)));
            items.push(None);
        }
        // 이 목록에서 None은 구분선이므로 새 탭 항목은 있을 때만 넣는다.
        if new_tab {
            items.push(Some((text("mipOpenInNewTab", "새 탭에서 열기"), "openInNewTab", true)));
        }
        items.extend([None]);
        if focused_file {
            items.extend([
                Some((text("mipOpenWith", "다른 프로그램으로 열기…"), "openWith", true)),
                Some((text("mipView", "파일 보기"), "textView", true)),
                Some((text("mipEdit", "파일 편집"), "textEdit", true)),
                None,
            ]);
        }
        items.extend([
            Some((text("mipEditCopy", "복사"), "copy", local && has)),
            Some((text("mipEditCut", "잘라내기"), "cut", local && has)),
            Some((text("mipEditPaste", "붙여넣기"), "paste", paste)),
            None,
            Some((text("mipCopyTo", "복사 위치…"), "copyToFolder", has)),
            Some((text("mipMoveTo", "이동 위치…"), "moveToFolder", local && has)),
        ]);

        items.extend([
            None,
            Some((text("mipDelete", "삭제"), "deleteRecycle", local && has)),
            Some((text("mipRename", "이름 변경…"), "rename", local && has)),
            Some((text("mipChangeAttr", "속성/시간 변경…"), "attributes", local && has)),
            Some((text("mipDescription", "설명 추가/편집…"), "description", local && has)),
            None,
            Some((text("mipAddToArchive", "압축 파일에 추가…"), "archiveAdd", local && has)),
            Some((text("mipArcExtractAuto", "자동 압축 풀기"), "extractAuto", auto_extract)),
            None,
            Some((text("mipProperties", "속성"), "properties", local && has)),
        ]);

        // VCL 메뉴(AutoLineReduction = maAutomatic)처럼 맨 앞·맨 뒤·겹친 구분선은 그리지 않는다.
        let mut reduced: Vec<Item> = Vec::new();
        for item in items {
            if item.is_none() && reduced.last().is_none_or(Option::is_none) {
                continue;
            }
            reduced.push(item);
        }
        while reduced.last().is_some_and(Option::is_none) {
            reduced.pop();
        }
        reduced
    }

    fn draw_list_menu(&self, ui: &mut egui::Ui, items: Vec<Option<(String, &'static str, bool)>>) -> Option<&'static str> {
        let mut chosen = None;
        for item in items {
            match item {
                None => {
                    ui.separator();
                }
                Some((label, command, enabled)) => {
                    let (label, access) = super::favorites_dialog::access_key(&label);
                    let response = ui.add_enabled(enabled, egui::Button::new(label));
                    let letter_hit = enabled && chosen.is_none() && self.note_menu_target(access.map(|c| c.to_ascii_lowercase()), &response);
                    if response.clicked() || letter_hit {
                        chosen = Some(command);
                    }
                }
            }
        }
        if chosen.is_some() {
            ui.close();
        }
        chosen
    }

    /// C# QueueFolderSizeCalculation: Left/RightAutoFolderSize(C# 판에만 있음, 처음 값 꺼짐)이고 로컬 폴더 줄이면 크기를 따로(동시에 둘까지) 센다.
    /// 세는 동안 "…", 끝나면 크기(읽지 못한 곳이 있으면 "≥ 크기"). 새 목록이 오면 멈춘다.
    pub(super) fn queue_folder_size(&mut self, index: usize, row: usize) {
        // 고친 것: 원본 alwaysshowsize("항상 파일크기 보기", Left/RightAlwaysShowFolderSize)는 자동 목록에 크기 칸을 늘 두는 것뿐인데(UpdateDisplayAuto 0x390),
        // C# 판처럼 그 값으로 모든 폴더 크기를 세었다. 폴더 크기 자동 세기는 따로 둔다.
        if !self.settings.flag(if index == 0 { "LeftAutoFolderSize" } else { "RightAutoFolderSize" }, false) {
            return;
        }
        self.enqueue_folder_size(index, row);
    }

    /// 폴더 줄의 크기를 따로 센다(자동 표시와 원본 miCalculateSelectedFolderSizeClick이 함께 쓴다).
    pub(super) fn enqueue_folder_size(&mut self, index: usize, row: usize) {
        let pane = &mut self.panes[index];
        if pane.archive.is_some() || pane.remote.is_some() {
            return;
        }
        let Some(entry) = pane.entries.get(row).filter(|e| e.is_dir && !e.drive && !e.is_parent()) else { return };
        if pane.folder_sizes.contains_key(&entry.path) {
            return;
        }
        pane.folder_sizes.insert(entry.path.clone(), None);
        let job = FolderSizeJob {
            pane: index,
            generation: pane.generation,
            path: entry.path.clone(),
            cancel: pane.folder_cancel.clone(),
            unit: self.settings.text("SizeUnit", "Auto"),
            thousands: self.settings.flag("UseThousandsSeparator", true),
        };
        let (queue, signal) = &*self.folder_size_queue;
        queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(job);
        signal.notify_one();
        if !self.folder_size_workers {
            self.folder_size_workers = true;
            // C# _folderSizeSemaphore = new(2, 2).
            for _ in 0..2 {
                let shared = self.folder_size_queue.clone();
                let tx = self.async_tx.clone();
                thread::spawn(move || folder_size_worker(shared, tx));
            }
        }
    }
}

pub(super) struct FolderSizeJob {
    pane: usize,
    generation: u64,
    path: PathBuf,
    cancel: Arc<AtomicBool>,
    unit: String,
    thousands: bool,
}

pub(super) type FolderSizeQueue = Arc<(Mutex<std::collections::VecDeque<FolderSizeJob>>, std::sync::Condvar)>;

fn folder_size_worker(shared: FolderSizeQueue, tx: Sender<AsyncResult>) {
    let (queue, signal) = &*shared;
    loop {
        let job = {
            let mut jobs = queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = jobs.pop_front() {
                    break job;
                }
                jobs = signal.wait(jobs).unwrap_or_else(|e| e.into_inner());
            }
        };
        if job.cancel.load(Ordering::Relaxed) {
            continue;
        }
        let Some(summary) = ndr::tools::folder_size_summary(std::slice::from_ref(&job.path), &job.cancel) else { continue };
        let size = ndr::parity::format_size(summary.total_bytes, &job.unit, job.thousands);
        let text = if summary.errors.is_empty() { size } else { format!("≥ {size}") };
        if tx.send(AsyncResult::FolderSizeRow { pane: job.pane, generation: job.generation, path: job.path, text, bytes: summary.total_bytes }).is_err() {
            return;
        }
    }
}
