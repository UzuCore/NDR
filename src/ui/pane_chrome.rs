//! C# pane frame (MainWindow.xaml LeftPane): header row ("left pane", N selected, new tab +, ☰ tab list, tree), tab row (UseTabs, × per tab,
//! right-click menu = new tab / close tab / close all other tabs), address bar below (ShowAddressBar: path segment buttons + ⌄ subfolders, ... type a path,
//! ▼ history, ↻ refresh, PathClickMode), NavigateAddressAsync (including "archive!/prefix").
//! Difference (HANDOFF): tabs do not keep separate histories (one history per pane). Path segments inside archives have no ⌄ subfolder button.
//! The "tree" button toggles the Rust folder tree area instead of the C# per-pane folder tree window (the folder tree window is on hold).
use super::*;
use std::fs;

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
        let pane = &mut self.panes[index];
        let Some(path) = pane.tabs.get(pane.tab).cloned() else { return };
        pane.archive = None;
        self.active = index;
        let special = super::panes::is_drives(&path) || path.to_string_lossy().starts_with("ftp://") || path.to_string_lossy().starts_with("sftp://");
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
        if lower.starts_with("ftp://") {
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
        let paste = (local || writable_archive) && (self.clipboard.is_some() || self.archive_clipboard.is_some() || native::clipboard_has_files());
        let auto_extract = local;
        // Original mipExplorerMenu (+0x770) is untouched by pmMainPopup, so it is always visible.
        let explorer = local && has;
        // Original text (with &): & is removed when drawing, and a letter typed while the menu is open (menu_letter) picks the item with that access letter.
        let text = |key: &str, fallback: &str| self.language_pack.raw(&format!("formMain.Menu.{key}")).unwrap_or_else(|| fallback.to_owned());
        type Item<'a> = Option<(String, &'static str, bool)>;
        // Original pmMain order (docs/original-menus.txt): Explorer menu | open in new tab | open with, view, edit | copy, cut, paste |
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

    /// C# QueueFolderSizeCalculation: with Left/RightAutoFolderSize (C# only, default off) on a local folder row, counts the size separately (up to two at once).
    /// Shows "..." while counting and the size when done ("≥ size" when some parts were unreadable). Stops when a new list arrives.
    pub(super) fn queue_folder_size(&mut self, index: usize, row: usize) {
        // Fixed: original alwaysshowsize ("always show file size", Left/RightAlwaysShowFolderSize) only keeps a size cell in the Auto list (UpdateDisplayAuto 0x390),
        // but like the C# version it used that value to count every folder size. Automatic folder size counting is now separate.
        if !self.settings.flag(if index == 0 { "LeftAutoFolderSize" } else { "RightAutoFolderSize" }, false) {
            return;
        }
        self.enqueue_folder_size(index, row);
    }

    /// Counts a folder row's size separately (shared by the automatic display and the original miCalculateSelectedFolderSizeClick).
    pub(super) fn enqueue_folder_size(&mut self, index: usize, row: usize) {
        let pane = &mut self.panes[index];
        if pane.vault.is_some() || pane.archive.is_some() || pane.remote.is_some() {
            return;
        }
        let Some(entry) = pane.entries.get(row).filter(|e| e.is_dir && !e.drive && !e.is_parent()) else { return };
        if entry.path.is_file() && entry.path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ndr")) {
            return; // The encrypted container is already a file on disk.
        }
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
        // Original DrawListItem: folder sizes (+0x60 != -1) are drawn with SizeToStr too.
        let size = ndr::parity::size_to_str(summary.total_bytes, &job.unit, job.thousands);
        let text = if summary.errors.is_empty() { size } else { format!("≥ {size}") };
        if tx.send(AsyncResult::FolderSizeRow { pane: job.pane, generation: job.generation, path: job.path, text, bytes: summary.total_bytes }).is_err() {
            return;
        }
    }
}
