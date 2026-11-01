//! In-process SQLCipher vault browser. No plaintext file is created except an
//! explicitly requested export. Passwords are never written to NDR settings.
use super::*;
use ndr::vault::{Vault, VaultItem, SessionPolicy, MAX_FILE_SIZE};
use zeroize::Zeroize;

pub(super) struct PaneVault {
    pub file: PathBuf,
    pub database: Vault,
    pub stack: Vec<(i64, String)>,
}

pub(super) struct VaultTextEditor {
    pane: usize,
    id: i64,
    name: String,
    body: String,
}
impl Drop for VaultTextEditor {
    fn drop(&mut self) { self.body.zeroize(); self.name.zeroize(); }
}

impl Drop for PaneVault {
    fn drop(&mut self) {
        for (_, name) in &mut self.stack { name.zeroize(); }
    }
}

impl PaneVault {
    fn new(file: PathBuf, database: Vault) -> Self {
        Self { file, database, stack: vec![(1, String::new())] }
    }
    fn folder(&self) -> i64 { self.stack.last().map_or(1, |item| item.0) }
    pub(super) fn virtual_path(&self) -> PathBuf {
        let mut result = self.file.clone();
        for (_, name) in self.stack.iter().skip(1) { result.push(name); }
        result
    }
}

pub(super) struct VaultDialog {
    path: PathBuf,
    creating: bool,
    folder_name: String,
    name_select_pending: bool,
    created_path: Option<PathBuf>,
    password: String,
    confirmation: String,
    vault: Option<Vault>,
    folder: i64,
    folder_stack: Vec<(i64, String)>,
    selected: Option<VaultItem>,
    name: String,
    import_path: String,
    export_path: String,
    text: Option<(i64, String)>,
    error: String,
}

impl App {
    /// Discard the in-memory connection AND all decrypted pane presentation
    /// buffers, including on tab switches and direct address navigation.
    pub(super) fn discard_vault(&mut self, pane: usize) {
        if self.panes[pane].vault.take().is_none() { return; }
        let state = &mut self.panes[pane];
        for entry in &mut state.entries { entry.name.zeroize(); }
        state.entries.clear();
        state.selected.clear();
        state.cursor = 0;
        state.summary = None;
        for row in &mut state.view.cells {
            for cell in row { cell.zeroize(); }
        }
        for row in &mut state.view.rows {
            if let super::list_view::DetailRow::Header(label) = row { label.zeroize(); }
        }
        state.view = PaneView::default();
        state.descriptions.clear();
        state.folder_sizes.clear();
        state.folder_bytes.clear();
        if let Some(key) = state.listed.as_mut() { key.zeroize(); }
        state.listed = None;
        state.address.zeroize();
        state.address = state.path.to_string_lossy().into_owned();
        if let Some(name) = self.pending_focus[pane].as_mut() { name.zeroize(); }
        self.pending_focus[pane] = None;
        if self.vault_text_editor.as_ref().is_some_and(|editor| editor.pane == pane) {
            self.vault_text_editor = None;
        }
    }

    pub(super) fn vault_security_tick(&mut self, ctx: &egui::Context) {
        let policy = SessionPolicy::from_settings(&self.settings);
        let minimized = self.settings.flag("VaultLockOnMinimize", false)
            && ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        for pane in 0..2 {
            let expired = self.panes[pane].vault.as_ref().is_some_and(|state|
                minimized || state.database.expired(policy));
            if expired {
                self.vault_text_editor = None;
                self.exit_vault(pane);
                self.status = "암호화 폴더가 잠겼습니다. 다시 열어 인증하세요.".into();
            }
        }
    }

    fn attach_vault(&mut self, pane: usize, container: PaneVault) {
        // The decrypted container remains in memory, never as a plain folder.
        self.discard_vault(pane);
        self.panes[pane].archive = None;
        self.panes[pane].vault = Some(container);
        let ctx = self.egui_ctx.clone();
        self.arm_folder_watch(pane, None, &ctx);
        self.refresh(pane);
    }

