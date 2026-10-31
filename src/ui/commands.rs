use super::*;

/// 도움말 메뉴의 홈페이지(사용자 요청: NDR 저장소).
pub(super) const HOMEPAGE_URL: &str = "https://github.com/UzuCore/NDR";

impl App {
    /// 원본 miSortBy의 항목들(miSortByNone … miSortByColor, 구분선, miSortDescend). 지금 정렬에 표시. 고른 명령.
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

    /// 원본 pmSortPopup(0x437464): 세로 막대 정렬 단추(bSort)의 pmSort는 miSortBy의 항목을 그대로 복사해(하위 메뉴 없이) 보이고,
    /// 활성 패널의 정렬 형식(GetSortType)과 내림차순(GetSortDescend)에 표시한다. 항목을 누르면 원래 항목의 OnClick(miSortByClick 등).
    pub(super) fn original_sort_popup(&mut self, ui: &mut egui::Ui, ko: bool) {
        if let Some(command) = self.sort_items(ui, ko) {
            self.sort_command(&command);
        }
    }

    /// C# AddSortMenu / AddFolderSortMenu / AddGroupMenu: 활성 패널 설정에 체크 표시가 붙는 메뉴.
    pub(super) fn sort_group_menus(&mut self, ui: &mut egui::Ui, ko: bool) {
        let mut chosen: Option<String> = None;
        // 원본 miSortBy("&A. 정렬"): 항목 miSortByNone … miSortByColor, miSortDescend.
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
            C::ExchangePath=>{/* 원본 FUN_0043d5ec(단축키 명령 0x23): 두 패널 모두 로컬 모드(0)일 때만 서로의 폴더로 간다. */ if self.panes.iter().all(|p|p.archive.is_none()&&p.remote.is_none()){let paths=self.panes.each_ref().map(|p|p.path.clone());self.navigate(0,paths[1].clone(),true);self.navigate(1,paths[0].clone(),true);}},
            C::ShowPathHistory=>self.show_history_menu(self.active,None),
            C::FocusAddressBar=>{/* C# BeginAddressEdit(side) = 원본 단축키 명령 0x25 FUN_0048d3c8(활성 패널). */ self.begin_address_edit(self.active,ctx)},
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
    /// 원본 FUN_0042d518(오른쪽으로)/FUN_0042d910(왼쪽으로) — 수직 막대 복사 단추와 단축키 명령(FUN_00443f14), 스킨 테마:
    /// 패널이 하나면 그만. 둘 다 로컬 폴더면 설정과 상관없이 TITLE_FILEOPERATION / MSG_TORIGHT·MSG_TOLEFT(%s = COPY 또는 MOVE)로 묻고,
    /// 로컬↔FTP는 올리기/내려받기. 원본은 압축 위치가 끼면 아무것도 하지 않았지만, 사용자 요청으로 NDR은 압축 안으로 더하기·압축에서 풀기를 한다.
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
        // 원본 FUN_0042d518/FUN_0042d910: 한쪽이 FTP면 올리기/내려받기(이동도 복사로).
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
        // 메뉴에는 원본 항목만 둔다(원본이 숨긴 항목도 숨긴다).
        let skin = self.current_skin();
        let shortcut_colors = skin.as_ref().map(super::skin_pane::skin_menu_shortcut_colors);
        let mut bar = egui::MenuBar::new();
        if let Some(skin) = &skin {
            // 원본 메뉴 색(스킨 [Menu]): 메뉴 줄 단추와 펼침 메뉴.
            let (_, bar_style) = super::skin_pane::skin_menu_bar_style(skin);
            bar = bar.style(bar_style).config(egui::containers::menu::MenuConfig::new().style(super::skin_pane::skin_menu_style(skin)));
        }
        bar.ui(ui, |ui| {
            // 지난 그림에 메뉴 줄의 메뉴가 열려 있었는지(마우스로 다른 메뉴로 옮겨 가기).
            let bar_open_id = egui::Id::new("ndr-menubar-open");
            let bar_was_open = ui.data(|d| d.get_temp::<bool>(bar_open_id)).unwrap_or(false) && egui::Popup::is_any_open(&ctx);
            let mut any_open = false;
            // 원본 TfMain: 메뉴 단추 tbFile … tbHelp(" 파일(&F) " 글)와 그 DropdownMenu pmFile … pmHelp.
            for (title_key, ko_title, en_title, items) in menu_data::MENUS.iter() {
                let raw_title = self.language_pack.raw(title_key).unwrap_or_else(|| (if ko { *ko_title } else { *en_title }).to_owned());
                let title = self.language_pack.text(title_key, if ko { ko_title } else { en_title }).trim().replace('&', "");
                let opened = ui.menu_button(title, |ui| {
                    // C# UpdateArchiveMenuItems(압축 메뉴를 열 때): 고른 압축 파일·지금 압축 파일에 따라 켜고/숨긴다.
                    let mut archive_state = self.archive_menu_state();
                    archive_state.original = true;
                    let shortcuts = compat::ShortcutMap::parse(self.settings.values.get("OriginalShortcutMapText").and_then(|v| v.as_str()));
                    // 구분선은 앞뒤에 보이는 항목이 있을 때만(숨긴 항목 때문에 겹치거나 끝에 남지 않게).
                    let (mut shown, mut pending) = (false, false);
                    // 하위 메뉴가 열려 있으면 친 글자는 그 하위 메뉴의 항목이 받는다.
                    let sub_open = Self::submenu_open(ui);
                    // 스킨 테마의 단축키 칸(원본 MenuMeasureItem: 단축키 최대 너비 + 0xe): 항목 뒤에 깔 자리와 지난 장면의 너비.
                    let width_id = egui::Id::new(("ndr-menu-shortcut-width", *title_key));
                    let previous_width = ui.data(|d| d.get_temp::<f32>(width_id)).unwrap_or(0.0);
                    let column_slot = shortcut_colors.map(|_| ui.painter().add(egui::Shape::Noop));
                    let mut widest = 0.0f32;
                    // 원본 MenuMeasureItem(0x42df68): 항목 너비 = 글 + 0x20 + 단축키 칸(가장 넓은 단축키 + 0xe). 줄마다 그 메뉴에서 가장
                    // 넓은 단축키 글과 빈칸 둘을 투명하게 잡아 이름 글이 단축키 칸 바탕에 물리지 않게 한다.
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
                            continue;
                        }
                        // 원본 miLanguage·miSkin은 하위 메뉴(FUN_00441830·FUN_0043e580).
                        if *id == "language" || *id == "theme" {
                            self.original_submenu(if *id == "language" { "language" } else { "skin" }, ui, &ctx, ko);
                            continue;
                        }
                        let label = self.menu_label(id, if ko { ko_label } else { en_label });
                        // 원본 pmViewPopup: 표시 전환·보기 방식은 체크 표시가 있는 항목.
                        let shortcut = Self::menu_shortcut_text(id, &shortcuts);
                        // 원본 MenuDrawItem: 체크 항목도 보통 항목처럼 그리고 체크되면 왼쪽에 CHECK 그림(보기 형식 등 라디오 항목은 RCHECK).
                        let button = if shortcut_colors.is_some() {
                            egui::Button::new(label).shortcut_text(RichText::new(reserve.clone()).color(Color32::TRANSPARENT))
                        } else {
                            egui::Button::new(label).shortcut_text(shortcut.clone())
                        };
                        let response = match self.menu_checked(id) {
                            Some(checked) => super::skin_pane::menu_check_button(ui, enabled, checked, Self::radio_menu_item(id), button),
                            None => ui.add_enabled(enabled, button),
                        };
                        if let Some(colors) = shortcut_colors {
                            // 고른 줄의 바탕(mhighlight)이 칸을 덮으므로 그 줄만 칸을 다시 칠한다(원본은 줄마다 칸을 칠한다).
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
                            self.menu_letter = None;
                            self.dispatch(id, &ctx);
                            ui.close();
                        }
                    }
                    if let (Some(slot), Some(colors)) = (column_slot, shortcut_colors) {
                        super::skin_pane::fill_menu_shortcut_column(ui, slot, widest, colors);
                        ui.data_mut(|d| d.insert_temp(width_id, widest));
                    }
                });
                let letter = super::favorites_dialog::access_key(&raw_title).1.map(|c| c.to_ascii_lowercase());
                if access.is_some() && letter == access {
                    egui::Popup::open_id(&ctx, egui::Popup::default_response_id(&opened.response));
                    ctx.request_repaint();
                }
                // Windows 메뉴 줄(원본 TToolBar 메뉴 단추): 한 메뉴가 열려 있는 동안 다른 메뉴 제목에 마우스를 올리면 그 메뉴로 바뀐다.
                let popup_id = egui::Popup::default_response_id(&opened.response);
                if opened.inner.is_some() {
                    any_open = true;
                } else if bar_was_open && opened.response.hovered() {
                    egui::Popup::open_id(&ctx, popup_id);
                    any_open = true;
                    ctx.request_repaint();
                }
            }
            ui.data_mut(|d| d.insert_temp(bar_open_id, any_open));
            // 원본 tbNewUpdateAvailable(조용한 업데이트 확인에서 새 버전을 찾았을 때). 원본 tbDonation(후원하기)은 사용자 요청으로 뺐다.
            if let Some(url) = self.update_available.clone() {
                let label = self.language_pack.text("ToolButton.tbNewUpdateAvailable", if ko { "새 업데이트" } else { "New Update Available" }).trim().to_owned();
                if ui.button(label).clicked() {
                    let _ = (self.shell_runner)(Path::new(&url), "", Path::new(""));
                }
            }
        });
    }

    /// 메뉴 명령의 원본 항목 이름(MENU_CAPTIONS, 아니면 ORIGINAL_ACTIONS). C# 판에만 있는 명령이면 없음.
    pub(super) fn original_menu_name(id: &str) -> Option<&'static str> {
        menu_data::MENU_CAPTIONS
            .iter()
            .find(|(command, _)| *command == id)
            .map(|(_, name)| *name)
            .or_else(|| menu_data::ORIGINAL_ACTIONS.iter().find(|(_, action)| *action == id).map(|(name, _)| *name))
    }

    /// 원본 메뉴에 없는(C# 판에만 있는) 메뉴 명령. 하위 메뉴("@…")는 원본에 있다.
    pub(super) fn csharp_only_menu_item(id: &str) -> bool {
        !id.starts_with('@') && Self::original_menu_name(id).is_none()
    }

    /// 메뉴 항목 글: 원본 항목 이름의 Menu.* 글, 없으면 C# 판 글. &는 뺀다.
    pub(super) fn menu_label(&self, id: &str, fallback: &str) -> String {
        Self::original_menu_name(id).map(|name| self.language_pack.text(&format!("Menu.{name}"), fallback)).unwrap_or_else(|| fallback.into()).replace('&', "")
    }

    /// 메뉴 항목의 접근 글자(원본 글 "&Q. 열기"의 q). 원본 글이 없으면(C# 판 항목) 없음.
    pub(super) fn menu_access_letter(&self, id: &str) -> Option<char> {
        let raw = self.language_pack.raw(&format!("Menu.{}", Self::original_menu_name(id)?))?;
        super::favorites_dialog::access_key(&raw).1.map(|c| c.to_ascii_lowercase())
    }

    /// 원본 Menu.<이름> 글(& 포함), 없으면 fallback.
    pub(super) fn raw_menu(&self, name: &str, fallback: &str) -> String {
        self.language_pack.raw(&format!("Menu.{name}")).unwrap_or_else(|| fallback.to_owned())
    }

    /// 메뉴가 열린 동안 친 글자가 이 원래 글(& 포함)의 접근 글자면 true. 글자는 써 버린다. `response` = 그 항목(같은 글자 항목 사이 이동용).
    pub(super) fn take_menu_letter(&mut self, raw: &str, response: &egui::Response) -> bool {
        let letter = super::favorites_dialog::access_key(raw).1.map(|c| c.to_ascii_lowercase());
        self.menu_target(letter, response)
    }

    /// 메뉴 명령 항목의 접근 글자(menu_access_letter)를 친 글자와 맞추어 본다. 맞으면 글자를 써 버린다.
    pub(super) fn take_item_letter(&mut self, id: &str, response: &egui::Response) -> bool {
        let letter = self.menu_access_letter(id);
        self.menu_target(letter, response)
    }

    /// 글자를 받을 수 있는 메뉴 항목 하나: 이번 장면 목록에 넣고(같은 글자가 둘 이상이면 keyboard가 선택만 옮긴다),
    /// 초점 요청이 이 항목이면 초점을 주고, 친 글자가 이 항목의 글자면 써 버리고 true.
    pub(super) fn menu_target(&mut self, letter: Option<char>, response: &egui::Response) -> bool {
        let hit = self.note_menu_target(letter, response);
        if hit {
            self.menu_letter = None;
        }
        hit
    }

    /// menu_target에서 글자를 써 버리지 않는 것(&self로 그리는 메뉴용): 목록에 넣고 초점을 주고, 친 글자와 맞는지만 돌려준다.
    pub(super) fn note_menu_target(&self, letter: Option<char>, response: &egui::Response) -> bool {
        let Some(letter) = letter else { return false };
        self.menu_targets.borrow_mut().push((letter, response.id));
        if self.menu_focus.get() == Some(response.id) {
            response.request_focus();
            self.menu_focus.set(None);
        }
        self.menu_letter == Some(letter)
    }

    /// 원본 하위 메뉴 항목(TMenuItem의 하위 항목, 예: "&H. 체크섬"). 메뉴가 열려 있고 다른 하위 메뉴가 열려 있지 않을 때
    /// 친 글자가 이 이름의 접근 글자면 Windows 메뉴처럼 하위 메뉴를 연다(egui MenuState.open_item). 하위 메뉴가 열린 동안에는
    /// 그 안의 항목이 글자를 받는다(take_menu_letter / take_item_letter).
    pub(super) fn access_submenu(&mut self, ui: &mut egui::Ui, raw_title: &str, add: impl FnOnce(&mut Self, &mut egui::Ui)) {
        use egui::containers::menu::{MenuState, SubMenu, SubMenuButton};
        let id = SubMenu::id_from_widget_id(ui.next_auto_id());
        let another_open = MenuState::from_ui(ui, |state, _| state.open_item.is_some_and(|open| open != id));
        let level_active = !Self::submenu_open(ui);
        let letter = super::favorites_dialog::access_key(raw_title).1.map(|c| c.to_ascii_lowercase());
        if !another_open && letter.is_some() && self.menu_letter == letter {
            self.menu_letter = None;
            // MenuState는 한 번도 그려지지 않은 하위 메뉴를 닫힌 것으로 되돌리므로 그려진 것으로 표시한다.
            MenuState::mark_shown(ui.ctx(), id);
            MenuState::from_ui(ui, |state, _| state.open_item = Some(id));
            ui.ctx().request_repaint();
        }
        let title = super::favorites_dialog::access_key(raw_title).0;
        let (response, _) = SubMenuButton::new(title).ui(ui, |ui| add(self, ui));
        // 하위 메뉴 이름도 같은 글자 항목 사이 이동의 대상이다(원본 보기 메뉴의 C = 기능키 표시줄·열 보기).
        if level_active {
            self.menu_target(letter, &response);
        }
    }

    /// 지금 메뉴에서 하위 메뉴가 열려 있는가(그러면 위 단계 항목은 친 글자를 받지 않는다).
    pub(super) fn submenu_open(ui: &egui::Ui) -> bool {
        egui::containers::menu::MenuState::from_ui(ui, |state, _| state.open_item.is_some())
    }

    /// 원본 TfMain DFM의 RadioItem = True 항목(miListStyleAuto·Names·Details·Thumbs). 체크되면 CHECK 대신 RCHECK.
    pub(super) fn radio_menu_item(command: &str) -> bool {
        matches!(command, "viewAuto" | "viewDetails" | "viewNames" | "viewThumbnails")
    }

    /// 원본 메뉴 항목 오른쪽의 단축키 글(TMenuItem.ShortCut, shortcut.ini가 바꾼 값을 Delphi ShortCutToText로). 없으면 빈 글.
    pub(super) fn menu_shortcut_text(id: &str, shortcuts: &compat::ShortcutMap) -> String {
        Self::original_menu_name(id)
            .and_then(|name| shortcuts.menu_shortcuts.get(name).copied())
            .filter(|code| *code != 0)
            .map(compat::shortcut_to_text)
            .unwrap_or_default()
    }

    /// 원본 메뉴의 하위 메뉴: 체크섬(miChecksum), 작업 폴더(miSetWorkFolder·miGoToWorkFolder), 보기 방식(miListStyle),
    /// 열(miColumn), 정렬(miSortBy, C# 판의 폴더 정렬·그룹 포함), 필터(miFilter).
    fn original_submenu(&mut self, name: &str, ui: &mut egui::Ui, ctx: &egui::Context, ko: bool) {
        match name {
            "checksum" => {
                let raw = self.raw_menu("miChecksum", if ko { "&H. 체크섬" } else { "&H. Checksum" });
                let mut chosen = None;
                self.access_submenu(ui, &raw, |app, ui| {
                    // 원본 miCRC·miMD5·miSHA1.
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
                // 원본 FUN_0043e580: skin\ 아래 폴더마다 항목, 지금 스킨에 체크. 누르면 FUN_0043e510(그 스킨 적용).
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
                // 원본 FUN_00441830: 언어마다 항목(지금 언어에 체크), 그 뒤 N23("-")와 miMoreLanguages(FUN_00445b88: xiles.app/languages).
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
                // 원본 miFTPConnect: FUN_00440734가 만든 사이트 줄("&1. 이름" …), N18("-"), miFTPManageSites.
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
        // 원본 패널 FTP 모드: 로컬 파일을 다루는 명령은 막는다.
        if self.panes[self.active].remote.is_some() && !super::ftp_pane::ftp_allows(id) {
            self.status = self.tr("FTP 패널에서는 쓸 수 없는 명령입니다.", "This command is not available in an FTP pane.").into_owned();
            return;
        }
        // 원본 pmConnect: 사이트 연결(FUN_00440734 줄 → FUN_0043e720), 빠른 연결, 끊기, 내려받기, 올리기.
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
        // 고친 것: 원본 단축키 표(shortcut.ini)가 팝업 항목(mipEditCopy 등)·하위 메뉴 이름에 글쇠를 걸면 그 이름의 명령 id가
        // 어디에서도 받지 않아 "Unknown command"가 되었다. 팝업 메뉴(pane_chrome)와 같은 명령으로 보낸다.
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
        // 하위 메뉴를 여는 항목(작업 폴더 지정·FTP 연결·스킨), 끌어 놓기 메뉴 항목(놓을 때만 뜸), 폴더 트리 창의 항목
        // (miTreeRefresh·miSubRefresh, TfMain에는 없다)은 주 창 글쇠로는 아무것도 하지 않는다.
        if matches!(id, "setWorkFolder" | "ftpConnect" | "theme" | "dropCopy" | "dropMove" | "dropShortcut" | "dropCancel" | "treeRefresh" | "treeRefreshRecursive") {
            return;
        }
        let i = self.active;
        // 원본 CopySelection(→ +0xd08): 복사·이동·삭제·완전 삭제·설명·고급 이름 바꾸기·이름 바꾸기·속성·압축 풀기·압축 더하기·
        // 바로 가기 만들기를 시작할 때 지금 표시를 "선택 복원"용으로 남긴다(스킨 테마). 목록 저장·합치기는 확인했을 때만 남긴다.
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
                // 원본 휴지통 단추 FUN_0042f13c: 누를 때의 Shift·Ctrl·Alt로 고른다.
                let Some(target)=recycle_button_target(ctx.input(|i|i.modifiers)) else { return };
                self.dispatch(target,ctx)
            },"extractToolbar"=>self.dispatch("extractSelected",ctx),
            "open"=>self.open_current(),
            "openAll"=>self.open_all_command(),
            "explorerMenu"=>result=ndr::shell_menu::show(&targets),
            "sortNone"|"sortName"|"sortExtension"|"sortSize"|"sortTime"|"sortAttributes"|"sortColor"|"folderSortDescending"|"groupDescending"|"groupShowName"=>{self.sort_command(id);},
            "sortDescending"=>{self.sort_command(id);},
            // 원본 miShowFoldersClick(0x445258): 지금 패널(+0xe08)의 트리를 켜고 끄며 그 패널 설정(+0x50 + 패널 번호)에 남긴다.
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
            // 원본 miCopyToClick(0x432e68)·miMoveToClick(0x432f40): 로컬 위치면 FUN_0042bdc4(대상 없음) = TfSelectDest(경로 칸 처음 값 =
            // 다른 패널, 고쳐서 같은 패널 안 폴더로도), 압축 위치면 FUN_00439bac(압축 풀기 대상 선택). FTP 위치는 올리기·내려받기.
            "copyToOther"|"moveToOther" if self.panes[i].remote.is_none()&&self.panes[1-i].remote.is_none()=>self.copy_move_to_folder(id=="moveToOther"),
            "copyToOther"|"moveToOther"=>self.copy_to_other(i,id=="moveToOther",ctx),
            "copyToFolder"|"moveToFolder"=>self.copy_move_to_folder(id=="moveToFolder"),
            "copy"|"cut"=>{if !targets.is_empty(){self.clipboard=Some((targets.clone(),id=="cut"));if self.panes[i].archive.is_none(){result=(self.file_clipboard)(&targets,id=="cut");}
                // 원본 0x433dd8: 로컬 위치에서 잘라내면 그 항목들(+100)을 흐린 아이콘으로 다시 그린다. 복사는 표시를 건드리지 않는다.
                if id=="cut"&&self.panes[i].archive.is_none()&&self.panes[i].remote.is_none(){self.panes[i].cut_marks.extend(targets.iter().cloned());}}},
            "paste"=>{let copied=native::read_clipboard_files().ok().filter(|(p,_)|!p.is_empty()).or_else(||self.clipboard.clone());if let Some((sources,moving))=copied{if self.panes[i].archive.is_some(){self.archive_add_into(i,sources,moving);}else{self.transfer_from(i,sources,path,moving,super::file_operation::TransferKind::Paste);}}},
            // 원본 miDeleteClick(0x431c04): 고를 때 Shift(GetKeyState 0x10)가 눌려 있으면 FUN_0042c734의 마지막 인자 1 = 영구 삭제.
            "deleteRecycle"|"deletePermanent"=>self.delete_items(id=="deleteRecycle"&&!ctx.input(|i|i.modifiers.shift)),
            "shred"=>self.shred_items(),
            "copyFullPath"|"copyPathOnly"|"copyName"=>{let escape=ctx.input(|input|input.modifiers.shift);self.copy_item_paths(match id{"copyName"=>ndr::parity::PathPart::Name,"copyPathOnly"=>ndr::parity::PathPart::Folder,_=>ndr::parity::PathPart::Full},escape);},
            "copyFolder"=>{result=(self.clipboard_writer)(&path.to_string_lossy());},
            "selectAll" => {let shift=ctx.input(|i|i.modifiers.shift);self.cycle_selection(shift);},
            "invertSelection" => self.original_invert_selection(),
            // 원본 편집 메뉴 miSelect("선택\tSpacebar"): Space와 같은 지금 항목 표시 전환.
            "select"=>self.select_current_and_move(),
            // 원본 miRestoreLastSelClick(0x43742c): 남긴 표시가 있을 때만 바꾼다(스킨 테마).
            "restoreSelection" if self.last_selection[i].is_empty() => {},
            "restoreSelection"=>{let last=&self.last_selection[i];self.panes[i].selected=self.panes[i].entries.iter().enumerate().filter(|(_,e)|last.contains(&e.path)).map(|(j,_)|j).collect();},
            "selectSameName"|"selectSameExt" => self.original_select_same(id=="selectSameExt"),
            "advancedSelect"|"advancedDeselect"=>self.advanced_select(id=="advancedDeselect"),
            "toggleFolderInfoBar"=>self.toggle_info_bar("ShowFolderDiskInfoBar"),"toggleFileInfoBar"=>self.toggle_info_bar("ShowFileInfoBar"),"toggleExtensionInfoBar"=>self.toggle_info_bar("ShowExtensionInfoBar"),
            "toggleHidden"=>self.toggle_hidden(),"toggleDual"=>self.dual=!self.dual,
            "viewAuto"|"viewDetails"|"viewNames"|"viewThumbnails"=>self.set_view_style(&id[4..]),
            // 원본 miFolderTreeClick 0x43c81a: 로컬 위치(FUN_00481948 == 0)면 FUN_0040ca90(지금 폴더) → 고른 경로가 있으면 FUN_0048d6e4(그 폴더로).
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
            // 원본 miFavoritesClick(0x4339d0): 도구 모음에 command가 favorites인 단추가 있으면 그 단추의 DropDown(단추 아래),
            // 없으면 pmFavorites.Popup(GetCursorPos).
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
            // 원본 miFileInfoClick(0x4452bc): 커서가 파일이면 https://ext.xiles.app/?ext=확장자 를 연다.
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
            // 사용자 요청: 컴퓨터 청소(원본 miCleanupDisk, TfCleanup)는 뺐다.
            "emptyTemp"=>self.empty_temp_command(),
            "desktop"|"documents"|"pictures"|"music"|"videos"|"downloads"|"temp"=>self.special_folder_command(id),
            "openRecycle"=>result=(self.system.run_verb)(Path::new("shell:RecycleBinFolder"),"open"),
            "emptyRecycle"=>self.empty_recycle_command(),
            "browseNetwork"=>self.browse_network_command(),
            "mapDrive"=>self.map_drive_command(),
            "unmapDrive"=>self.unmap_drive_command(),
            // 원본 miControlPanelClick은 control.exe, miAddRemovePorgClick(0x432fc8)은 rundll32.exe "shell32.dll,Control_RunDLL appwiz.cpl".
            "controlPanel"=>result=(self.system.run_verb)(Path::new("control.exe"),"open"),"addRemovePrograms"=>result=(self.shell_runner)(Path::new("rundll32.exe"),"shell32.dll,Control_RunDLL appwiz.cpl",Path::new("")),
            "shellIntegration"=>self.explorer_integration_command(),
            // 사용자 요청: 홈페이지는 NDR 저장소(원본 miHomepage는 xiles.app), 원본 miEmail(지원 이메일 복사)은 뺐다.
            "homepage"=>result=(self.shell_runner)(Path::new(HOMEPAGE_URL),"",Path::new("")),
            // 사용자 요청: 도움말도 NDR 저장소(원본 miHelpClick(0x445b60)은 "https://www.xiles.app/help/nexusfile/").
            "help"=>result=(self.shell_runner)(Path::new(HOMEPAGE_URL),"",Path::new("")),
            // 원본 miMoreLanguagesClick(FUN_00445b88): ShellExecute "open" https://www.xiles.app/languages/#NexusFile (0x8ee3bc).
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
            // 원본·C#: 압축 안 항목 복사는 그 폴더로 풀기, 이동은 안 됐다. 사용자 요청으로 고칠 수 있는 압축이면 풀고 압축에서 지운다.
            if moving {
                let keys = sources.iter().map(|k| k.to_string_lossy().replace('\\', "/")).collect();
                return self.archive_transfer_out(i, keys, destination, None, true);
            }
            return self.extract_keys_to(i, sources, destination);
        }
        self.local_transfer(sources, destination, moving, kind);
    }
    /// 다른 패널로 복사·이동할 때 압축이 끼면(사용자 요청, 원본에는 없음): 대상이 압축 안이면 그 안 폴더에 더하고,
    /// 원본이 압축 안이면 풀거나(대상이 압축이면 그 압축에 더하고) 이동이면 원래 압축에서 지운다. 압축이 안 끼면 false.
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
                let same = self.panes[source].archive.as_ref().is_some_and(|a| ndr::names::equals_ignore_case(&ndr::archive::first_volume_path(&a.file).to_string_lossy(), &into.0.to_string_lossy()));
                if same && moving {
                    self.notice("같은 압축 파일 안에서는 이동할 수 없습니다. 이름 바꾸기를 사용하세요.", "Items cannot be moved within the same archive. Use rename instead.");
                    return true;
                }
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
    /// start_task를 모달 진행 창으로(끝나면 저절로 닫힌다). 이미 다른 작업이 돌고 있으면 시작하지 않는다.
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
    /// 원본 TfProgress(취소 단추만)를 보이며 도는 작업.
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
/// 원본 휴지통 단추 FUN_0042f13c(GetKeyState): 아무 키 없음 = 휴지통(FUN_0042c734 형식 0), Shift만 = 삭제(1),
/// Shift+Ctrl = 완전 삭제(2), Ctrl+Alt = 휴지통 열기(FUN_00432fe4). 다른 조합은 아무것도 하지 않는다.
pub(super) fn recycle_button_target(m: egui::Modifiers) -> Option<&'static str> {
    match (m.shift, m.ctrl, m.alt) {
        (false, false, false) => Some("deleteRecycle"),
        (true, false, false) => Some("deletePermanent"),
        (true, true, false) => Some("shred"),
        (false, true, true) => Some("openRecycle"),
        _ => None,
    }
}
/// C# FileOperationControl.WaitIfPausedAsync: 일시 정지 동안 기다린다(취소하면 바로 돌아간다).
pub(super) fn wait_while_paused(pause: &AtomicBool, cancel: &AtomicBool) {
    while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(50));
    }
}
pub(super) fn run_nested(operation:Operation,events:&Sender<Event>,cancel:&Arc<AtomicBool>)->Result<(),String>{let(tx,rx)=mpsc::channel();let flag=cancel.clone();thread::spawn(move||fsops::run(operation,tx,flag));let mut failure=None;for event in rx{match event{Event::Finished{..}=>{},Event::Error{ref message,..}=>{failure=Some(message.clone());let _=events.send(event);},_=>{let _=events.send(event);}}}failure.map_or(Ok(()),Err)}

pub(super) fn ask_conflict(events:&Sender<Event>,cancel:&AtomicBool,source:&Path,destination:&Path)->ConflictChoice{
    let(tx,rx)=mpsc::channel();if events.send(Event::Conflict{source:source.to_path_buf(),destination:destination.to_path_buf(),allow_resume:false,reply:tx}).is_err(){return ConflictChoice::Cancel;}
    loop{if cancel.load(Ordering::Relaxed){return ConflictChoice::Cancel;}match rx.recv_timeout(Duration::from_millis(100)){Ok(choice)=>return choice,Err(mpsc::RecvTimeoutError::Timeout)=>{},Err(_)=>return ConflictChoice::Cancel}}
}
/// 원본 ARC_CHANGEVOL: 다음 볼륨을 넣고 확인하면 true(다시 시도). 취소·작업 취소면 false.
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
