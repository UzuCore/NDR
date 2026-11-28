//! File list key order (original FormShortCut -> list KeyDown FUN_0048423c -> WMChar):
//! Esc (EscapeKeyAction) -> original shortcut table (shortcut.ini) -> favorites Shift(+Ctrl)+F1-F10 -> function key table -> rest of KeyDown
//! ('.' parent folder, Ctrl(+Shift)+Tab tabs, Shift+letter drive) -> list movement (Up/Down/Home/End/Tab; egui has no list box).
//! Also: CycleFileSelection, ToggleFocusedItemSelection, favoriteOpen:/workOpen:/ of ExecuteShortcutActionAsync and ExecuteMenuCommandAsync,
//! workSet:/column:, RememberDriveFolder/RestoreLastDriveFolder, drive buttons (ShowDriveButtons, ready drives, volume name tooltips).
//! Difference (HANDOFF): while a text field has focus no list key is taken (C# takes a few such as back and forward).
use super::remote_window::wpf_key;
use super::*;

/// Convert an owned-window Win32 Alt press back to the original key
/// namespace. A native chord is authoritative only for a configured
/// command, never for text input, AltGr or an OS-reserved chord.
fn alt_virtual_key(vk:u16)->Option<egui::Key>{
    match vk{
        0x30..=0x39|0x41..=0x5a=>egui::Key::from_name(&(vk as u8 as char).to_string()),
        0x70..=0x7b=>egui::Key::from_name(&format!("F{}",vk-0x70+1)),
        0x0d=>Some(egui::Key::Enter),0x21=>Some(egui::Key::PageUp),0x22=>Some(egui::Key::PageDown),
        0x23=>Some(egui::Key::End),0x24=>Some(egui::Key::Home),0x25=>Some(egui::Key::ArrowLeft),
        0x26=>Some(egui::Key::ArrowUp),0x27=>Some(egui::Key::ArrowRight),0x28=>Some(egui::Key::ArrowDown),
        0xba=>Some(egui::Key::Semicolon),0xbb=>Some(egui::Key::Equals),0xbc=>Some(egui::Key::Comma),
        0xbd=>Some(egui::Key::Minus),0xbe=>Some(egui::Key::Period),0xbf=>Some(egui::Key::Slash),
        0xc0=>Some(egui::Key::Backtick),0xdb=>Some(egui::Key::OpenBracket),0xdc=>Some(egui::Key::Backslash),
        0xdd=>Some(egui::Key::CloseBracket),0xde=>Some(egui::Key::Quote),_=>None,
    }
}

pub(super) fn restore_native_alt_shortcuts(raw:&mut egui::RawInput,presses:&[ndr::shell_menu::NativeAltKey],custom:Option<&str>)->usize{
    if presses.is_empty(){return 0;}
    let map=compat::ShortcutMap::parse(custom);
    let mut used=vec![false;raw.events.len()];
    let mut recovered=0;
    for press in presses{
        let Some(key)=alt_virtual_key(press.vk)else{continue};
        let modifiers=egui::Modifiers{alt:true,shift:press.shift,..Default::default()};
        if map.resolve(compat::text_to_shortcut(&original_shortcut_text(key,modifiers))).is_none(){continue;}
        let matching=|e:&egui::Event|matches!(e,egui::Event::Key{key:k,physical_key,pressed:true,modifiers:m,..}
            if !m.ctrl && (*k==key||*physical_key==Some(key)));
        let preferred=raw.events.iter().take(used.len()).enumerate().position(|(i,e)|!used[i]&&matching(e)&&matches!(e,egui::Event::Key{modifiers:m,..} if m.alt));
        let index=preferred.or_else(||raw.events.iter().take(used.len()).enumerate().position(|(i,e)|!used[i]&&matching(e)));
        if let Some(i)=index{
            used[i]=true;
            if let egui::Event::Key{key:k,physical_key,modifiers:m,..}=&mut raw.events[i]{
                if *k!=key||*m!=modifiers{recovered+=1;}
                *k=key;*physical_key=Some(key);*m=modifiers;
            }
        }else{
            raw.events.push(egui::Event::Key{key,physical_key:Some(key),pressed:true,repeat:false,modifiers});
            // A framework release can exist without a press. Keep key
            // state from remaining latched when Alt has already gone up.
            if !raw.modifiers.alt{raw.events.push(egui::Event::Key{key,physical_key:Some(key),pressed:false,repeat:false,modifiers:raw.modifiers});}
            recovered+=1;
        }
    }
    recovered
}

#[derive(Debug)]
pub(super) enum DeferredListAction {
    Dispatch(String),
    CopyOther { source:usize, moving:bool, confirm:bool },
}
pub(super) struct DeferredListCommand {
    action:DeferredListAction,
    source:usize,
    active:usize,
    locations:[String;2],
    tabs:[usize;2],
    generation:u64,
    modifiers:Option<egui::Modifiers>,
    cursor:Option<Entry>,
    targets:Vec<Entry>,
    deadline:Instant,
}

fn same_command_entry(a:&Entry,b:&Entry)->bool{
    a.path==b.path && a.name==b.name && a.is_dir==b.is_dir && a.size==b.size
        && a.modified==b.modified && a.attributes==b.attributes && a.drive==b.drive
}

pub(super) fn selection_command(id:&str)->bool{
    matches!(id,
        "open"|"openAll"|"openInNewTab"|"openWith"|"openWithParameters"|
        "rename"|"advancedRename"|"batchRename"|"copy"|"cut"|"copyFullPath"|"copyPathOnly"|"copyName"|"copyToOther"|"moveToOther"|
        "copyToFolder"|"moveToFolder"|"deleteRecycle"|"deletePermanent"|"deleteToolbar"|"shred"|
        "textView"|"textEdit"|"properties"|"attributes"|"archiveProperties"|"description"|
        "checksum"|"calculateFolderSize"|"split"|"join"|"merge"|"archiveAdd"|"archiveMove"|
        "createArchive"|"createZip"|"compressEach"|"extractEach"|"extractSelected"|"extractCurrent"|"extractNewFolder"|
        "extractToolbar"|"archiveInstall"|"shortcut"|"explorerMenu"|
        "contextView"|"contextEdit"|"contextCopy"|"contextCut"|"contextCopyTo"|"contextMoveTo"|
        "contextDelete"|"contextRename"|"contextAttributes"|"contextDescription"|
        "contextArchiveAdd"|"contextProperties")
        || id.starts_with("checksum:")
}

impl App {
    /// In the original main window keys always go to the list (TNXCustomFileListView) (buttons have no TabStop). egui moves key focus to buttons
    /// with Tab or arrows, turns Enter/Space on a focused button into a click, and wants_keyboard_input is true meanwhile, cutting off list keys.
    /// So when a widget other than a text field on the main window (background layer) has focus it is released, and with no focus egui focus movement is disabled.
    /// Focus in floating windows and menus (middle and foreground layers) is left alone.
    pub(super) fn drop_widget_focus(&mut self, ctx: &egui::Context) {
        if let Some(id) = ctx.memory(|m| m.focused()) {
            let text_edit = egui::text_edit::TextEditState::load(ctx, id).is_some();
            let background = ctx.read_response(id).is_none_or(|r| r.layer_id.order == egui::Order::Background);
            if text_edit || !background {
                return;
            }
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }

    /// Use the previous frame's actual list rectangle, never an address
    /// bar, tree, toolbar, or floating window.
    pub(super) fn list_press_pane(&self,ctx:&egui::Context,events:&[egui::Event])->Option<usize>{
        events.iter().find_map(|e|match e{
            egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,..}
                if ctx.layer_id_at(*pos).is_none_or(|l|l.order==egui::Order::Background)
                =>self.panes.iter().enumerate().find(|(_,p)|p.list_rect.is_some_and(|r|r.contains(*pos))).map(|(i,_)|i),
            _=>None,
        })
    }