    pub(super) fn refresh_vault(&mut self, pane: usize) {
        let policy = SessionPolicy::from_settings(&self.settings);
        let Some(container) = self.panes[pane].vault.as_mut() else { return };
        if container.database.expired(policy)
            || self.settings.flag("VaultLockOnMinimize", false)
                && self.egui_ctx.input(|i| i.viewport().minimized.unwrap_or(false)) {
            self.exit_vault(pane);
            self.status = "암호화 폴더가 잠겼습니다. 다시 열면 비밀번호를 요청합니다.".into();
            return;
        }
        let current = container.virtual_path();
        let parent = container.file.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        let listing = container.database.list(container.folder());
        let entries = match listing {
            Ok(items) => {
                let mut rows = vec![Entry::parent_item(parent, None)];
                rows.extend(items.into_iter().map(|item| Entry {
                    path: current.join(&item.name), name: item.name.clone(), is_dir: item.folder,
                    size: item.size.max(0) as u64, modified: None, hidden: false,
                    attributes: if item.folder { 0x10 } else { 0x80 }, drive: false,
                }));
                rows
            }
            Err(error) => { self.status = error; return; }
        };
        let focus = self.pending_focus[pane].take().or_else(|| {
            let old = &self.panes[pane];
            (old.listed.as_deref() == Some(format!("vault|{}", current.display()).as_str()))
                .then(|| old.entries.get(old.cursor).map(|entry| entry.name.clone())).flatten()
        });
        let state = &mut self.panes[pane];
        state.generation = state.generation.wrapping_add(1);
        state.loading = false;
        state.address = current.to_string_lossy().into_owned();
        state.entries = entries;
        state.error = None;
        state.cursor = 0;
        state.selected.clear();
        state.listed = Some(format!("vault|{}", current.display()));
        state.disk = None;
        state.summary = None;
        state.descriptions.clear();
        state.folder_sizes.clear();
        state.folder_bytes.clear();
        self.sort_pane(pane);
        if let Some(name) = focus {
            if let Some(index) = self.panes[pane].entries.iter().position(|entry| entry.name == name) {
                self.panes[pane].cursor = index;
                self.panes[pane].reveal = true;
            }
        }
    }

    pub(super) fn exit_vault(&mut self, pane: usize) {
        let Some(container) = self.panes[pane].vault.as_ref() else { return };
        let parent = container.file.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        let file_name = container.file.file_stem().map(|n| n.to_string_lossy().into_owned());
        self.discard_vault(pane);
        self.pending_focus[pane] = file_name;
        self.navigate(pane, parent, false);
    }

    pub(super) fn enter_vault_item(&mut self, pane: usize, entry: &Entry) {
        if entry.is_parent() {
            let state = self.panes[pane].vault.as_mut().unwrap();
            if state.stack.len() == 1 { self.exit_vault(pane); }
            else {
                self.pending_focus[pane] = state.stack.pop().map(|(_, name)| name);
                self.refresh_vault(pane);
            }
            return;
        }
        let Some(state) = self.panes[pane].vault.as_mut() else { return };
        if let Ok(items) = state.database.list(state.folder()) {
            if let Some(item) = items.into_iter().find(|v| v.name == entry.name) {
                if item.folder {
                    state.stack.push((item.id, item.name.clone()));
                    self.pending_focus[pane] = None;
                    state.database.touch();
                    self.refresh_vault(pane);
                } else {
                    self.open_vault_text(pane, item);
                }
            }
        }
    }
    fn open_vault_text(&mut self, pane: usize, item: VaultItem) {
        let Some(state) = self.panes[pane].vault.as_mut() else { return };
        match state.database.read_file(item.id) {
            Ok(bytes) if bytes.len() <= 1024 * 1024 => match String::from_utf8(bytes) {
                Ok(body) => self.vault_text_editor = Some(VaultTextEditor {
                    pane, id: item.id, name: item.name.clone(), body,
                }),
                Err(_) => self.status = "바이너리 파일은 텍스트 편집기로 열 수 없습니다. 암호화 폴더 안에서 안전하게 보관됩니다.".into(),
            },
            Ok(_) => self.status = "텍스트 편집은 1 MiB 이하 파일에만 지원됩니다.".into(),
            Err(e) => self.status = e,
        }
    }

    pub(super) fn vault_text_ui(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.vault_text_editor.take() else { return };
        let mut open = true;
        let mut save = false;
        egui::Window::new(format!("암호화 폴더 - {}", editor.name))
            .id(egui::Id::new("ndr-vault-text-editor"))
            .open(&mut open).resizable(true).default_size(egui::vec2(700.0, 490.0))
            .show(ctx, |ui| {
                ui.add(egui::TextEdit::multiline(&mut editor.body).desired_width(f32::INFINITY).desired_rows(22));
                if ui.button("암호화하여 저장").clicked() { save = true; }
            });
        if save {
            if let Some(state) = self.panes[editor.pane].vault.as_mut() {
                match state.database.update_file(editor.id, editor.body.as_bytes()) {
                    Ok(()) => { self.refresh_vault(editor.pane); open = false; },
                    Err(e) => self.status = e,
                }
            } else { open = false; }
        }
        if open { self.vault_text_editor = Some(editor); }
    }

    pub(super) fn vault_new_item(&mut self, folder: bool) {
        let pane = self.active;
        let title = if folder { "새 폴더" } else { "새 파일" };
        let message = if folder { "암호화 폴더에 만들 폴더 이름을 입력하세요." }
            else { "암호화 폴더에 만들 파일 이름을 입력하세요." };
        self.prompt_file_name(title, message, "", move |app, name, _| {
            let Some(name) = name.filter(|n| !n.is_empty()) else { return };
            let Some(state) = app.panes[pane].vault.as_mut() else { return };
            let current = state.folder();
            let result = if folder { state.database.add_folder(current, &name).map(|_| ()) }
                else { state.database.add_file(current, &name, b"").map(|_| ()) };
            match result {
                Ok(()) => { app.pending_focus[pane] = Some(name); app.refresh_vault(pane) },
                Err(e) => app.status = e,
            }
        });
    }

