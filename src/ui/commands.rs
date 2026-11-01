use super::*;

/// Homepage in the Help menu (user request: the NDR repository).
pub(super) const HOMEPAGE_URL: &str = "https://github.com/UzuCore/NDR";

impl App {
    /// Items of the original miSortBy (miSortByNone ... miSortByColor, separator, miSortDescend), checked by the current sort. Returns the chosen command.
    pub(super) fn sort_items(&mut self, ui: &mut egui::Ui, ko: bool) -> Option<String> {
        let i = self.active;
        let mut chosen = None;
        let mode = self.settings.text(&App::side_key(i, "SortMode"), "Name");
        for (value, command, fallback) in [
            ("None", "sortNone", if ko { "정렬 안 함" } else { "No sort" }),
            ("Name", "sortName", if ko { "이름순" } else { "By name" }),
            ("Extension", "sortExtension", if ko { "확장자순" } else { "By extension" }),
            ("Size", "sortSize", if ko { "크기순" } else { "By size" }),
            ("Modified", "sortTime", if ko { "시간순" } else { "By time" }),
            ("Attributes", "sortAttributes", if ko { "속성순" } else { "By attributes" }),
            ("Color", "sortColor", if ko { "색상순" } else { "By color" }),
        ] {
            let key = menu_data::ORIGINAL_ACTIONS.iter().find(|(_, action)| *action == command).map(|(name, _)| format!("Menu.{name}"));
            let label = key.map(|key| self.language_pack.text(&key, fallback)).unwrap_or_else(|| fallback.into()).replace('&', "");
            let response = super::skin_pane::menu_check_button(ui, true, mode == value, true, egui::Button::new(label));
            if response.clicked() || self.take_item_letter(command, &response) {
                chosen = Some(format!("sort:{value}"));
                ui.close();
            }
        }
        ui.separator();
        let descend = self.raw_menu("miSortDescend", if ko { "&D. 내림차순" } else { "&D. Descend" });
        let descending = self.settings.flag(&App::side_key(i, "SortDescending"), false);
        let response = super::skin_pane::menu_check_button(ui, true, descending, false, egui::Button::new(super::favorites_dialog::access_key(&descend).0));
        if response.clicked() || self.take_menu_letter(&descend, &response) {
            chosen = Some("sortDescending".into());
            ui.close();
        }
        chosen
    }

    /// Original pmSortPopup (0x437464): pmSort of the vertical bar sort button (bSort) copies the miSortBy items as they are (without submenus) and
    /// checks the active pane's sort type (GetSortType) and descending flag (GetSortDescend). Clicking runs the original item's OnClick (miSortByClick and so on).
    pub(super) fn original_sort_popup(&mut self, ui: &mut egui::Ui, ko: bool) {
        if let Some(command) = self.sort_items(ui, ko) {
            self.sort_command(&command);
        }
    }