    pub(super) fn release_list_text_focus(&mut self,ctx:&egui::Context){
        if let Some(id)=ctx.memory(|m|m.focused()){
            if id==egui::Id::new("ndr-toolbar-query")||id==egui::Id::new(("ndr-address",0usize))||id==egui::Id::new(("ndr-address",1usize)){
                ctx.memory_mut(|m|m.surrender_focus(id));
                for pane in 0..2{if self.panes[pane].address_editing{self.end_address_edit(pane);}}
            }
        }
    }

    /// If the click and Alt chord arrived in the same batch, draw and
    /// apply the row press first. Never run against the old pane/cursor.
    pub(super) fn keyboard_after_list_press(&mut self,ctx:&egui::Context)->bool{
        let events=ctx.input(|i|{
            (i.pointer.primary_pressed()&&i.events.iter().any(|e|matches!(e,egui::Event::Key{pressed:true,modifiers:m,..} if m.alt&&!m.ctrl))).then(||i.events.clone())
        });
        let Some(events)=events else{return false};
        let map=compat::ShortcutMap::parse(self.settings.values.get("OriginalShortcutMapText").and_then(|v|v.as_str()));
        let bound=events.iter().any(|e|match e{
            egui::Event::Key{key,pressed:true,modifiers:m,..} if m.alt&&!m.ctrl=>map.resolve(compat::text_to_shortcut(&original_shortcut_text(*key,*m))).is_some(),
            _=>false,
        });
        let pending=if bound{self.list_press_pane(ctx,&events)}else{None};
        if pending.is_some()&&!self.keyboard_blocked(ctx){
            self.release_list_text_focus(ctx);
            self.menu_letter=None;
            self.menu_access=None;
            true
        }else{false}
    }