    pub(super) fn vault_rename_item(&mut self) {
        let pane = self.active;
        let Some(entry) = self.panes[pane].entries.get(self.panes[pane].cursor).cloned()
            .filter(|e| !e.is_parent()) else { return };
        let old = entry.name;
        self.prompt_file_name("이름 바꾸기", "새 이름을 입력하세요.", &old.clone(), move |app, new, _| {
            let Some(new) = new.filter(|n| !n.is_empty()) else { return };
            let Some(state) = app.panes[pane].vault.as_mut() else { return };
            let result = state.database.list(state.folder()).and_then(|items| {
                let id = items.iter().find(|v| v.name == old).ok_or("항목을 찾을 수 없습니다.")?.id;
                state.database.rename(id, &new)
            });
            match result {
                Ok(()) => { app.pending_focus[pane] = Some(new); app.refresh_vault(pane) },
                Err(e) => app.status = e,
            }
        });
    }

    pub(super) fn vault_remove_items(&mut self) {
        let pane = self.active;
        let names: Vec<String> = self.target_entries(pane).into_iter().map(|e| e.name).collect();
        if names.is_empty() { return; }
        let label = format!("암호화 폴더에서 {}개 항목을 영구 삭제할까요?", names.len());
        self.system_box("암호화 폴더", &label,
            ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONEXCLAMATION, move |app, confirmed, _| {
                if confirmed != ndr::msgbox::IDYES { return; }
                let Some(state) = app.panes[pane].vault.as_mut() else { return };
                let result = state.database.list(state.folder()).and_then(|items| {
                    let mut ids = Vec::new();
                    for item in items {
                        if names.contains(&item.name) { ids.push(item.id); }
                    }
                    if ids.len() != names.len() { return Err("삭제할 항목이 변경되었습니다.".into()); }
                    state.database.remove_many(&ids)
                });
                match result { Ok(()) => app.refresh_vault(pane), Err(e) => app.status = e }
            });
    }

    /// Paste source bytes directly into SQLCipher without plaintext staging.
    pub(super) fn vault_paste_files(&mut self) {
        let pane = self.active;
        let from_system = (self.clipboard_files_reader)().ok().filter(|(paths, _)| !paths.is_empty());
        let from_internal = self.clipboard.clone().filter(|_| self.clipboard_sequence == Some(native::clipboard_sequence_number()));
        let Some((files, was_cut)) = from_system.or(from_internal) else {
            self.status = "붙여넣을 파일 또는 폴더가 없습니다.".into();
            return;
        };
        if was_cut {
            self.status = "암호화 폴더로 이동은 지원하지 않습니다. 원본을 유지하는 복사를 사용하세요.".into();
            return;
        }
        self.vault_import_paths(pane, files);
    }

    /// Copy to other pane (F3) from an unlocked vault is an explicit plaintext
    /// export. Warn before releasing decrypted bytes to the filesystem.
    pub(super) fn vault_export_to_other(&mut self, source: usize) {
        let target = 1 - source;
        if self.panes[target].vault.is_some() || self.panes[target].archive.is_some()
            || self.panes[target].remote.is_some() || !self.panes[target].path.is_dir() {
            self.status = "내보내기 대상은 일반 디렉터리여야 합니다.".into();
            return;
        }
        let names: Vec<String> = self.target_entries(source).into_iter().map(|entry| entry.name).collect();
        if names.is_empty() { return; }
        let destination = self.panes[target].path.clone();
        let message = format!("선택한 {}개 항목을 암호화하지 않은 일반 파일로 내보낼까요?\n대상: {}\n기존 파일은 덮어쓰지 않습니다.", names.len(), destination.display());
        self.system_box("암호화 폴더에서 내보내기", &message,
            ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONEXCLAMATION, move |app, answer, _| {
                if answer != ndr::msgbox::IDYES { return; }
                let Some(state) = app.panes[source].vault.as_mut() else { return };
                match state.database.export_items(state.folder(), &names, &destination) {
                    Ok(count) => {
                        app.refresh(target);
                        app.status = format!("{count}개 항목을 일반 파일로 내보냈습니다. 대상 디렉터리는 암호화되지 않았습니다.");
                    }
                    Err(error) => app.status = format!("암호화 폴더 내보내기 실패: {error}"),
                }
            });
    }

    pub(super) fn vault_import_paths(&mut self, pane: usize, files: Vec<PathBuf>) {
        if files.is_empty() { return; }
        let Some(state) = self.panes[pane].vault.as_mut() else { return };
        match state.database.import_paths(state.folder(), &files) {
            Ok(count) => {
                self.refresh_vault(pane);
                self.status = format!("암호화 폴더에 {count}개 항목을 가져왔습니다.");
            }
            Err(error) => {
                self.status = format!("암호화 폴더 가져오기 실패 (모든 변경 취소): {error}");
            }
        }
    }
}

