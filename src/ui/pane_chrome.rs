//! C# pane frame (MainWindow.xaml LeftPane): header row ("left pane", N selected, new tab +, ☰ tab list, tree), tab row (UseTabs, × per tab,
//! right-click menu = new tab / close tab / close all other tabs), address bar below (ShowAddressBar: path segment buttons + ⌄ subfolders, ... type a path,
//! ▼ history, ↻ refresh, PathClickMode), NavigateAddressAsync (including "archive!/prefix").
//! Difference (HANDOFF): tabs do not keep separate histories (one history per pane). Path segments inside archives have no ⌄ subfolder button.
//! The "tree" button toggles the Rust folder tree area instead of the C# per-pane folder tree window (the folder tree window is on hold).
use super::*;
use std::fs;

fn pane_text(language: &compat::Language, key: &str, en: &str, ko: &str) -> String {
    language.text(key, if language.code.eq_ignore_ascii_case("ko") { ko } else { en })
}

/// Extensions Rust opens as archive panes (same as open_current).
fn is_archive(path: &Path) -> bool {
    let extension = path.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    ["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "iso", "cab", "lzh"].contains(&extension.as_str())
}

impl App {

    /// Original pmTab (DFM): pmiCloseTab, pmiCloseAllTab, N02 ("-"), pmiNewTab. The two close items are enabled only with two or more tabs, like C#.
    /// The original tab row (FUN_0048a534) disables the two close items when no tab is under the pointer (None).
    pub(super) fn tab_actions(&self, ui: &mut egui::Ui, index: usize, tab: Option<usize>) -> Option<String> {
        let menu = |key: &str, fallback: &str| self.language_pack.text(&format!("formMain.Menu.{key}"), fallback);
        let ko = self.language == "ko";
        let several = self.panes[index].tabs.len() > 1 && tab.is_some();
        let tab = tab.unwrap_or(self.panes[index].tab);
        let mut chosen = None;
        if ui.add_enabled(several, egui::Button::new(menu("pmiCloseTab", if ko { "탭 닫기" } else { "Close Tab" }))).clicked() {
            chosen = Some(format!("closeTab:{tab}"));
        }
        if ui.add_enabled(several, egui::Button::new(pane_text(&self.language_pack, "NDR.UI_PANE_CLOSE_OTHER_TABS", "Close other tabs", "다른 탭 모두 닫기"))).clicked() {
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

    /// Tab commands (C# PaneState per tab: each tab keeps its own back/forward history).
    pub(super) fn tab_command(&mut self, index: usize, command: &str, ctx: &egui::Context) {
        let number = |prefix: &str| command.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok());
        if let Some(t) = number("tab:") {
            return self.select_tab(index, t);
        }
        if let Some(t) = number("newTabFrom:") {
            // C# AddTab(side, state): a new tab with that tab's location (empty history).
            let path = self.new_tab_location(index, t);
            let pane = &mut self.panes[index];
            pane.tab_history.resize(pane.tabs.len(), Default::default());
            pane.tabs.push(path);
            pane.tab_history.push(Default::default());
            let last = pane.tabs.len() - 1;
            return self.select_tab(index, last);
        }
        if let Some(t) = number("closeTab:") {
            // Original pmiCloseTabClick (0x442578) -> FUN_00442414, the × button TNXTabSet.CloseTabClick (0x7d17c8): after deleting a tab, TabsChanged (0x7d161c)
            // keeps TabIndex at the same number clamped to [0, count - 1]. So closing a tab before the current one shifted the mark to the next tab;
            // closing from the menu switched to that tab (OnChange), while closing with × only moved the mark and left the pane as is (mark and pane out of sync).
            // Fixed: NDR keeps the current tab when an earlier tab closes. Closing the current tab goes to the tab with the same number (or the last) like the original.
            // FUN_0048a3dc (tab removal): with one tab nothing happens; otherwise the tab row stays visible only while more than one tab
            // will remain (Visible = count > 2 before removal), then the tab object is freed and removed.
            let pane = &mut self.panes[index];
            if pane.tabs.len() <= 1 || t >= pane.tabs.len() {
                return;
            }
            pane.tab_history.resize(pane.tabs.len(), Default::default());
            let selected = pane.tab == t;
            if !selected {
                // Keep the current tab's history and remove only the closed tab.
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
            // Original pmiCloseAllTabClick (0x4425d8): the tab under the popup point (IndexOfTabAt) is kept with its object, every other tab
            // object is freed, the list is cleared and the kept tab added again (TabIndex 0), and the tab row stays visible only with 2+ tabs.
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

    /// Original mipOpenInNewTabClick (0x4453b4): with tabs on (+0xf3) and a local pane (FUN_00481948 == 0), appends a tab ("%s|%s") for the parent of the current folder
    /// (PathRemoveFileSpecW) when the focused item is "..", otherwise current folder\name, and switches to it (SetTabIndex -> FUN_00489d20).
    /// Nothing happens with tabs off or in archive/FTP panes. Fixed: previously it created the tab and then opened the item, so with tabs off it
    /// opened in the current tab, and files were run (the original goes to the file's folder and puts the cursor on it).
    pub(super) fn open_in_new_tab(&mut self, index: usize) {
        if !self.settings.flag("UseTabs", true) {
            return;
        }
        let pane = &self.panes[index];
        if pane.archive.is_some() || pane.remote.is_some() || pane.loading {
            return;
        }
        let Some(entry) = pane.entries.get(pane.cursor) else { return };
        let path = if entry.is_parent() { pane.path.parent().map(Path::to_path_buf).unwrap_or_else(|| pane.path.clone()) } else { entry.path.clone() };
        let pane = &mut self.panes[index];
        pane.tab_history.resize(pane.tabs.len(), Default::default());
        pane.tabs.push(path);
        pane.tab_history.push(Default::default());
        let last = pane.tabs.len() - 1;
        self.select_tab(index, last);
    }

    /// Location of a new tab. Original miNewTabClick (0x4420f4) and pmiNewTabClick (0x44229c) create a "c:\\\tc:\\" (0x8ede2a) tab in FTP mode (2) (switching to it disconnects FTP).
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

    /// Opens the chosen tab's location (without adding to history). Original FUN_00489d20 (tab switch): an existing location goes to the folder (for a file, its folder with the cursor
    /// on the file); otherwise an existing parent; if none exists, the tab is removed. Fixed: previously it tried to open missing locations as is and read the same location twice.
    fn show_tab(&mut self, index: usize) {
        self.discard_vault(index);
        if self.panes[index].vault.is_some() { return; }
        let pane = &mut self.panes[index];
        let Some(path) = pane.tabs.get(pane.tab).cloned() else { return };
        pane.archive = None;
        self.active = index;
        let special = super::panes::is_drives(&path) || path.to_string_lossy().starts_with("ftp://") || path.to_string_lossy().starts_with("ftps://") || path.to_string_lossy().starts_with("sftp://");
        let mut target = path.clone();
        if !special {
            while !target.exists() {
                match target.parent() {
                    Some(parent) if parent != target && !parent.as_os_str().is_empty() => target = parent.to_path_buf(),
                    _ => break,
                }
            }
            if !target.exists() {
                let tab = self.panes[index].tab;
                if self.panes[index].tabs.len() > 1 {
                    let ctx = egui::Context::default();
                    return self.tab_command(index, &format!("closeTab:{tab}"), &ctx);
                }
            } else if target.is_file() {
                self.pending_focus[index] = target.file_name().map(|n| n.to_string_lossy().into_owned());
                target = target.parent().map(Path::to_path_buf).unwrap_or(target);
            }
        }
        if target == self.panes[index].path {
            self.refresh(index);
        } else {
            self.navigate(index, target, false);
        }
    }

    /// C# address bar (ShowAddressBar).
    pub(super) fn address_bar(&mut self, ui: &mut egui::Ui, index: usize, ctx: &egui::Context) {
        if !self.settings.flag("ShowAddressBar", true) {
            return;
        }
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
                        if ui.small_button("↻").on_hover_text(pane_text(&self.language_pack, "NDR.UI_PANE_REFRESH_FOLDER", "Refresh the current folder", "현재 폴더 새로 고침")).clicked() {
                            action = Some(AddressAction::Refresh);
                        }
                        let history = !self.panes[index].back.is_empty() || !self.panes[index].forward.is_empty();
                        let button = ui.add_enabled(history, egui::Button::new("▼").small()).on_hover_text(pane_text(&self.language_pack, "RES.FUNC_SHOWPATHHISTORY", "Path history", "경로 기록"));
                        if button.clicked() {
                            action = Some(AddressAction::History(button.rect.left_bottom()));
                        }
                        if ui.small_button("…").on_hover_text(pane_text(&self.language_pack, "NDR.UI_PANE_TYPE_PATH", "Type a path", "경로 직접 입력")).clicked() {
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
        // Original pmAddressBar: right-clicking the address bar (including over path segments) shows copy/edit address and the favorites menu.
        let right_click = ui.input(|i| i.pointer.secondary_clicked().then(|| i.pointer.interact_pos()).flatten()).filter(|p| rect.contains(*p));
        if let Some(position) = right_click.filter(|_| !self.panes[index].address_editing) {
            return self.show_address_bar_menu(index, position);
        }
        // PathClickMode 1: clicking outside the buttons, 2: clicking anywhere (including segment buttons) shows the favorites menu.
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
        pane.address = if let Some(vault) = &pane.vault {
            vault.virtual_path().to_string_lossy().into_owned()
        } else { pane.path.to_string_lossy().into_owned() };
    }

    /// C# RenderPathBreadcrumb: "archive name › folder..." for archives, otherwise "root › folder..." (current folder bold). Each local segment has ⌄ (subfolders, the drive list at a drive root).
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
        if let Some(vault) = &self.panes[index].vault {
            let labels: Vec<String> = std::iter::once(vault.file.file_stem().unwrap_or_default().to_string_lossy().into_owned())
                .chain(vault.stack.iter().skip(1).map(|(_, name)| name.clone())).collect();
            for (depth, label) in labels.iter().enumerate() {
                if depth > 0 { separator(ui); }
                if crumb(ui, label, depth + 1 == labels.len()) {
                    if let Some(vault) = self.panes[index].vault.as_mut() {
                        vault.stack.truncate(depth + 1);
                    }
                    self.refresh_vault(index);
                }
            }
            return None;
        }
        if let Some(remote) = self.panes[index].remote.clone() {
            // Original FUN_0048aa30 (FTP mode): "/%s%s" path. The host segment = root.
            let parts: Vec<&str> = remote.path.split('/').filter(|p| !p.is_empty()).collect();
            if crumb(ui, &remote.endpoint(), parts.is_empty()) {
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
            // Original FUN_0048aa30 (archive mode): TNXPathLabel.SetPath(PathCombine(archive file, inner path)), shown from the folder containing the archive.
            // Clicking that folder segment leaves the archive and goes to that folder.
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
        // C#: "::drives::" is one "My PC" segment (clicking rereads).
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
            let tip = pane_text(&self.language_pack, "NDR.UI_PANE_SUBFOLDERS", "Subfolders", "하위 폴더");
            ui.menu_button(RichText::new("▼").size(10.0).weak(), |ui| {
                let children = self.crumb_children(index, &path, drive_root);
                if children.is_empty() {
                    ui.add_enabled(false, egui::Button::new(pane_text(&self.language_pack, "NDR.UI_PANE_NO_SUBFOLDERS", "No subfolders", "하위 폴더 없음")));
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

    /// C# ShowLocalPathMenu: subfolders (hidden ones only when hidden items show), or the drive list ("name (C:)") at a drive root. Read once while open.
    /// Original FUN_0048c548 by pane kind (FUN_00481948): local = drives or FindFirstFileW subfolders; FTP = the leading ▼ adds nothing and
    /// ">" asks the enumerator for the remote folder's subfolders (FUN_0045c948, body 0x45c94e: hourglass cursor; FTP changes to the folder,
    /// lists with "AList" and keeps entries that are not files, then returns to the pane folder; SFTP SetCurrentDir + ListDir keeps type 2,
    /// then TStringList Sorted); archive = nothing. An empty list shows no menu. Fixed: "." and ".." from servers that list them are dropped.
    pub(super) fn crumb_children(&mut self, index: usize, path: &Path, drives: bool) -> Vec<(PathBuf, String)> {
        // Remote paths are per connection, so their cache key carries the pane (the cache is cleared whenever a listing arrives).
        let key = match &self.panes[index].remote {
            Some(_) => (PathBuf::from(format!("\u{1}ftp{index}{}", path.to_string_lossy())), drives),
            None => (path.to_path_buf(), drives),
        };
        if let Some(cached) = self.crumb_cache.get(&key) {
            return cached.clone();
        }
        if self.panes[index].archive.is_some() || (drives && self.panes[index].remote.is_some()) {
            return Vec::new();
        }
        if let Some(remote) = &self.panes[index].remote {
            let folder = path.to_string_lossy().replace('\\', "/");
            // The listing shares the pane's connection; while a transfer holds it the menu stays closed instead of freezing the window.
            let Ok(mut fs) = remote.fs.try_lock() else { return Vec::new() };
            let Ok(items) = fs.list(&folder) else { return Vec::new() };
            drop(fs);
            let mut children: Vec<(PathBuf, String)> = items
                .into_iter()
                .filter(|e| e.is_dir && e.name != "." && e.name != "..")
                .map(|e| (PathBuf::from(super::ftp_pane::join_remote(&folder, &e.name)), e.name))
                .collect();
            children.sort_by(|a, b| a.1.to_uppercase().cmp(&b.1.to_uppercase()));
            self.crumb_cache.insert(key, children.clone());
            return children;
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
                        .collect::<Vec<_>>()
                })
                .map(|mut children| {
                    // Original FUN_0048c548 uses FindFirstFileW/FindNextFileW order (NTFS = upper-cased name order). Fixed: read_dir order
                    // is unspecified and could differ from name order.
                    children.sort_by(|a, b| a.1.to_uppercase().cmp(&b.1.to_uppercase()));
                    children
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

    /// Original address field Enter (FUN_0048cb30): text starting with "http://" opens in the default program (ShellExecute "open"), "ftp://" starts that pane's
    /// FTP quick connect (FUN_0043e9f8 -> TfFTPQuickConnect, host field = text), anything else goes to that folder (FUN_00459968). Case-insensitive.
    /// Fixed: the original treated "https://" as a folder name and could not go there. Now it opens like "http://".
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
        if lower.starts_with("ftp://") || lower.starts_with("ftps://") || lower.starts_with("sftp://") {
            return self.ftp_quick_connect(value.to_owned());
        }
        self.navigate_address(index, value);
    }

    /// C# NavigateAddressAsync + ResolveLocalNavigationPath: environment variables, "~", relative paths (from the current folder), "archive!/prefix".
    /// A missing folder warns "go to folder".
    pub(super) fn navigate_address(&mut self, index: usize, address: &str) {
        let value = native::expand_environment(address.trim().trim_matches('"'));
        let base = match &self.panes[index].archive {
            Some(location) => location.file.parent().map(Path::to_path_buf).unwrap_or_default(),
            None => self.panes[index].path.clone(),
        };
        let resolve = |text: &str| -> PathBuf {
            // Original TNXFileSystemEnumerator::SetPath 0x45a10c: a relative path (or "..") is entered from the pane folder with
            // SetCurrentDirectoryW and read back with GetCurrentDirectoryW; std::path::absolute (GetFullPathNameW) gives the same full path.
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
        // C# NavigateLocalAsync: a UNC share requiring credentials shows the Windows connect window; cancel stops, failure reports an error.
        if !path.is_dir() {
            let code = (self.system.network_path_error)(&path);
            if matches!(code, 5 | 65 | 86 | 1326) {
                let connection = (self.system.connect_share)(&path.to_string_lossy());
                if connection == 1223 {
                    return;
                }
                if connection != 0 {
                    let message = native::win32_message(connection);
                    let fallback = if ko { "네트워크 공유 인증에 실패했습니다. Windows 오류 {0}: {1}" } else { "Network share authentication failed. Windows error {0}: {1}" };
                    let text = super::file_commands::operation_format(&self.language_pack, "NDR.UI_PANE_SHARE_AUTH_FAILED", fallback, &[&connection, &message]);
                    let title = pane_text(&self.language_pack, "NDR.UI_PANE_GO_TO_FOLDER", "Go to folder", "폴더 이동");
                    return self.system_box(&title, &text, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
                }
            }
        }
        if !path.is_dir() {
            let title = pane_text(&self.language_pack, "NDR.UI_PANE_GO_TO_FOLDER", "Go to folder", "폴더 이동");
            let fallback = if ko { "폴더를 찾을 수 없습니다: {0}" } else { "The folder was not found: {0}" };
            let text = super::file_commands::operation_format(&self.language_pack, "NDR.UI_PANE_FOLDER_NOT_FOUND", fallback, &[&path.display()]);
            return self.system_box(&title, &text, ndr::msgbox::MB_OK | ndr::msgbox::MB_ICONEXCLAMATION, |_, _, _| {});
        }
        self.panes[index].archive = None;
        self.active = index;
        self.navigate(index, path, true);
    }
}

/// C# TabName: folder name (the path when none). "::drives::" uses the original MYCOMPUTER text.
pub(super) fn tab_name(path: &Path, my_computer: &str) -> String {
    if super::panes::is_drives(path) {
        return my_computer.to_owned();
    }
    path.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub(super) enum CrumbTarget {
    Local(PathBuf),
    Archive(String),
    /// Remote folder of the pane FTP mode.
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
    /// C# BuildContextMenu (list right-click menu): menu texts use [formMain] Menu.mip.../mi... keys, enabled and disabled on open by the same conditions as C#.
    pub(super) fn list_context_menu(&self, ui: &mut egui::Ui, index: usize) -> Option<&'static str> {
        let items = self.list_menu_items(index);
        self.draw_list_menu(ui, items)
    }

    /// pmMain items (text, command, enabled). None is a separator (leading, trailing and doubled separators removed).
    pub(super) fn list_menu_items(&self, index: usize) -> Vec<Option<(String, &'static str, bool)>> {
        let pane = &self.panes[index];
        if pane.remote.is_some() { return self.ftp_list_menu_items(index); }
        let rows: Vec<usize> = if pane.loading {
            vec![]
        } else if pane.selected.is_empty() {
            pane.entries.get(pane.cursor).map(|_| vec![pane.cursor]).unwrap_or_default()
        } else {
            pane.selected.iter().copied().filter(|i| *i < pane.entries.len()).collect()
        };
        let local = pane.archive.is_none();
        let archive = pane.archive.is_some();
        let writable_archive = archive && self.editable_archive(index).is_some();
        let has = !rows.is_empty();
        // Original pmMainPopup (0x437f54): when the focused item is a file (+0x3c == 3) shows mipOpenWith, mipView and mipEdit; shows mipOpenInNewTab only on a local pane (kind 0)
        // when the focused item is a folder (1). mipArcExtractAuto is enabled on local panes.
        let focused = (!pane.loading).then(|| pane.entries.get(pane.cursor)).flatten();
        let focused_file = focused.is_some_and(|e| !e.is_dir);
        // Original 0x438003: mipOpenInNewTab.Visible = pane kind 0 (local) && focused item kind 1 (folder). Independent of the tabs setting (the handler checks it on click).
        // Drive rows of the My PC view are original kind 4, so it is not shown.
        let new_tab = local && pane.remote.is_none() && focused.is_some_and(|e| e.is_dir && !e.drive) && !super::panes::is_drives(&pane.path);
        // Original pmMainPopup (0x437f54): mipEditPaste.Enabled = IsClipboardFormatAvailable(CF_HDROP), mipArcExtractAuto.Enabled =
        // local pane. Other items are always enabled (the menu appears only when an item is clicked on a local pane, `list_menu_kind`).
        // Fixed: previously every frame while the menu was open opened the clipboard and read the file list (blocking other programs' clipboard writes,
        // and a clipboard change at the same time released the data being read). Like the original it only checks whether the format exists.
        let paste = (local || writable_archive) && (self.vault_clipboard.is_some() || self.clipboard.is_some() || self.archive_clipboard.is_some() || native::clipboard_has_files());
        let auto_extract = local;
        // Original mipExplorerMenu (+0x770) is untouched by pmMainPopup, so it is always visible.
        let explorer = local && has;
        // Original text (with &): & is removed when drawing, and a letter typed while the menu is open (menu_letter) picks the item with that access letter.
        let text = |key: &str, fallback: &str| self.language_pack.raw(&format!("formMain.Menu.{key}")).unwrap_or_else(|| fallback.to_owned());
        type Item<'a> = Option<(String, &'static str, bool)>;
        // NDR context-menu order: Explorer menu | open in new tab | open with, view, edit | copy, cut, paste |
        // copy to, move to | delete, rename, attributes/time, description | add to archive, auto extract | properties.
        let mut items: Vec<Item> = Vec::new();
        if explorer {
            items.push(Some((text("mipExplorerMenu", "탐색기 메뉴"), "explorerMenu", true)));
            items.push(None);
        }
        // None is a separator in this list, so the new tab item is added only when present.
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
            Some((text("mipEditCopy", "복사"), "copy", (local || archive) && has)),
            Some((text("mipEditCut", "잘라내기"), "cut", (local || writable_archive) && has)),
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

        // Like VCL menus (AutoLineReduction = maAutomatic), leading, trailing and doubled separators are not drawn.
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

    /// Remote paths are not Windows files. Only expose commands which
    /// have a real remote implementation, even when Explorer-menu mode
    /// is enabled for local panels.
    fn ftp_list_menu_items(&self, index: usize) -> Vec<Option<(String, &'static str, bool)>> {
        let pane = &self.panes[index];
        let remote = pane.remote.as_ref().expect("remote menu");
        let idle = !remote.busy.load(Ordering::Relaxed) && !pane.loading && !pane.ftp_connect_pending;
        let valid = |e: &Entry| !e.is_parent() && !e.drive;
        let has = !pane.loading && if pane.selected.is_empty() {
            pane.entries.get(pane.cursor).is_some_and(valid)
        } else {
            pane.selected.iter().any(|&i| pane.entries.get(i).is_some_and(valid))
        };
        let folder = !pane.loading && pane.entries.get(pane.cursor).is_some_and(|e| e.is_dir);
        let other = &self.panes[1-index];
        let local_other = self.dual && other.remote.is_none() && other.archive.is_none()
            && other.vault.is_none() && !other.loading && !super::panes::is_drives(&other.path);
        let upload_selected = local_other && if other.selected.is_empty() {
            other.entries.get(other.cursor).is_some_and(valid)
        } else {
            other.selected.iter().any(|&i| other.entries.get(i).is_some_and(valid))
        };
        let paste = self.vault_clipboard.is_none() && (self.clipboard.is_some() || self.archive_clipboard.is_some() || native::clipboard_has_files());
        let text = |key: &str, fallback: &str| self.language_pack.text(key, fallback);
        vec![
            Some((text("formMain.Menu.miOpen", "Open"), "open", idle && folder)),
            Some((text("formMain.Menu.miFTPGet", "Download selected items"), "ftpDownload", idle && has)),
            Some((text(if index == 0 { "RES.FUNC_COPYTORIGHT" } else { "RES.FUNC_COPYTOLEFT" }, "Copy to the other pane"), "copyToOther", idle && has && local_other)),
            None,
            Some((text("formMain.Menu.miFTPPut", "Upload selected items"), "ftpUpload", idle && upload_selected)),
            Some((text("NDR.NET_SELECT_FILES_TO_UPLOAD", "Select files to upload"), "ftpUploadFiles", idle)),
            Some((text("NDR.NET_SELECT_A_FOLDER_TO_UPLOAD", "Select a folder to upload"), "ftpUploadFolder", idle)),
            Some((text("formMain.Menu.mipEditPaste", "Paste"), "paste", idle && paste)),
            None,
            Some((text("formMain.Menu.miNewFolder", "New folder"), "newFolder", idle)),
            Some((text("formMain.Menu.mipRename", "Rename"), "rename", idle && has)),
            Some((text("formMain.Menu.mipDelete", "Delete"), "deletePermanent", idle && has)),
            Some((text("NDR.NET_REMOTE_PERMISSIONS", "Remote permissions"), "attributes", idle && has)),
            None,
            Some((text("formMain.Menu.miCopyFileName", "Copy file name"), "copyName", has)),
            Some((text("formMain.Menu.miCopyFullPath", "Copy full path"), "copyFullPath", has)),
            Some((text("formMain.Menu.miCopyPath", "Copy folder path"), "copyPathOnly", has)),
            Some((text("formMain.Menu.mipProperties", "Properties"), "properties", has)),
            None,
            Some((text("NDR.NET_REFRESH_REMOTE_FOLDER", "Refresh remote folder"), "refresh", idle)),
            Some((text("formMain.Menu.miFTPDisconnect", "Disconnect"), "ftpDisconnect", !remote.busy.load(Ordering::Relaxed))),
        ]
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

    /// Revalidate visible cached totals without clearing their display.
    /// A clean recursive watch needs no filesystem enumeration, even after
    /// the fallback TTL has elapsed. Manual totals are unaffected.
    /// Painting and admission are separate: a full first-column queue
    /// must not prevent another visible column from replaying its sizes.
    fn observe_visible_folder_size(&mut self,index:usize,row:usize)->bool{
        if !self.settings.flag(if index==0{"LeftAutoFolderSize"}else{"RightAutoFolderSize"},false){return false}
        let pane=&mut self.panes[index];
        if pane.vault.is_some()||pane.remote.is_some()||pane.archive.is_some(){return false}
        let Some(entry)=pane.entries.get(row).filter(|e|e.is_dir&&!e.drive&&!e.is_parent()&&e.attributes&0x410==0x10)else{return false};
        // Encrypted containers have a virtual directory attribute in the
        // display Entry. The ordinary folder path must not scan them.
        if entry.path.extension().is_some_and(|e|e.eq_ignore_ascii_case("ndr"))
            &&entry.path.file_name().is_some_and(|n|n.to_string_lossy()!=entry.name){return false}
        let path=&entry.path;
        pane.folder_auto_visible.insert(path.clone());
        if pane.loading||pane.error.is_some(){return false}
        let local=*pane.folder_auto_volume.get_or_insert_with(||native::folder_size_local_volume(&pane.path));
        if !local{return false}
        // Always restore a known display before checking cooldown, busy
        // state or pending tokens. This is also the very first paint after
        // navigation. No filesystem access or recursive scan is required.
        if let Some(observation)=self.folder_size_cache.get_mut(path){
            observation.last_used=Instant::now();
            let bytes=observation.bytes;
            let no_value=!pane.folder_sizes.get(path).is_some_and(Option::is_some);
            let newer= pane.folder_bytes.get(path)!=Some(&bytes)&&observation.current();
            if no_value||newer {
                let text=ndr::parity::size_to_str(bytes,&self.settings.text("SizeUnit","Auto"),self.settings.flag("UseThousandsSeparator",true));
                pane.folder_auto_paths.insert(path.clone());
                pane.folder_hint_bytes.remove(path);
                pane.folder_bytes.insert(path.clone(),bytes);
                pane.folder_sizes.insert(path.clone(),Some(text));
            }
        }else if !pane.folder_sizes.get(path).is_some_and(Option::is_some){
            if let Some((bytes,at))=self.folder_size_hints.get_mut(path){
                *at=Instant::now();
                pane.folder_auto_paths.insert(path.clone());
                pane.folder_hint_bytes.insert(path.clone(),*bytes);
                pane.folder_sizes.insert(path.clone(),Some(ndr::parity::size_to_str(*bytes,&self.settings.text("SizeUnit","Auto"),self.settings.flag("UseThousandsSeparator",true))));
            }
        }
        !pane.folder_auto_pending.contains_key(path)&&!pane.folder_manual_pending.contains_key(path)
            &&!pane.folder_auto_times.get(path).is_some_and(|time|*time>Instant::now())
    }
    /// rank is visual row-major order even when rendering fills columns
    /// top-to-bottom. A separate admission pass sees both complete panes.
    pub(super) fn note_visible_folder_size(&mut self,index:usize,row:usize,rank:usize){
        if self.observe_visible_folder_size(index,row){
            let pane=&mut self.panes[index];pane.folder_auto_candidates.push((pane.generation,rank,row));
        }
    }
    #[cfg(test)]
    pub(super) fn queue_folder_size(&mut self,index:usize,row:usize){
        self.sync_folder_size_activity();
        if self.observe_visible_folder_size(index,row){self.enqueue_folder_size_job(index,row,true);}
    }
    pub(super) fn insert_folder_hint(hints:&mut HashMap<PathBuf,(u64,Instant)>,path:PathBuf,bytes:u64){
        if hints.len()>=2048&&!hints.contains_key(&path){
            if let Some(old)=hints.iter().min_by_key(|(_,(_,at))|*at).map(|(p,_)|p.clone()){hints.remove(&old);}
        }
        hints.insert(path,(bytes,Instant::now()));
    }
    pub(super) fn remember_folder_hint(&mut self,path:PathBuf,bytes:u64){
        Self::insert_folder_hint(&mut self.folder_size_hints,path,bytes);
    }
    /// A prior-session cache hint is much cheaper than a recursive scan.
    /// Warm every visible column independently of the 25-scan admission
    /// cap. At most one short-lived batch is active; no idle worker or
    /// additional persistent database. An absent hint is retried no more
    /// than once per 30 seconds.
    fn warm_visible_folder_hints(&mut self){
        if cfg!(test)||self.folder_hint_pending{return}
        let now=Instant::now();
        self.folder_hint_checked.retain(|_,at|now.saturating_duration_since(*at)<Duration::from_secs(30));
        let mut paths=Vec::new();
        for pane in &self.panes{
            if pane.loading||pane.error.is_some()||pane.folder_auto_volume!=Some(true)||pane.vault.is_some()||pane.remote.is_some()||pane.archive.is_some(){continue}
            for path in &pane.folder_auto_visible{
                if paths.len()>=256{break}
                if pane.folder_sizes.get(path).is_some_and(Option::is_some)||self.folder_size_cache.contains_key(path)
                    ||self.folder_size_hints.contains_key(path)||self.folder_hint_checked.contains_key(path)||paths.contains(path){continue}
                paths.push(path.clone());
            }
        }
        if paths.is_empty(){return}
        if self.folder_hint_checked.len()+paths.len()>2048{
            let mut oldest:Vec<_>=self.folder_hint_checked.iter().map(|(p,t)|(p.clone(),*t)).collect();
            oldest.sort_unstable_by_key(|(_,t)|*t);
            for (p,_) in oldest.into_iter().take(self.folder_hint_checked.len()+paths.len()-2048){self.folder_hint_checked.remove(&p);}
        }
        for path in &paths{self.folder_hint_checked.insert(path.clone(),now);}
        let profile=self.profile.clone();
        let tx=self.async_tx.clone();let stop=self.folder_size_shutdown.clone();let epoch=self.folder_hint_epoch;
        let stats=self.folder_worker_stats.clone();
        let spawned=thread::Builder::new().name("ndr-folder-hint-batch".into()).spawn(move||{
            unsafe{
                use windows_sys::Win32::System::Threading::{GetCurrentThread,SetThreadPriority,THREAD_MODE_BACKGROUND_BEGIN};
                SetThreadPriority(GetCurrentThread(),THREAD_MODE_BACKGROUND_BEGIN as i32);
            }
            let hints={
                let mut store=stats.disk.lock().unwrap_or_else(|e|e.into_inner());
                let disk=store.get_or_insert_with(||ndr::folder_cache::DiskStore::open(&profile,true));
                paths.into_iter().filter_map(|p|disk.candidate_hint(&p).map(|hint|(p,hint))).collect::<Vec<_>>()
            };
            let mut values=Vec::new();
            for (path,hint) in hints{
                if stop.load(Ordering::Relaxed){break}
                if let Some(bytes)=hint.validated(&path){values.push((path,bytes));}
            }
            let _=tx.send(AsyncResult::FolderHintBatch{epoch,values});
        });
        self.folder_hint_pending=spawned.is_ok();
    }
    pub(super) fn admit_visible_folder_sizes(&mut self){
        // A never-measured row wins over recurring checks. Equal urgency
        // is interleaved across columns and panes rather than favoring
        // whichever column/pane is drawn first.
        let mut candidates=Vec::new();
        for index in 0..2{
            let pane=&mut self.panes[index];
            for (generation,rank,row) in pane.folder_auto_candidates.drain(..){
                if generation!=pane.generation{continue}
                if let Some(e)=pane.entries.get(row){
                    let measured=pane.folder_sizes.get(&e.path).is_some_and(Option::is_some);
                    candidates.push((measured,rank,(index+2-self.folder_schedule_turn)%2,index,row));
                }
            }
        }
        candidates.sort_unstable();
        self.folder_schedule_turn^=1;
        // enqueue handles the 48 queued/25 per-pane cap, a clean cached
        // observation, foreground activity and duplicate tokens. In the
        // next frame unmeasured rows still rank ahead of measured rows.
        for (_,_,_,index,row) in candidates{
            self.enqueue_folder_size_job(index,row,true);
        }
    }

    /// Keep only pending scans for rows painted during this frame. The worker
    /// checks each job's token between directory entries, so even an ongoing
    /// large off-screen scan yields to the newly visible rows.
    pub(super) fn finish_visible_folder_size_frame(&mut self, ctx: &egui::Context) {
        let mut cancelled = false;
        for pane in &mut self.panes {
            pane.folder_auto_pending.retain(|path, token| {
                if pane.loading || pane.folder_auto_visible.contains(path) {
                    return true;
                }
                token.store(true, Ordering::Relaxed);
                if !pane.folder_sizes.get(path).is_some_and(Option::is_some) {
                    pane.folder_auto_paths.remove(path);
                    pane.folder_sizes.remove(path);
                    pane.folder_auto_times.remove(path);
                }
                cancelled = true;
                false
            });
            // At this point the *entire* visible set is known. Evicting
            // inside a row's paint can drop the next visible row before
            // it is added to that set (and recreate marker/size flicker).
            const MAX_PANE_TOTALS: usize = 512;
            let excess = pane.folder_auto_paths.len().saturating_sub(MAX_PANE_TOTALS);
            if excess > 0 {
                let mut evictable: Vec<_> = pane.folder_auto_paths.iter()
                    .filter(|p| !pane.folder_auto_pending.contains_key(*p)
                        && !pane.folder_manual_pending.contains_key(*p) && !pane.folder_auto_visible.contains(*p))
                    .collect();
                evictable.sort_unstable_by_key(|p| pane.folder_auto_times.get(*p));
                let evict: Vec<_> = evictable.into_iter().take(excess).cloned().collect();
                for path in evict {
                    pane.folder_auto_times.remove(&path);
                    pane.folder_auto_paths.remove(&path);
                    pane.folder_sizes.remove(&path);
                    pane.folder_bytes.remove(&path);
                    pane.folder_hint_bytes.remove(&path);
                }
            }
        }
        if cancelled {
            self.folder_size_queue.0.lock().unwrap_or_else(|e|e.into_inner()).retain(|j|!j.cancel.load(Ordering::Relaxed));
        }
        self.warm_visible_folder_hints();
        self.admit_visible_folder_sizes();
        // One deadline per frame, not one repaint request per visible row.
        // Align stable observations onto shared ticks to prevent dozens of
        // slightly offset timers from keeping a large folder list animated.
        let now = Instant::now();
        let next = self.panes.iter().flat_map(|pane| pane.folder_auto_visible.iter()
            .filter(|path| !pane.folder_auto_pending.contains_key(*path) && !pane.folder_manual_pending.contains_key(*path))
            .filter_map(|path| pane.folder_auto_times.get(path).copied()))
            .filter(|due| *due > now).min();
        if let Some(due) = next {
            ctx.request_repaint_after(due.saturating_duration_since(now).max(Duration::from_millis(25)));
        }
        if cancelled { ctx.request_repaint(); }
    }

    pub(super) fn record_folder_size_time(&mut self, index: usize, path: &Path, time: Instant) {
        self.panes[index].folder_auto_times.insert(path.to_path_buf(), time);
    }

    /// Coalesce the periodic checks of all visible rows. A notification may
    /// be observed slightly before or after the nominal interval, but
    /// directory identity audits still run on the low-priority worker.
    pub(super) fn folder_check_deadline(&self, interval: Duration) -> Instant {
        coalesced_folder_deadline(self.started, Instant::now(), interval)
    }

    pub(super) fn clear_automatic_folder_sizes(&mut self, index: usize) {
        let pane = &mut self.panes[index];
        pane.cancel_automatic_folder_jobs();
        for path in pane.folder_auto_paths.drain() {
            pane.folder_sizes.remove(&path);
            pane.folder_bytes.remove(&path);
            pane.folder_hint_bytes.remove(&path);
        }
        pane.folder_auto_volume = None;
        if self.folder_size_workers {
            let (queue, _, _) = &*self.folder_size_queue;
            queue.lock().unwrap_or_else(|e| e.into_inner()).retain(|j| !j.cancel.load(Ordering::Relaxed));
        }
    }

    pub(super) fn sync_folder_size_activity(&mut self) {
        let busy = self.job.as_ref().is_some_and(|j| !j.finished)
            || self.background_jobs.iter().any(|j| !j.finished)
            || self.vault_transfer.is_some()
            || self.disk_copy_active()
            || self.panes.iter().any(|p| p.remote.as_ref().is_some_and(|r| r.busy.load(Ordering::Relaxed)))
            || self.original_dialogs.iter().any(|d| matches!(&d.kind, super::original_dialogs::DialogKind::RemoteTransfer(_)));
        if busy && !self.folder_auto_busy {
            self.folder_size_recent_reset.fetch_add(1, Ordering::Relaxed);
            // A scan interrupted by a write could otherwise combine sizes
            // measured before and after the write. Restart it when idle.
            for pane in &mut self.panes {
                pane.cancel_automatic_folder_jobs();
            }
            self.folder_size_queue.0.lock().unwrap_or_else(|e| e.into_inner()).retain(|job| !job.cancel.load(Ordering::Relaxed));
            for observation in self.folder_size_cache.values_mut() {
                if observation.watch.is_none() { observation.invalidate_verification(); }
            }
        }
        if !busy && self.folder_auto_busy {
            self.folder_size_recent_reset.fetch_add(1, Ordering::Relaxed);
            // Notification-free volumes might have changed after the last
            // verification. Revalidate as soon as foreground I/O ends.
            for pane in &mut self.panes { pane.folder_auto_times.clear(); }
            for observation in self.folder_size_cache.values_mut() {
                if observation.watch.is_none() { observation.invalidate_verification(); }
            }
            self.egui_ctx.request_repaint();
        }
        self.folder_auto_busy = busy;
    }

    /// Known mutations invalidate affected paths, including incomplete and
    /// cancelled operations. If a notification is unavailable or missing,
    /// force a full audit rather than treating a silent handle as proof.
    pub(super) fn invalidate_folder_mutations(&mut self, scope: &FolderMutationScope) {
        self.folder_size_recent_reset.fetch_add(1, Ordering::Relaxed);
        for (path, observation) in &mut self.folder_size_cache {
            if scope.affects(path) {
                observation.invalidate_verification();
                if observation.watch.as_ref().is_none_or(|w| w.unchanged()) {
                    observation.audit_at = Instant::now() - ndr::folder_cache::FULL_AUDIT_AGE;
                }
            }
        }
        for pane in &mut self.panes {
            pane.folder_auto_times.retain(|path, _| !scope.affects(path));
        }
        self.egui_ctx.request_repaint();
    }

    /// A fixed budget bounds path memory and notification handles. Keep
    /// completed totals when retiring watches, and never evict a live scan.
    pub(super) fn prune_folder_observations(&mut self) {
        const MAX_WATCHES: usize = 128;
        const WATCH_IDLE: Duration = Duration::from_secs(180);
        let mut active = 0usize;
        for observation in self.folder_size_cache.values_mut() {
            if observation.watch.is_some() {
                if observation.last_used.elapsed() >= WATCH_IDLE {
                    observation.watch = None;
                    observation.invalidate_verification();
                } else { active += 1; }
            }
        }
        if active > MAX_WATCHES {
            let visible = &self.panes;
            let mut oldest: Vec<_> = self.folder_size_cache.iter_mut()
                .filter(|(_, o)| o.watch.is_some())
                .map(|(path, o)| (visible.iter().any(|p| p.folder_auto_visible.contains(path)), o)).collect();
            oldest.sort_unstable_by_key(|(visible, o)| (*visible, o.last_used));
            // Evict old/off-screen watches first; retain their byte totals.
            for (_, o) in oldest.into_iter().take(active - MAX_WATCHES + 15) {
                o.watch = None;
                o.invalidate_verification();
            }
        }
    }

    pub(super) fn cache_folder_size(&mut self, path: PathBuf, bytes: u64, watch: Option<Arc<native::FolderSizeWatch>>, audit_at: Instant, media: native::FolderSizeMedia) {
        // Display totals are small; notification handles are limited
        // separately to 128. Retain more last-known values for scrolling
        // and switching tabs without reintroducing a large I/O cache.
        const MAX_CACHED: usize = 2048;
        if self.folder_size_cache.len() >= MAX_CACHED && !self.folder_size_cache.contains_key(&path) {
            let visible = &self.panes;
            if let Some(oldest) = self.folder_size_cache.iter().min_by_key(|(path, observation)| (
                visible.iter().any(|p| p.folder_auto_visible.contains(*path)), observation.last_used
            )).map(|(path, _)| path.clone()) {
                self.folder_size_cache.remove(&oldest);
            }
        }
        let now = Instant::now();
        let new_watch = watch.as_ref().is_some_and(|w| self.folder_size_cache.get(&path)
            .and_then(|o| o.watch.as_ref()).is_none_or(|old| !Arc::ptr_eq(old, w)));
        self.folder_size_cache.insert(path, FolderSizeObservation { bytes, observed_at: now, last_used: now, watch, audit_at, media });
        if new_watch && self.folder_size_cache.len() > 128 { self.prune_folder_observations(); }
    }

    /// Counts a folder row's size separately (shared by the automatic display and the original miCalculateSelectedFolderSizeClick).
    pub(super) fn enqueue_folder_size(&mut self, index: usize, row: usize) {
        // Manual work must not wait for a huge automatic scan. Preserve
        // completed totals and all other manual requests.
        for pane in &mut self.panes {
            for (path, token) in pane.folder_auto_pending.drain() {
                token.store(true, Ordering::Relaxed);
                pane.folder_auto_visible.remove(&path);
                if !pane.folder_sizes.get(&path).is_some_and(Option::is_some) {
                    pane.folder_auto_paths.remove(&path);
                    pane.folder_sizes.remove(&path);
                    pane.folder_auto_times.remove(&path);
                }
            }
        }
        let (queue, _, _) = &*self.folder_size_queue;
        queue.lock().unwrap_or_else(|e| e.into_inner()).retain(|job| !job.cancel.load(Ordering::Relaxed));
        self.enqueue_folder_size_job(index, row, false);
    }

    fn enqueue_folder_size_job(&mut self, index: usize, row: usize, automatic: bool) {
        let can_use_filesystem = self.vault_has_filesystem(index);
        let epoch = self.started;
        let pane = &mut self.panes[index];
        if !can_use_filesystem || pane.loading || pane.error.is_some() || pane.archive.is_some() || pane.remote.is_some() {
            return;
        }
        let Some(entry) = pane.entries.get(row).filter(|e| e.is_dir && !e.drive && !e.is_parent()) else { return };
        if pane.folder_manual_pending.contains_key(&entry.path)
            || automatic && pane.folder_auto_pending.contains_key(&entry.path) { return; }
        if entry.path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ndr")) && entry.attributes&0x10==0 {
            return; // The encrypted container is already a file on disk.
        }
        let mut prior = None;
        if automatic {
            // No automatic enumeration of encrypted virtual folders,
            // reparse points, UNC shares, or mapped network drives.
            if pane.vault.is_some() || entry.attributes & 0x400 != 0 {
                return;
            }
            let local = *pane.folder_auto_volume.get_or_insert_with(|| native::folder_size_local_volume(&pane.path));
            if !local {
                return;
            }
            if let Some(observation) = self.folder_size_cache.get_mut(&entry.path) {
                let bytes = observation.bytes;
                observation.last_used = Instant::now();
                // Last-known-good display is independent of freshness.
                // A dirty/expired/retired watch must never blank a size
                // when the row returns after scrolling or navigation.
                if !pane.folder_sizes.get(&entry.path).is_some_and(Option::is_some) {
                    let text = ndr::parity::size_to_str(bytes, &self.settings.text("SizeUnit", "Auto"), self.settings.flag("UseThousandsSeparator", true));
                    pane.folder_auto_paths.insert(entry.path.clone());
                    pane.folder_hint_bytes.remove(&entry.path);
                    pane.folder_bytes.insert(entry.path.clone(), bytes);
                    pane.folder_sizes.insert(entry.path.clone(), Some(text));
                }
                if observation.current() {
                    if pane.folder_bytes.get(&entry.path) != Some(&bytes) {
                        let text=ndr::parity::size_to_str(bytes,&self.settings.text("SizeUnit","Auto"),self.settings.flag("UseThousandsSeparator",true));
                        pane.folder_auto_paths.insert(entry.path.clone());
                        pane.folder_bytes.insert(entry.path.clone(),bytes);
                        pane.folder_sizes.insert(entry.path.clone(),Some(text));
                    }
                    pane.folder_auto_paths.insert(entry.path.clone());
                    pane.folder_hint_bytes.remove(&entry.path);
                    let path = entry.path.clone();
                    let interval = if observation.watch.is_some() { observation.media.check_interval() }
                        else { observation.media.verify_interval() };
                    let deadline = coalesced_folder_deadline(epoch, Instant::now(), interval);
                    // Foreground I/O may change a cached folder immediately
                    // after this replay. Do not arm a new cooldown during
                    // that busy period: the first idle paint must recheck.
                    if !self.folder_auto_busy {
                        self.record_folder_size_time(index, &path, deadline);
                    }
                    return;
                }
                if let Some(watch) = observation.watch.as_ref() {
                    prior = Some((observation.bytes, watch.clone(), observation.audit_at, observation.media));
                }
            }
        }
        if automatic && self.folder_auto_busy { return; }
        let (queue, signal, _) = &*self.folder_size_queue;
        let mut jobs = queue.lock().unwrap_or_else(|e| e.into_inner());
        // At most 48 waiting jobs plus one running scan. A fast scroll
        // cannot create an unbounded backlog of expensive I/O.
        if automatic && (jobs.len() >= 48 || pane.folder_auto_pending.len() >= 25) {
            return;
        }
        // Do not replace a known total with the folder marker while waiting.
        pane.folder_sizes.entry(entry.path.clone()).or_insert(None);
        let cancel = Arc::new(AtomicBool::new(false));
        if automatic {
            pane.folder_auto_requests += 1;
            pane.folder_auto_paths.insert(entry.path.clone());
            pane.folder_auto_visible.insert(entry.path.clone());
            pane.folder_auto_pending.insert(entry.path.clone(), cancel.clone());
        } else {
            pane.folder_manual_pending.insert(entry.path.clone(),cancel.clone());
        }
        let hint_needed = automatic && !pane.folder_sizes.get(&entry.path).is_some_and(Option::is_some);
        let job = FolderSizeJob {
            bulk_only:!automatic,
            hint_needed,
            pane: index,
            path: entry.path.clone(),
            cancel,
            automatic,
            track:automatic||pane.vault.is_none()&&pane.remote.is_none()&&pane.archive.is_none()
                &&entry.attributes&0x400==0&&native::folder_size_local_volume(&entry.path)
                &&self.settings.flag(if index==0{"LeftAutoFolderSize"}else{"RightAutoFolderSize"},false),
            prior,
        };
        if automatic { jobs.push_back(job); } else { jobs.push_front(job); }
        drop(jobs);
        signal.notify_all();
        if !self.folder_size_workers {
            self.folder_size_workers = true;
            self.start_folder_lane(false);
        }
        // Unit tests can deliberately set folder_size_workers without
        // spawning a worker in order to inspect a deterministic queue.
        if automatic&&!self.folder_size_helper_started&&!self.folder_size_done.is_empty(){
            self.folder_size_helper_started=self.start_folder_lane(true);
        }
    }
    fn start_folder_lane(&mut self,quick:bool)->bool{
        if self.folder_size_owner.is_none(){
            self.folder_size_owner=Some(FolderWorkerOwner{
                shared:self.folder_size_queue.clone(),shutdown:self.folder_size_shutdown.clone()
            });
        }
        let shared=self.folder_size_queue.clone();
        let tx=self.async_tx.clone();let ctx=self.egui_ctx.clone();
        let profile=self.profile.clone();
        let shutdown=self.folder_size_shutdown.clone();
        let purge=self.folder_size_purge.clone();
        let recent_reset=self.folder_size_recent_reset.clone();
        let stats=self.folder_worker_stats.clone();
        let (done,finished)=mpsc::channel();
        let name=if quick{"ndr-folder-short"}else{"ndr-folder-full"};
        if thread::Builder::new().name(name.into()).spawn(move||{
            folder_size_worker(shared,tx,ctx,profile,shutdown,purge,recent_reset,stats,quick);
            let _=done.send(());
        }).is_ok(){
            self.folder_size_done.push(finished);true
        }else{
            // If the optional short lane cannot start, the normal lane
            // still drains all work. If the full lane cannot start, retire
            // its tokens and allow the next frame to retry.
            if !quick{
                self.folder_size_workers=false;
                for pane in &mut self.panes{
                    pane.cancel_automatic_folder_jobs();pane.cancel_manual_folder_jobs();
                }
                self.folder_size_queue.0.lock().unwrap_or_else(|e|e.into_inner()).clear();
            }
            false
        }
    }
}

fn coalesced_folder_deadline(epoch: Instant, now: Instant, interval: Duration) -> Instant {
    let period = interval.as_millis().max(1) as u64;
    let elapsed = now.saturating_duration_since(epoch).as_millis() as u64;
    // Minimum lead avoids an immediate repaint when a batch completes
    // a few milliseconds before the common tick.
    let next = ((elapsed.saturating_add(150) / period) + 1).saturating_mul(period);
    epoch + Duration::from_millis(next)
}

#[test]
fn folder_deadlines_share_ticks_and_have_minimum_lead() {
    let epoch = Instant::now();
    let interval = Duration::from_secs(1);
    for ms in [10, 100, 400, 700, 849] {
        assert_eq!(coalesced_folder_deadline(epoch, epoch + Duration::from_millis(ms), interval), epoch + interval);
    }
    assert_eq!(coalesced_folder_deadline(epoch, epoch+Duration::from_millis(900), interval), epoch+interval*2);
    assert_eq!(coalesced_folder_deadline(epoch, epoch+Duration::from_millis(1300), interval*2), epoch+interval*2);
}

#[cfg(test)]
pub(super) const FOLDER_SIZE_TTL: Duration = Duration::from_secs(15);

/// A capped set of known operation roots. A very large selection is
/// conservatively reduced to parent folders; only an exceptional selection
/// spanning many different parents needs broad invalidation.
#[derive(Default)]
pub(super) struct FolderMutationScope { roots: Vec<PathBuf>, all: bool }
impl FolderMutationScope {
    pub(super) fn from_operation(op: &Operation) -> Self {
        let mut scope = Self::default();
        let mut add = |path: &Path| {
            if scope.all { return; }
            if !scope.roots.iter().any(|p| p == path) {
                if scope.roots.len() == 64 { scope.roots.clear(); scope.all = true; }
                else { scope.roots.push(path.to_path_buf()); }
            }
        };
        let add_sources = |add: &mut dyn FnMut(&Path), sources: &[PathBuf]| {
            for path in sources {
                add(if sources.len() > 32 { path.parent().unwrap_or(path) } else { path });
            }
        };
        match op {
            Operation::Copy { sources, destination, move_files } => {
                if sources.len() > 32 { add(destination); }
                else {
                    for source in sources { add(&destination.join(source.file_name().unwrap_or_default())); }
                }
                if *move_files { add_sources(&mut add, sources); }
            }
            Operation::Delete { sources, .. } | Operation::DeleteSkipping { sources, .. }
            | Operation::Shred { sources } => add_sources(&mut add, sources),
            Operation::EmptyFolder { folder } => add(folder),
            Operation::Mkdir { path } => add(path),
            Operation::Rename { source, destination } => { add(source); add(destination); }
        }
        scope
    }
    pub(super) fn affects(&self, path: &Path) -> bool {
        self.all || self.roots.iter().any(|root| native::folder_paths_related(root, path))
    }
}

pub(super) struct FolderSizeObservation {
    pub(super) bytes: u64,
    pub(super) observed_at: Instant,
    last_used: Instant,
    pub(super) watch: Option<Arc<native::FolderSizeWatch>>,
    pub(super) audit_at: Instant,
    pub(super) media: native::FolderSizeMedia,
}

impl FolderSizeObservation {
    fn current(&self) -> bool {
        self.observed_at.elapsed() < self.media.verify_interval()
            && self.audit_at.elapsed() < ndr::folder_cache::FULL_AUDIT_AGE
            && self.watch.as_ref().is_none_or(|watch| watch.unchanged())
    }

    pub(super) fn invalidate_verification(&mut self) {
        self.observed_at = Instant::now() - self.media.verify_interval();
    }
}

pub(super) struct FolderSizeJob {
    bulk_only:bool,
    pane: usize,
    pub(super) path: PathBuf,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) automatic: bool,
    track:bool,
    prior: Option<(u64, Arc<native::FolderSizeWatch>, Instant, native::FolderSizeMedia)>,
    hint_needed: bool,
}

pub(super) type FolderSizeQueue = Arc<(Mutex<std::collections::VecDeque<FolderSizeJob>>,
    std::sync::Condvar,Mutex<HashMap<PathBuf,(bool,Arc<AtomicBool>)>>)>;
use std::sync::atomic::AtomicU64;

/// The UI owns this guard; workers own only its shared resources. This
/// handles both ordinary window exit and App destruction without on_exit.
/// Do not infer ownership from Arc counts: two workers plus reservations
/// hold their own references even after the UI no longer exists.
pub(super) struct FolderWorkerOwner{
    shared:FolderSizeQueue,
    shutdown:Arc<AtomicBool>,
}
impl Drop for FolderWorkerOwner{
    fn drop(&mut self){
        self.shutdown.store(true,Ordering::Relaxed);
        let mut queue=self.shared.0.lock().unwrap_or_else(|e|e.into_inner());
        for job in queue.iter(){job.cancel.store(true,Ordering::Relaxed);}
        queue.clear();
        for (_,cancel) in self.shared.2.lock().unwrap_or_else(|e|e.into_inner()).values(){
            cancel.store(true,Ordering::Relaxed);
        }
        self.shared.1.notify_all();
    }
}

#[derive(Default)]
pub(super) struct FolderWorkerStats {
    quick_completed:AtomicU64,
    quick_deferred:AtomicU64,
    full_scans:AtomicU64,
    shared_results:AtomicU64,
    peak_active:AtomicU64,
    quick_while_full:AtomicU64,
    // Both lanes use one bounded disk cache. Otherwise two delayed
    // flushes within the same second could publish an older total.
    disk:Mutex<Option<ndr::folder_cache::DiskStore>>,
    recent:Mutex<RecentFolderProofs>,
}
struct SharedFolderRecent{
    path:PathBuf,bytes:u64,watch:Arc<native::FolderSizeWatch>,audit_at:Instant,
    media:native::FolderSizeMedia,at:Instant,purge:u64,reset:u64,
}
// A short bounded handoff history prevents a burst of visible sibling
// results from overwriting the one proof another pane is about to request.
// Reuse still checks the path identity, known links and original audit age.
const RECENT_FOLDER_LIMIT:usize=8;
const RECENT_FOLDER_AGE:Duration=Duration::from_secs(3);
#[derive(Default)]
struct RecentFolderProofs{
    items:std::collections::VecDeque<SharedFolderRecent>,
}
impl RecentFolderProofs{
    fn prune(&mut self,purge:u64,reset:u64){
        self.items.retain(|r|r.purge==purge&&r.reset==reset&&r.at.elapsed()<RECENT_FOLDER_AGE&&r.watch.unchanged());
    }
    fn remember(&mut self,proof:SharedFolderRecent){
        self.prune(proof.purge,proof.reset);
        self.items.retain(|r|r.path!=proof.path);
        if self.items.len()>=RECENT_FOLDER_LIMIT{self.items.pop_front();}
        self.items.push_back(proof);
    }
    fn find(&self,path:&Path,purge:u64,reset:u64)->Option<&SharedFolderRecent>{
        self.items.iter().rev().find(|r|r.path==path&&r.purge==purge&&r.reset==reset
            &&r.at.elapsed()<RECENT_FOLDER_AGE&&r.watch.unchanged())
    }
}
#[test]
fn recent_folder_proofs_are_bounded_and_discard_changed_or_expired_paths(){
    let root=std::env::temp_dir().join(format!("ndr-recent-proof-{}-{}",std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(&root).unwrap();
    let mut recent=RecentFolderProofs::default();
    let paths:Vec<_>=(0..10).map(|i|root.join(i.to_string())).collect();
    for path in &paths{fs::create_dir(path).unwrap();}
    let now=Instant::now();
    for (i,path) in paths.iter().enumerate(){
        let watch=Arc::new(native::FolderSizeWatch::open(path).unwrap());
        recent.remember(SharedFolderRecent{path:path.clone(),bytes:i as u64,watch,
            audit_at:now,media:native::FolderSizeMedia::Fast,at:now,purge:1,reset:2});
    }
    assert_eq!(recent.items.len(),RECENT_FOLDER_LIMIT);
    assert!(recent.find(&paths[0],1,2).is_none());
    assert_eq!(recent.find(&paths[2],1,2).unwrap().bytes,2);
    assert!(recent.find(&paths[2],1,3).is_none());
    assert_eq!(recent.find(&paths[9],1,2).unwrap().audit_at,now);
    recent.items.front_mut().unwrap().at=now-RECENT_FOLDER_AGE;
    recent.prune(1,2);assert!(recent.find(&paths[2],1,2).is_none());
    fs::write(paths[3].join("changed"),b"new").unwrap();
    let deadline=Instant::now()+Duration::from_secs(2);
    while recent.find(&paths[3],1,2).is_some(){
        assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(10));
    }
    recent.prune(1,2);assert!(recent.items.iter().all(|r|r.path!=paths[3]));
    recent.prune(1,3);assert!(recent.items.is_empty());
    drop(recent);fs::remove_dir_all(root).unwrap();
}
impl FolderWorkerStats{
    pub(super) fn report(&self)->serde_json::Value{
        serde_json::json!({"quickCompleted":self.quick_completed.load(Ordering::Relaxed),
            "quickDeferred":self.quick_deferred.load(Ordering::Relaxed),
            "fullScans":self.full_scans.load(Ordering::Relaxed),
            "sharedResults":self.shared_results.load(Ordering::Relaxed),
            "peakActive":self.peak_active.load(Ordering::Relaxed),
            "quickWhileFull":self.quick_while_full.load(Ordering::Relaxed)})
    }
}
/// The queue lock precedes the active-map lock everywhere. Keep a path
/// reserved until its result has been posted and queued aliases coalesced.
struct ActiveFolderJob { shared:FolderSizeQueue,path:PathBuf }
impl Drop for ActiveFolderJob {
    fn drop(&mut self){
        let _queue=self.shared.0.lock().unwrap_or_else(|e|e.into_inner());
        self.shared.2.lock().unwrap_or_else(|e|e.into_inner()).remove(&self.path);
        self.shared.1.notify_all();
    }
}
fn select_folder_job(jobs:&std::collections::VecDeque<FolderSizeJob>,
    active:&HashMap<PathBuf,(bool,Arc<AtomicBool>)>,quick:bool)->Option<usize>{
    if quick&&(active.values().any(|(automatic,_)|!*automatic)||jobs.iter().any(|j|!j.automatic&&!j.cancel.load(Ordering::Relaxed))){
        return None
    }
    jobs.iter().position(|j|{
        !j.cancel.load(Ordering::Relaxed)&&!active.contains_key(&j.path)&&
        (!quick||j.automatic&&!j.bulk_only&&!jobs.iter().any(|other|other.path==j.path&&other.bulk_only&&!other.cancel.load(Ordering::Relaxed)))
    })
}
fn defer_folder_job(queue:&Mutex<std::collections::VecDeque<FolderSizeJob>>,mut job:FolderSizeJob,stats:&FolderWorkerStats){
    if job.cancel.load(Ordering::Relaxed){return}
    let mut jobs=queue.lock().unwrap_or_else(|e|e.into_inner());
    if job.cancel.load(Ordering::Relaxed){return}
    for other in jobs.iter_mut().filter(|j|j.automatic&&j.path==job.path){other.bulk_only=true;}
    job.bulk_only=true;job.hint_needed=false;jobs.push_back(job);
    stats.quick_deferred.fetch_add(1,Ordering::Relaxed);
}


/// Satisfy queued automatic observations of the same folder with one
/// completed scan. Explicit, incomplete or cancelled work is independent.
fn take_matching_automatic_jobs(queue: &mut std::collections::VecDeque<FolderSizeJob>, completed: &FolderSizeJob, complete: bool) -> Vec<FolderSizeJob> {
    let mut matching = Vec::new();
    if complete && completed.automatic && !completed.cancel.load(Ordering::Relaxed) {
        let mut index = 0;
        while index < queue.len() {
            if queue[index].automatic && queue[index].path == completed.path && !queue[index].cancel.load(Ordering::Relaxed) {
                matching.push(queue.remove(index).expect("index is in queue"));
            } else { index += 1; }
        }
    }
    matching
}

#[test]
fn matching_folder_size_jobs_only_reuse_complete_live_automatic_results() {
    let job = |pane, path: &str, automatic:bool, cancelled| FolderSizeJob {
        bulk_only:!automatic,pane, path: PathBuf::from(path), cancel: Arc::new(AtomicBool::new(cancelled)),
        automatic, track:automatic, prior: None, hint_needed: false,
    };
    let mut queue = std::collections::VecDeque::from([
        job(1, "same", true, false), job(1, "same", false, false),
        job(1, "same", true, true), job(1, "other", true, false),
    ]);
    let first = job(0, "same", true, false);
    assert!(take_matching_automatic_jobs(&mut queue, &first, false).is_empty());
    assert!(take_matching_automatic_jobs(&mut queue, &job(0, "same", false, false), true).is_empty());
    assert!(take_matching_automatic_jobs(&mut queue, &job(0, "same", true, true), true).is_empty());
    let matches = take_matching_automatic_jobs(&mut queue, &first, true);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].pane, 1);
    assert_eq!(queue.len(), 3);
    assert!(!queue[0].automatic);
    assert!(queue[1].cancel.load(Ordering::Relaxed));
    assert_eq!(queue[2].path, PathBuf::from("other"));
}

#[test]
fn short_lane_skips_active_aliases_deferred_jobs_and_manual_work(){
    let job=|path:&str,automatic,bulk_only|FolderSizeJob{pane:0,path:path.into(),
        cancel:Arc::new(AtomicBool::new(false)),automatic,track:automatic,prior:None,hint_needed:false,bulk_only};
    let mut jobs=std::collections::VecDeque::from([job("big",true,true),job("big",true,false),
        job("busy",true,false),job("small",true,false)]);
    let mut active=HashMap::from([(PathBuf::from("busy"),(true,Arc::new(AtomicBool::new(false))))]);
    assert_eq!(select_folder_job(&jobs,&active,true),Some(3));
    assert_eq!(select_folder_job(&jobs,&active,false),Some(0));
    active.insert(PathBuf::from("big"),(true,Arc::new(AtomicBool::new(false))));
    assert_eq!(select_folder_job(&jobs,&active,false),Some(3));
    jobs.push_front(job("manual",false,true));
    assert_eq!(select_folder_job(&jobs,&active,true),None);
    assert_eq!(select_folder_job(&jobs,&active,false),Some(0));
    active.insert(PathBuf::from("manual"),(false,Arc::new(AtomicBool::new(false))));jobs.pop_front();
    assert_eq!(select_folder_job(&jobs,&active,true),None);
}

fn folder_size_worker(shared: FolderSizeQueue, tx: WakeSender<AsyncResult>, ctx: egui::Context, profile: PathBuf,
    shutdown: Arc<AtomicBool>, purge: Arc<AtomicU64>, recent_reset: Arc<AtomicU64>,
    stats:Arc<FolderWorkerStats>,quick:bool) {
    use ndr::folder_cache::{DiskStore, ScanEngine, ScanWork, FULL_AUDIT_AGE};
    let mut engine = ScanEngine::default();
    let mut last_flush = Instant::now();
    let mut media_by_parent: HashMap<PathBuf, (native::FolderSizeMedia, Instant)> = HashMap::new();
    // Keep interactive foreground I/O responsive even while a scan is active.
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
        SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN as i32);
    }
    let (queue, signal, active) = &*shared;
    let mut purge_seen=purge.load(Ordering::Relaxed);
    let mut reset_seen=recent_reset.load(Ordering::Relaxed);
    loop {
        if shutdown.load(Ordering::Relaxed) {
            if let Some(store)=stats.disk.lock().unwrap_or_else(|e|e.into_inner()).as_mut(){let _=store.flush();}
            return;
        }
        let new_purge=purge.load(Ordering::Relaxed);
        let new_reset=recent_reset.load(Ordering::Relaxed);
        if new_purge!=purge_seen{engine=ScanEngine::default();purge_seen=new_purge;}
        if new_reset!=reset_seen{reset_seen=new_reset;}
        {
            let mut recent=stats.recent.lock().unwrap_or_else(|e|e.into_inner());
            recent.prune(purge_seen,reset_seen);
        }
        if last_flush.elapsed() >= Duration::from_secs(10) {
            if let Some(store)=stats.disk.lock().unwrap_or_else(|e|e.into_inner()).as_mut(){let _=store.flush();}
            engine.prune_idle();
            media_by_parent.retain(|_, (_, when)| when.elapsed() < Duration::from_secs(60));
            last_flush = Instant::now();
        }
        let (mut job,_active_job) = {
            let mut jobs = queue.lock().unwrap_or_else(|e| e.into_inner());
            if shutdown.load(Ordering::Relaxed){drop(jobs);continue}
            jobs.retain(|j|!j.cancel.load(Ordering::Relaxed));
            let mut running=active.lock().unwrap_or_else(|e|e.into_inner());
            if let Some(index)=select_folder_job(&jobs,&running,quick){
                let job=jobs.remove(index).expect("selected queue position");
                running.insert(job.path.clone(),(job.automatic,job.cancel.clone()));
                stats.peak_active.fetch_max(running.len() as u64,Ordering::Relaxed);
                let guard=ActiveFolderJob{shared:shared.clone(),path:job.path.clone()};
                (job,guard)
            }else{
                drop(running);
                let wait = Duration::from_secs(10).saturating_sub(last_flush.elapsed()).max(Duration::from_millis(100));
                let wait=if !stats.recent.lock().unwrap_or_else(|e|e.into_inner()).items.is_empty(){wait.min(RECENT_FOLDER_AGE)}else{wait};
                drop(signal.wait_timeout(jobs, wait).unwrap_or_else(|e| e.into_inner()));
                continue;
            }
        };
        if job.cancel.load(Ordering::Relaxed) {
            continue;
        }
        if job.automatic {
            // Warm all queued visible rows before a potentially huge scan.
            // Only metadata-validated prior-session totals are published.
            // They are last-known display values, not new measurements.
            let mut hints = Vec::new();
            let mut add_hint = |j: &FolderSizeJob| {
                if j.automatic && j.hint_needed && !j.cancel.load(Ordering::Relaxed) {
                    hints.push((j.pane, j.path.clone(), j.cancel.clone()));
                }
            };
            add_hint(&job);
            {
                let mut jobs = queue.lock().unwrap_or_else(|e| e.into_inner());
                // Do not perform dozens of metadata probes ahead of
                // the first visible calculation on a slow drive.
                for j in jobs.iter_mut().filter(|j| j.automatic && j.hint_needed).take(7) {
                    add_hint(j);
                    j.hint_needed = false;
                }
            }
            let hints={
                let mut store=stats.disk.lock().unwrap_or_else(|e|e.into_inner());
                let disk=store.get_or_insert_with(||DiskStore::open(&profile,!cfg!(test)));
                hints.into_iter().filter_map(|(pane,path,cancel)|disk.candidate_hint(&path)
                    .map(|hint|(pane,path,cancel,hint))).collect::<Vec<_>>()
            };
            let mut hinted = false;
            for (pane,path,cancel,hint) in hints {
                if !cancel.load(Ordering::Relaxed) {
                    if let Some(bytes)=hint.validated(&path){
                        if tx.send(AsyncResult::FolderSizeHint{pane,path,bytes,cancel}).is_ok(){hinted=true;}
                    }
                }
            }
            if hinted { ctx.request_repaint(); }
            // A second-pane job can arrive just after the worker posted
            // its first result, but before the UI applied that result.
            // Share a bounded recent observation rather than scan twice.
            if job.prior.is_none(){
                let recent=stats.recent.lock().unwrap_or_else(|e|e.into_inner());
                if let Some(r)=recent.find(&job.path,purge_seen,reset_seen){
                    job.prior=Some((r.bytes,r.watch.clone(),r.audit_at,r.media));
                }
            }
        }
        let force_full = job.prior.as_ref().is_none_or(|(_, _, audit_at, _)| audit_at.elapsed() >= FULL_AUDIT_AGE);
        let (bytes, complete, watch, mut stable, scanned, work, audit_at, media) =
            if let Some((bytes, watch, audit_at, media)) = job.prior.as_ref().filter(|(_, watch, _, _)| !force_full && watch.reusable(&job.path)) {
                (*bytes, true, Some(watch.clone()), true, false, ScanWork::default(), *audit_at, *media)
            } else {
                let media = if !job.automatic { native::FolderSizeMedia::Unknown }
                    else if let Some((_, _, _, media)) = &job.prior { *media }
                    else {
                        let parent = job.path.parent().unwrap_or(&job.path);
                        if let Some((media, _)) = media_by_parent.get(parent) { *media }
                        else {
                            if media_by_parent.len() >= 32 {
                                if let Some(old) = media_by_parent.iter().min_by_key(|(_, (_, at))| *at).map(|(p, _)| p.clone()) { media_by_parent.remove(&old); }
                            }
                            let media = native::folder_size_media(parent);
                            media_by_parent.insert(parent.to_path_buf(), (media, Instant::now()));
                            media
                        }
                    };
                if quick&&(media!=native::FolderSizeMedia::Fast||job.prior.is_some()){
                    defer_folder_job(queue,job,&stats);continue
                }
                let watch = if job.track { native::FolderSizeWatch::open(&job.path).map(Arc::new) } else { None };
                let identity=if watch.is_none()&&job.track{native::folder_size_identity(&job.path)}else{None};
                let promoted=if !quick&&job.automatic&&job.prior.is_none(){
                    watch.as_ref().and_then(|w|engine.reuse_checkpoint(&job.path,&job.cancel,w))
                }else{None};
                let mut promoted_audit=None;
                let (summary, work) = if let Some((summary,measured))=promoted{
                    promoted_audit=Some(measured);
                    (summary,ScanWork{visited:0,reused:1,root_stable:true})
                }else if quick{
                    let result=watch.as_ref().and_then(|w|ScanEngine::quick_scan(&job.path,&job.cancel,w,256,Duration::from_millis(60)));
                    let Some(result)=result else{defer_folder_job(queue,job,&stats);continue};
                    stats.quick_completed.fetch_add(1,Ordering::Relaxed);
                    result
                }else if let Some(watch) = &watch {
                    let Some(result) = engine.scan_for_media(&job.path, &job.cancel, force_full, watch, media) else { continue };
                    stats.full_scans.fetch_add(1,Ordering::Relaxed);
                    result
                } else {
                    let Some(summary) = ndr::tools::folder_size_summary(std::slice::from_ref(&job.path), &job.cancel) else { continue };
                    stats.full_scans.fetch_add(1,Ordering::Relaxed);
                    let work = ScanWork { visited: summary.files + summary.folders, reused: 0, root_stable: true };
                    (summary, work)
                };
                // The scan engine already validated the root identity after
                // enumeration and before committing child checkpoints.
                let stable = work.root_stable && watch.as_ref().is_none_or(|watch| watch.matches_path(&job.path))
                    &&identity.is_none_or(|id|native::folder_size_identity(&job.path)==Some(id));
                let audit_at = promoted_audit.unwrap_or_else(||if force_full{Instant::now()}else{job.prior.as_ref().unwrap().2});
                (summary.total_bytes,summary.errors.is_empty(),watch,stable,promoted_audit.is_none(),work,audit_at,media)
            };
        if job.cancel.load(Ordering::Relaxed) {
            continue;
        }
        if job.track&&(purge.load(Ordering::Relaxed)!=purge_seen||recent_reset.load(Ordering::Relaxed)!=reset_seen){
            stable=false; // a mutation raced the scan or its reuse proof
        }
        if quick&&scanned&&complete&&stable&&active.lock().unwrap_or_else(|e|e.into_inner()).len()>1{
            stats.quick_while_full.fetch_add(1,Ordering::Relaxed);
        }
        if job.track && complete && stable {
            if let Some(watch) = &watch {
                stats.recent.lock().unwrap_or_else(|e|e.into_inner()).remember(SharedFolderRecent{
                    path:job.path.clone(),bytes,watch:watch.clone(),audit_at,media,at:Instant::now(),purge:purge_seen,reset:reset_seen});
                if scanned {
                    stats.disk.lock().unwrap_or_else(|e|e.into_inner())
                        .get_or_insert_with(||DiskStore::open(&profile,!cfg!(test))).record(&job.path,watch,bytes);
                }
            }
        } else if !job.automatic {
            stats.recent.lock().unwrap_or_else(|e|e.into_inner()).items.clear();
        }
        let job_automatic = job.automatic;
        let matches = take_matching_automatic_jobs(&mut queue.lock().unwrap_or_else(|e| e.into_inner()), &job, complete && stable);
        stats.shared_results.fetch_add(matches.len() as u64,Ordering::Relaxed);
        for job in std::iter::once(job).chain(matches) {
            if job.cancel.load(Ordering::Relaxed) { continue; }
            // Carry raw bytes only; render with the current settings.
            if tx.send(AsyncResult::FolderSizeRow { pane: job.pane, path: job.path, bytes, automatic: job.automatic, complete, watch: watch.clone(), stable, scanned, work, audit_at, media, cancel: job.cancel }).is_err() {
                return;
            }
        }
        // Unchanged metadata-only results need to retire their pending
        // tokens, but do not need an immediate separate frame per folder.
        if scanned || !complete || !stable || !job_automatic {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}