    /// C# AddSortMenu / AddFolderSortMenu / AddGroupMenu: menus checked by the active pane settings.
    pub(super) fn sort_group_menus(&mut self, ui: &mut egui::Ui, ko: bool) {
        let mut chosen: Option<String> = None;
        // Original miSortBy ("&A. Sort by"): items miSortByNone ... miSortByColor, miSortDescend.
        let raw = self.raw_menu("miSortBy", if ko { "&A. 정렬" } else { "&A. Sort by" });
        self.access_submenu(ui, &raw, |app, ui| {
            chosen = app.sort_items(ui, ko);
        });
        if let Some(command) = chosen {
            self.sort_command(&command);
        }
    }
    pub(super) fn original_internal(&mut self,name:&str,ctx:&egui::Context)->bool{
        // r62 internal commands: same table as C# ExecuteFunctionKeyMacroAsync (ndr::parity::internal_command).
        use ndr::parity::InternalCommand as C;
        match ndr::parity::internal_command(name){
            C::None=>{},
            C::Unknown=>return false,
            C::CopyToOther{source,moving}=>self.original_copy_to_other(source,moving,ctx),
            C::CopyActiveToOther{moving}=>self.copy_to_other(self.active,moving,ctx),
            C::FindNext=>self.find_next(),
            C::ExchangePath=>{/* Original FUN_0043d5ec (shortcut command 0x23): the panes swap folders only when both are in local mode (0). */ if self.panes.iter().all(|p|p.archive.is_none()&&p.remote.is_none()){let paths=self.panes.each_ref().map(|p|p.path.clone());self.navigate(0,paths[1].clone(),true);self.navigate(1,paths[0].clone(),true);}},
            C::ShowPathHistory=>{/* Original case 0x24: WM_LBUTTONDOWN/UP to the history button (+0x324), so the menu opens under that ▼ button. */ let at=ctx.data(|d|d.get_temp::<egui::Pos2>(egui::Id::new(("ndr-history-anchor",self.active))));self.show_history_menu(self.active,at)},
            C::FocusAddressBar=>{/* C# BeginAddressEdit(side) = original shortcut command 0x25 FUN_0048d3c8 (active pane). */ self.begin_address_edit(self.active,ctx)},
            C::SelectAll{with_folders}=>{let i=self.active;let folders=with_folders||self.settings.flag("SelectFolders",false);let pane=&mut self.panes[i];pane.selected=pane.entries.iter().enumerate().filter(|(_,e)|!e.drive&&!e.is_parent()&&(folders||!e.is_dir)).map(|(j,_)|j).collect();},
            C::SelectCurrentAndMove=>self.select_current_and_move(),
            C::SelectCurrent=>self.toggle_focused_selection(),
            C::DeselectAll=>self.panes[self.active].selected.clear(),
            C::CloseTab=>self.dispatch("closeTab",ctx),
            C::Back=>self.dispatch("back",ctx),
            C::Forward=>self.dispatch("forward",ctx),
            C::NewTab=>self.dispatch("newTab",ctx),
            C::FavoritesMenu=>self.show_favorites_menu(),
            C::Menu(action)=>self.dispatch(action,ctx),
        }
        true
    }
    /// C# CopyToOtherAsync / MoveToOtherAsync: ConfirmDragDrop asks the original TfMsgBox
    /// (TITLE_FILEOPERATION, MSG_TORIGHT / MSG_TOLEFT, MB_YESNO | MB_ICONQUESTION) when both panes are local folders.
    /// Original FUN_0042d518 (to the right) / FUN_0042d910 (to the left): the vertical bar copy buttons and shortcut command (FUN_00443f14), skin theme:
    /// nothing with a single pane. When both are local folders, always ask with TITLE_FILEOPERATION / MSG_TORIGHT or MSG_TOLEFT (%s = COPY or MOVE);
    /// local<->FTP uploads or downloads. The original did nothing when an archive location was involved; per user request NDR adds into or extracts from archives.
    pub(super) fn original_copy_to_other(&mut self,source:usize,moving:bool,_ctx:&egui::Context){
        if !self.dual{return;}
        let target=1-source;
        if self.panes[source].archive.is_some()||self.panes[target].archive.is_some(){self.archive_route_to_other(source,moving);return;}
        if self.ftp_route_copy(source){return;}
        let sources=self.panes[source].targets();
        if sources.is_empty(){return;}
        let destination=self.panes[target].path.clone();
        self.confirm_copy_to_other(source,sources,destination,moving);
    }
    fn confirm_copy_to_other(&mut self,source:usize,sources:Vec<PathBuf>,destination:PathBuf,moving:bool){
        let verb=self.language_pack.text(if moving{"RES.MOVE"}else{"RES.COPY"},if moving{"Move"}else{"Copy"});
        let message=if source==0{self.language_pack.text("RES.MSG_TORIGHT","Do you want to %s to the right?")}else{self.language_pack.text("RES.MSG_TOLEFT","Do you want to %s to the left?")}.replace("%s",&verb);
        let title=self.language_pack.text("RES.TITLE_FILEOPERATION","File Operation");
        self.message_box(&title,"",&message,ndr::msgbox::MB_YESNO|ndr::msgbox::MB_ICONQUESTION,move|app,result,_|{
            if result==ndr::msgbox::IDYES{app.transfer_from(source,sources,destination,moving,super::file_operation::TransferKind::OtherPane);}
        });
    }
    pub(super) fn copy_to_other(&mut self,source:usize,moving:bool,_ctx:&egui::Context){
        // Original FUN_0042d518/FUN_0042d910: when one side is FTP, upload/download (a move becomes a copy).
        if self.ftp_route_copy(source){return;}
        if self.archive_route_to_other(source,moving){return;}
        let target=1-source;
        let sources=self.panes[source].targets();
        if sources.is_empty(){return;}
        let destination=self.panes[target].path.clone();
        let local=|p:&Pane|p.archive.is_none();
        if local(&self.panes[source])&&local(&self.panes[target])&&self.settings.flag("ConfirmDragDrop",true){
            return self.confirm_copy_to_other(source,sources,destination,moving);
        }
        self.transfer_from(source,sources,destination,moving,super::file_operation::TransferKind::OtherPane);
    }
    /// C# CopyPathToClipboard (original 0x43317c): items like FUN_00485bbc, Shift doubles every backslash,
    /// CRLF between lines, nothing copied without items, clipboard failures are silent.
    pub(super) fn copy_item_paths(&mut self,part:ndr::parity::PathPart,escape:bool){
        let pane=&self.panes[self.active];
        let indices=ndr::parity::collect_items(pane.entries.len(),&pane.selected,(!pane.loading).then_some(pane.cursor));
        let archive_file=pane.archive.as_ref().map(|a|a.file.to_string_lossy().into_owned());
        let strings:Vec<(String,String)>=indices.iter().filter_map(|i|pane.entries.get(*i)).map(|e|(e.path.to_string_lossy().into_owned(),e.name.clone())).collect();
        let items:Vec<ndr::parity::ItemPath>=strings.iter().map(|(path,name)|{
            if let Some(archive)=&archive_file{ndr::parity::ItemPath::Archive{archive,key:path,name}}
            else{ndr::parity::ItemPath::Local(path)}
        }).collect();
        if let Some(text)=ndr::parity::copy_path_text(&items,part,escape){let _=(self.clipboard_writer)(&text);}
    }
    /// C# OriginalFindAsync (miFindClick 0x437b2c): TfGetText(TITLE_FIND, MSG_FIND, ""), clears the typed quick-search
    /// text, then FUN_00437828 substring search. The last find text is stored only on a hit.
    pub(super) fn original_find(&mut self,text:&str){
        self.typed_search[self.active] = TypedSearch::default();
        self.find_in_active(text,false);
    }
    fn find_in_active(&mut self,text:&str,prefix:bool){
        let pane=&mut self.panes[self.active];
        if pane.loading{return;}
        let names:Vec<String>=pane.entries.iter().map(|e|e.name.clone()).collect();
        if let Some(index)=ndr::parity::find_in_list(&names,Some(pane.cursor),text,prefix){
            pane.cursor=index;
            self.last_find=text.to_owned();
        }
    }
    pub(super) fn menu(&mut self, ui: &mut egui::Ui) {
        let ko = self.language == "ko";
        let ctx = ui.ctx().clone();
        let access = self.menu_access.take();
        // The menu holds only original items (items the original hides stay hidden).
        let skin = self.current_skin();
        let shortcut_colors = skin.as_ref().map(super::skin_pane::skin_menu_shortcut_colors);
        let mut bar = egui::MenuBar::new();
        if let Some(skin) = &skin {
            // Original menu colors (skin [Menu]): menu bar buttons and drop-down menus.
            let (_, bar_style) = super::skin_pane::skin_menu_bar_style(skin);
            bar = bar.style(bar_style).config(egui::containers::menu::MenuConfig::new().style(super::skin_pane::skin_menu_style(skin)));
        }
        bar.ui(ui, |ui| {
            // Whether a menu bar menu was open last frame (moving to another menu by mouse).
            let bar_open_id = egui::Id::new("ndr-menubar-open");
            let bar_was_open = ui.data(|d| d.get_temp::<bool>(bar_open_id)).unwrap_or(false) && egui::Popup::is_any_open(&ctx);
            let mut any_open = false;
            // Original TfMain: menu buttons tbFile ... tbHelp (text " 파일(&F) ") and their DropdownMenu pmFile ... pmHelp.
            for (title_key, ko_title, en_title, items) in menu_data::MENUS.iter() {
                let raw_title = self.language_pack.raw(title_key).unwrap_or_else(|| (if ko { *ko_title } else { *en_title }).to_owned());
                let title = self.language_pack.text(title_key, if ko { ko_title } else { en_title }).trim().replace('&', "");
                let opened = ui.menu_button(title, |ui| {
                    // Windows menu keyboard (original TToolBar menu buttons with pmFile ... pmHelp): a menu opened from the keyboard (Alt+letter) highlights
                    // its first row; ↑/↓ move the highlight over every shown row (greyed rows too, wrapping); Enter runs the highlighted command when it is
                    // enabled. Previously nothing was highlighted and Enter and the arrows did nothing.
                    let row_id = egui::Id::new(("ndr-menu-row", *title_key));
                    let count_id = egui::Id::new(("ndr-menu-rows", *title_key));
                    let mut highlight = ui.data(|d| d.get_temp::<usize>(row_id));
                    if ui.data_mut(|d| d.remove_temp::<bool>(egui::Id::new(("ndr-menu-focus-first", *title_key)))).unwrap_or(false) {
                        highlight = Some(0);
                    }
                    // C# UpdateArchiveMenuItems (when the archive menu opens): enable or hide items by the selected archive and the current archive.
                    let mut archive_state = self.archive_menu_state();
                    archive_state.original = true;
                    let shortcuts = compat::ShortcutMap::parse(self.settings.values.get("OriginalShortcutMapText").and_then(|v| v.as_str()));
                    // Separators only when visible items are on both sides (so hidden items leave no doubled or trailing separator).
                    let (mut shown, mut pending) = (false, false);
                    // While a submenu is open, typed letters go to that submenu's items.
                    let sub_open = Self::submenu_open(ui);
                    let rows_last = ui.data(|d| d.get_temp::<usize>(count_id)).unwrap_or(0);
                    // Enter is not consumed: when an access letter has put the egui focus on an item, egui clicks that item; otherwise the highlighted
                    // command runs after the rows are drawn.
                    let (down, up, enter_key) = if sub_open {
                        (false, false, false)
                    } else {
                        ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.key_pressed(egui::Key::Enter)))
                    };
                    let (mut any_focus, mut highlighted_command) = (false, None);
                    if rows_last > 0 {
                        if down {
                            highlight = Some(highlight.map_or(0, |h| (h + 1) % rows_last));
                        } else if up {
                            highlight = Some(highlight.map_or(rows_last - 1, |h| (h + rows_last - 1) % rows_last));
                        }
                    }
                    // Access letters move the egui focus between items sharing a letter (menu_target); the arrows take over from there, so drop that focus.
                    if down || up {
                        ui.memory_mut(|m| {
                            if let Some(focused) = m.focused() {
                                m.surrender_focus(focused);
                            }
                        });
                    }
                    let mut row = 0usize;
                    // Skin theme shortcut column (original MenuMeasureItem: widest shortcut + 0xe): space reserved after items and last frame's width.
                    let width_id = egui::Id::new(("ndr-menu-shortcut-width", *title_key));
                    let previous_width = ui.data(|d| d.get_temp::<f32>(width_id)).unwrap_or(0.0);
                    let column_slot = shortcut_colors.map(|_| ui.painter().add(egui::Shape::Noop));
                    let mut widest = 0.0f32;
                    // Original MenuMeasureItem (0x42df68): item width = text + 0x20 + shortcut column (widest shortcut + 0xe). Each row reserves that menu's widest
                    // shortcut text plus two blanks transparently so the name text does not run into the shortcut column background.
                    let font = egui::TextStyle::Button.resolve(ui.style());
                    let reserve = items
                        .iter()
                        .map(|(id, _, _)| Self::menu_shortcut_text(id, &shortcuts))
                        .max_by(|a, b| {
                            let w = |s: &str| ui.fonts_mut(|f| f.layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE).size().x);
                            w(a).total_cmp(&w(b))
                        })
                        .unwrap_or_default();
                    let reserve = format!("{reserve}    ");
                    for (id, ko_label, en_label) in *items {
                        if *id == "-" {
                            pending = shown;
                            continue;
                        }
                        let (visible, enabled) = archive_state.item(id);
                        if !visible || Self::csharp_only_menu_item(id) || menu_data::ORIGINAL_HIDDEN.contains(id) {
                            continue;
                        }
                        if pending {
                            ui.separator();
                            pending = false;
                        }
                        shown = true;
                        if let Some(submenu) = id.strip_prefix('@') {
                            self.original_submenu(submenu, ui, &ctx, ko);
                            row += 1;
                            continue;
                        }
                        // Original miLanguage and miSkin are submenus (FUN_00441830, FUN_0043e580).
                        if *id == "language" || *id == "theme" {
                            self.original_submenu(if *id == "language" { "language" } else { "skin" }, ui, &ctx, ko);
                            row += 1;
                            continue;
                        }
                        let label = self.menu_label(id, if ko { ko_label } else { en_label });
                        // Original pmViewPopup: toggles and view styles are checkable items.
                        let shortcut = Self::menu_shortcut_text(id, &shortcuts);
                        // Original MenuDrawItem: checkable items draw like ordinary items with a CHECK image on the left when checked (radio items like view style use RCHECK).
                        let button = if shortcut_colors.is_some() {
                            egui::Button::new(label).shortcut_text(RichText::new(reserve.clone()).color(Color32::TRANSPARENT))
                        } else {
                            egui::Button::new(label).shortcut_text(shortcut.clone())
                        };
                        let button = button.selected(highlight == Some(row));
                        let response = match self.menu_checked(id) {
                            Some(checked) => super::skin_pane::menu_check_button(ui, enabled, checked, Self::radio_menu_item(id), button),
                            None => ui.add_enabled(enabled, button),
                        };
                        // A moving mouse takes the keyboard highlight too (Windows menus share one highlighted row); a resting pointer does not.
                        if response.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                            highlight = Some(row);
                        }
                        any_focus |= response.has_focus();
                        if enabled && highlight == Some(row) {
                            highlighted_command = Some(*id);
                        }
                        row += 1;
                        if let Some(colors) = shortcut_colors {
                            // The highlighted row background (mhighlight) covers the column, so only that row repaints the column (the original paints the column on every row).
                            if response.hovered() {
                                let left = ui.min_rect().right() - (previous_width.max(8.0) + 14.0);
                                ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(left, response.rect.top()), egui::pos2(ui.min_rect().right(), response.rect.bottom())), 0.0, colors.0);
                            }
                            widest = widest.max(super::skin_pane::paint_menu_shortcut(ui, response.rect, &shortcut, colors, response.hovered()));
                        }
                        let letter_hit = !sub_open && enabled && {
                            let letter = self.menu_access_letter(id);
                            self.menu_target(letter, &response)
                        };
                        if response.clicked() || letter_hit {
                            Self::drop_menu_keys(ui);
                            self.menu_letter = None;
                            self.dispatch(id, &ctx);
                            ui.close();
                        }
                    }
                    if any_focus {
                        highlight = None;
                    } else if let Some(id) = highlighted_command.filter(|_| enter_key) {
                        Self::drop_menu_keys(ui);
                        self.menu_letter = None;
                        self.dispatch(id, &ctx);
                        ui.close();
                    }
                    ui.data_mut(|d| {
                        d.insert_temp(count_id, row);
                        match highlight {
                            Some(h) => d.insert_temp(row_id, h),
                            None => d.remove::<usize>(row_id),
                        }
                    });
                    if let (Some(slot), Some(colors)) = (column_slot, shortcut_colors) {
                        super::skin_pane::fill_menu_shortcut_column(ui, slot, widest, colors);
                        ui.data_mut(|d| d.insert_temp(width_id, widest));
                    }
                });
                let letter = super::favorites_dialog::access_key(&raw_title).1.map(|c| c.to_ascii_lowercase());
                if access.is_some() && letter == access {
                    egui::Popup::open_id(&ctx, egui::Popup::default_response_id(&opened.response));
                    ctx.data_mut(|d| d.insert_temp(egui::Id::new(("ndr-menu-focus-first", *title_key)), true));
                    ctx.request_repaint();
                }
                // Windows menu bar (original TToolBar menu buttons): while one menu is open, hovering another menu title switches to that menu.
                let popup_id = egui::Popup::default_response_id(&opened.response);
                if opened.inner.is_some() {
                    any_open = true;
                } else {
                    // A closed menu forgets its keyboard highlight (a menu opened by the mouse starts with none).
                    ctx.data_mut(|d| d.remove::<usize>(egui::Id::new(("ndr-menu-row", *title_key))));
                    if bar_was_open && opened.response.hovered() {
                        egui::Popup::open_id(&ctx, popup_id);
                        any_open = true;
                        ctx.request_repaint();
                    }
                }
            }
            ui.data_mut(|d| d.insert_temp(bar_open_id, any_open));
            // Original tbNewUpdateAvailable (a quiet update check found a new version). Original tbDonation (donate) was removed per user request.
            if let Some(url) = self.update_available.clone() {
                let label = self.language_pack.text("ToolButton.tbNewUpdateAvailable", if ko { "새 업데이트" } else { "New Update Available" }).trim().to_owned();
                if ui.button(label).clicked() {
                    if url.starts_with("https://github.com/UzuCore/NDR/releases/download/build-") {
                        self.start_update_check(false);
                    } else {
                        let _ = (self.shell_runner)(Path::new(&url), "", Path::new(""));
                    }
                }
            }
        });
    }

    /// Original item name of a menu command (MENU_CAPTIONS, else ORIGINAL_ACTIONS). None for commands only in the C# version.
    pub(super) fn original_menu_name(id: &str) -> Option<&'static str> {
        menu_data::MENU_CAPTIONS
            .iter()
            .find(|(command, _)| *command == id)
            .map(|(_, name)| *name)
            .or_else(|| menu_data::ORIGINAL_ACTIONS.iter().find(|(_, action)| *action == id).map(|(name, _)| *name))
    }

    /// Menu commands not in the original menu (only in the C# version). Submenus ("@...") exist in the original.
    pub(super) fn csharp_only_menu_item(id: &str) -> bool {
        !id.starts_with('@') && Self::original_menu_name(id).is_none()
    }

    /// Menu item text: the Menu.* text of the original item name, else the C# version text. & is removed.
    pub(super) fn menu_label(&self, id: &str, fallback: &str) -> String {
        Self::original_menu_name(id).map(|name| self.language_pack.text(&format!("Menu.{name}"), fallback)).unwrap_or_else(|| fallback.into()).replace('&', "")
    }

    /// Access letter of a menu item (q in the original text "&Q. Open"). None without original text (C# version items).
    pub(super) fn menu_access_letter(&self, id: &str) -> Option<char> {
        let raw = self.language_pack.raw(&format!("Menu.{}", Self::original_menu_name(id)?))?;
        super::favorites_dialog::access_key(&raw).1.map(|c| c.to_ascii_lowercase())
    }

    /// Original Menu.<name> text (with &), else fallback.
    pub(super) fn raw_menu(&self, name: &str, fallback: &str) -> String {
        self.language_pack.raw(&format!("Menu.{name}")).unwrap_or_else(|| fallback.to_owned())
    }

    /// True when the letter typed while the menu is open is the access letter of this original text (with &). Consumes the letter. `response` = the item (to move between items sharing a letter).
    pub(super) fn take_menu_letter(&mut self, raw: &str, response: &egui::Response) -> bool {
        let letter = super::favorites_dialog::access_key(raw).1.map(|c| c.to_ascii_lowercase());
        self.menu_target(letter, response)
    }

    /// Matches the typed letter against the access letter (menu_access_letter) of a menu command item. Consumes the letter on a match.
    pub(super) fn take_item_letter(&mut self, id: &str, response: &egui::Response) -> bool {
        let letter = self.menu_access_letter(id);
        self.menu_target(letter, response)
    }

    /// One menu item that can take letters: adds it to this frame's list (with several items on one letter, keyboard only moves the selection),
    /// focuses it when the focus request is this item, and consumes the typed letter and returns true when it is this item's letter.
    pub(super) fn menu_target(&mut self, letter: Option<char>, response: &egui::Response) -> bool {
        let hit = self.note_menu_target(letter, response);
        if hit {
            self.menu_letter = None;
        }
        hit
    }

    /// Like menu_target without consuming the letter (for menus drawn with &self): adds to the list, focuses, and only reports whether the typed letter matches.
    pub(super) fn note_menu_target(&self, letter: Option<char>, response: &egui::Response) -> bool {
        let Some(letter) = letter else { return false };
        self.menu_targets.borrow_mut().push((letter, response.id));
        if self.menu_focus.get() == Some(response.id) {
            response.request_focus();
            self.menu_focus.set(None);
        }
        self.menu_letter == Some(letter)
    }

    /// Original submenu item (sub-items of a TMenuItem, e.g. "&H. Checksum"). While the menu is open and no other submenu is open,
    /// typing this name's access letter opens the submenu like a Windows menu (egui MenuState.open_item). While the submenu is open
    /// its items take the letters (take_menu_letter / take_item_letter).
    pub(super) fn access_submenu(&mut self, ui: &mut egui::Ui, raw_title: &str, add: impl FnOnce(&mut Self, &mut egui::Ui)) {
        use egui::containers::menu::{MenuState, SubMenu, SubMenuButton};
        let id = SubMenu::id_from_widget_id(ui.next_auto_id());
        let another_open = MenuState::from_ui(ui, |state, _| state.open_item.is_some_and(|open| open != id));
        let level_active = !Self::submenu_open(ui);
        let letter = super::favorites_dialog::access_key(raw_title).1.map(|c| c.to_ascii_lowercase());
        if !another_open && letter.is_some() && self.menu_letter == letter {
            self.menu_letter = None;
            // MenuState resets a submenu that was never drawn to closed, so mark it as drawn.
            MenuState::mark_shown(ui.ctx(), id);
            MenuState::from_ui(ui, |state, _| state.open_item = Some(id));
            ui.ctx().request_repaint();
        }
        let title = super::favorites_dialog::access_key(raw_title).0;
        let (response, _) = SubMenuButton::new(title).ui(ui, |ui| add(self, ui));
        // Submenu names are also targets for moving between items sharing a letter (C = function key bar and columns in the original View menu).
        if level_active {
            self.menu_target(letter, &response);
        }
    }

    /// Whether a submenu is open in the current menu (then parent-level items do not take typed letters).
    pub(super) fn submenu_open(ui: &egui::Ui) -> bool {
        egui::containers::menu::MenuState::from_ui(ui, |state, _| state.open_item.is_some())
    }

    /// A menu item has run from the keyboard or the mouse: this frame's key presses and typed text belong to the menu, so they are removed before
    /// the window the command opens reads them. Previously the Enter (or the access letter) that picked the item also reached that window: Enter
    /// pressed its OK (FTP quick connect closed at once, the FTP download destination box picked its first row).
    pub(super) fn drop_menu_keys(ui: &egui::Ui) {
        ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::Text(_))));
    }

    /// Items with RadioItem = True in the original TfMain DFM (miListStyleAuto, Names, Details, Thumbs). Checked items use RCHECK instead of CHECK.
    pub(super) fn radio_menu_item(command: &str) -> bool {
        matches!(command, "viewAuto" | "viewDetails" | "viewNames" | "viewThumbnails")
    }

    /// Shortcut text at the right of an original menu item (TMenuItem.ShortCut, as changed by shortcut.ini, via Delphi ShortCutToText). Empty when none.
    pub(super) fn menu_shortcut_text(id: &str, shortcuts: &compat::ShortcutMap) -> String {
        Self::original_menu_name(id)
            .and_then(|name| shortcuts.menu_shortcuts.get(name).copied())
            .filter(|code| *code != 0)
            .map(compat::shortcut_to_text)
            .unwrap_or_default()
    }

    /// Submenus of the original menu: checksum (miChecksum), work folders (miSetWorkFolder, miGoToWorkFolder), view style (miListStyle),
    /// columns (miColumn), sort (miSortBy, including the C# folder sort and grouping), filter (miFilter).
    fn original_submenu(&mut self, name: &str, ui: &mut egui::Ui, ctx: &egui::Context, ko: bool) {
        match name {
            "checksum" => {
                let raw = self.raw_menu("miChecksum", if ko { "&H. 체크섬" } else { "&H. Checksum" });
                let mut chosen = None;
                self.access_submenu(ui, &raw, |app, ui| {
                    // Original miCRC, miMD5, miSHA1.
                    for (key, fallback, algorithm) in [("miCRC", "&C. CRC", "Crc32"), ("miMD5", "&M. MD5", "Md5"), ("miSHA1", "&S. SHA1", "Sha1")] {
                        let raw = app.raw_menu(key, fallback);
                        let response = ui.button(super::favorites_dialog::access_key(&raw).0);
                if response.clicked() || app.take_menu_letter(&raw, &response) {
                            chosen = Some(algorithm);
                            ui.close();
                        }
                    }
                });
                if let Some(algorithm) = chosen {
                    self.dispatch(&format!("checksum:{algorithm}"), ctx);
                }
            }
            "workFolders" => self.work_folder_menus(ui, ctx),
            "skin" => {
                // Original FUN_0043e580: one item per folder under skin\, the current skin checked. Clicking runs FUN_0043e510 (apply that skin).
                let raw = self.raw_menu("miSkin", if ko { "&N. 스킨" } else { "&N. Skin" });
                let names = self.skin_names();
                let current = self.settings.text("Theme", "Black");
                let mut chosen = None;
                self.access_submenu(ui, &raw, |_, ui| {
                    for name in &names {
                        if super::skin_pane::menu_check_button(ui, true, name.eq_ignore_ascii_case(&current), true, egui::Button::new(*name)).clicked() {
                            chosen = Some(*name);
                            ui.close();
                        }
                    }
                });
                if let Some(name) = chosen {
                    self.apply_theme(name);
                }
            }
            "language" => {
                // Original FUN_00441830: one item per language (current language checked), then N23 ("-") and miMoreLanguages (FUN_00445b88: xiles.app/languages).
                let raw = self.raw_menu("miLanguage", if ko { "&O. 언어" } else { "&O. UI Language" });
                let languages = super::settings_window::available_languages(&self.resources);
                let current = self.language.clone();
                let more = self.raw_menu("miMoreLanguages", if ko { "&M. 다른 언어..." } else { "&M. More Languages..." });
                let mut chosen = None;
                let mut open_more = false;
                self.access_submenu(ui, &raw, |app, ui| {
                    for (code, name) in &languages {
                        if super::skin_pane::menu_check_button(ui, true, code.eq_ignore_ascii_case(&current), true, egui::Button::new(name.as_str())).clicked() {
                            chosen = Some(code.clone());
                            ui.close();
                        }
                    }
                    ui.separator();
                    let response = ui.button(super::favorites_dialog::access_key(&more).0);
                    if response.clicked() || app.take_menu_letter(&more, &response) {
                        open_more = true;
                        ui.close();
                    }
                });
                if let Some(code) = chosen {
                    self.apply_language(&code);
                }
                if open_more {
                    self.dispatch("moreLanguages", ctx);
                }
            }
            "ftpSites" => {
                // Original miFTPConnect: site rows built by FUN_00440734 ("&1. name" ...), N18 ("-"), miFTPManageSites.
                let raw = self.raw_menu("miFTPConnect", if ko { "&C. FTP 연결하기" } else { "&C. FTP Connect" });
                let items = self.ftp_site_items();
                let mut chosen = None;
                self.access_submenu(ui, &raw, |app, ui| {
                    for (index, caption) in &items {
                        let response = ui.button(super::favorites_dialog::access_key(caption).0);
                if response.clicked() || app.take_menu_letter(caption, &response) {
                            chosen = Some(format!("ftpConnect:{index}"));
                            ui.close();
                        }
                    }
                    ui.separator();
                    let manage = app.raw_menu("miFTPManageSites", if ko { "&Z. FTP 사이트 관리..." } else { "&Z. FTP Site Manager..." });
                    let response = ui.button(super::favorites_dialog::access_key(&manage).0);
                if response.clicked() || app.take_menu_letter(&manage, &response) {
                        chosen = Some("sites".to_owned());
                        ui.close();
                    }
                });
                if let Some(id) = chosen {
                    self.dispatch(&id, ctx);
                }
            }
            "columns" => self.column_menu(ui, ctx),
            "sort" => self.sort_group_menus(ui, ko),
            "listStyle" => {
                let raw = self.raw_menu("miListStyle", if ko { "&S. 보기 방식" } else { "&S. List Style" });
                let mut chosen = None;
                self.access_submenu(ui, &raw, |app, ui| {
                    for (id, ko_label, en_label) in [("viewAuto", "Auto List", "Auto List"), ("viewNames", "이름 목록", "Names"), ("viewDetails", "상세 목록", "Details"), ("viewThumbnails", "미리 보기 목록", "Thumbnails")] {
                        let label = app.menu_label(id, if ko { ko_label } else { en_label });
                        let response = ui.radio(app.menu_checked(id).unwrap_or(false), label);
                if response.clicked() || app.take_item_letter(id, &response) {
                            chosen = Some(id);
                            ui.close();
                        }
                    }
                });
                if let Some(id) = chosen {
                    self.dispatch(id, ctx);
                }
            }
            _ => {
                let raw = self.raw_menu("miFilter", if ko { "&F. 필터" } else { "&F. Filter" });
                let mut chosen = false;
                self.access_submenu(ui, &raw, |app, ui| {
                    let response = ui.button(app.menu_label("filter", if ko { "폴더 필터…" } else { "Custom..." }));
                if response.clicked() || app.take_item_letter("filter", &response) {
                        chosen = true;
                        ui.close();
                    }
                });
                if chosen {
                    self.dispatch("filter", ctx);
                }
            }
        }
    }
    pub(super) fn dispatch(&mut self, id: &str, ctx: &egui::Context) {
        // Original pane FTP mode: commands that work on local files do nothing (seen in the running original: Insert on an FTP pane sends nothing
        // and shows nothing). Previously a C# status notice appeared.
        if self.panes[self.active].remote.is_some() && !super::ftp_pane::ftp_allows(id) {
            return;
        }
        // Original pmConnect: site connect (FUN_00440734 rows -> FUN_0043e720), quick connect, disconnect, download, upload.
        if let Some(index) = id.strip_prefix("ftpConnect:").and_then(|n| n.parse::<usize>().ok()) {
            return self.ftp_connect_site(index);
        }
        match id {
            "ftpQuickConnect" => return self.ftp_quick_connect(String::new()),
            "ftpDisconnect" => return self.ftp_disconnect(),
            "ftpDownload" => return self.ftp_download_command(),
            "ftpUpload" => return self.ftp_upload_command(),
            "toggleResults" => return self.toggle_results(),
            _ => {}
        }
        // C# AddChecksumSubmenu: "checksum:Crc32|Md5|Sha1|Sha256".
        if let Some(algorithm) = id.strip_prefix("checksum:").and_then(super::tools_dialog::ChecksumAlgorithm::parse) {
            return self.checksum_command(Some(algorithm));
        }
        // C# ExecuteMenuCommandAsync: favoriteOpen:/workOpen:/workSet:/column:.
        if self.prefixed_command(id) {
            return;
        }
        // Fixed: when the original shortcut table (shortcut.ini) bound a key to a popup item (mipEditCopy etc.) or a submenu name, no one handled that name's command id
        // and it became "Unknown command". Send it to the same command as the popup menu (pane_chrome).
        let alias = match id {
            "contextView" => Some("textView"),
            "contextEdit" => Some("textEdit"),
            "contextCopy" => Some("copy"),
            "contextCut" => Some("cut"),
            "contextPaste" => Some("paste"),
            "contextCopyTo" => Some("copyToFolder"),
            "contextMoveTo" => Some("moveToFolder"),
            "contextDelete" => Some("deleteRecycle"),
            "contextRename" => Some("rename"),
            "contextAttributes" => Some("attributes"),
            "contextDescription" => Some("description"),
            "contextArchiveAdd" => Some("archiveAdd"),
            "contextAutoExtract" => Some("extractAuto"),
            "contextProperties" => Some("properties"),
            "lastDestination" => Some("goLastDestination"),
            "shortcutsHelp" => Some("shortcuts"),
            _ => None,
        };
        if let Some(target) = alias {
            return self.dispatch(target, ctx);
        }
        // Items that open submenus (set work folder, FTP connect, skin), drag-and-drop menu items (only shown on drop) and folder tree window items
        // (miTreeRefresh, miSubRefresh, not in TfMain) do nothing from main window keys.
        if matches!(id, "setWorkFolder" | "ftpConnect" | "theme" | "dropCopy" | "dropMove" | "dropShortcut" | "dropCancel" | "treeRefresh" | "treeRefreshRecursive") {
            return;
        }
        let i = self.active;
        // Original CopySelection (-> +0xd08): starting copy, move, delete, shred, description, advanced rename, rename, attributes, extract, add to archive or
        // create shortcut saves the current marks for "Restore Selection" (skin theme). Save list and merge save them only when confirmed.
        const REMEMBERS: [&str; 26] = [
            "copyToOther", "moveToOther", "copyToFolder", "moveToFolder", "contextCopyTo", "contextMoveTo", "deleteRecycle", "deletePermanent", "contextDelete", "shred",
            "description", "contextDescription", "advancedRename", "rename", "contextRename", "attributes", "contextAttributes",
            "extractSelected", "extractCurrent", "extractNewFolder", "extractAuto", "contextAutoExtract", "archiveAdd", "shortcut", "quickLaunchShortcut", "batchRename",
        ];
        if REMEMBERS.contains(&id) {
            self.remember_marks();
        }
        let targets = self.panes[i].targets();
        let path = self.panes[i].path.clone();
        let mut result = Ok(());
        if id.contains(':') && self.sort_command(id) {
            return;
        }
        match id {
            "historyBack"=>self.dispatch("back",ctx),"historyForward"=>self.dispatch("forward",ctx),"batchRename"=>self.dispatch("advancedRename",ctx),
            "deleteToolbar"=>{
                // Original Recycle Bin button FUN_0042f13c: chooses by Shift/Ctrl/Alt at the time of the click.
                let Some(target)=recycle_button_target(ctx.input(|i|i.modifiers)) else { return };
                self.dispatch(target,ctx)
            },"extractToolbar"=>self.dispatch("extractSelected",ctx),
            "open"=>self.open_current(),
            "openAll"=>self.open_all_command(),
            "explorerMenu"=>result=ndr::shell_menu::show(&targets),
            "sortNone"|"sortName"|"sortExtension"|"sortSize"|"sortTime"|"sortAttributes"|"sortColor"|"folderSortDescending"|"groupDescending"|"groupShowName"=>{self.sort_command(id);},
            "sortDescending"=>{self.sort_command(id);},
            // Original miShowFoldersClick (0x445258): toggles the current pane's (+0xe08) tree and stores it in that pane's setting (+0x50 + pane index).
            "toggleFolders"|"toggleFolderTree"=>{
                let key=if self.active==0{"ShowLeftFolderTree"}else{"ShowRightFolderTree"};
                let shown=!self.settings.flag(key,false);
                self.settings.set_flag(key,shown);
                self.save();
            },
            "toggleToolBar"|"toggleFunctionBar"|"toggleVerticalBar"|"toggleDriveButtons"=>{let (key,default)=match id{"toggleToolBar"=>("ShowToolBar",true),"toggleFunctionBar"=>("ShowFunctionBar",true),"toggleVerticalBar"=>("ShowVerticalBar",true),_=>("ShowDriveButtons",false)};let old=self.settings.flag(key,default);self.settings.set_flag(key,!old);self.save();},
            "exit"=>ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            "refresh"=>{super::panes::DriveListCache::invalidate(&self.drive_list);self.refresh(i)},"goParent"=>self.parent(),
            "goRoot"=>self.go_root(),
            "back"=>{if let Some(path)=self.panes[i].back.pop(){self.panes[i].forward.push(self.panes[i].path.clone());self.navigate(i,path,false);}},
            "forward"=>{if let Some(path)=self.panes[i].forward.pop(){self.panes[i].back.push(self.panes[i].path.clone());self.navigate(i,path,false);}},
            "goLastDestination"=>self.go_last_destination(),
            "newTab"=>{if self.settings.flag("UseTabs",true){let tab=self.panes[i].tab;self.tab_command(i,&format!("newTabFrom:{tab}"),ctx);}},
            "closeTab"=>{if self.settings.flag("UseTabs",true){let tab=self.panes[i].tab;self.tab_command(i,&format!("closeTab:{tab}"),ctx);}},
            "openInNewTab"=>self.open_in_new_tab(i),
            "newFolder"=>self.new_folder(),
            "newEmpty"=>self.create_empty_file(),
            "rename"=>self.rename_items(),
            "advancedRename"=>self.advanced_rename(),
            // Original miCopyToClick (0x432e68) / miMoveToClick (0x432f40): at a local location FUN_0042bdc4 (no destination) = TfSelectDest (path field starts with
            // the other pane, editable to a folder in the same pane); at an archive location FUN_00439bac (choose the extraction destination). FTP locations upload/download.
            "copyToOther"|"moveToOther" if self.panes[i].remote.is_none()&&self.panes[1-i].remote.is_none()=>self.copy_move_to_folder(id=="moveToOther"),
            "copyToOther"|"moveToOther"=>self.copy_to_other(i,id=="moveToOther",ctx),
            "copyToFolder"|"moveToFolder"=>self.copy_move_to_folder(id=="moveToFolder"),
            "copy"|"cut"=>{
                if !targets.is_empty(){
                    if self.panes[i].archive.is_some() {
                        // Archive entry paths are virtual, not local CF_HDROP
                        // files. Extract to a real staging folder first.
                        self.copy_archive_to_clipboard(i, id=="cut");
                    } else {
                        result=(self.file_clipboard)(&targets,id=="cut");
                        if result.is_ok() {
                            self.clipboard=Some((targets.clone(),id=="cut"));
                            self.clipboard_sequence=Some(native::clipboard_sequence_number());
                            if let Some(previous)=self.archive_clipboard.take(){let _=std::fs::remove_dir_all(previous.stage);}
                            // Original 0x433dd8: cut items have dimmed icons until a new listing.
                            if id=="cut"&&self.panes[i].remote.is_none(){self.panes[i].cut_marks.extend(targets.iter().cloned());}
                        }
                    }
                }
            },
            "paste"=>{
                let archive_copy = self.archive_clipboard.clone().filter(|c| cfg!(test) || native::clipboard_sequence_number()==c.sequence);
                if let Some(clip) = archive_copy {
                    if self.panes[i].remote.is_some() {
                        let files=clip.keys.iter().map(|key|clip.stage.join(key.rsplit('/').next().unwrap_or(key))).collect();
                        let paste=super::ftp_pane::PasteUpload { moving:clip.moving, sequence:clip.sequence, archive:Some(clip) };
                        self.ftp_upload_paths(i,files,Some(paste));
                    } else {
                        self.paste_archive_clipboard(i,clip);
                    }
                } else {
                    if let Some(stale)=self.archive_clipboard.take(){let _=std::fs::remove_dir_all(stale.stage);}
                    let now=native::clipboard_sequence_number();
                    let internal=self.clipboard.clone().filter(|_|self.clipboard_sequence==Some(now));
                    let copied=(self.clipboard_files_reader)().ok().filter(|(p,_)|!p.is_empty()).or(internal);
                    if self.clipboard_sequence!=Some(now){self.clipboard=None;self.clipboard_sequence=None;}
                    if let Some((sources,moving))=copied{
                        if self.panes[i].remote.is_some(){
                            self.ftp_upload_paths(i,sources,Some(super::ftp_pane::PasteUpload{moving,archive:None,sequence:now}));
                        }else if self.panes[i].archive.is_some(){self.archive_add_into(i,sources,moving);}
                        else{self.transfer_from(i,sources,path,moving,super::file_operation::TransferKind::Paste);}
                    }
                }
            },
            // Original miDeleteClick (0x431c04): if Shift (GetKeyState 0x10) is down when chosen, the last argument of FUN_0042c734 is 1 = permanent delete.
            "deleteRecycle"|"deletePermanent"=>self.delete_items(id=="deleteRecycle"&&!ctx.input(|i|i.modifiers.shift)),
            "shred"=>self.shred_items(),
            "copyFullPath"|"copyPathOnly"|"copyName"=>{let escape=ctx.input(|input|input.modifiers.shift);self.copy_item_paths(match id{"copyName"=>ndr::parity::PathPart::Name,"copyPathOnly"=>ndr::parity::PathPart::Folder,_=>ndr::parity::PathPart::Full},escape);},
            "copyFolder"=>{result=(self.clipboard_writer)(&path.to_string_lossy());},
            "selectAll" => {let shift=ctx.input(|i|i.modifiers.shift);self.cycle_selection(shift);},
            "invertSelection" => self.original_invert_selection(),
            // Original Edit menu miSelect ("Select\tSpacebar"): toggles the current item's mark like Space.
            "select"=>self.select_current_and_move(),
            // Original miRestoreLastSelClick (0x43742c): changes only when saved marks exist (skin theme).
            "restoreSelection" if self.last_selection[i].is_empty() => {},
            "restoreSelection"=>{let last=&self.last_selection[i];self.panes[i].selected=self.panes[i].entries.iter().enumerate().filter(|(_,e)|last.contains(&e.path)).map(|(j,_)|j).collect();},
            "selectSameName"|"selectSameExt" => self.original_select_same(id=="selectSameExt"),
            "advancedSelect"|"advancedDeselect"=>self.advanced_select(id=="advancedDeselect"),
            "toggleFolderInfoBar"=>self.toggle_info_bar("ShowFolderDiskInfoBar"),"toggleFileInfoBar"=>self.toggle_info_bar("ShowFileInfoBar"),"toggleExtensionInfoBar"=>self.toggle_info_bar("ShowExtensionInfoBar"),
            "toggleHidden"=>self.toggle_hidden(),"toggleDual"=>self.dual=!self.dual,
            "viewAuto"|"viewDetails"|"viewNames"|"viewThumbnails"=>self.set_view_style(&id[4..]),
            // Original miFolderTreeClick 0x43c81a: at a local location (FUN_00481948 == 0) FUN_0040ca90 (current folder) -> with a chosen path FUN_0048d6e4 (go to that folder).
            "folderTree"|"browseFolder"=>{
                if self.panes[i].archive.is_none()&&self.panes[i].remote.is_none(){
                    let start=self.panes[i].path.clone();
                    self.folder_tree_dialog(&start,None,move|app,path,_|{
                        if let Some(path)=path.filter(|p|!p.as_os_str().is_empty()){app.navigate(i,path,true);}
                    });
                }
            },
            "filter"=>self.show_filter_prompt(),
            "find"=>self.find_command(),"findNext"=>self.find_next(),
            // Original miFavoritesClick (0x4339d0): if a toolbar button has the command favorites, drop down that button (below it),
            // otherwise pmFavorites.Popup(GetCursorPos).
            "favorites"=>self.manage_favorites(),"favoritesMenu"=>{
                self.show_favorites_menu();
                let at=self.favorites_button.or_else(||ctx.input(|i|i.pointer.latest_pos()));
                if let Some(popup)=self.popup.as_mut(){popup.position=at;}
            },"addToFavorites"=>self.add_to_favorites_command(),
            
            "options"=>self.open_settings_window(),
            "importLegacy"=>self.import_legacy_command(),
            "shortcuts"=>self.show_shortcut_window(),
            "textView"|"textEdit"=>self.text_command(id=="textEdit"),
            "properties"=>self.properties_command(),
            "attributes"=>self.change_attributes(),
            // Original miFileInfoClick (0x4452bc): if the cursor is a file, opens https://ext.xiles.app/?ext=<extension>.
            "fileInfo" => self.open_extension_info(i),
            "archiveProperties"=>self.archive_properties_command(),
            "description"=>self.edit_descriptions(),
            "clearDescriptions"=>self.clear_descriptions_confirmed(i),
            "calculateFolderSize"=>self.calculate_folder_size(),
            "checksum"=>self.checksum_command(None),
            "search"=>self.search_command(),
            "compare"=>self.compare_command(),"synchronize"=>self.synchronize_command(),
            "saveList"=>self.save_list_command(),
            "split"=>self.split_command(),
            "join"=>self.join_command(),"merge"=>self.merge_text_files_command(),
            "archiveAdd"=>self.archive_command(super::archive_dialog::ArchiveKind::Add),"archiveMove"=>self.archive_command(super::archive_dialog::ArchiveKind::Move),"createArchive"=>self.archive_command(super::archive_dialog::ArchiveKind::Create),"createZip"=>self.archive_command(super::archive_dialog::ArchiveKind::Zip),
            "extractSelected"=>self.extract_command(super::archive_dialog::ExtractMode::Selected),"extractCurrent"=>self.extract_command(super::archive_dialog::ExtractMode::CurrentFolder),"extractNewFolder"=>self.extract_command(super::archive_dialog::ExtractMode::NewFolder),"extractAuto"=>self.extract_command(super::archive_dialog::ExtractMode::Auto),
            "archiveInstall"=>self.install_archive_command(),
            "command"=>self.command_prompt(),
            "openWithParameters"=>self.open_with_parameters(),"openWith"=>self.open_with_command(),
            "remote"=>self.open_remote_window(None,None),"sites"=>self.open_remote_sites(),
            "remoteCommand"=>self.run_remote_command(),
            "shortcut"|"quickLaunchShortcut"=>self.shortcut_command(id=="quickLaunchShortcut"),
            "pasteShortcut"=>self.paste_shortcut_command(),
            "diskCopy"=>self.open_disk_copy(),
            // User request: Disk Cleanup (original miCleanupDisk, TfCleanup) was removed.
            "emptyTemp"=>self.empty_temp_command(),
            "desktop"|"documents"|"pictures"|"music"|"videos"|"downloads"|"temp"=>self.special_folder_command(id),
            "openRecycle"=>result=(self.system.run_verb)(Path::new("shell:RecycleBinFolder"),"open"),
            "emptyRecycle"=>self.empty_recycle_command(),
            "browseNetwork"=>self.browse_network_command(),
            "mapDrive"=>self.map_drive_command(),
            "unmapDrive"=>self.unmap_drive_command(),
            // Original miControlPanelClick runs control.exe, miAddRemovePorgClick (0x432fc8) runs rundll32.exe "shell32.dll,Control_RunDLL appwiz.cpl".
            "controlPanel"=>result=(self.system.run_verb)(Path::new("control.exe"),"open"),"addRemovePrograms"=>result=(self.shell_runner)(Path::new("rundll32.exe"),"shell32.dll,Control_RunDLL appwiz.cpl",Path::new("")),
            "shellIntegration"=>self.explorer_integration_command(),
            // User request: the homepage is the NDR repository (original miHomepage was xiles.app); the original miEmail (copy the support address) was removed.
            "homepage"=>result=(self.shell_runner)(Path::new(HOMEPAGE_URL),"",Path::new("")),
            // User request: Help also opens the NDR repository (original miHelpClick (0x445b60) opened "https://www.xiles.app/help/nexusfile/").
            "help"=>result=(self.shell_runner)(Path::new(HOMEPAGE_URL),"",Path::new("")),
            // Original miMoreLanguagesClick (FUN_00445b88): ShellExecute "open" https://www.xiles.app/languages/#NexusFile (0x8ee3bc).
            "moreLanguages"=>result=(self.shell_runner)(Path::new("https://www.xiles.app/languages/#NexusFile"),"",Path::new("")),
            "about"=>self.about_command(),
            "update"=>self.check_for_updates(),
            _=>self.status=format!("Unknown command: {id}"),
        }
        if let Err(e) = result {
            self.status = e;
        }
    }
    /// C# FindNextQuickSearchMatch (miFindNextClick 0x437cd8, internal FindNext): typed quick-search text -> prefix
    /// search, else the last find text -> substring search, else nothing.
    pub(super) fn find_next(&mut self) {
        let (typed, last) = (self.typed_search[self.active].query.clone(), self.last_find.clone());
        if let Some((text, prefix)) = ndr::parity::find_next_query(&typed, &last) {
            self.find_in_active(&text.to_owned(), prefix);
        }
    }
    /// C# ClearDescriptionsAsync (original FUN_00434ca0): only when descript.dat exists, TfMsgBox(TITLE_DELDESCFILE, '',
    /// MSG_DELDESCFILE, MB_YESNO | MB_ICONQUESTION), then descript.dat is deleted.
    pub(super) fn clear_descriptions_confirmed(&mut self, pane: usize) {
        if self.panes[pane].archive.is_some() {
            self.status = self.tr("압축 파일이나 원격 폴더에서는 설명을 지울 수 없습니다.", "Comments cannot be cleared inside an archive or a remote folder.").into();
            return;
        }
        let folder = self.panes[pane].path.clone();
        if !folder.join("descript.dat").is_file() {
            return;
        }
        let title = self.language_pack.text("RES.TITLE_DELDESCFILE", "Delete Comments");
        let text = self.language_pack.text("RES.MSG_DELDESCFILE", "Are you sure you want to delete all comment of files in this folder?");
        self.message_box(&title, "", &text, ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONQUESTION, move |app, result, _| {
            if result != ndr::msgbox::IDYES {
                return;
            }
            match compat::clear_descriptions(&folder) {
                Ok(()) => app.refresh(pane),
                Err(error) => app.status = error,
            }
        });
    }
    pub(super) fn transfer_from(&mut self, i: usize, sources: Vec<PathBuf>, destination: PathBuf, moving: bool, kind: super::file_operation::TransferKind) {
        if sources.is_empty() {
            return;
        }
        if self.panes[i].archive.is_some() {
            // Original and C#: copying archive items extracted to that folder, moving was not possible. Per user request a modifiable archive is extracted and the items removed from it.
            if moving {
                let keys = sources.iter().map(|k| k.to_string_lossy().replace('\\', "/")).collect();
                return self.archive_transfer_out(i, keys, destination, None, true);
            }
            return self.extract_keys_to(i, sources, destination);
        }
        self.local_transfer(sources, destination, moving, kind);
    }
    /// Copy/move to the other pane when an archive is involved (user request, not in the original): if the destination is inside an archive, add into that folder;
    /// if the source is inside an archive, extract (or add to the destination archive) and on a move delete from the original archive. False when no archive is involved.
    pub(super) fn archive_route_to_other(&mut self, source: usize, moving: bool) -> bool {
        let target = 1 - source;
        if self.panes[source].archive.is_none() && self.panes[target].archive.is_none() {
            return false;
        }
        if self.panes[source].remote.is_some() || self.panes[target].remote.is_some() {
            return false;
        }
        let sources = self.panes[source].targets();
        if sources.is_empty() {
            return true;
        }
        if self.panes[target].archive.is_some() {
            let Some(into) = self.editable_archive(target) else {
                self.archive_read_only_notice();
                return true;
            };
            if self.panes[source].archive.is_some() {
                // Same-archive F3/F4 now follows the same guarded transfer
                // path as Ctrl+C/Ctrl+X. All source/destination overlap is
                // checked before any archive mutation there.
                let keys = sources.iter().map(|k| k.to_string_lossy().replace('\\', "/")).collect();
                let destination = self.panes[target].path.clone();
                self.archive_transfer_out(source, keys, destination, Some(into), moving);
            } else {
                self.archive_add_into(target, sources, moving);
            }
            return true;
        }
        let destination = self.panes[target].path.clone();
        self.transfer_from(source, sources, destination, moving, super::file_operation::TransferKind::OtherPane);
        true
    }
    pub(super) fn start_task(
        &mut self,
        title: String,
        task: impl FnOnce(Sender<Event>, Arc<AtomicBool>) -> Result<(), String> + Send + 'static,
    ) -> bool {
        if self.job.as_ref().is_some_and(|j| !j.finished) {
            self.status = "An operation is already running".into();
            return false;
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            if let Err(message) = task(tx.clone(), flag.clone()) {
                let _ = tx.send(Event::Error {
                    path: PathBuf::new(),
                    message,
                });
            }
            let _ = tx.send(Event::Finished {
                cancelled: flag.load(Ordering::Relaxed),
                files: 0,
                bytes: 0,
            });
        });
        self.job = Some(Job::new(rx, cancel, title));
        true
    }
    /// Runs start_task as a modal progress window (closes by itself when done). Does not start while another job is running.
    pub(super) fn start_modal_task(
        &mut self,
        title: String,
        modal: ModalProgress,
        task: impl FnOnce(Sender<Event>, Arc<AtomicBool>) -> Result<(), String> + Send + 'static,
    ) -> bool {
        if !self.start_task(title, task) {
            return false;
        }
        if let Some(job) = self.job.as_mut() {
            job.modal = Some(modal);
            job.progress = None;
        }
        true
    }
    /// Job that runs while showing the original TfProgress (cancel button only).
    pub(super) fn start_progress_task(
        &mut self,
        title: String,
        progress: super::file_operation::ProgressInfo,
        task: impl FnOnce(Sender<Event>, Arc<AtomicBool>) -> Result<(), String> + Send + 'static,
    ) -> bool {
        if !self.start_task(title, task) {
            return false;
        }
        if let Some(job) = self.job.as_mut() {
            job.progress = Some(progress);
        }
        true
    }
}
/// Original Recycle Bin button FUN_0042f13c (GetKeyState): no key = Recycle Bin (FUN_0042c734 mode 0), Shift only = delete (1),
/// Shift+Ctrl = shred (2), Ctrl+Alt = open the Recycle Bin (FUN_00432fe4). Other combinations do nothing.
pub(super) fn recycle_button_target(m: egui::Modifiers) -> Option<&'static str> {
    match (m.shift, m.ctrl, m.alt) {
        (false, false, false) => Some("deleteRecycle"),
        (true, false, false) => Some("deletePermanent"),
        (true, true, false) => Some("shred"),
        (false, true, true) => Some("openRecycle"),
        _ => None,
    }
}
/// C# FileOperationControl.WaitIfPausedAsync: waits while paused (returns at once on cancel).
pub(super) fn wait_while_paused(pause: &AtomicBool, cancel: &AtomicBool) {
    while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(50));
    }
}
pub(super) fn run_nested(operation:Operation,events:&Sender<Event>,cancel:&Arc<AtomicBool>)->Result<(),String>{let(tx,rx)=mpsc::channel();let flag=cancel.clone();thread::spawn(move||fsops::run(operation,tx,flag));let mut failure=None;for event in rx{match event{Event::Finished{..}=>{},Event::Error{ref message,..}=>{failure=Some(message.clone());let _=events.send(event);},_=>{let _=events.send(event);}}}failure.map_or(Ok(()),Err)}