impl Drop for VaultDialog {
    fn drop(&mut self) {
        self.folder_name.zeroize();
        self.password.zeroize();
        self.confirmation.zeroize();
        if let Some((_, body)) = &mut self.text { body.zeroize(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f3_copies_regular_files_and_folders_into_vault_and_exports_with_confirmation() {
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-f5", "ko");
        let vault_file = root.join("right/safe.ndr");
        let database = Vault::create(&vault_file, "strong password 123456".into()).unwrap();
        app.attach_vault(1, PaneVault::new(vault_file.clone(), database));

        app.active = 0;
        let file_index = app.panes[0].entries.iter().position(|entry| entry.name == "alpha.txt").unwrap();
        app.panes[0].cursor = file_index;
        app.dispatch("copyToOther", &ctx);
        let listing = app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap();
        let imported = listing.iter().find(|item| item.name == "alpha.txt").unwrap();
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.read_file(imported.id).unwrap(), b"alpha.txt");

        let folder_index = app.panes[0].entries.iter().position(|entry| entry.name == "folder1").unwrap();
        app.panes[0].cursor = folder_index;
        app.dispatch("copyToOther", &ctx);
        let items = app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap();
        assert!(items.iter().any(|item| item.name == "folder1" && item.folder));

        // Ctrl+V can paste nested regular folders, not just single files.
        let source_tree = root.join("external_tree");
        std::fs::create_dir_all(source_tree.join("nested")).unwrap();
        std::fs::write(source_tree.join("nested/inside.txt"), b"nested import").unwrap();
        app.active = 1;
        app.clipboard = Some((vec![source_tree.clone()], false));
        app.clipboard_sequence = Some(native::clipboard_sequence_number());
        app.vault_paste_files();
        let items = app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap();
        let imported = items.iter().find(|item| item.name == "external_tree").unwrap();
        let nested = app.panes[1].vault.as_mut().unwrap().database.list(imported.id).unwrap();
        let file = app.panes[1].vault.as_mut().unwrap().database.list(nested[0].id).unwrap();
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.read_file(file[0].id).unwrap(), b"nested import");
        app.clipboard = None;

        let output = root.join("export");
        std::fs::create_dir_all(&output).unwrap();
        app.navigate(0, output.clone(), false);
        app.active = 1;
        let file_index = app.panes[1].entries.iter().position(|entry| entry.name == "alpha.txt").unwrap();
        app.panes[1].cursor = file_index;
        app.dispatch("copyToOther", &ctx);
        assert!(!app.msgboxes.is_empty(), "plaintext export must require a confirmation");
        assert!(!output.join("alpha.txt").exists(), "confirmation must precede plaintext export");
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        assert_eq!(std::fs::read(output.join("alpha.txt")).unwrap(), b"alpha.txt");
        assert!(vault_file.is_file());
        app.discard_vault(1);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn leaving_vault_clears_all_decrypted_pane_buffers() {
        let (root, _ctx, mut app) = super::super::tools_tests::fixture("vault-clear", "ko");
        let file = root.join("right/locked.ndr");
        let mut vault = Vault::create(&file, "strong password 123456".into()).unwrap();
        vault.add_file(1, "SECRET_DO_NOT_LEAK.txt", b"confidential").unwrap();
        app.attach_vault(1, PaneVault::new(file, vault));
        assert!(app.panes[1].entries.iter().any(|entry| entry.name.contains("SECRET")));
        app.discard_vault(1);
        assert!(app.panes[1].entries.is_empty());
        assert!(app.panes[1].view.cells.is_empty());
        assert!(app.panes[1].listed.is_none());
        assert!(!app.panes[1].address.contains("locked.ndr"));
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn creating_encrypted_folder_immediately_enters_unlocked_pane() {
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-new-folder", "ko");
        let parent = root.join("left");
        app.open_vault(parent.join("placeholder.ndr"), true);
        let dialog = app.vault_dialog.as_mut().unwrap();
        dialog.folder_name = "티라노사우루스".into();
        dialog.password = "strong password 123456".into();
        dialog.confirmation = dialog.password.clone();
        dialog.authenticate();
        assert!(dialog.error.is_empty(), "{}", dialog.error);
        let _ = ctx.run(egui::RawInput::default(), |ctx| { app.vault_window_ui(ctx); });
        assert!(app.vault_dialog.is_none());
        assert!(app.panes[0].vault.is_some());
        assert!(app.panes[0].entries.iter().any(|e| e.is_parent()));
        assert!(parent.join("티라노사우루스.ndr").is_file());
        assert!(!parent.join("티라노사우루스").exists());
        app.parent();
        assert!(app.panes[0].vault.is_none());
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn encrypted_container_behaves_like_a_folder_in_both_panes() {
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-pane", "ko");
        let parent = root.join("left");
        let file = parent.join("스테고사우루스.ndr");
        // Start with a container created by the current SQLCipher backend.
        let mut database = Vault::create(&file, "correct horse battery staple".into()).unwrap();
        database.add_folder(1, "비밀 서류").unwrap();
        database.add_file(1, "notes.txt", b"private text").unwrap();
        drop(database);

        app.refresh(0);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.panes[0].loading {
            assert!(std::time::Instant::now() < until, "pane listing timed out");
            app.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let index = app.panes[0].entries.iter().position(|entry| entry.name == "스테고사우루스").unwrap();
        assert!(app.panes[0].entries[index].is_dir, "vault must appear as a folder");
        assert!(app.panes[0].entries[index].path.is_file(), "container bytes stay in one file");
        app.panes[0].cursor = index;
        app.open_current();
        let dialog = app.vault_dialog.as_mut().unwrap();
        dialog.password = "correct horse battery staple".into();
        dialog.authenticate();
        let _ = ctx.run(egui::RawInput::default(), |ctx| { app.vault_window_ui(ctx); });
        assert!(app.vault_dialog.is_none(), "no further sign-in modal after successful unlock");
        assert!(app.panes[0].vault.is_some(), "authentication enters the pane");
        let sub = app.panes[0].entries.iter().position(|e| e.name == "비밀 서류").unwrap();
        assert!(app.panes[0].entries[sub].is_dir);
        app.panes[0].cursor = sub;
        app.open_current();
        assert_eq!(app.panes[0].vault.as_ref().unwrap().stack.len(), 2);
        app.parent();
        assert!(app.panes[0].vault.is_some(), "leaving subfolder does not require reauthentication");
        assert!(app.panes[0].entries.iter().any(|e| e.name == "notes.txt"));
        app.parent();
        assert!(app.panes[0].vault.is_none(), "leaving the container closes the session");
        assert!(file.is_file());
        assert!(!parent.join("스테고사우루스").exists(), "no plaintext folder was created");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn random_dinosaur_name_starts_focused_and_fully_selected() {
        let ctx = egui::Context::default();
        let mut dialog = VaultDialog::new(PathBuf::from("C:\\temp\\new.ndr"), true);
        let length = dialog.folder_name.chars().count();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                dialog.draw(ui, SessionPolicy::CloseOnLeave);
            });
        });
        assert!(!dialog.name_select_pending, "initial selection must happen once");
        let state = egui::TextEdit::load_state(&ctx, egui::Id::new("ndr-vault-folder-name")).unwrap();
        let selection = state.cursor.char_range().unwrap();
        assert_eq!(selection.primary.index.min(selection.secondary.index), 0);
        assert_eq!(selection.primary.index.max(selection.secondary.index), length);
    }

    #[test]
    fn encrypted_folder_dialog_accepts_plain_name_and_keeps_extension_internal() {
        let root = std::env::temp_dir().join(format!("ndr-vault-create-ui-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let mut dialog = VaultDialog::new(root.join("새 암호화 폴더.ndr"), true);
        assert!(!dialog.folder_name.is_empty());
        assert!(dialog.name_select_pending);
        assert!(!dialog.folder_name.ends_with(".ndr"));
        dialog.folder_name = "비밀 문서".into();
        dialog.password = "long secure passphrase 12345!".into();
        dialog.confirmation = dialog.password.clone();
        dialog.authenticate();
        assert!(dialog.error.is_empty(), "{}", dialog.error);
        assert_eq!(dialog.created_path.as_deref(), Some(root.join("비밀 문서.ndr").as_path()));
        assert!(root.join("비밀 문서.ndr").is_file());
        assert!(dialog.vault.is_some(), "the created folder should stay authenticated for entry");
        assert!(Vault::unlock(&root.join("비밀 문서.ndr"), "wrong password 123456".into()).is_err());
        assert!(Vault::unlock(&root.join("비밀 문서.ndr"), "long secure passphrase 12345!".into()).is_ok());
        drop(dialog);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn encrypted_folder_name_cannot_escape_parent_or_overwrite() {
        let root = Path::new("C:\\Users\\example");
        for name in ["", "..", "../outside", "sub\\outside", "CON", "bad:name", "trailing."] {
            assert!(VaultDialog::container_path(root, name).is_err(), "{name}");
        }
        assert_eq!(VaultDialog::container_path(root, "자료.ndr").unwrap(), root.join("자료.ndr"));
    }
}

impl VaultDialog {
    fn new(path: PathBuf, creating: bool) -> Self {
        let folder_name = if creating {
            super::dinosaur_names::suggest(path.parent().unwrap_or_else(|| Path::new("")))
        }
            else { String::new() };
        Self { path, creating, folder_name, name_select_pending: creating, created_path: None, password: String::new(), confirmation: String::new(), vault: None,
            folder: 1, folder_stack: vec![(1, String::from("/"))], selected: None,
            name: String::new(), import_path: String::new(), export_path: String::new(), text: None, error: String::new() }
    }

    /// Store .ndr only on disk; the name shown in the creation dialog is
    /// exactly the same kind of plain folder name the New Folder dialog uses.
    fn container_path(parent: &Path, name: &str) -> Result<PathBuf, String> {
        let name = name.trim();
        if name.is_empty() || name == "." || name == ".." || name.len() > 240
            || name.ends_with('.') || name.ends_with(' ')
            || name.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c)) {
            return Err("올바른 폴더 이름을 입력해 주세요.".into());
        }
        let base = name.strip_suffix(".ndr").or_else(|| name.strip_suffix(".NDR")).unwrap_or(name);
        if base.is_empty() || base.ends_with('.') || base.ends_with(' ') {
            return Err("올바른 폴더 이름을 입력해 주세요.".into());
        }
        let device = base.split('.').next().unwrap_or("").to_ascii_uppercase();
        if matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9" | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9") {
            return Err("Windows에서 사용할 수 없는 폴더 이름입니다.".into());
        }
        Ok(parent.join(format!("{base}.ndr")))
    }

    fn authenticate(&mut self) {
        if self.creating {
            let parent = self.path.parent().unwrap_or_else(|| Path::new(""));
            match Self::container_path(parent, &self.folder_name) {
                Ok(path) => self.path = path,
                Err(message) => { self.error = message; return; }
            }
        }
        if self.creating && self.password != self.confirmation {
            self.error = "비밀번호가 일치하지 않습니다.".into();
            return;
        }
        let secret = std::mem::take(&mut self.password);
        self.confirmation.zeroize();
        self.confirmation.clear();
        let outcome = if self.creating { Vault::create(&self.path, secret) }
            else { Vault::unlock(&self.path, secret) };
        match outcome {
            Ok(vault) => {
                if self.creating {
                    self.created_path = Some(self.path.clone());
                    self.vault = Some(vault);
                    self.password.zeroize();
                    self.confirmation.zeroize();
                    self.error.clear();
                    return;
                }
                self.vault = Some(vault);
                self.creating = false;
                self.folder = 1;
                self.folder_stack = vec![(1, "/".into())];
                self.error.clear();
            }
            Err(message) => self.error = message,
        }
    }

    fn lock(&mut self) {
        self.vault = None;
        self.selected = None;
        if let Some((_, body)) = &mut self.text { body.zeroize(); }
        self.text = None;
        self.folder = 1;
        for (_, name) in &mut self.folder_stack { name.zeroize(); }
        self.folder_stack = vec![(1, "/".into())];
        self.import_path.zeroize();
        self.export_path.zeroize();
        self.name.zeroize();
        self.password.zeroize();
        self.password.clear();
        self.confirmation.zeroize();
        self.confirmation.clear();
    }

    fn apply(&mut self, operation: impl FnOnce(&mut Vault) -> Result<(), String>) {
        if let Some(vault) = &mut self.vault {
            match operation(vault) {
                Ok(()) => self.error.clear(),
                Err(message) => self.error = message,
            }
        }
    }

    /// Same centered movable VCL-style dialog and 444px content width as
    /// New Folder / TfGetText. Only the additional password fields make it
    /// taller. No separate heading, grid, or native egui::Window chrome.
    fn draw_create(&mut self, ui: &mut egui::Ui) -> bool {
        let (area, _) = ui.allocate_exact_size(egui::vec2(444.0, 250.0), egui::Sense::hover());
        let at = |x: f32, y: f32, width: f32, height: f32| {
            egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(width, height))
        };

        ui.put(at(18.0, 8.0, 411.0, 25.0), egui::Label::new("암호화할 새 폴더의 이름을 입력하세요."));
        ui.put(at(18.0, 47.0, 100.0, 21.0), egui::Label::new("폴더 이름"));
        let response = ui.put(at(122.0, 43.0, 307.0, 23.0),
            egui::TextEdit::singleline(&mut self.folder_name)
                .id(egui::Id::new("ndr-vault-folder-name")));
        if self.name_select_pending {
            response.request_focus();
            super::original_dialogs::select_all(ui, response.id, self.folder_name.chars().count());
            self.name_select_pending = false;
        }
        ui.put(at(18.0, 80.0, 100.0, 21.0), egui::Label::new("비밀번호"));
        ui.put(at(122.0, 76.0, 307.0, 23.0),
            egui::TextEdit::singleline(&mut self.password).password(true));
        ui.put(at(18.0, 113.0, 100.0, 21.0), egui::Label::new("비밀번호 확인"));
        ui.put(at(122.0, 109.0, 307.0, 23.0),
            egui::TextEdit::singleline(&mut self.confirmation).password(true));

        ui.put(at(18.0, 145.0, 411.0, 19.0), egui::Label::new(
            egui::RichText::new("폴더 안의 파일과 이름이 암호화됩니다. 비밀번호는 6자 이상입니다.").small().weak()));
        ui.put(at(18.0, 165.0, 411.0, 19.0), egui::Label::new(
            egui::RichText::new("※ 비밀번호를 잊으면 복구할 수 없습니다.").small().weak()));

        // Align exactly with the bottom-right controls of New Folder:
        // x=265 / 351, width=79, height=25; only Y moves for extra fields.
        let mut close = false;
        if ui.put(at(265.0, 215.0, 79.0, 25.0), egui::Button::new("확인")).clicked() {
            self.authenticate();
        }
        if ui.put(at(351.0, 215.0, 79.0, 25.0), egui::Button::new("취소")).clicked() {
            close = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) { close = true; }
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) && !close && self.created_path.is_none() {
            self.authenticate();
        }
        if !self.error.is_empty() {
            ui.painter().text(at(18.0, 189.0, 411.0, 18.0).left_top(),
                egui::Align2::LEFT_TOP, &self.error,
                egui::FontId::proportional(11.0), ui.visuals().error_fg_color);
        }
        close || self.created_path.is_some()
    }