    /// Only Alt+Q/extraction diagnostics, stored locally and bounded.
    /// Never record actual paths, text input, clipboard, or passwords.
    pub(super) fn trace_auto_extract(&self,stage:&str,detail:&str){
        use std::io::Write;
        let dir=self.profile.join("cache");
        let path=dir.join("shortcut-diagnostics.log");
        if ndr::fsops::ensure_plain_path(&path).is_err(){return;}
        if std::fs::create_dir_all(&dir).is_err(){return;}
        let reset=std::fs::metadata(&path).is_ok_and(|m|m.len()>=64*1024);
        let mut options=std::fs::OpenOptions::new();
        options.create(true).write(true);
        if reset{options.truncate(true);}else{options.append(true);}
        if let Ok(mut file)=options.open(path){
            let _=writeln!(file,"{} stage={} {}",chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),stage,detail);
        }
    }

    pub(super) fn keyboard(&mut self,ctx:&egui::Context){
        self.shortcut_routing=true;
        self.keyboard_inner(ctx);
        self.shortcut_routing=false;
        self.shortcut_modifiers=None;
    }

    pub(super) fn command_modifiers(&self,ctx:&egui::Context)->egui::Modifiers{
        self.shortcut_modifiers.unwrap_or_else(||ctx.input(|i|i.modifiers))
    }

    fn keyboard_inner(&mut self, ctx: &egui::Context) {
        let auto=ctx.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Key{key:egui::Key::Q,pressed:true,modifiers:m,..} if m.alt&&!m.ctrl&&!m.shift)));
        if auto{
            let p=&self.panes[self.active];
            self.trace_auto_extract("keyboard",&format!("pane={} loading={} archive={} remote={} vault={} marked={} cursor={} job_block={} dialogs={} messages={} popup={} focused={} wants_keyboard={}",
                self.active,p.loading,p.archive.is_some(),p.remote.is_some(),p.vault.is_some(),p.selected.len(),p.cursor,
                self.job.as_ref().is_some_and(|j|!j.finished),self.original_dialogs.len(),self.msgboxes.len(),self.popup.is_some()||egui::Popup::is_any_open(ctx),
                ctx.input(|i|i.focused),ctx.wants_keyboard_input()));
        }
        self.menu_letter = None;
        self.menu_targets_seen = std::mem::take(self.menu_targets.get_mut());
        // Key focus of the pane folder tree: returns to the list when another pane becomes current or the pane is clicked outside the tree (the list).
        if self.tree_focus.is_some_and(|i| i != self.active) {
            self.tree_focus = None;
        }
        if let Some(i) = self.tree_focus {
            let (rects, tree) = (self.pane_rects, self.panes[i].tree_rect);
            let outside = ctx.input(|input| input.pointer.primary_pressed() && input.pointer.interact_pos().is_some_and(|p| rects.iter().any(|r| r.contains(p)) && !tree.is_some_and(|t| t.contains(p))));
            if outside {
                self.tree_focus = None;
            }
        }
        if self.job.as_ref().is_some_and(|j| !j.finished && j.progress.is_some() && j.reservation.is_none())
            || !self.msgboxes.is_empty()
            || !self.original_dialogs.is_empty()
            || self.job.as_ref().is_some_and(|j| j.modal.is_some())
            || self.popup.is_some()
        {
            self.pending_numpad.clear();
            self.pending_list_command=None;
            return;
        }
        // Original ApplicationEvents1Message (0x442984): WM_XBUTTONDOWN (0x20b) with XBUTTON1 is HistoryBack, XBUTTON2 is HistoryForward.
        // (NDR ignores them while a dialog or progress window is open.)
        let (back, forward) = ctx.input(|i| (i.pointer.button_pressed(egui::PointerButton::Extra1), i.pointer.button_pressed(egui::PointerButton::Extra2)));
        if back {
            self.dispatch("back", ctx);
        } else if forward {
            self.dispatch("forward", ctx);
        }
        // While a menu is open (original Windows menu mode) keys do not go to the pane. A typed letter picks that menu's item with the access letter.
        if egui::Popup::is_any_open(ctx) {
            self.pending_numpad.clear();
            self.pending_list_command=None;
            self.menu_letter = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Text(t) = e { t.chars().next() } else { None })).map(|c| c.to_ascii_lowercase());
            // Windows menus: with several items on one access letter, the selection only moves to the next one (Enter runs it).
            if let Some(letter) = self.menu_letter {
                let same: Vec<egui::Id> = self.menu_targets_seen.iter().filter(|(c, _)| *c == letter).map(|(_, id)| *id).collect();
                if same.len() > 1 {
                    let focused = ctx.memory(|m| m.focused());
                    let next = focused.and_then(|f| same.iter().position(|id| *id == f)).map_or(0, |i| (i + 1) % same.len());
                    self.menu_focus.set(Some(same[next]));
                    self.menu_letter = None;
                    ctx.request_repaint();
                }
            }
            return;
        }
        self.drop_widget_focus(ctx);
        // Discard native events when a real text widget owns focus.
        let numpad=std::mem::take(&mut self.pending_numpad);
        if ctx.wants_keyboard_input() || !ctx.input(|i|i.focused) {
            self.pending_list_command=None;
            return;
        }
        if !numpad.is_empty(){self.pending_list_command=None;}
        if self.resume_list_command(ctx) { return; }
        // One command per frame: never run a second numpad command into
        // a dialog opened by the first, or leave a command pending.
        for (original,shift) in numpad {
            self.shortcut_modifiers=Some(egui::Modifiers{shift,..ctx.input(|i|i.modifiers)});
            if self.run_original_shortcut(&original,shift,ctx) {
                self.consume_shortcut_input(ctx);
                return;
            }
        }
        // Explicit command chords and non-text function keys must be
        // resolved before IME/quick search. Use each key's own modifier
        // snapshot, not the end-of-frame state (keys may already be up).
        let (keys,has_text,has_ime)=ctx.input(|i|{
            let keys=i.events.iter().filter_map(|event|match event {
                egui::Event::Key{key,pressed:true,modifiers,..}=>Some((*key,*modifiers)),
                _=>None,
            }).collect::<Vec<_>>();
            (keys,i.events.iter().any(|e|matches!(e,egui::Event::Text(t) if !t.is_empty())),
                i.events.iter().any(|e|matches!(e,egui::Event::Ime(_))))
        });
        let altgr=|m:egui::Modifiers|m.ctrl&&m.alt&&has_text;
        let priority=|key:egui::Key,m:egui::Modifiers| {
            !altgr(m) && (m.ctrl||m.alt
                || key.name().strip_prefix('F').is_some_and(|n|n.parse::<u8>().is_ok())
                || matches!(key,egui::Key::Insert|egui::Key::Delete)&&!has_ime)
        };
        for &(key,modifiers) in &keys {
            if !priority(key,modifiers){continue;}
            self.shortcut_modifiers=Some(modifiers);
            if self.shortcut_key(key,modifiers,ctx) {
                self.consume_shortcut_input(ctx);
                return;
            }
            // Ctrl+Home/End/Page and Ctrl+Shift range navigation are
            // list commands too, not IME text.
            if modifiers.ctrl&&!modifiers.alt&&self.list_navigation_key(key,modifiers) {
                self.consume_shortcut_input(ctx);
                self.clear_released_shift(ctx);
                return;
            }
            if self.unbound_menu_access(key,modifiers) {
                self.consume_shortcut_input(ctx);
                return;
            }
        }
        // When the pane folder tree has focus, TTreeView keys come first (the rest go to the same KeyDown FUN_0048423c as the list).
        if self.tree_keys(ctx) {
            return;
        }
        // Original KeyDown (0x7d7338) -> ArrowKey handles [ ] first, then the WMChar quick search (which also takes the same letters).
        self.bracket_keys(ctx);
        // A real Shift+drive-letter key first belongs to the original KeyDown
        // command (FUN_0048423c), not WMChar quick search. egui also supplies
        // Text("C") with Shift+C; allowing quick search to consume that text
        // silently prevents the drive shortcut even with hanshdrive enabled.
        let shift_drive = keys.iter().copied().any(|(key, modifiers)| {
            if !modifiers.shift || modifiers.ctrl || modifiers.alt || self.panes[self.active].remote.is_some()
                || !(native::english_input_language() || self.settings.flag("UseAlternateDriveHandling", false))
            {
                return false;
            }
            wpf_key(key).is_some_and(|name| name.len() == 1
                && name.as_bytes()[0].is_ascii_uppercase()
                && Path::new(&format!("{name}:\\")).is_dir())
        });
        // Original TNXCustomFileListView quick search (WMChar 0x7d5a18 -> QuickSearch 0x7d5f9c).
        if !shift_drive && self.original_typed_search(ctx) {
            return;
        }
        // Check every remaining press, not just the first key in a
        // coalesced event batch. Do not dispatch a printable AltGr chord.
        for (key,modifiers) in keys {
            if priority(key,modifiers)||altgr(modifiers){continue;}
            self.shortcut_modifiers=Some(modifiers);
            if self.shortcut_key(key,modifiers,ctx) {
                self.consume_shortcut_input(ctx);
                return;
            }
            if self.unbound_menu_access(key,modifiers) { return; }
        }
        self.shortcut_modifiers=None;
        self.list_navigation(ctx);
    }

    /// A watcher relist is not a navigation. For commands that require
    /// fresh rows, remember the exact visible selection for a bounded
    /// interval. Never execute against stale rows from another folder.
    pub(super) fn defer_list_command(&mut self,source:usize,action:DeferredListAction,ctx:&egui::Context)->bool{
        let pane=&self.panes[source];
        if !self.shortcut_routing || !pane.loading || pane.archive.is_some()
            || pane.remote.is_some() || pane.vault.is_some() {return false;}
        let location=location_key(pane);
        if pane.listed.as_deref()!=Some(location.as_str()) || pane.entries.is_empty(){
            self.status=self.catalog("NDR.UI_SHORTCUT_LOADING","폴더를 읽는 중입니다. 목록이 표시되면 다시 실행하세요.","Reading the folder. Retry when the list is ready.");
            return true;
        }
        let cursor=pane.entries.get(pane.cursor).cloned();
        let targets=self.displayed_target_entries(source);
        let deadline=Instant::now()+Duration::from_secs(3);
        self.pending_list_command=Some(DeferredListCommand{
            action,source,active:self.active,locations:self.panes.each_ref().map(location_key),
            tabs:self.panes.each_ref().map(|p|p.tab),generation:pane.generation,
            modifiers:self.shortcut_modifiers,cursor,targets,deadline,
        });
        self.status=self.catalog("NDR.UI_SHORTCUT_WAIT_REFRESH","목록을 갱신한 후 선택한 작업을 실행합니다.","Running the selected command after the list refreshes.");
        ctx.request_repaint_after(Duration::from_secs(3));
        true
    }

    fn resume_list_command(&mut self,ctx:&egui::Context)->bool{
        let Some(pending)=self.pending_list_command.take()else{return false};
        // A new key/click means a new intention. Release events and
        // pointer motion alone must not cancel a quick key chord.
        let new_input=ctx.input(|i|i.events.iter().any(|e|matches!(e,
            egui::Event::Key{pressed:true,..}|egui::Event::PointerButton{pressed:true,..}|
            egui::Event::Text(_)|egui::Event::Copy|egui::Event::Cut|egui::Event::Paste(_)|
            egui::Event::Ime(egui::ImeEvent::Commit(_)|egui::ImeEvent::Preedit(_)))));
        if new_input {return false;}
        let pane=&self.panes[pending.source];
        let still_same=self.active==pending.active && self.panes.each_ref().map(location_key)==pending.locations
            && self.panes.each_ref().map(|p|p.tab)==pending.tabs && pane.generation==pending.generation;
        if !still_same || Instant::now()>=pending.deadline || pane.error.is_some(){
            self.status=self.catalog("NDR.UI_SHORTCUT_RETRY","목록이 변경되었거나 갱신 중입니다. 작업을 다시 실행하세요.","The list changed or is still refreshing. Retry the command.");
            return false;
        }
        if pane.loading{
            ctx.request_repaint_after(pending.deadline.saturating_duration_since(Instant::now()));
            self.pending_list_command=Some(pending);
            return false;
        }
        let cursor=pane.entries.get(pane.cursor);
        let cursor_matches=match (pending.cursor.as_ref(),cursor){
            (Some(a),Some(b))=>same_command_entry(a,b),(None,None)=>true,_=>false,
        };
        let targets=self.target_entries(pending.source);
        let targets_match=targets.len()==pending.targets.len() && targets.iter().zip(&pending.targets).all(|(a,b)|same_command_entry(a,b));
        if !cursor_matches || !targets_match{
            self.status=self.catalog("NDR.UI_SHORTCUT_RETRY","목록이 변경되었거나 갱신 중입니다. 작업을 다시 실행하세요.","The list changed or is still refreshing. Retry the command.");
            return false;
        }
        self.shortcut_modifiers=pending.modifiers;
        match pending.action{
            DeferredListAction::Dispatch(id)=>self.dispatch(&id,ctx),
            DeferredListAction::CopyOther{source,moving,confirm:true}=>self.original_copy_to_other(source,moving,ctx),
            DeferredListAction::CopyOther{source,moving,confirm:false}=>self.copy_to_other(source,moving,ctx),
        }
        true
    }

    fn consume_shortcut_input(&mut self,ctx:&egui::Context) {
        // The opened dialog must not see the triggering Enter, text or
        // stale IME composition. Preserve quick search for ordinary keys.
        if ctx.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Ime(_)))) {
            self.typed_search[self.active]=TypedSearch::default();
        }
        ctx.input_mut(|i|i.events.retain(|e|!matches!(e,egui::Event::Key{pressed:true,..}|egui::Event::Text(_)|egui::Event::Ime(_)|egui::Event::Copy|egui::Event::Cut|egui::Event::Paste(_))));
    }

    fn unbound_menu_access(&mut self,key:egui::Key,modifiers:egui::Modifiers)->bool{
        let name=key.name();
        if modifiers.alt&&!modifiers.ctrl&&!modifiers.shift&&name.len()==1&&name.as_bytes()[0].is_ascii_alphanumeric(){
            self.menu_access=name.chars().next().map(|c|c.to_ascii_lowercase());
            true
        }else{false}
    }

    /// Sets work folder N (original FUN_0042fa5c / FUN_00479904): set and save, with no notice (the C# status text was removed).
    pub(super) fn set_work_folder(&mut self, index: usize, path: String) {
        let mut folders = self.settings.strings("WorkingFolders");
        folders.resize(5, String::new());
        folders[index] = path;
        self.settings.set_strings("WorkingFolders", folders);
        self.save();
    }

    /// Menu item or internal command chosen by the original shortcut table (shortcut.ini). None passes it on.
    fn original_shortcut(&mut self, key: egui::Key, modifiers: egui::Modifiers, ctx: &egui::Context) -> bool {
        let custom = self.settings.values.get("OriginalShortcutMapText").and_then(|v| v.as_str());
        let shortcuts = compat::ShortcutMap::parse(custom);
        let text = original_shortcut_text(key, modifiers);
        let Some(original) = shortcuts.resolve(compat::text_to_shortcut(&text)) else { return false };
        self.run_original_shortcut(&original, modifiers.shift, ctx)
    }

    /// Items bound to numpad keys in the original shortcut table (VK_NUMPAD0-9, VK_MULTIPLY/ADD/SUBTRACT/DECIMAL/DIVIDE = "Num 0"..."Num /").
    pub(super) fn numpad_shortcut(&self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> Option<String> {
        let name = match vk {
            0x60..=0x69 => format!("Num {}", vk - 0x60),
            0x6a => "Num *".into(),
            0x6b => "Num +".into(),
            0x6d => "Num -".into(),
            0x6e => "Num .".into(),
            0x6f => "Num /".into(),
            _ => return None,
        };
        let text = format!("{}{}{}{name}", if ctrl { "Ctrl+" } else { "" }, if shift { "Shift+" } else { "" }, if alt { "Alt+" } else { "" });
        let custom = self.settings.values.get("OriginalShortcutMapText").and_then(|v| v.as_str());
        compat::ShortcutMap::parse(custom).resolve(compat::text_to_shortcut(&text))
    }

    /// Whether the file list would hold the keyboard focus in the original: the window is focused, no text field takes keys, and no dialog,
    /// message box or modal progress window is open (those take the focus from the list). Menus are left out: Windows menu mode keeps the focus.
    pub(super) fn list_has_focus(&self, ctx: &egui::Context) -> bool {
        ctx.input(|i| i.focused)
            && !ctx.wants_keyboard_input()
            && self.msgboxes.is_empty()
            && self.original_dialogs.is_empty()
            && !self.job.as_ref().is_some_and(|j| j.modal.is_some() || (!j.finished && j.progress.is_some() && j.reservation.is_none()))
    }

    /// State where the list does not take keys (a dialog, message box, progress window or menu is open). Same as the first check in keyboard.
    pub(super) fn keyboard_blocked(&self, ctx: &egui::Context) -> bool {
        self.job.as_ref().is_some_and(|j| !j.finished && j.progress.is_some() && j.reservation.is_none())
            || !self.msgboxes.is_empty()
            || !self.original_dialogs.is_empty()
            || self.job.as_ref().is_some_and(|j| j.modal.is_some())
            || self.popup.is_some()
            || egui::Popup::is_any_open(ctx)
    }

    /// Runs the item chosen by the original shortcut table (a menu item or an internal command).
    fn run_original_shortcut(&mut self, original: &str, shift: bool, ctx: &egui::Context) -> bool {
        // Special item of C# OriginalMenuItemAction: select all cycles selection.
        // Hidden menu items still have valid shortcuts: don't swallow them.
        match original {
            "miSelectAll" => {
                self.cycle_selection(shift);
                return true;
            }
            // Original miFind (Ctrl+F) = find input box (TfGetText). The C# version focuses the toolbar quick search field with the same key,
            // so behave like C# only when that field is on the toolbar; otherwise (original toolbar) open the original find box.
            "miFind" if !self.toolbar.items.iter().any(|i| i.kind == ndr::commands::ToolbarKind::QuickSearch) => {
                self.dispatch("find", ctx);
                return true;
            }
            _ => {}
        }
        if let Some((_, action)) = menu_data::ORIGINAL_ACTIONS.iter().find(|(name, _)| *name == original) {
            self.dispatch(action, ctx);
            return true;
        }
        self.original_internal(original, ctx)
    }

    /// Handles one key press in original order: original settings esckey (Esc action) and hanshdrive (Shift+letter drive with a non-English input language),
    /// then original FormShortCut (shortcut table, function key table). True when handled.
    pub(super) fn shortcut_key(&mut self, key: egui::Key, modifiers: egui::Modifiers, ctx: &egui::Context) -> bool {
        let Some(name) = wpf_key(key) else { return false };
        // NDR-specific encrypted-folder creation. Keep this distinct from
        // the original Alt+K shortcut for an ordinary New Folder.
        if key == egui::Key::K && modifiers.ctrl && !modifiers.alt && !modifiers.shift {
            self.dispatch("newVault", ctx);
            return true;
        }
        if key == egui::Key::Escape {
            match self.settings.values.get("EscapeKeyAction").and_then(|v| v.as_i64()) {
                Some(0) => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                Some(1) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Some(2) => self.dispatch("command", ctx),
                Some(3) => self.panes[self.active].selected.clear(),
                Some(4) => self.dispatch("goParent", ctx),
                _ => return false,
            }
            return true;
        }
        // Original FormShortCut (0x42b36c): menu item and internal command shortcuts (shortcut.ini table, menu IsShortCut) come first.
        // (Alt+Enter is miDescription "attach description" like the original. The fullscreen toggle once added per user request was removed.)
        if self.original_shortcut(key, modifiers, ctx) {
            return true;
        }
        // Shortcuts of the original favorites menu items (FUN_0042eedc, menu IsShortCut): items 0-9 Shift+F1-F10, items 10-19 Shift+Ctrl+F1-F10.
        // Items exist only for named favorites, so keys of empty slots pass on.
        if modifiers.shift && !modifiers.alt && name.starts_with('F') {
            if let Some(n) = name[1..].parse::<usize>().ok().filter(|n| (1..=10).contains(n)) {
                let index = if modifiers.ctrl { n + 9 } else { n - 1 };
                let (paths, names) = (self.settings.strings("Favorites"), self.settings.strings("FavoriteNames"));
                if paths.get(index).is_some_and(|p| !p.is_empty()) && names.get(index).is_some_and(|n| !n.is_empty()) {
                    self.prefixed_command(&format!("favoriteOpen:{index}"));
                    return true;
                }
            }
        }
        // Then the function key table ([FuncKey]: F1-F12 and Ctrl+F1-F11 "^F1"-"^F11" from the TfPreferences function key list, FUN_0047cc18 -> FUN_0044381c).
        if !(modifiers.ctrl || modifiers.alt || modifiers.shift) && name.starts_with('F') && name[1..].parse::<u8>().is_ok_and(|n| (1..=12).contains(&n)) && self.invoke_function_key(&name, ctx) {
            return true;
        }
        if modifiers.ctrl && !(modifiers.alt || modifiers.shift) && name.starts_with('F') && name[1..].parse::<u8>().is_ok_and(|n| (1..=11).contains(&n)) && self.invoke_function_key(&format!("^{name}"), ctx) {
            return true;
        }
        self.original_keydown(key, modifiers, &name, ctx)
    }

    /// Keys of the original list and pane tree KeyDown (FUN_0048423c) not taken by the shortcut table.
    fn original_keydown(&mut self, key: egui::Key, modifiers: egui::Modifiers, name: &str, ctx: &egui::Context) -> bool {
        let index = self.active;
        let plain = !(modifiers.ctrl || modifiers.alt || modifiers.shift);
        // '.' (VK_OEM_PERIOD, 0x71 + 0x4d): parent folder when the quick search text (+0x3e0) is empty (FUN_00459ba8(1); at an archive root, the folder containing the archive).
        // Fixed: previously not handled.
        if plain && key == egui::Key::Period && self.typed_search[index].query.is_empty() {
            self.dispatch("goParent", ctx);
            return true;
        }
        // Ctrl+Tab / Ctrl+Shift+Tab: with tabs on (+0xf3) and two or more tabs, next/previous tab (wrapping). Fixed: previously missing.
        if key == egui::Key::Tab && modifiers.ctrl && !modifiers.alt && self.settings.flag("UseTabs", true) && self.panes[index].tabs.len() > 1 {
            let (count, tab) = (self.panes[index].tabs.len(), self.panes[index].tab);
            let next = if modifiers.shift { (tab + count - 1) % count } else { (tab + 1) % count };
            self.tab_command(index, &format!("tab:{next}"), ctx);
            return true;
        }
        // Shift+letter (Shift only): at local and archive locations that drive (its last folder if any, FUN_0046f714).
        // With a non-English input language such as Korean the IME takes the letter, so only when hanshdrive (UseAlternateDriveHandling: Shift switches the IME
        // to English) is on. Fixed: previously even English input needed that setting.
        if modifiers.shift && !(modifiers.ctrl || modifiers.alt)
            && name.len() == 1
            && name.as_bytes()[0].is_ascii_uppercase()
            && self.panes[index].remote.is_none()
            && (native::english_input_language() || self.settings.flag("UseAlternateDriveHandling", false))
        {
            let root = PathBuf::from(format!("{name}:\\"));
            if root.is_dir() {
                self.navigate_drive(index, root);
                return true;
            }
        }
        false
    }

    /// favoriteOpen:/workOpen:/workSet:/column: of ExecuteMenuCommandAsync. True when handled.
    pub(super) fn prefixed_command(&mut self, id: &str) -> bool {
        // Original menu names (openWorkFolderN/saveWorkFolderN/columnN in menu_data) = workOpen:/workSet:/column: of C# OriginalMenuItemAction.
        let alias = |prefix: &str| id.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok()).filter(|n| (1..=6).contains(n));
        if let Some(n) = alias("openWorkFolder").filter(|n| *n <= 5) {
            return self.prefixed_command(&format!("workOpen:{}", n - 1));
        }
        if let Some(n) = alias("saveWorkFolder").filter(|n| *n <= 5) {
            return self.prefixed_command(&format!("workSet:{}", n - 1));
        }
        if let Some(n) = alias("column") {
            return self.prefixed_command(&format!("column:{n}"));
        }
        let number = |prefix: &str| id.strip_prefix(prefix).and_then(|n| n.parse::<usize>().ok());
        if let Some(index) = number("favoriteOpen:") {
            let paths = self.settings.strings("Favorites");
            let names = self.settings.strings("FavoriteNames");
            if index < 20 && paths.get(index).is_some_and(|p| !p.is_empty()) && names.get(index).is_some_and(|n| !n.is_empty()) {
                let path = paths[index].clone();
                self.open_favorite(&path);
            }
            return true;
        }
        if let Some(index) = number("workOpen:") {
            let path = self.settings.strings("WorkingFolders").get(index).cloned().unwrap_or_default();
            // Original miGoToWorkFolderN -> FUN_0042fdf0 navigates to WF<n> (FUN_0048d6e4); an empty slot does nothing (seen in the running original:
            // Alt+3 twice with WF3 empty kept the pane where it was). Only the work folder button (FUN_0042fc84, work_folder_click) sets an empty slot.
            // Previously an empty slot showed a "not set" box.
            if path.trim().is_empty() {
                return true;
            } else if Path::new(&path).is_dir() {
                self.navigate(self.active, PathBuf::from(path), true);
            } else {
                let text = super::file_commands::operation_format(&self.language_pack, "formNDRDestination.Message.InvalidWorkFolder", "Work folder {0} is not set or is unavailable.", &[&(index + 1)]);
                self.system_box("NDR", &text, ndr::msgbox::MB_OK, |_, _, _| {});
            }
            return true;
        }
        if let Some(index) = number("workSet:") {
            if index < 5 {
                let pane = &self.panes[self.active];
                let path = match &pane.archive {
                    Some(archive) => archive.file.parent().map(Path::to_path_buf).unwrap_or_default(),
                    None => pane.path.clone(),
                };
                let path = path.to_string_lossy().into_owned();
                // Original FUN_0042ff58: an already set work folder asks TfMsgBox(WORKFOLDER, "", MSG_UNSETWORKFOLDER with %s = current value,
                // MB_YESNO | MB_ICONQUESTION) and changes only on yes. An empty slot is set at once.
                let existing = self.settings.strings("WorkingFolders").get(index).cloned().unwrap_or_default();
                if existing.is_empty() {
                    self.set_work_folder(index, path);
                } else {
                    let title = self.language_pack.text("RES.WORKFOLDER", "Work Folder");
                    let text = self
                        .language_pack
                        .text("RES.MSG_UNSETWORKFOLDER", "A following folder is already assigned to this work folder. You want to replace this with current folder?\n%s")
                        .replace("%s", &existing);
                    self.message_box(&title, "", &text, ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONQUESTION, move |app, answer, _| {
                        if answer == ndr::msgbox::IDYES {
                            app.set_work_folder(index, path);
                        }
                    });
                }
            }
            return true;
        }
        if let Some(count) = id.strip_prefix("column:").and_then(|n| n.parse::<i64>().ok()) {
            let key = if self.active == 0 { "LeftColumnCount" } else { "RightColumnCount" };
            self.settings.values.insert(key.into(), serde_json::json!(count.clamp(0, 19)));
            self.save();
            return true;
        }
        false
    }

    /// C# CycleFileSelection (original miSelectAllClick 0x4311c4).
    pub(super) fn cycle_selection(&mut self, shift: bool) {
        let select_folders = self.settings.flag("SelectFolders", false);
        let pane = &mut self.panes[self.active];
        let is_file = |e: &Entry| !e.is_dir && !e.drive && !e.is_parent();
        let is_folder = |e: &Entry| e.is_dir && !e.drive && !e.is_parent();
        if pane.selected.is_empty() {
            let include_folders = !pane.entries.iter().any(is_file) || (select_folders ^ shift);
            pane.selected = pane.entries.iter().enumerate()
                .filter(|(_, e)| is_file(e) || (include_folders && is_folder(e)))
                .map(|(i, _)| i).collect();
            return;
        }
        // No file/folder index vectors: scan the sorted marks and rows
        // together in linear time instead of N tree lookups.
        let mut marks = pane.selected.iter().copied().peekable();
        let mut all_files_marked = true;
        let mut any_folder = false;
        let mut folder_marked = false;
        for (i, e) in pane.entries.iter().enumerate() {
            while marks.peek().is_some_and(|&m| m < i) { marks.next(); }
            let selected = marks.peek().is_some_and(|&m| m == i);
            if is_file(e) && !selected { all_files_marked = false; }
            if is_folder(e) { any_folder = true; folder_marked |= selected; }
        }
        if any_folder && !folder_marked && all_files_marked {
            pane.selected.extend(pane.entries.iter().enumerate().filter(|(_, e)| is_folder(e)).map(|(i, _)| i));
        } else {
            pane.selected.clear();
        }
    }

    /// C# ToggleFocusedItemSelection: toggles only the current item's mark (the cursor stays).
    /// Original pane OnEnter (FUN_004849c8): when the list gets focus (current pane) on a local pane (FUN_00481948 = 0), its folder becomes
    /// the process current directory (SetCurrentDirectoryW). Programs started inherit it. Tests skip it because it is process-wide.
    pub(super) fn sync_current_directory(&mut self) {
        let pane = &self.panes[self.active];
        if pane.archive.is_some() || pane.remote.is_some() || pane.loading || super::panes::is_drives(&pane.path) {
            return;
        }
        if self.current_directory.as_ref() == Some(&pane.path) {
            return;
        }
        self.current_directory = Some(pane.path.clone());
        if !cfg!(test) {
            let _ = std::env::set_current_dir(&pane.path);
        }
    }

    /// Original Select (0x7dc834): flips the cursor item's selection. Drive items (kind 4), kind 2 and ".." are not selected.
    pub(super) fn toggle_focused_selection(&mut self) {
        let pane = &mut self.panes[self.active];
        let Some(entry) = pane.entries.get(pane.cursor) else { return };
        if entry.drive || entry.is_parent() {
            return;
        }
        if !pane.selected.insert(pane.cursor) {
            pane.selected.remove(&pane.cursor);
        }
    }

    /// Original SelectCurrAndMove (0x7dca04): miSelect (Spacebar, miSelectClick 0x4311a4) and the internal command SelectCurrAndMove.
    /// Flips the cursor item's selection (Select) and moves with the Right key in the thumbnail style, otherwise the Down key (ArrowKey). Both go to the next item.
    pub(super) fn select_current_and_move(&mut self) {
        self.toggle_focused_selection();
        let pane = &mut self.panes[self.active];
        let count = pane.entries.len();
        if count == 0 {
            return;
        }
        let next = (pane.cursor + 1).min(count - 1);
        if next != pane.cursor {
            pane.cursor = next;
            pane.reveal = true;
        }
    }

    /// C# BeginAddressEdit: puts the current path in the address field ("archive!/prefix" inside an archive), focuses it and selects all.
    pub(super) fn begin_address_edit(&mut self, index: usize, ctx: &egui::Context) {
        let text = self.address_text(index);
        let pane = &mut self.panes[index];
        pane.address_editing = true;
        pane.address = text;
        let id = egui::Id::new(("ndr-address", index));
        ctx.memory_mut(|memory| memory.request_focus(id));
        let length = pane.address.chars().count();
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(length))));
        state.store(ctx, id);
    }

    /// C# RestoreLastDriveFolder: with SaveDriveFolders at a drive root, the last folder opened on that drive (if any).
    pub(super) fn restore_drive_folder(&self, path: PathBuf) -> PathBuf {
        if !self.settings.flag("SaveDriveFolders", true) {
            return path;
        }
        let full = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
        let is_root = full.parent().is_none();
        if !is_root {
            return path;
        }
        let root = full.to_string_lossy().into_owned();
        let last = self.settings.values.get("LastFoldersByDrive").and_then(|v| v.as_object()).and_then(|m| {
            m.iter().find(|(key, _)| key.eq_ignore_ascii_case(&root)).and_then(|(_, value)| value.as_str()).map(str::to_owned)
        });
        match last {
            Some(last) if Path::new(&last).is_dir() => std::path::absolute(&last).unwrap_or_else(|_| PathBuf::from(last)),
            _ => path,
        }
    }

    /// C# RememberDriveFolder: with SaveDriveFolders, LastFoldersByDrive[root] = full path.
    pub(super) fn remember_drive_folder(&mut self, path: &Path) {
        if !self.settings.flag("SaveDriveFolders", true) || super::panes::is_drives(path) {
            return;
        }
        let Ok(full) = std::path::absolute(path) else { return };
        let Some(root) = full.ancestors().last().map(|r| r.to_string_lossy().into_owned()).filter(|r| !r.is_empty()) else { return };
        let mut map = self.settings.values.get("LastFoldersByDrive").and_then(|v| v.as_object()).cloned().unwrap_or_default();
        map.retain(|key, _| !key.eq_ignore_ascii_case(&root));
        map.insert(root, serde_json::Value::String(full.to_string_lossy().into_owned()));
        self.settings.values.insert("LastFoldersByDrive".into(), serde_json::Value::Object(map));
    }

    /// Drive buttons and Shift+letter: restores and opens the last folder (NavigateLocalAsync restoreDriveFolder).
    pub(super) fn navigate_drive(&mut self, index: usize, root: PathBuf) {
        self.active = index;
        let target = self.restore_drive_folder(root);
        self.navigate(index, target, true);
    }

    /// Original ArrowKey `[` / `]` (without Ctrl, Alt, Shift): moves between the drive items at the end of the list (`list_keys::bracket_target`).
    fn bracket_keys(&mut self,ctx:&egui::Context) {
        let keys=ctx.input(|i|i.events.iter().filter_map(|e|match e{
            egui::Event::Key{key:key @ (egui::Key::OpenBracket|egui::Key::CloseBracket),pressed:true,modifiers,..}
                if !modifiers.ctrl&&!modifiers.alt&&!modifiers.command=>Some((*key,modifiers.shift)),
            _=>None,
        }).collect::<Vec<_>>());
        for (key,shift) in keys {
            let pane=&mut self.panes[self.active];
            if pane.loading||pane.entries.is_empty(){return;}
            let count=pane.entries.len();
            let cursor=pane.cursor.min(count-1);
            let on_drive=pane.entries[cursor].drive;
            if let Some(target)=list_keys::bracket_target(cursor,count,pane.drive_count(),on_drive,key==egui::Key::OpenBracket) {
                if target!=cursor {
                    pane.cursor=target;
                    pane.reveal=true;
                    if shift {
                        let anchor=*pane.shift_anchor.get_or_insert(cursor);
                        pane.selected.clear();
                        for i in anchor.min(target)..=anchor.max(target){pane.selected.insert(i);}
                        pane.drop_drive_marks();
                    }else{pane.shift_anchor=None;}
                }
            }
        }
    }

    /// Navigation uses the modifier snapshot on each press. A frame may
    /// contain both the press and modifier release, or several arrows.
    fn list_navigation(&mut self,ctx:&egui::Context){
        let keys=ctx.input(|i|i.events.iter().filter_map(|e|match e{
            egui::Event::Key{key,pressed:true,modifiers,..}=>Some((*key,*modifiers)),_=>None
        }).collect::<Vec<_>>());
        for (key,m) in keys{self.list_navigation_key(key,m);}
        self.clear_released_shift(ctx);
    }

    fn clear_released_shift(&mut self,ctx:&egui::Context){
        if !ctx.input(|i|i.modifiers.shift){
            for pane in &mut self.panes{pane.shift_anchor=None;}
        }
    }

    fn list_navigation_key(&mut self,key:egui::Key,m:egui::Modifiers)->bool{
        use list_keys::ListKey;
        if m.alt{return false;}
        if key==egui::Key::Tab&&!m.ctrl{
            self.active=1-self.active;
            return true;
        }
        let list_key=match key{
            egui::Key::ArrowUp=>ListKey::Up,
            egui::Key::ArrowDown=>ListKey::Down,
            egui::Key::ArrowLeft=>ListKey::Left,
            egui::Key::ArrowRight=>ListKey::Right,
            egui::Key::PageUp=>ListKey::PageUp,
            egui::Key::PageDown=>ListKey::PageDown,
            egui::Key::Home=>ListKey::Home,
            egui::Key::End=>ListKey::End,
            _=>return false,
        };
        if m.ctrl&&!m.shift&&!matches!(list_key,ListKey::PageUp|ListKey::PageDown|ListKey::Home|ListKey::End){
            return false;
        }
        let pane=&mut self.panes[self.active];
        if !m.shift{pane.shift_anchor=None;}
        let length=pane.entries.len();
        if length==0 || pane.loading&&pane.listed.as_deref()!=Some(location_key(pane).as_str()){return true;}
        let old=pane.cursor.min(length-1);
        let cursor=if m.ctrl&&!m.shift{
            let drives=pane.drive_count();
            let folders=pane.entries.iter().filter(|e|e.is_dir&&!e.drive).count();
            list_keys::ctrl_target(folders,length-folders-drives,drives,length,list_key).unwrap_or(old)
        }else{
            if pane.page.style==list_keys::ListStyle::Details{
                let step=match key{egui::Key::ArrowLeft=>-20.0,egui::Key::ArrowRight=>20.0,_=>0.0};
                pane.detail_x=(pane.detail_x+step).max(0.0);
            }
            if let (list_keys::ListStyle::Thumbnails,ListKey::PageUp|ListKey::PageDown,Some((tops,heights,offset,view,columns)))=(pane.page.style,list_key,pane.thumb_rows.as_ref()){
                list_keys::thumbnail_page_target(tops,heights,*columns,*offset,*view,old,length,list_key==ListKey::PageDown)
            }else{list_keys::arrow_target(&pane.page,old,length,list_key)}
        };
        if cursor!=old{
            pane.cursor=cursor;
            pane.reveal=true;
            if m.shift{
                let anchor=*pane.shift_anchor.get_or_insert(old);
                let from=if m.ctrl{old}else{anchor};
                if !m.ctrl{pane.selected.clear();}
                for i in from.min(cursor)..=from.max(cursor){pane.selected.insert(i);}
                pane.drop_drive_marks();
            }
        }
        true
    }

    /// Items of the original miColumn (miColAuto, separator, miCol1-6), checked by the current value. Other set values (7-19) are shown too like C#. Returns the chosen command.
    pub(super) fn column_items(&mut self, ui: &mut egui::Ui) -> Option<String> {
        let columns_title = self.language_pack.text("Menu.miColumn", "Columns").replace('&', "");
        let current = self.settings.values.get(if self.active == 0 { "LeftColumnCount" } else { "RightColumnCount" }).and_then(|v| v.as_i64()).unwrap_or(0);
        let mut chosen = None;
        // Original miColAuto and miCol1-6 are RadioItems: RCHECK on the current value.
        let radio = |ui: &mut egui::Ui, on: bool, text: String| super::skin_pane::menu_check_button(ui, true, on, true, egui::Button::new(text));
        let auto = self.raw_menu("miColAuto", "&A. Auto");
        let response = radio(ui, current == 0, super::favorites_dialog::access_key(&auto).0);
        if response.clicked() || self.take_menu_letter(&auto, &response) {
            chosen = Some("column:0".into());
        }
        ui.separator();
        if (7..=19).contains(&current) && radio(ui, true, format!("{current} {columns_title}")).clicked() {
            chosen = Some(format!("column:{current}"));
        }
        for count in 1..=6 {
            let raw = self.raw_menu(&format!("miCol{count}"), &format!("&{count}  Column"));
            let response = radio(ui, current == count, super::favorites_dialog::access_key(&raw).0);
            if response.clicked() || self.take_menu_letter(&raw, &response) {
                chosen = Some(format!("column:{count}"));
            }
        }
        chosen
    }

    /// Original pmColCountPopup (0x437634): pmColCount of the vertical bar column button (bColCount) copies the miColumn items as is (without submenus)
    /// and checks the first item (auto) when the active pane's column count (+0x388) is 0, or the item of that number when below 7. No work folder menu.
    pub(super) fn original_column_popup(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(command) = self.column_items(ui) {
            ui.close();
            self.dispatch(&command, ctx);
        }
    }

    /// Original miColumn (View menu): auto, 1-6 columns (current value checked).
    pub(super) fn column_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut chosen: Option<String> = None;
        let raw = self.raw_menu("miColumn", "&C. Columns");
        self.access_submenu(ui, &raw, |app, ui| {
            chosen = app.column_items(ui);
        });
        if let Some(command) = chosen {
            ui.close();
            self.dispatch(&command, ctx);
        }
    }

    /// miSetWorkFolder (set work folder 1-5) and miGoToWorkFolder (go to work folder 1-5, shown with paths like C#) of the original Folder menu.
    pub(super) fn work_folder_menus(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let folders = self.settings.strings("WorkingFolders");
        let mut chosen: Option<String> = None;
        let set_raw = self.raw_menu("miSetWorkFolder", "&W. Set Work Folder");
        self.access_submenu(ui, &set_raw, |app, ui| {
            for index in 0..5 {
                let raw = app.raw_menu(&format!("miWorkFolder{}", index + 1), &format!("Work Folder &{}", index + 1));
                let response = ui.button(super::favorites_dialog::access_key(&raw).0);
                if response.clicked() || app.take_menu_letter(&raw, &response) {
                    chosen = Some(format!("workSet:{index}"));
                }
            }
        });
        let go_raw = self.raw_menu("miGoToWorkFolder", "&G. Go to Work Folder");
        self.access_submenu(ui, &go_raw, |app, ui| {
            for index in 0..5 {
                let path = folders.get(index).cloned().unwrap_or_default();
                let raw = app.raw_menu(&format!("miGoToWorkFolder{}", index + 1), &format!("Work Folder &{}", index + 1));
                let open = super::favorites_dialog::access_key(&raw).0;
                let label = if path.trim().is_empty() {
                    let empty = app.language_pack.text("formNDRDestination.Slot.Empty", "Not configured");
                    format!("{open} ({empty})")
                } else {
                    format!("{open} — {path}")
                };
                let response = ui.button(label);
                if response.clicked() || app.take_menu_letter(&raw, &response) {
                    chosen = Some(format!("workOpen:{index}"));
                }
            }
        });
        if let Some(command) = chosen {
            ui.close();
            self.dispatch(&command, ctx);
        }
    }

    /// Drive button list (root, tooltip). After 5 seconds a separate reread starts and the previous list is used meanwhile.
    pub(super) fn drive_button_list(&mut self) -> Vec<(PathBuf, String)> {
        let stale = self.drive_cache.as_ref().is_none_or(|(at, _)| at.elapsed() > Duration::from_secs(5));
        if stale && !self.drive_cache_pending {
            self.drive_cache_pending = true;
            let tx = self.async_tx.clone();
            let generation = self.drive_cache_generation;
            thread::spawn(move || {
                // Original TNXDriveList.Refresh: drives of types other than 0 and 1 get a button even without media. Fixed: previously only ready drives.
                let drives = native::listed_drives()
                    .into_iter()
                    .map(|root| {
                        let kind = native::drive_type_name(&root);
                        let tip = match native::volume_label(&root).filter(|l| !l.trim().is_empty()) {
                            Some(label) => format!("{} {label} · {kind}", root.display()),
                            None => format!("{} · {kind}", root.display()),
                        };
                        (root, tip)
                    })
                    .collect();
                let _ = tx.send(AsyncResult::Drives(generation, drives));
            });
        }
        // Cache OS data, not translated text: changing the language is immediate,
        // even when an asynchronous drive enumeration is still pending.
        self.drive_cache.as_ref().map(|(_, drives)| drives.iter().map(|(root, tip)| {
            let tip = tip.rsplit_once(" · ").map_or_else(|| tip.clone(), |(name, kind)| {
                format!("{name} · {}", localized_drive_kind(&self.language_pack, kind))
            });
            (root.clone(), tip)
        }).collect()).unwrap_or_default()
    }
}