pub(super) fn ask_conflict(events:&Sender<Event>,cancel:&AtomicBool,source:&Path,destination:&Path)->ConflictChoice{
    ask_conflict_resumable(events,cancel,source,destination,false)
}
/// `allow_resume`: shows the resume button in the conflict window (remote transfers, original btnResume).
pub(super) fn ask_conflict_resumable(events:&Sender<Event>,cancel:&AtomicBool,source:&Path,destination:&Path,allow_resume:bool)->ConflictChoice{
    let(tx,rx)=mpsc::channel();if events.send(Event::Conflict{source:source.to_path_buf(),destination:destination.to_path_buf(),allow_resume,reply:tx}).is_err(){return ConflictChoice::Cancel;}
    loop{if cancel.load(Ordering::Relaxed){return ConflictChoice::Cancel;}match rx.recv_timeout(Duration::from_millis(100)){Ok(choice)=>return choice,Err(mpsc::RecvTimeoutError::Timeout)=>{},Err(_)=>return ConflictChoice::Cancel}}
}
/// Original ARC_CHANGEVOL: true when the user inserts the next volume and confirms (retry). False on cancel or job cancel.
pub(super) fn ask_change_volume(events: &Sender<Event>, cancel: &AtomicBool, volume: &str) -> bool {
    let (tx, rx) = mpsc::channel();
    if events.send(Event::ChangeVolume { volume: volume.to_owned(), reply: tx }).is_err() {
        return false;
    }
    loop {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(retry) => return retry,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => return false,
        }
    }
}
pub(super) fn ask_password(events:&Sender<Event>,cancel:&AtomicBool,archive:&Path)->Option<String>{
    let(tx,rx)=mpsc::channel();if events.send(Event::Password{archive:archive.to_path_buf(),reply:tx}).is_err(){return None;}
    loop{if cancel.load(Ordering::Relaxed){return None;}match rx.recv_timeout(Duration::from_millis(100)){Ok(password)=>return password,Err(mpsc::RecvTimeoutError::Timeout)=>{},Err(_)=>return None}}
}