    fn draw(&mut self, ui: &mut egui::Ui, policy: SessionPolicy) -> bool {
        if self.vault.as_ref().is_some_and(|vault| vault.expired(policy)) {
            self.lock();
            self.error = "세션이 만료되었습니다. 비밀번호를 다시 입력해야 합니다.".into();
        }
        if self.vault.is_none() {
            if self.creating { return self.draw_create(ui); }
            ui.heading("암호화 폴더 잠금 해제");
            ui.add_space(8.0);
            ui.label(self.path.file_stem().unwrap_or_default().to_string_lossy());
            ui.horizontal(|ui| {
                ui.label("비밀번호");
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
            });
            if ui.button("잠금 해제").clicked() { self.authenticate(); }
            ui.label(egui::RichText::new("비밀번호는 저장되지 않습니다.").small().weak());
        } else {
            ui.horizontal(|ui| {
                if ui.button("⬅ 상위 폴더").clicked() && self.folder_stack.len() > 1 {
                    if let Some((_, mut name)) = self.folder_stack.pop() { name.zeroize(); }
                    self.folder = self.folder_stack.last().map_or(1, |v| v.0);
                    self.selected = None;
                    self.text = None;
                    if let Some(vault) = &mut self.vault { vault.touch(); }
                }
                ui.label(self.folder_stack.iter().map(|f| f.1.as_str()).collect::<Vec<_>>().join(" / "));
                if ui.button("즉시 잠금").clicked() { self.lock(); }
            });
            if let Some(vault) = &mut self.vault {
                match vault.list(self.folder) {
                    Ok(items) => {
                        egui::ScrollArea::vertical().max_height(225.0).show(ui, |ui| {
                            for item in items {
                                let label = format!("{}  {} {}", if item.folder { "📁" } else { "📄" }, item.name,
                                    if item.folder { String::new() } else { format!("({} B)", item.size) });
                                let chosen = self.selected.as_ref().is_some_and(|v| v.id == item.id);
                                let response = ui.selectable_label(chosen, label);
                                if response.clicked() { vault.touch(); self.selected = Some(item.clone()); self.text = None; }
                                if response.double_clicked() && item.folder {
                                    self.folder = item.id;
                                    self.folder_stack.push((item.id, item.name.clone()));
                                    self.selected = None;
                                    self.text = None;
                                }
                            }
                        });
                    }
                    Err(error) => self.error = error,
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("이름");
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(240.0));
                if ui.button("새 폴더").clicked() {
                    let folder = self.folder; let name = self.name.clone();
                    self.apply(|v| v.add_folder(folder, &name).map(|_| ()));
                }
                if ui.button("새 빈 파일").clicked() {
                    let folder = self.folder; let name = self.name.clone();
                    self.apply(|v| v.add_file(folder, &name, b"").map(|_| ()));
                }
            });
            if let Some(selected) = self.selected.clone() {
                ui.horizontal(|ui| {
                    if ui.button("선택 항목 이름 변경").clicked() {
                        let name = self.name.clone(); self.apply(|v| v.rename(selected.id, &name));
                    }
                    if ui.button("선택 항목 삭제").clicked() {
                        self.apply(|v| v.remove(selected.id));
                        if self.error.is_empty() { self.selected = None; self.text = None; }
                    }
                    if !selected.folder && ui.button("텍스트 편집").clicked() {
                        if let Some(vault) = &mut self.vault {
                            match vault.read_file(selected.id) {
                                Ok(data) if data.len() <= 1024 * 1024 => match String::from_utf8(data) {
                                    Ok(text) => { self.text = Some((selected.id, text)); self.error.clear(); }
                                    Err(_) => self.error = "UTF-8 텍스트가 아닙니다.".into(),
                                },
                                Ok(_) => self.error = "1 MiB 이하 텍스트만 편집할 수 있습니다.".into(),
                                Err(message) => self.error = message,
                            }
                        }
                    }
                });
            }
            if let Some((id, content)) = self.text.as_mut() {
                ui.label("메모리 내 텍스트 편집 (임시 평문 파일 생성 없음)");
                ui.add(egui::TextEdit::multiline(content).desired_rows(5).desired_width(f32::INFINITY));
                if ui.button("암호화 컨테이너에 저장").clicked() {
                    let id = *id; let data = content.as_bytes().to_vec();
                    self.apply(|v| v.update_file(id, &data));
                }
            }
            ui.horizontal(|ui| {
                ui.label("가져올 로컬 파일 경로");
                ui.add(egui::TextEdit::singleline(&mut self.import_path).desired_width(260.0));
                if ui.button("암호화하여 가져오기").clicked() {
                    let source = PathBuf::from(self.import_path.trim());
                    match std::fs::metadata(&source) {
                        Ok(meta) if meta.is_file() && meta.len() <= MAX_FILE_SIZE as u64 => {
                            let folder = self.folder;
                            match (source.file_name().and_then(|n| n.to_str()), std::fs::read(&source)) {
                                (Some(name), Ok(data)) => {
                                    let data = zeroize::Zeroizing::new(data);
                                    self.apply(|v| v.add_file(folder, name, &data).map(|_| ()));
                                }
                                _ => self.error = "가져올 파일을 읽을 수 없습니다.".into(),
                            }
                        }
                        _ => self.error = "파일을 찾지 못했거나 64 MiB 크기 제한을 초과했습니다.".into(),
                    }
                }
            });
            if self.selected.as_ref().is_some_and(|item| !item.folder) {
                ui.horizontal(|ui| {
                    ui.label("명시적 평문 내보내기 경로");
                    ui.add(egui::TextEdit::singleline(&mut self.export_path).desired_width(260.0));
                    if ui.button("평문 파일 내보내기").clicked() {
                        let path = PathBuf::from(self.export_path.trim());
                        let id = self.selected.as_ref().unwrap().id;
                        if path.as_os_str().is_empty() || path.exists() {
                            self.error = "비어 있지 않은 새 경로만 지정할 수 있습니다 (덮어쓰기 금지).".into();
                        } else if let Some(vault) = &mut self.vault {
                            match vault.read_file(id).and_then(|data| {
                                let data = zeroize::Zeroizing::new(data);
                                use std::io::Write;
                                let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
                                out.write_all(&data).map_err(|e| e.to_string())?;
                                out.sync_all().map_err(|e| e.to_string())
                            }) {
                                Ok(()) => self.error = "요청한 경로에 평문을 내보냈습니다. 직접 보안 관리가 필요합니다.".into(),
                                Err(message) => self.error = message,
                            }
                        }
                    }
                });
            }
        }
        if !self.error.is_empty() { ui.colored_label(ui.visuals().error_fg_color, &self.error); }
        ui.separator();
        ui.label("보안: 닫거나 세션이 만료되면 잠금 해제 정보가 폐기됩니다. 잠금 해제 때마다 비밀번호를 다시 입력합니다.");
        ui.button("닫기 (즉시 잠금)").clicked()
    }
}