/// Turns the pressed key into original shortcut text (the form TextToShortCut reads, e.g. "Ctrl+Shift+F2", "/").
/// Fixed: punctuation keys are written as the character ("/") instead of the egui name ("Slash") (original TextToShortCut("/") = VK_OEM_2).
/// Previously shortcut.ini entries such as miCommandLine=/ and miToRoot=\ did not work.
pub(super) fn original_shortcut_text(key: egui::Key, modifiers: egui::Modifiers) -> String {
    let key_name = match key {
        egui::Key::ArrowLeft => "Left",
        egui::Key::ArrowRight => "Right",
        egui::Key::ArrowUp => "Up",
        egui::Key::ArrowDown => "Down",
        egui::Key::Space => "Spacebar",
        other => {
            let symbol = other.symbol_or_name();
            if symbol.chars().count() == 1 && symbol.chars().all(|c| c.is_ascii_punctuation()) { symbol } else { other.name() }
        }
    };
    format!("{}{}{}{key_name}", if modifiers.ctrl { "Ctrl+" } else { "" }, if modifiers.shift { "Shift+" } else { "" }, if modifiers.alt { "Alt+" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether every native default shortcut (except numpad "Num" keys, which go through raw_input_hook)
    /// turns into the same shortcut value from egui key events.
    #[test]
    fn every_default_original_shortcut_is_reachable_from_egui_keys() {
        let mut checked = 0;
        for &(name, chord) in ndr::compat::default_shortcut_bindings() {
            let text = ndr::compat::shortcut_to_text(chord);
            let text = text.as_str();
            if text.is_empty() || text.contains("Num ") {
                continue;
            }
            let tokens: Vec<&str> = if let Some(head) = text.strip_suffix("++") { head.split('+').chain(["+"]).collect() } else { text.split('+').collect() };
            let (last, mods) = tokens.split_last().unwrap();
            let modifiers = egui::Modifiers { ctrl: mods.contains(&"Ctrl"), shift: mods.contains(&"Shift"), alt: mods.contains(&"Alt"), ..Default::default() };
            let key = match *last {
                "Spacebar" => egui::Key::Space,
                "Left" => egui::Key::ArrowLeft,
                "Right" => egui::Key::ArrowRight,
                "Up" => egui::Key::ArrowUp,
                "Down" => egui::Key::ArrowDown,
                "/" => egui::Key::Slash,
                "\\" => egui::Key::Backslash,
                "," => egui::Key::Comma,
                "." => egui::Key::Period,
                "`" => egui::Key::Backtick,
                "-" => egui::Key::Minus,
                "=" => egui::Key::Equals,
                ";" => egui::Key::Semicolon,
                "'" => egui::Key::Quote,
                "[" => egui::Key::OpenBracket,
                "]" => egui::Key::CloseBracket,
                other => egui::Key::from_name(other).unwrap_or_else(|| panic!("{name}={text}: no egui key for {other:?}")),
            };
            assert_eq!(ndr::compat::text_to_shortcut(&original_shortcut_text(key, modifiers)), ndr::compat::text_to_shortcut(text), "{name}={text}");
            checked += 1;
        }
        assert!(checked > 90, "{checked}");
        // Character keys changed by Shift (egui may report Shift+/ as Questionmark) map to the same shortcut too.
        let shift = egui::Modifiers::SHIFT;
        assert_eq!(ndr::compat::text_to_shortcut(&original_shortcut_text(egui::Key::Questionmark, shift)), ndr::compat::text_to_shortcut("Shift+/"));
        assert_eq!(ndr::compat::text_to_shortcut(&original_shortcut_text(egui::Key::Pipe, shift)), ndr::compat::text_to_shortcut("Shift+\\"));
    }
}