impl App {
    pub(super) fn open_vault(&mut self, path: PathBuf, creating: bool) {
        self.vault_dialog = Some(VaultDialog::new(path, creating));
    }

    pub(super) fn vault_window_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.vault_dialog.take() else { return };
        if self.settings.flag("VaultLockOnMinimize", false)
            && ctx.input(|input| input.viewport().minimized.unwrap_or(false)) {
            dialog.lock();
        }
        let policy = SessionPolicy::from_settings(&self.settings);
        let mut open = true;
        let mut close = false;
        if dialog.creating {
            // The ordinary New Folder prompt uses the same modal and title
            // helpers. This places both dialogs at the same owner-window
            // center and gives them matching chrome and title dragging.
            let id = egui::Id::new("ndr-vault-create-modal");
            super::keep_opaque(ctx, id);
            super::movable_modal(ctx, id).show(ctx, |ui| {
                let title = super::modal_title_start(ui, "새 암호화 폴더");
                close = dialog.draw_create(ui);
                close |= super::modal_title_finish(ui, id, title);
            });
        } else {
            egui::Window::new("암호화 폴더")
                .id(egui::Id::new("ndr-sqlcipher-vault"))
                .open(&mut open).resizable(true).default_width(700.0)
                .show(ctx, |ui| { close = dialog.draw(ui, policy); });
        }
        if let Some(database) = dialog.vault.take() {
            self.attach_vault(self.active, PaneVault::new(dialog.path.clone(), database));
            return;
        }
        if open && !close { self.vault_dialog = Some(dialog); }
        // Otherwise the connection and all plaintext/password UI buffers drop.
    }
}
