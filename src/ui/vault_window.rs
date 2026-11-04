//! In-process encrypted vault browser. Opening in an external application
//! explicitly creates a temporary decrypted copy; passwords are not stored.
use super::*;
use ndr::vault::{Vault, VaultItem, SessionPolicy};
use zeroize::Zeroize;

pub(super) struct PaneVault {
    pub file: PathBuf,
    pub database: Vault,
    pub stack: Vec<(i64, String)>,
}

impl PaneVault {
    fn new(file: PathBuf, database: Vault) -> Self {
        Self { file, database, stack: vec![(1, String::new())] }
    }
    pub(super) fn folder(&self) -> i64 { self.stack.last().map_or(1, |item| item.0) }
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
    pub(super) fn vault_status(&self, ko: &str, en: &str) -> String {
        if self.language == "ko" { ko } else { en }.to_owned()
    }

    /// Discard the in-memory connection AND all decrypted pane presentation
    /// buffers, including on tab switches and direct address navigation.
    pub(super) fn discard_vault(&mut self, pane: usize) {
        #[cfg(feature = "winfsp-vault")]
        {
            // Navigating the NDR pane is independent of external editors.
            // Keep busy WinFsp volumes alive in the background so a program
            // can save the document even after the user navigates to "..".
            let current = self.panes[pane].vault.as_ref().map(|v| v.file.clone());
            if let Some(current) = current {
                let mut index = 0;
                while index < self.vault_mounts.len() {
                    if self.vault_mounts[index].vault_path != current {
                        index += 1;
                        continue;
                    }
                    if self.vault_mounts[index].has_open_files() {
                        index += 1;
                        continue;
                    }
                    if let Err(error) = self.vault_mounts[index].finish() {
                        eprintln!("NDR encrypted mount finalization failed: {error}");
                        self.status = self.vault_status(
                            "암호화 파일을 저장하지 못했습니다. 편집기에서 다시 저장해 주세요.",
                            "Encrypted file save failed. Retry saving it from the editor.");
                        // Never discard credentials on a failed commit.
                        return;
                    }
                    self.vault_mounts.remove(index);
                }
                if let Some(state) = self.panes[pane].vault.as_mut() {
                    if let Err(error) = state.database.reload_after_mount() {
                        eprintln!("NDR encrypted folder refresh failed: {error}");
                        self.status = self.vault_status(
                            "암호화 폴더 목록을 갱신하지 못했습니다.",
                            "Could not refresh the encrypted folder list.");
                        return;
                    }
                }
            }
        }
        // A file transfer may own an open SQLCipher transaction between UI
        // frames. Roll it back before caching or dropping the unlocked vault.
        if self.vault_transfer.as_ref().is_some_and(|job|
            job.finished.is_none() && job.stream.is_some() &&
            (job.source == pane || job.target == pane)) {
            if let Some(job) = self.vault_transfer.as_mut() { job.cancelled = true; }
            let ctx = self.egui_ctx.clone();
            self.vault_transfer_tick(&ctx);
        }
        let Some(mut container) = self.panes[pane].vault.take() else { return };
        for (_, name) in &mut container.stack { name.zeroize(); }
        // Keep only the authenticated SQLCipher connection, never the
        // decrypted pane view or editor text. No password is stored on disk.
        let policy = SessionPolicy::from_settings(&self.settings);
        if !container.database.expired(policy) {
            let key = std::fs::canonicalize(&container.file).unwrap_or(container.file.clone());
            self.vault_sessions.insert(key, container.database);
        }
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
    }

    pub(super) fn vault_security_tick(&mut self, ctx: &egui::Context) {
        let policy = SessionPolicy::from_settings(&self.settings);
        let minimized = self.settings.flag("VaultLockOnMinimize", false)
            && ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        self.vault_sessions.retain(|_, database| !minimized && !database.expired(policy));
        for pane in 0..2 {
            let expired = self.panes[pane].vault.as_ref().is_some_and(|state|
                minimized || state.database.expired(policy));
            if expired {
                // Session expiry/minimize locks the pane, not an external
                // application's still-open WinFsp handle. Its in-process
                // mount stays alive until the file can be disconnected safely.
                self.exit_vault(pane);
                self.status = self.language_pack.text("NDRVault.StatusLocked", "Encrypted folder locked. Reopen to authenticate.");
            }
        }
        // Even if exiting an active pane temporarily cached its session,
        // minimize-lock must invalidate every retained authentication.
        if minimized { self.vault_sessions.clear(); }
        #[cfg(feature = "winfsp-vault")]
        {
            let mut index = 0;
            while index < self.vault_trees.len() {
                let file = &self.vault_trees[index].vault_path;
                // discard_vault stores sessions by their canonical file path,
                // which can be \?\D:\... rather than the pane's D:\... path.
                let session_key = std::fs::canonicalize(file).unwrap_or_else(|_| file.clone());
                let authenticated = self.panes.iter().any(|pane|
                    pane.vault.as_ref().is_some_and(|state| state.file == *file))
                    || self.vault_sessions.contains_key(&session_key);
                // Revocation is one-way. Reauthenticating before an old
                // external handle closes creates a NEW drive; the previously
                // revoked volume still needs to be unmounted when idle.
                if authenticated && !self.vault_trees[index].is_revoked() {
                    index += 1;
                    continue;
                }
                // Closing/expiry revokes new file handles, but a previously
                // opened editor may finish its authenticated save.
                self.vault_trees[index].revoke();
                if self.vault_trees[index].has_open_files() {
                    index += 1;
                    continue;
                }
                match self.vault_trees[index].finish() {
                    Ok(()) => { self.vault_trees.remove(index); },
                    Err(error) => {
                        eprintln!("NDR encrypted session unmount failed: {error}");
                        index += 1;
                    }
                }
            }
        }
    }

    fn attach_vault(&mut self, pane: usize, container: PaneVault) {
        // The decrypted container remains in memory, never as a plain folder.
        self.discard_vault(pane);
        self.panes[pane].archive = None;
        self.panes[pane].vault = Some(container);
        #[cfg(feature = "winfsp-vault")]
        self.ensure_vault_tree(pane);
        let ctx = self.egui_ctx.clone();
        self.arm_folder_watch(pane, None, &ctx);
        self.refresh(pane);
    }

    /// Mount the authenticated container *once* as a real read/write Windows
    /// drive so Explorer and external applications can use files and nested
    /// folders normally. No temporary decrypted mirror is ever created.
    #[cfg(feature = "winfsp-vault")]
    fn ensure_vault_tree(&mut self, pane: usize) {
        if cfg!(test) { return; }
        if !super::vault_winfsp::runtime_available() {
            self.offer_winfsp_installation();
            return;
        }
        let Some(vault) = self.panes[pane].vault.as_ref() else { return };
        if self.vault_trees.iter().any(|m| m.vault_path == vault.file && !m.is_revoked()) {
            return;
        }
        let file = vault.file.clone();
        let gate = self.vault_mounts.iter().find(|m| m.vault_path == file)
            .map(|m| m.writer_gate())
            .or_else(|| self.vault_trees.iter().find(|m| m.vault_path == file)
                .map(|m| m.writer_gate()))
            .unwrap_or_else(|| std::sync::Arc::new(std::sync::Mutex::new(())));
        let occupied: Vec<char> = self.vault_trees.iter().filter_map(|m| m.drive_letter())
            .chain(self.vault_mounts.iter().filter_map(|m| m.drive_letter())).collect();
        match super::vault_winfsp::VaultTreeMount::start(
            file.clone(), &vault.database, gate, &occupied) {
            Ok(tree) => {
                let root = tree.mount_root.display().to_string();
                self.vault_trees.push(tree);
                self.status = self.vault_status(
                    &format!("암호화 폴더가 {root}에 연결됐습니다. 다른 프로그램에서도 일반 폴더처럼 사용할 수 있습니다."),
                    &format!("Encrypted folder mounted at {root}. Explorer and other applications can use it."));
            }
            Err(error) => {
                eprintln!("NDR full encrypted directory mount failed for {}: {error}", file.display());
                self.status = self.vault_status(
                    "암호화 폴더를 Windows 가상 드라이브에 연결하지 못했습니다. NDR 내부 파일 작업은 계속 사용할 수 있습니다.",
                    "Could not mount encrypted folder as a Windows drive; NDR internal operations remain available.");
            }
        }
    }

    pub(super) fn refresh_vault(&mut self, pane: usize) {
        let policy = SessionPolicy::from_settings(&self.settings);
        let Some(container) = self.panes[pane].vault.as_mut() else { return };
        if container.database.expired(policy)
            || self.settings.flag("VaultLockOnMinimize", false)
                && self.egui_ctx.input(|i| i.viewport().minimized.unwrap_or(false)) {
            self.exit_vault(pane);
            self.status = self.language_pack.text("NDRVault.StatusLocked", "Encrypted folder locked. Reopen to authenticate.");
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
        if self.panes[pane].vault.is_some() { return; }
        self.pending_focus[pane] = file_name;
        self.navigate(pane, parent, false);
    }

    pub(super) fn enter_vault_item(&mut self, pane: usize, entry: &Entry) {
        // Navigating back must remain possible without the former modal
        // Close-and-save button. Leaving the vault finalizes its mounted file.
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
        if let Ok(found) = state.database.find_child(state.folder(), &entry.name) {
            if let Some(item) = found {
                if item.folder {
                    state.stack.push((item.id, item.name.clone()));
                    self.pending_focus[pane] = None;
                    state.database.touch();
                    self.refresh_vault(pane);
                } else {
                    self.open_vault_item(pane, item);
                }
            }
        }
    }
    /// Open a vault entry through the authenticated WinFsp virtual drive.
    /// No plaintext file is exported to %TEMP% or any physical directory.
    fn open_vault_item(&mut self, pane: usize, item: VaultItem) {
        #[cfg(not(feature = "winfsp-vault"))]
        {
            let _ = (pane, item);
            self.error_notice("보안상 파일 열기 차단", "Encrypted file opening blocked",
                "보안 문제로 암호화 폴더 내부 파일을 실행하려면 WinFsp가 설치되어 있어야 합니다.");
        }
        #[cfg(feature = "winfsp-vault")]
        {
            if !super::vault_winfsp::runtime_available() {
                self.offer_winfsp_installation();
                return;
            }
            if self.vault_transfer.is_some() {
                self.status = self.vault_status(
                    "암호화 파일 전송이 끝난 후 다시 시도해 주세요.",
                    "Finish the encrypted file transfer before opening another file.");
                return;
            }
            let Some(state) = self.panes[pane].vault.as_ref() else { return };
            // When a whole directory is mounted, every item shares the same
            // real virtual-drive path. Other applications can create sibling
            // files or use atomic rename/save patterns without a new mount.
            if let Some(tree) = self.vault_trees.iter().find(|m|
                m.vault_path == state.file && !m.is_revoked()) {
                let relative = match state.database.relative_folder_path(state.folder()) {
                    Ok(path) => path,
                    Err(error) => { self.status = error; return; }
                };
                let path = tree.mount_root.join(relative).join(&item.name);
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let textual = ["txt", "md", "log", "ini", "cfg", "json", "xml", "csv", "yaml", "yml", "toml", "rs", "py", "bat", "ps1"]
                    .iter().any(|known| ext.eq_ignore_ascii_case(known));
                let opened = if textual {
                    native::shell_open(Path::new("notepad.exe"),
                        Some(&native::quote_arg(&path.to_string_lossy())), "open")
                } else {
                    native::shell_open(&path, None, "open")
                };
                if let Err(error) = opened {
                    eprintln!("NDR encrypted directory file open failed: {error}");
                    self.error_notice("암호화 파일 열기 실패", "Encrypted file open failed",
                        self.vault_status("외부 프로그램에서 파일을 열지 못했습니다.",
                                          "Could not open the encrypted file."));
                }
                return;
            }
            // Reopening the same item must reuse its active drive: two
            // independent writers to the same file would lose changes.
            if let Some(mount) = self.vault_mounts.iter().find(|mount|
                mount.vault_path == state.file && mount.parent_id == state.folder()
                    && mount.item_name == item.name) {
                let path = mount.file_path.clone();
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                let textual = ["txt", "md", "log", "ini", "cfg", "json", "xml", "csv", "yaml", "yml", "toml", "rs", "py", "bat", "ps1"]
                    .iter().any(|known| ext.eq_ignore_ascii_case(known));
                let opened = if textual {
                    native::shell_open(std::path::Path::new("notepad.exe"),
                        Some(&native::quote_arg(&path.to_string_lossy())), "open")
                } else { native::shell_open(&path, None, "open") };
                if let Err(error) = opened {
                    eprintln!("NDR virtual file reopen failed: {error}");
                    self.error_notice("암호화 파일 열기 실패", "Encrypted file open failed",
                        self.vault_status("파일을 다시 열 수 없습니다.", "Could not reopen the file."));
                }
                return;
            }
            let existing = self.vault_mounts.iter().find(|mount| mount.vault_path == state.file);
            let writer_gate = existing.map(|mount| mount.writer_gate())
                .unwrap_or_else(|| std::sync::Arc::new(std::sync::Mutex::new(())));
            let occupied: Vec<char> = self.vault_mounts.iter().filter_map(|mount| mount.drive_letter()).collect();
            let result = super::vault_winfsp::VaultMount::start(
                pane, state.file.clone(), &state.database, state.folder(), item,
                writer_gate, &occupied);
            match result {
                Ok(mut mount) => {
                    // Plain text must go to a real editor, not a file-type
                    // association that may choose a read-only previewer.
                    let extension = mount.file_path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
                    let textual = ["txt", "md", "log", "ini", "cfg", "json", "xml", "csv", "yaml", "yml", "toml", "rs", "py", "bat", "ps1"]
                        .iter().any(|known| extension.eq_ignore_ascii_case(known));
                    let opened = if textual {
                        native::shell_open(std::path::Path::new("notepad.exe"),
                            Some(&native::quote_arg(&mount.file_path.to_string_lossy())), "open")
                    } else {
                        native::shell_open(&mount.file_path, None, "open")
                    };
                    if let Err(error) = opened {
                        let _ = mount.finish();
                        eprintln!("NDR encrypted file open failed: {error}");
                        self.error_notice("암호화 파일 열기 실패", "Encrypted file open failed",
                            self.vault_status("파일을 외부 프로그램으로 열지 못했습니다.", "Cannot open the file in its external application."));
                    } else {
                        self.vault_mounts.push(mount);
                    }
                }
                Err(error) => {
                    if error.contains("not installed") {
                        self.offer_winfsp_installation();
                    } else {
                        eprintln!("NDR virtual file mount failed: {error}");
                        self.error_notice("가상 드라이브 연결 실패", "WinFsp mount failed",
                            self.vault_status("암호화 파일 연결에 실패했습니다. 가상 드라이브 사용 상태를 확인해 주세요.", "Unable to connect encrypted virtual file."));
                    }
                }
            }
        }
    }

    #[cfg(feature = "winfsp-vault")]
    fn offer_winfsp_installation(&mut self) {
        if self.msgboxes.iter().any(|dialog| dialog.title == "WinFsp") { return; }
        let message = if self.language == "ko" {
            "보안 문제로 암호화 폴더 내부 파일을 열려면\nWinFsp가 설치되어 있어야 합니다.\n\nNDR에 포함된 WinFsp를 설치하시겠습니까?\n(Windows 관리자 승인 필요)"
        } else {
            "WinFsp must be installed to securely open\nfiles inside encrypted folders.\n\nInstall the WinFsp driver included with NDR?\n(Windows administrator approval required)"
        };
        self.system_box("WinFsp", message,
            ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONEXCLAMATION,
            |app, answer, _| {
                if answer == ndr::msgbox::IDYES {
                    match super::vault_winfsp::launch_embedded_installer() {
                        Ok(()) => app.status = if app.language == "ko" {
                            "설치 완료 후 암호화 파일을 다시 열어 주세요."
                        } else { "After installation completes, open the encrypted file again." }.into(),
                        Err(error) => app.error_notice("WinFsp 설치 실패", "WinFsp installation failed", error),
                    }
                }
            });
    }

    #[cfg(feature = "winfsp-vault")]
    pub(super) fn vault_mount_ui(&mut self, _ctx: &egui::Context) {
        // Files save through WinFsp Flush/Close, without a blocking dialog.
        // Each mount owns a separate virtual letter, but commits share a
        // writer gate so their authenticated metadata cannot overwrite peers.
        let mut updated = [false; 2];
        let mut errors = Vec::new();
        for tree in &mut self.vault_trees {
            if tree.take_saved_revision() {
                for pane in 0..2 {
                    if self.panes[pane].vault.as_ref()
                        .is_some_and(|state| state.file == tree.vault_path) {
                        updated[pane] = true;
                    }
                }
            }
            if let Some(error) = tree.take_save_error() {
                errors.push(format!("{}: {error}", tree.vault_path.display()));
            }
        }
        for mount in &mut self.vault_mounts {
            match mount.take_saved_revision() {
                Ok(true) => {
                    if self.panes[mount.pane].vault.as_ref()
                        .is_some_and(|vault| vault.file == mount.vault_path) {
                        updated[mount.pane] = true;
                    }
                },
                Ok(false) => {},
                Err(error) => eprintln!("NDR vault revision check: {error}"),
            }
            if mount.take_save_error_notice() {
                errors.push(mount.file_path.file_name().unwrap_or_default().to_string_lossy().into_owned());
            }
        }
        for pane in 0..2 {
            if !updated[pane] { continue; }
            if let Some(vault) = self.panes[pane].vault.as_mut() {
                match vault.database.reload_after_mount() {
                    Ok(()) => self.refresh_vault(pane),
                    Err(error) => {
                        eprintln!("NDR vault listing refresh failed: {error}");
                        self.error_notice("암호화 폴더 갱신 실패", "Encrypted folder refresh failed",
                            self.vault_status("저장된 파일 목록을 갱신하지 못했습니다.",
                                "Could not refresh the encrypted file list."));
                    }
                }
            }
        }
        for file in errors {
            self.error_notice("암호화 저장 실패", "Encrypted file save failed",
                self.vault_status(&format!("{file}: 변경 내용을 암호화 저장하지 못했습니다. 편집기에서 다시 저장해 주세요."),
                    &format!("{file}: Could not encrypt changes. Retry saving from the editor.")));
        }
    }

    /// Finish idle file volumes before a container metadata mutation.
    /// Active editors are never disconnected, and writable operations are
    /// never allowed against an index that an existing mount can overwrite.
    fn vault_prepare_write(&mut self, pane: usize) -> bool {
        #[cfg(feature = "winfsp-vault")]
        {
            let Some(path) = self.panes[pane].vault.as_ref().map(|v| v.file.clone()) else {
                return false;
            };
            // The whole-directory mount serializes every authenticated edit,
            // so an external program using another file does not block this
            // NDR pane's create/rename/delete operations.
            if self.vault_trees.iter().any(|tree|
                tree.vault_path == path && !tree.is_revoked()) {
                if let Some(vault) = self.panes[pane].vault.as_mut() {
                    if let Err(error) = vault.database.reload_after_mount() {
                        self.status = error;
                        return false;
                    }
                }
                return true;
            }
            if self.vault_mounts.iter().any(|m| m.vault_path == path && m.has_open_files()) {
                self.status = self.vault_status(
                    "외부 프로그램에서 사용 중인 암호화 파일이 있습니다. 해당 파일을 닫고 다시 시도해 주세요.",
                    "Close externally opened encrypted files before changing this container.");
                return false;
            }
            let mut index = 0;
            while index < self.vault_mounts.len() {
                if self.vault_mounts[index].vault_path != path { index += 1; continue; }
                if let Err(error) = self.vault_mounts[index].finish() {
                    eprintln!("NDR vault metadata operation: mount finalization error: {error}");
                    self.status = self.vault_status(
                        "암호화 파일 저장이 완료되지 않아 작업을 중단했습니다.",
                        "The encrypted file has not saved; operation cancelled.");
                    return false;
                }
                self.vault_mounts.remove(index);
            }
            if let Some(vault) = self.panes[pane].vault.as_mut() {
                if let Err(error) = vault.database.reload_after_mount() {
                    eprintln!("NDR vault metadata operation: refresh failed: {error}");
                    self.status = self.vault_status("암호화 폴더 갱신에 실패했습니다.",
                        "Encrypted folder refresh failed.");
                    return false;
                }
            }
        }
        true
    }

    /// The NDR panel and the WinFsp driver share a single authenticated
    /// writer lease. Refresh the encrypted index *inside* that lease so a
    /// later Windows file save cannot overwrite a new folder/rename/delete.
    fn vault_modify_metadata<T>(&mut self, pane: usize,
        operation: impl FnOnce(&mut Vault, i64) -> Result<T, String>) -> Result<T, String> {
        let path = self.panes[pane].vault.as_ref()
            .ok_or("Encrypted folder is closed")?.file.clone();
        #[cfg(feature = "winfsp-vault")]
        let gate = self.vault_trees.iter().find(|m| m.vault_path == path && !m.is_revoked())
            .map(|mount| mount.writer_gate());
        #[cfg(feature = "winfsp-vault")]
        let _guard = gate.as_ref().map(|g| g.lock().map_err(|e| e.to_string())).transpose()?;
        #[cfg(not(feature = "winfsp-vault"))]
        let _ = path;
        let state = self.panes[pane].vault.as_mut()
            .ok_or("Encrypted folder is closed")?;
        state.database.reload_after_mount()?;
        let folder = state.folder();
        operation(&mut state.database, folder)
    }

    /// Clipboard is an in-app virtual reference, not a plaintext temp file
    /// and never the physical .ndr container path. Ctrl+V in another NDR
    /// pane invokes the normal confirmed streaming export.
    pub(super) fn vault_copy_selection(&mut self, moving: bool) {
        let source = self.active;
        let Some(state) = self.panes[source].vault.as_ref() else { return };
        let names: Vec<String> = self.target_entries(source).into_iter()
            .filter(|e| !e.is_parent()).map(|e| e.name).collect();
        if names.is_empty() { return; }
        let file = state.file.clone();
        let folder = state.folder();
        // Do not leave an old CF_HDROP clipboard entry referencing the
        // physical .ndr behind. Only a harmless explanatory text marker
        // enters the Windows clipboard; actual data stays authenticated in
        // the virtual NDR clipboard until the user confirms Ctrl+V export.
        let marker = self.vault_status(
            "NDR 암호화 폴더에서 복사한 항목은 NDR 일반 폴더 패널에서 Ctrl+V로 내보낼 수 있습니다.",
            "Encrypted entries can be exported by pressing Ctrl+V in a regular NDR folder pane.");
        let _ = (self.clipboard_writer)(&marker);
        let sequence = native::clipboard_sequence_number();
        self.vault_clipboard = Some((source, file, folder, names.clone(), moving, sequence));
        self.status = self.vault_status(
            "선택한 암호화 항목을 NDR 내부 클립보드에 보관했습니다. 반대쪽 일반 폴더에서 Ctrl+V로 내보내세요.",
            "Encrypted selection copied within NDR. Switch to the regular destination pane and press Ctrl+V.");
    }

    pub(super) fn vault_paste_selection_to_local(&mut self) -> bool {
        let Some((source, path, folder, names, moving, sequence)) = self.vault_clipboard.clone() else {
            return false;
        };
        if native::clipboard_sequence_number() != sequence {
            self.vault_clipboard = None;
            return false; // A different application owns the newest clipboard.
        }
        let destination = self.active;
        if source == destination || self.panes[source].vault.as_ref().is_none_or(
            |v| v.file != path || v.folder() != folder) {
            self.vault_clipboard = None;
            return false; // allow the real Windows clipboard as fallback
        }
        self.vault_export_named_to_other(source, names, moving);
        true
    }

    pub(super) fn vault_new_item(&mut self, folder: bool) {
        if !self.vault_prepare_write(self.active) { return; }
        let pane = self.active;
        let title = self.language_pack.text(if folder { "NDRVault.NewFolderTitle" } else { "NDRVault.NewFileTitle" },
            if folder { "New folder" } else { "New file" });
        let message = self.language_pack.text(if folder { "NDRVault.NewFolderPrompt" } else { "NDRVault.NewFilePrompt" },
            if folder { "Enter a folder name inside the encrypted container." } else { "Enter a file name inside the encrypted container." });
        self.prompt_file_name(&title, &message, "", move |app, name, _| {
            let Some(name) = name.filter(|n| !n.is_empty()) else { return };
            if !app.vault_prepare_write(pane) { return; }
            let result = app.vault_modify_metadata(pane, |vault, current| {
                if folder { vault.add_folder(current, &name).map(|_| ()) }
                else { vault.add_file(current, &name, b"").map(|_| ()) }
            });
            match result {
                Ok(()) => {
                    app.pending_focus[pane] = Some(name.clone());
                    app.refresh_vault(pane);
                    app.completion_notice(if folder { "암호화 폴더 생성 완료" } else { "암호화 파일 생성 완료" },
                        if folder { "Encrypted folder created" } else { "Encrypted file created" }, name);
                },
                Err(e) => app.status = e,
            }
        });
    }

    pub(super) fn vault_rename_item(&mut self) {
        if !self.vault_prepare_write(self.active) { return; }
        let pane = self.active;
        let Some(entry) = self.panes[pane].entries.get(self.panes[pane].cursor).cloned()
            .filter(|e| !e.is_parent()) else { return };
        let old = entry.name;
        let title = self.language_pack.text("NDRVault.RenameTitle", "Rename");
        let message = self.language_pack.text("NDRVault.RenamePrompt", "Enter a new name.");
        self.prompt_file_name(&title, &message, &old.clone(), move |app, new, _| {
            let Some(new) = new.filter(|n| !n.is_empty()) else { return };
            if !app.vault_prepare_write(pane) { return; }
            #[cfg(feature = "winfsp-vault")]
            if app.vault_trees.iter().any(|tree| {
                app.panes[pane].vault.as_ref().is_some_and(|v|
                    tree.vault_path == v.file && tree.is_open_item(v.folder(), &old))
            }) {
                app.status = app.vault_status(
                    "외부 프로그램에서 사용 중인 파일이나 폴더는 이름을 변경할 수 없습니다.",
                    "Close the external application before renaming this item.");
                return;
            }
            let result = app.vault_modify_metadata(pane, |database, folder| {
                let items = database.list(folder)?;
                let id = items.iter().find(|v| v.name == old)
                    .ok_or("Encrypted item was not found")?.id;
                database.rename(id, &new)
            });
            match result {
                Ok(()) => {
                    app.pending_focus[pane] = Some(new.clone());
                    app.refresh_vault(pane);
                    app.completion_notice("이름 변경 완료", "Rename completed", new);
                },
                Err(e) => app.status = e,
            }
        });
    }

    /// Explicit encrypted-only compaction. The user chooses when to incur
    /// the cost, leaving the normal fast delete and copy path unchanged.
    pub(super) fn vault_compact_confirm(&mut self) {
        #[cfg(feature = "winfsp-vault")]
        if self.vault_mounts.iter().any(|mount| mount.pane == self.active) {
            self.status = self.vault_status(
                "열려 있는 암호화 파일을 외부 프로그램에서 닫은 다음 다시 시도해 주세요.",
                "Close externally opened encrypted files before modifying the container.");
            return;
        }
        let pane=self.active;
        if self.panes[pane].vault.is_none() { return; }
        if self.vault_transfer.is_some() {
            self.status="Finish the encrypted transfer before compacting".into();
            return;
        }
        let title=self.language_pack.text("NDRVault.DeleteTitle", "Encrypted folder");
        let prompt=if self.language.eq_ignore_ascii_case("ko") {
            "현재 .ndr에서 삭제된 암호화 기록을 제외하고 나머지 파일을 다시 저장합니다. SSD나 백업의 과거 기록은 제거되지 않습니다. 계속할까요?"
        }else{
            "Rebuild the current .ndr without deleted records. This may take time; SSD remnants and external backups are not erased. Continue?"
        };
        self.system_box(&title,prompt,ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONEXCLAMATION,
            move |app,confirmed,_|{
                if confirmed!=ndr::msgbox::IDYES { return; }
                if app.vault_transfer.is_some() {
                    app.status="Finish the encrypted transfer before compacting".into();
                    return;
                }
                #[cfg(feature = "winfsp-vault")]
                {
                    let Some(path) = app.panes[pane].vault.as_ref().map(|v| v.file.clone())
                        else { return; };
                    if app.vault_trees.iter().any(|tree|
                        tree.vault_path == path && tree.has_open_files()) {
                        app.status = app.vault_status(
                            "암호화 드라이브를 사용하는 외부 프로그램을 닫고 다시 시도해 주세요.",
                            "Close external applications using this encrypted drive.");
                        return;
                    }
                    let mut index = 0;
                    while index < app.vault_trees.len() {
                        if app.vault_trees[index].vault_path != path {
                            index += 1;
                            continue;
                        }
                        if let Err(error) = app.vault_trees[index].finish() {
                            app.status = format!("Cannot compact an active encrypted drive: {error}");
                            return;
                        }
                        app.vault_trees.remove(index);
                    }
                }
                let Some(container)=app.panes[pane].vault.take() else{return;};
                let PaneVault{file,database,stack}=container;
                // The old container remains intact on any pre-swap error.
                // If compacting fails, close this view and allow the user to
                // reauthenticate safely rather than retaining stale handles.
                match database.compact() {
                    Ok(database)=>{
                        app.panes[pane].vault=Some(PaneVault{file,database,stack});
                        #[cfg(feature = "winfsp-vault")]
                        app.ensure_vault_tree(pane);
                        app.refresh_vault(pane);
                        app.status=if app.language.eq_ignore_ascii_case("ko"){
                            "암호화 폴더 정리와 무결성 검사가 완료되었습니다.".into()
                        }else{"Encrypted folder compaction and integrity verification completed.".into()};
                        app.play_operation_completion_sound();
                        app.completion_notice("암호화 폴더 정리 완료", "Encrypted folder compacted",
                            "Encrypted container rebuilt and verified");
                    }
                    Err(error)=>{
                        app.error_notice("암호화 폴더 정리 실패", "Encrypted folder compaction failed", error.clone());
                        let parent=file.parent().unwrap_or_else(||Path::new(".")).to_path_buf();
                        app.pending_focus[pane]=file.file_stem().map(|n|n.to_string_lossy().into_owned());
                        app.navigate(pane,parent,false);
                        app.status=format!("Encrypted folder compaction failed (original retained if replacement did not occur): {error}");
                    }
                }
            });
    }
    pub(super) fn vault_remove_items(&mut self) {
        let pane = self.active;
        let names: Vec<String> = self.target_entries(pane).into_iter().map(|e| e.name).collect();
        if names.is_empty() { return; }
        let label = self.language_pack.text("NDRVault.DeleteConfirm", "Permanently delete {count} encrypted items?")
            .replace("{count}", &names.len().to_string());
        let title = self.language_pack.text("NDRVault.DeleteTitle", "Encrypted folder");
        self.system_box(&title, &label,
            ndr::msgbox::MB_YESNO | ndr::msgbox::MB_ICONEXCLAMATION, move |app, confirmed, _| {
                if confirmed != ndr::msgbox::IDYES { return; }
                let Some(path) = app.panes[pane].vault.as_ref().map(|v| v.file.clone()) else { return };
                // An in-use source may still be saved by its editor. Reject
                // deletion of that source or its ancestor directory, while
                // allowing unrelated files to be deleted immediately.
                let folder = app.panes[pane].vault.as_ref().unwrap().folder();
                let has_legacy = app.vault_mounts.iter().any(|m| m.vault_path == path);
                let selected_mounted = app.vault_mounts.iter().any(|m|
                    m.vault_path == path && m.parent_id == folder &&
                    names.iter().any(|name| name.eq_ignore_ascii_case(&m.item_name)))
                    || app.vault_trees.iter().any(|tree|
                        tree.vault_path == path && names.iter().any(|name|
                            tree.is_open_item(folder, name)));
                if selected_mounted {
                    app.status = app.vault_status(
                        "외부 프로그램에서 사용 중인 암호화 파일이나 폴더는 삭제할 수 없습니다.",
                        "An encrypted item is open in an external application.");
                    return;
                }
                let result = app.vault_modify_metadata(pane, |database, current_folder| {
                    let current = database.list(current_folder)?;
                    let chosen: Vec<_> = current.iter().filter(|item|
                        names.iter().any(|name| name == &item.name)).collect();
                    if chosen.len() != names.len() {
                        return Err("Encrypted selection changed before deletion".into());
                    }
                    if has_legacy && chosen.iter().any(|item| item.folder) {
                        return Err("Close connected legacy files before deleting a folder".into());
                    }
                    let ids: Vec<i64> = chosen.iter().map(|item| item.id).collect();
                    database.remove_many(&ids)
                });
                match result {
                    Ok(()) => {
                        app.refresh_vault(pane);
                        app.play_operation_completion_sound();
                        app.completion_notice("암호화 항목 삭제 완료", "Encrypted items deleted",
                            if app.language == "ko" { format!("{}개 항목 삭제", names.len()) }
                            else { format!("{} item(s) deleted", names.len()) });
                    }
                    Err(e) => {
                        eprintln!("NDR encrypted delete failed: {e}");
                        app.status = app.vault_status(
                            "암호화 파일을 삭제하지 못했습니다. 사용 중인 파일이 있는지 확인해 주세요.",
                            "Could not delete encrypted items. Check whether a file is still in use.");
                    },
                }
            });
    }

    /// Paste source bytes directly into the authenticated container without plaintext staging.
    pub(super) fn vault_paste_files(&mut self) {
        #[cfg(feature = "winfsp-vault")]
        if self.vault_mounts.iter().any(|mount| mount.pane == self.active) {
            self.status = self.vault_status(
                "열려 있는 암호화 파일을 외부 프로그램에서 닫은 다음 다시 시도해 주세요.",
                "Close externally opened encrypted files before modifying the container.");
            return;
        }
        let pane = self.active;
        let from_system = (self.clipboard_files_reader)().ok().filter(|(paths, _)| !paths.is_empty());
        let from_internal = self.clipboard.clone().filter(|_| self.clipboard_sequence == Some(native::clipboard_sequence_number()));
        let Some((files, was_cut)) = from_system.or(from_internal) else {
            self.status = self.language_pack.text("NDRVault.NoPaste", "No files or folders to paste.");
            return;
        };
        // Ctrl+X from Explorer has the same meaning as F4: the original
        // disappears only after a complete authenticated encrypted commit.
        self.vault_transfer_from_local(1 - pane, pane, files, was_cut);
    }

    /// F3/F4 from an unlocked container behave like ordinary folder
    /// operations. Explicitly selecting Copy or Move is the user's consent
    /// to write the destination; only actual name conflicts need a dialog.
    pub(super) fn vault_export_to_other(&mut self, source: usize, moving: bool) {
        let names: Vec<String> = self.target_entries(source).into_iter()
            .filter(|e| !e.is_parent()).map(|e| e.name).collect();
        self.vault_export_named_to_other(source, names, moving);
    }

    fn vault_export_named_to_other(&mut self, source: usize,
                                  names: Vec<String>, moving: bool) {
        if moving {
            let Some(state) = self.panes[source].vault.as_ref() else { return };
            #[cfg(feature = "winfsp-vault")]
            if self.vault_trees.iter().any(|tree|
                tree.vault_path == state.file && names.iter().any(|name|
                    tree.is_open_item(state.folder(), name))) {
                self.status = self.vault_status(
                    "사용 중인 암호화 파일이나 해당 폴더는 이동할 수 없습니다. 편집기를 닫고 다시 시도해 주세요.",
                    "Cannot move a file or folder used by an external application.");
                return;
            }
            let path = state.file.clone();
            let folder = state.folder();
            let relevant: Vec<_> = self.vault_mounts.iter()
                .filter(|mount| mount.vault_path == path).collect();
            if !relevant.is_empty() {
                // A running editor may still save the same file. Only move
                // unrelated files; never remove a live file or its parent
                // directory while a virtual-file writer can update it.
                let overlaps = relevant.iter().any(|mount|
                    mount.parent_id == folder && names.contains(&mount.item_name));
                let includes_folder = names.iter().any(|name|
                    state.database.find_child(folder, name).ok().flatten()
                        .is_some_and(|item| item.folder));
                if overlaps || includes_folder {
                    self.status = self.vault_status(
                        "외부 프로그램에 연결된 파일이나 해당 폴더는 아직 이동할 수 없습니다. 편집기를 닫고 다시 시도해 주세요.",
                        "Cannot move a mounted file or folder while an external editor is connected.");
                    return;
                }
            } else if !self.vault_prepare_write(source) {
                return;
            }
        }
        {
            // Observe already committed edits before copying or moving a
            // file. Plaintext is written only to the selected destination.
            if let Some(vault) = self.panes[source].vault.as_mut() {
                if let Err(error) = vault.database.reload_after_mount() {
                    eprintln!("NDR vault export refresh failed: {error}");
                    self.status = self.vault_status("암호화 파일 목록을 갱신하지 못했습니다.",
                        "Could not refresh encrypted files for export.");
                    return;
                }
            }
        }
        use super::vault_transfer_ui::VaultTransferKind;
        let target = 1 - source;
        if self.panes[target].vault.is_some() || self.panes[target].archive.is_some()
            || self.panes[target].remote.is_some() || !self.panes[target].path.is_dir() {
            self.status = self.language_pack.text("NDRVault.ExportInvalidTarget", "Export destination must be a regular directory.");
            return;
        }
        if names.is_empty() { return; }
        self.begin_vault_transfer(VaultTransferKind::Export, source, target, moving, names, vec![], None);
    }

    /// Import first, then remove local originals. Failure to delete a source
    /// never destroys the successfully encrypted copy; the status reports it.
    pub(super) fn vault_transfer_from_local(&mut self, source: usize, target: usize, files: Vec<PathBuf>, moving: bool) {
        #[cfg(feature = "winfsp-vault")]
        if self.vault_mounts.iter().any(|mount| mount.pane == self.active) {
            self.status = self.vault_status(
                "열려 있는 암호화 파일을 외부 프로그램에서 닫은 다음 다시 시도해 주세요.",
                "Close externally opened encrypted files before modifying the container.");
            return;
        }
        use super::vault_transfer_ui::VaultTransferKind;
        if files.is_empty() { return; }
        if !moving { return self.vault_import_paths(target, files); }
        let Some(names) = files.iter()
            .map(|file| file.file_name().and_then(|name| name.to_str()).map(str::to_owned))
            .collect::<Option<Vec<_>>>() else {
            self.status = self.language_pack.text("NDRVault.InvalidImportPath",
                "Cannot transfer a path without a valid Unicode filename.");
            return;
        };
        self.begin_vault_transfer(VaultTransferKind::Import, source, target, true, names, files, None);
    }

    /// Explorer/another NDR panel dropped files onto an unlocked vault
    /// folder. Keep the destination as an authenticated item ID, not a
    /// physical container path that Windows might copy next to the vault.
    pub(super) fn vault_import_drop(&mut self, pane: usize, paths: Vec<PathBuf>,
        destination: &Path, moving: bool) {
        let Some(state) = self.panes[pane].vault.as_ref() else { return; };
        let current = state.virtual_path();
        let folder = if destination == current {
            state.folder()
        } else if destination.parent() == Some(current.as_path()) {
            let Some(name) = destination.file_name().and_then(|n| n.to_str()) else { return; };
            let Ok(Some(child)) = state.database.find_child(state.folder(), name) else { return; };
            if !child.folder { return; }
            child.id
        } else {
            self.status = self.vault_status(
                "??? ?? ?? ??? ?? ??? ??? ???.",
                "Choose a destination inside the encrypted folder.");
            return;
        };
        #[cfg(feature = "winfsp-vault")]
        if let Some(tree) = self.vault_trees.iter().find(|m|
            m.vault_path == state.file && !m.is_revoked()) {
            let relative = state.database.relative_folder_path(folder).unwrap_or_default();
            let target = tree.mount_root.join(relative).to_string_lossy()
                .trim_end_matches(['\\', '/']).to_lowercase();
            if paths.iter().any(|path| path.is_dir() && {
                let src = path.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
                target == src || target.starts_with(&(src + "\\"))
            }) {
                self.status = self.vault_status(
                    "폴더를 자기 자신이나 하위 폴더로 복사할 수 없습니다.",
                    "Cannot copy a folder into itself or one of its descendants.");
                return;
            }
        }
        let Some(names) = paths.iter()
            .map(|path| path.file_name().and_then(|name| name.to_str()).map(str::to_owned))
            .collect::<Option<Vec<_>>>() else { return; };
        if paths.is_empty() { return; }
        self.begin_vault_transfer(super::vault_transfer_ui::VaultTransferKind::Import,
            1 - pane, pane, moving, names, paths, Some(folder));
    }

    pub(super) fn vault_import_paths(&mut self, pane: usize, files: Vec<PathBuf>) {
        #[cfg(feature = "winfsp-vault")]
        if self.vault_mounts.iter().any(|mount| mount.pane == self.active) {
            self.status = self.vault_status(
                "열려 있는 암호화 파일을 외부 프로그램에서 닫은 다음 다시 시도해 주세요.",
                "Close externally opened encrypted files before modifying the container.");
            return;
        }
        use super::vault_transfer_ui::VaultTransferKind;
        if files.is_empty() { return; }
        let Some(names) = files.iter()
            .map(|file| file.file_name().and_then(|name| name.to_str()).map(str::to_owned))
            .collect::<Option<Vec<_>>>() else {
            self.status = self.language_pack.text("NDRVault.InvalidImportPath", "Cannot transfer a path without a valid Unicode filename.");
            return;
        };
        self.begin_vault_transfer(VaultTransferKind::Import, 1 - pane, pane, false, names, files, None);
    }

}

impl App {
    /// Move/copy inside a single SQLCipher container; no plaintext staging.
    pub(super) fn vault_transfer_between_panes(&mut self, source: usize, target: usize, moving: bool) {
        #[cfg(feature = "winfsp-vault")]
        if self.vault_mounts.iter().any(|mount| mount.pane == self.active) {
            self.status = self.vault_status(
                "열려 있는 암호화 파일을 외부 프로그램에서 닫은 다음 다시 시도해 주세요.",
                "Close externally opened encrypted files before modifying the container.");
            return;
        }
        use super::vault_transfer_ui::VaultTransferKind;
        let names: Vec<String> = self.target_entries(source).into_iter().map(|entry| entry.name).collect();
        if names.is_empty() { return; }
        let same_container = {
            let a = self.panes[source].vault.as_ref().unwrap();
            let b = self.panes[target].vault.as_ref().unwrap();
            match (std::fs::canonicalize(&a.file), std::fs::canonicalize(&b.file)) {
                (Ok(left), Ok(right)) => left == right,
                _ => true,
            }
        };
        if same_container {
            self.status = self.language_pack.text("NDRVault.SameContainer", "Open two different encrypted containers for cross-container transfer.");
            return;
        }
        self.begin_vault_transfer(VaultTransferKind::Between, source, target, moving, names, vec![], None);
    }

    pub(super) fn vault_transfer_inside(&mut self, moving: bool) {
        #[cfg(feature = "winfsp-vault")]
        if self.vault_mounts.iter().any(|mount| mount.pane == self.active) {
            self.status = self.vault_status(
                "열려 있는 암호화 파일을 외부 프로그램에서 닫은 다음 다시 시도해 주세요.",
                "Close externally opened encrypted files before modifying the container.");
            return;
        }
        use super::vault_transfer_ui::VaultTransferKind;
        let pane = self.active;
        let names: Vec<String> = self.target_entries(pane).into_iter().map(|item| item.name).collect();
        if names.is_empty() { return; }
        let title = self.language_pack.text(if moving { "NDRVault.InsideMoveTitle" } else { "NDRVault.InsideCopyTitle" },
            if moving { "Move within encrypted container" } else { "Copy within encrypted container" });
        let message = self.language_pack.text("NDRVault.InsidePrompt", "Destination path relative to container root (e.g. documents/archive or /)");
        self.prompt(&title, &message, "/", None, false, move |app, destination, _| {
            let Some(destination) = destination else { return; };
            let Some(state) = app.panes[pane].vault.as_mut() else { return; };
            match state.database.resolve_folder(&destination) {
                Ok(folder) => app.begin_vault_transfer(VaultTransferKind::Inside, pane, pane, moving, names, vec![], Some(folder)),
                Err(error) => app.status = format!("{}: {error}", app.language_pack.text("NDRVault.InsideFailed", "Encrypted folder operation failed")),
            }
        });
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
    fn vault_ctrl_c_and_ctrl_v_export_selected_item_without_copying_ndr_path() {
        let (root, ctx, mut app) =
            super::super::tools_tests::fixture("vault-internal-clipboard", "ko");
        let path = root.join("right/clipboard.ndr");
        let mut database = Vault::create(&path, "clipboard test password".into()).unwrap();
        database.add_file(1, "only-this.txt", b"test payload").unwrap();
        database.add_file(1, "not-selected.txt", b"other payload").unwrap();
        app.attach_vault(1, PaneVault::new(path.clone(), database));
        app.active = 1;
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|e| e.name == "only-this.txt").unwrap();
        app.dispatch("copy", &ctx);
        let (pane, file, folder, names, cut, _) = app.vault_clipboard.as_ref().unwrap();
        assert_eq!(*pane, 1);
        assert_eq!(*file, path);
        assert_eq!(*folder, 1);
        assert_eq!(names, &vec!["only-this.txt".to_string()]);
        assert!(!cut);
        assert!(app.clipboard.is_none(),
            "no CF_HDROP of the physical .ndr container is allowed");
        app.active = 0;
        app.dispatch("paste", &ctx);
        assert!(app.msgboxes.is_empty(), "copying from an unlocked vault must not prompt");
        finish_transfer(&mut app);
        assert_eq!(std::fs::read(root.join("left/only-this.txt")).unwrap(), b"test payload");
        assert!(!root.join("left/not-selected.txt").exists());
        assert!(app.panes[1].vault.as_mut().unwrap().database
            .find_child(1, "only-this.txt").unwrap().is_some(),
            "Ctrl+C must not delete the encrypted original");
        app.active = 1;
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|e| e.name == "not-selected.txt").unwrap();
        app.dispatch("cut", &ctx);
        assert!(app.vault_clipboard.as_ref().unwrap().4, "Ctrl+X marks virtual move");
        app.active = 0;
        app.dispatch("paste", &ctx);
        assert!(app.msgboxes.is_empty(), "moving from an unlocked vault must not prompt");
        finish_transfer(&mut app);
        assert_eq!(std::fs::read(root.join("left/not-selected.txt")).unwrap(), b"other payload");
        assert!(app.panes[1].vault.as_mut().unwrap().database
            .find_child(1, "not-selected.txt").unwrap().is_none(),
            "Ctrl+X removes the original only after export succeeds");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explorer_drop_to_unlocked_vault_imports_files_and_folders_without_prompt() {
        let (root, _ctx, mut app) =
            super::super::tools_tests::fixture("vault-explorer-drop", "ko");
        let encrypted = root.join("right/drop.ndr");
        let mut database = Vault::create(&encrypted, "drop test password 123".into()).unwrap();
        database.add_folder(1, "nested").unwrap();
        app.attach_vault(1, PaneVault::new(encrypted, database));
        let current = app.panes[1].vault.as_ref().unwrap().virtual_path();
        let source_file = root.join("left/alpha.txt");
        app.handle_drop(1, vec![source_file.clone()], current.clone(),
            egui::Modifiers::default());
        assert!(app.msgboxes.is_empty(), "drop has no security confirmation");
        assert!(source_file.is_file(), "a copy leaves the original intact");
        finish_transfer(&mut app);
        let item = app.panes[1].vault.as_mut().unwrap().database
            .find_child(1, "alpha.txt").unwrap().unwrap();
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.read_file(item.id).unwrap(),
            b"alpha.txt");
        let folder_source = root.join("left/folder1");
        app.handle_drop(1, vec![folder_source.clone()], current.join("nested"),
            egui::Modifiers::default());
        finish_transfer(&mut app);
        let nested = app.panes[1].vault.as_mut().unwrap().database
            .find_child(1, "nested").unwrap().unwrap().id;
        assert!(app.panes[1].vault.as_mut().unwrap().database
            .find_child(nested, "folder1").unwrap().is_some());
        let moved = root.join("left/move-by-shift.txt");
        std::fs::write(&moved, b"move safely").unwrap();
        let mut shift = egui::Modifiers::default();
        shift.shift = true;
        app.handle_drop(1, vec![moved.clone()], current, shift);
        assert!(moved.is_file(), "move must not delete before encrypted commit");
        finish_transfer(&mut app);
        assert!(!moved.exists(), "Shift+drop removes source after verified import");
        let item = app.panes[1].vault.as_mut().unwrap().database
            .find_child(1, "move-by-shift.txt").unwrap().unwrap();
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.read_file(item.id).unwrap(),
            b"move safely");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "winfsp-vault")]
    #[test]
    #[ignore = "requires installed WinFsp driver and a free drive letter; disposable vault only"]
    fn unlocked_vault_full_drive_supports_explorer_and_ndr_transfer_without_warning() {
        use std::io::Write;
        let (root, ctx, mut app) =
            super::super::tools_tests::fixture("full-vault-app-interop", "ko");
        let path = root.join("right/full.ndr");
        let mut database = Vault::create(&path, "interop test passphrase 2026".into()).unwrap();
        database.add_file(1, "secret.txt", b"original").unwrap();
        app.attach_vault(1, PaneVault::new(path.clone(), database));
        let tree = super::super::vault_winfsp::VaultTreeMount::start(
            path.clone(), &app.panes[1].vault.as_ref().unwrap().database,
            std::sync::Arc::new(std::sync::Mutex::new(())), &[]).unwrap();
        let drive = tree.mount_root.clone();
        app.vault_trees.push(tree);
        assert_eq!(std::fs::read(drive.join("secret.txt")).unwrap(), b"original");
        // File from outside NDR copied onto the connected encrypted drive.
        std::fs::copy(root.join("left/alpha.txt"), drive.join("imported.txt")).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.vault_mount_ui(&ctx);
            if app.panes[1].entries.iter().any(|e| e.name == "imported.txt") { break; }
            assert!(std::time::Instant::now() < deadline, "NDR did not refresh after Explorer copy");
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        app.active = 1;
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|e| e.name == "secret.txt").unwrap();
        app.dispatch("copyToOther", &ctx);
        assert!(app.msgboxes.is_empty(), "explicit F3 needs no plaintext warning");
        finish_transfer(&mut app);
        assert_eq!(std::fs::read(root.join("left/secret.txt")).unwrap(), b"original");
        app.active = 1;
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|e| e.name == "secret.txt").unwrap();
        app.dispatch("copy", &ctx);
        assert!(app.vault_clipboard.is_some());
        app.begin_file_drag(1, app.panes[1].cursor, false, egui::Modifiers::default());
        assert!(app.file_drag.as_ref().is_some_and(|drag|
            drag.paths.iter().any(|p| p == &drive.join("secret.txt"))),
            "outward drag must advertise a real WinFsp path, never the physical .ndr");
        app.file_drag = None;
        let mut held = std::fs::OpenOptions::new().read(true).write(true)
            .open(drive.join("secret.txt")).unwrap();
        app.vault_modify_metadata(1, |vault, folder| {
            vault.add_folder(folder, "while-editor-open").map(|_| ())
        }).unwrap();
        held.write_all(b"updated").unwrap();
        held.set_len(b"updated".len() as u64).unwrap();
        held.sync_all().unwrap();
        drop(held);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.vault_mount_ui(&ctx);
            let state = app.panes[1].vault.as_mut().unwrap();
            state.database.reload_after_mount().unwrap();
            if state.database.read_file(state.database.find_child(1, "secret.txt").unwrap().unwrap().id)
                .is_ok_and(|content| content == b"updated") { break; }
            assert!(std::time::Instant::now() < deadline, "external edit did not save");
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let state = app.panes[1].vault.as_mut().unwrap();
        assert!(state.database.find_child(1, "while-editor-open").unwrap().is_some(),
            "editing a different file must not erase NDR's new directory");
        assert!(state.database.find_child(1, "imported.txt").unwrap().is_some(),
            "editing a different file must not erase Explorer imports");
        for tree in &mut app.vault_trees { tree.finish().unwrap(); }
        app.vault_trees.clear();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "winfsp-vault")]
    #[test]
    #[ignore = "requires installed WinFsp driver and a free drive letter"]
    fn live_editor_allows_copy_and_delete_of_unrelated_item() {
        let (root, ctx, mut app) =
            super::super::tools_tests::fixture("vault-live-export-delete", "ko");
        let path = root.join("right/test.ndr");
        let mut database = Vault::create(&path, "live test password".into()).unwrap();
        database.add_file(1, "opened.txt", b"keep mounted").unwrap();
        database.add_file(1, "export.txt", b"export me").unwrap();
        database.add_file(1, "delete.txt", b"remove me").unwrap();
        database.add_file(1, "move.txt", b"move me").unwrap();
        let opened = database.find_child(1, "opened.txt").unwrap().unwrap();
        app.attach_vault(1, PaneVault::new(path.clone(), database));
        let mount = super::super::vault_winfsp::VaultMount::start(
            1, path.clone(), &app.panes[1].vault.as_ref().unwrap().database,
            1, opened, std::sync::Arc::new(std::sync::Mutex::new(())), &[]).unwrap();
        let mut held = std::fs::OpenOptions::new().read(true).write(true)
            .open(&mount.file_path).unwrap();
        app.vault_mounts.push(mount);
        app.active = 1;
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|e| e.name == "export.txt").unwrap();
        app.dispatch("copyToOther", &ctx);
        assert!(app.msgboxes.is_empty(), "F3 must not prompt while another file is open");
        finish_transfer(&mut app);
        assert_eq!(std::fs::read(root.join("left/export.txt")).unwrap(), b"export me");
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|e| e.name == "delete.txt").unwrap();
        app.dispatch("deleteToolbar", &ctx);
        assert_eq!(app.msgboxes.len(), 1, "delete confirmation must appear with a live mount");
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        assert!(app.panes[1].vault.as_mut().unwrap().database
            .find_child(1, "delete.txt").unwrap().is_none());
        assert_eq!(app.vault_mounts.len(), 1, "editing the other file stays possible");
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|e| e.name == "move.txt").unwrap();
        app.dispatch("moveToOther", &ctx);
        assert!(app.msgboxes.is_empty(), "F4 must not prompt for explicit move");
        finish_transfer(&mut app);
        assert_eq!(std::fs::read(root.join("left/move.txt")).unwrap(), b"move me");
        assert!(app.panes[1].vault.as_mut().unwrap().database
            .find_child(1, "move.txt").unwrap().is_none());
        use std::io::Write;
        held.write_all(b"edited mount").unwrap();
        held.sync_all().unwrap();
        drop(held);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.vault_mounts[0].has_open_files()
            || app.vault_mounts[0].state_saved_revision_for_test() == 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let source = app.panes[1].vault.as_mut().unwrap();
        source.database.reload_after_mount().unwrap();
        assert!(source.database.find_child(1, "delete.txt").unwrap().is_none(),
            "later editor commits must preserve an earlier unrelated deletion");
        assert!(source.database.find_child(1, "move.txt").unwrap().is_none(),
            "later editor commits must not resurrect moved files");
        let opened = source.database.find_child(1, "opened.txt").unwrap().unwrap();
        assert!(source.database.read_file(opened.id).unwrap().starts_with(b"edited mount"));
        app.exit_vault(1);
        app.vault_sessions.clear();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "winfsp-vault")]
    #[test]
    #[ignore = "requires installed WinFsp driver and a free drive letter"]
    fn parent_navigation_does_not_disconnect_busy_external_editor() {
        use std::io::Write;
        let (root, _ctx, mut app) =
            super::super::tools_tests::fixture("winfsp-parent-busy", "ko");
        app.settings.values.insert("VaultSessionPolicy".into(), serde_json::json!("exit"));
        let path = root.join("left/parent-busy.ndr");
        let mut vault = Vault::create(&path, "test navigation password".into()).unwrap();
        vault.add_file(1, "note.txt", b"before").unwrap();
        let item = vault.find_child(1, "note.txt").unwrap().unwrap();
        app.attach_vault(0, PaneVault::new(path.clone(), vault));
        app.active = 0;
        let mount = {
            let database = &app.panes[0].vault.as_ref().unwrap().database;
            super::super::vault_winfsp::VaultMount::start(
                0, path.clone(), database, 1, item,
                std::sync::Arc::new(std::sync::Mutex::new(())), &[]
            ).unwrap()
        };
        let mut editor = std::fs::OpenOptions::new().write(true).open(&mount.file_path).unwrap();
        app.vault_mounts.push(mount);
        app.parent(); // same navigation path as clicking the ".." entry
        assert!(app.panes[0].vault.is_none(), "parent must be accessible with editor open");
        assert_eq!(app.vault_mounts.len(), 1, "external file mount must stay available");
        editor.write_all(b"after!").unwrap();
        editor.sync_all().unwrap();
        drop(editor);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.vault_mounts[0].has_open_files() {
            assert!(std::time::Instant::now() < deadline, "WinFsp close timed out");
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        app.open_vault(path.clone(), false);
        assert!(app.vault_dialog.is_none(), "session reentry should not demand password");
        let active = app.panes[0].vault.as_mut().unwrap();
        let note = active.database.find_child(1, "note.txt").unwrap().unwrap();
        assert_eq!(active.database.read_file(note.id).unwrap(), b"after!");
        app.exit_vault(0);
        assert!(app.vault_mounts.is_empty(), "closed background file can unmount safely");
        app.vault_sessions.clear();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(feature = "winfsp-vault")]
    #[test]
    fn winfsp_install_notice_uses_ndr_shared_message_dialog() {
        let (root, ctx, mut app) = super::super::tools_tests::fixture("winfsp-dialog-layout", "ko");
        app.offer_winfsp_installation();
        app.offer_winfsp_installation();
        assert_eq!(app.msgboxes.len(), 1, "duplicate file-open attempts must not stack prompts");
        let dialog = &app.msgboxes[0];
        assert_eq!(dialog.title, "WinFsp");
        assert!(dialog.text.contains("보안 문제"));
        assert_eq!(dialog.flags & 0xF, ndr::msgbox::MB_YESNO);
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.msgbox_ui(ctx));
        let layout = app.msgboxes[0].layout.as_ref().expect("NDR message layout");
        assert!(layout.client.x >= ndr::msgbox::minimum_client_width(2));
        assert!(!layout.title_shown, "no extra WinFsp title strip inside the dialog");
        assert!(app.msgboxes[0].system, "use NDR's standard system-style message layout");
        assert_eq!(layout.slots.iter().filter(|s| s.is_some()).count(), 2);
        assert!(layout.slots.iter().flatten().all(|rect| rect.height() == 25.0));
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }


    #[test]
    fn encrypted_pane_border_is_pink_and_reverts_on_exit_for_each_skin() {
        let (root, _ctx, mut app) = super::super::tools_tests::fixture("vault-pink-frame", "en");
        let container = root.join("left/pink.ndr");
        let mut initial_database = Some(Vault::create(&container, "pink frame password".into()).unwrap());
        let expected_pink = egui::Color32::from_rgb(0xFF, 0x69, 0xB4);
        for skin_name in ["Black", "Classic"] {
            app.settings.set_text("Theme", skin_name);
            let skin = app.current_skin().expect("skin should be available");
            app.active = 0;
            let local = app.skin_frame_stroke(&skin, 0);
            assert_ne!(local.color, expected_pink, "regular folder must use its usual skin color");
            if skin_name == "Black" {
                app.attach_vault(0, PaneVault::new(container.clone(), initial_database.take().unwrap()));
            } else {
                // The authenticated session survives leaving the container,
                // so the same database can be reused without password input.
                let db = app.vault_sessions.remove(&std::fs::canonicalize(&container).unwrap()).unwrap();
                app.attach_vault(0, PaneVault::new(container.clone(), db));
            }
            assert_eq!(app.skin_frame_stroke(&skin, 0).color, expected_pink,
                "vault root border must be vivid pink with active focus");
            app.active = 1;
            let back = super::super::skin_pane::skin_color(&skin, "lback", egui::Color32::from_gray(0x14));
            let channel = |bg: u8, pink: u8| ((u32::from(bg) * 40 + u32::from(pink) * 60) / 100) as u8;
            assert_eq!(app.skin_frame_stroke(&skin, 0).color,
                egui::Color32::from_rgb(channel(back.r(), 255), channel(back.g(), 105), channel(back.b(), 180)),
                "inactive vault border should keep the existing 40/60% blending");
            app.active = 0;
            app.exit_vault(0);
            assert_eq!(app.skin_frame_stroke(&skin, 0), local,
                "leaving an encrypted folder must restore the original folder border");
        }
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// The same PlayCompletionSound switch and packaged complete.wav must
    /// cover encrypted work; never signal completion on skip/cancel/failure.
    #[test]
    fn vault_completion_sound_respects_setting_and_is_once_per_successful_operation() {
        use super::super::tools_dialog::PLAYED;
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-operation-sound", "en");
        let encrypted = root.join("right/secure.ndr");
        let database = Vault::create(&encrypted, "test password 123456".into()).unwrap();
        app.attach_vault(1, PaneVault::new(encrypted, database));
        let expected_wave = Some(app.resources.join("complete.wav"));
        PLAYED.with(|p| p.borrow_mut().clear());
        app.settings.set_flag("PlayCompletionSound", true);

        // Import: a single file may take several streaming frames, but its
        // successful job emits the cue exactly once.
        app.vault_import_paths(1, vec![root.join("left/alpha.txt")]);
        finish_transfer(&mut app);
        assert_eq!(PLAYED.with(|p| p.borrow().clone()), vec![expected_wave.clone()]);
        let refresh_ctx = app.egui_ctx.clone();
        app.vault_transfer_tick(&refresh_ctx);
        assert_eq!(PLAYED.with(|p| p.borrow().len()), 1, "no repeat on later UI frames");

        // A duplicate skipped by choice is not a completed transfer.
        app.vault_import_paths(1, vec![root.join("left/alpha.txt")]);
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(
            Some((ConflictChoice::Skip, false))), &ctx);
        app.vault_transfer_tick(&ctx);
        assert_eq!(PLAYED.with(|p| p.borrow().len()), 1, "skipped operation stays silent");
        app.vault_transfer = None;

        // Aborting an attempted overwrite preserves existing vault contents
        // and also does not play the success cue.
        app.vault_import_paths(1, vec![root.join("left/alpha.txt")]);
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(
            Some((ConflictChoice::Cancel, false))), &ctx);
        app.vault_transfer_tick(&ctx);
        assert_eq!(PLAYED.with(|p| p.borrow().len()), 1, "cancelled operation stays silent");
        app.vault_transfer = None;

        app.settings.set_flag("PlayCompletionSound", false);
        app.vault_import_paths(1, vec![root.join("left/beta.txt")]);
        finish_transfer(&mut app);
        assert_eq!(PLAYED.with(|p| p.borrow().len()), 1, "setting off silences vault jobs");

        // Vault delete uses the exact same successful-operation audio hook.
        app.settings.set_flag("PlayCompletionSound", true);
        app.active = 1;
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|entry| entry.name == "beta.txt").unwrap();
        app.vault_remove_items();
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        assert_eq!(PLAYED.with(|p| p.borrow().clone()),
            vec![expected_wave.clone(), expected_wave.clone()]);

        // Export from .ndr back to an ordinary directory is also a regular
        // successful transfer and should chime once, not for every chunk.
        let exported_folder = root.join("exported-sounds");
        std::fs::create_dir(&exported_folder).unwrap();
        app.navigate(0, exported_folder.clone(), false);
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|entry| entry.name == "alpha.txt").unwrap();
        app.vault_export_to_other(1, false);
        finish_transfer(&mut app);
        assert_eq!(std::fs::read(exported_folder.join("alpha.txt")).unwrap(), b"alpha.txt");
        assert_eq!(PLAYED.with(|p| p.borrow().len()), 3, "export should sound exactly once");

        // An invalid local source causes an error and must not sound.
        app.vault_import_paths(1, vec![root.join("left/no-such-file.txt")]);
        app.vault_transfer.as_mut().unwrap().started = Instant::now() - Duration::from_millis(250);
        app.vault_transfer_tick(&ctx);
        assert!(app.vault_transfer.as_ref().unwrap().failed.is_some());
        assert_eq!(PLAYED.with(|p| p.borrow().len()), 3, "failed operation stays silent");
        app.vault_transfer = None;

        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ui_large_file_updates_bytes_and_cancel_rolls_back_partial_encryption() {
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-stream-cancel", "ko");
        let input = root.join("left/stream-24mb.bin");
        let file = std::fs::File::create(&input).unwrap();
        file.set_len(24 * 1024 * 1024 + 7).unwrap();
        drop(file);
        let container = root.join("right/stream.ndr");
        let database = Vault::create(&container, "stream password 123456".into()).unwrap();
        app.attach_vault(1, PaneVault::new(container, database));
        app.active = 0;
        app.vault_import_paths(1, vec![input.clone()]);
        app.vault_transfer.as_mut().unwrap().started = Instant::now() - Duration::from_millis(250);
        app.vault_transfer_tick(&ctx); // create file stream and begin encrypted transaction
        assert!(app.vault_transfer.as_ref().unwrap().stream.is_some());
        app.vault_transfer_tick(&ctx); // one 8 MiB slice
        let stream = app.vault_transfer.as_ref().unwrap().stream.as_ref().unwrap();
        assert_eq!(stream.done, ndr::vault::STREAM_CHUNK_SIZE as u64);
        assert_eq!(stream.total, 24 * 1024 * 1024 + 7);
        app.vault_transfer.as_mut().unwrap().cancelled = true;
        app.vault_transfer_tick(&ctx);
        assert!(app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap().is_empty(),
            "cancel must roll back every previously inserted 8 MiB chunk");
        assert!(input.is_file(), "cancel must retain the original large file");
        app.vault_transfer = None;

        app.vault_import_paths(1, vec![input.clone()]);
        app.vault_transfer.as_mut().unwrap().started = Instant::now() - Duration::from_millis(250);
        for _ in 0..20 {
            app.vault_transfer_tick(&ctx);
            if app.vault_transfer.as_ref().unwrap().finished.is_some() { break; }
        }
        let job = app.vault_transfer.as_ref().unwrap();
        assert!(job.finished.is_some() && job.failed.is_none());
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap()[0].size,
            24 * 1024 * 1024 + 7);
        app.vault_transfer = None;

        let out = root.join("plain-export");
        std::fs::create_dir(&out).unwrap();
        app.navigate(0, out.clone(), false);
        app.active = 1;
        app.panes[1].cursor = app.panes[1].entries.iter()
            .position(|entry| entry.name == "stream-24mb.bin").unwrap();
        app.vault_export_to_other(1, false);
        app.vault_transfer.as_mut().unwrap().started = Instant::now() - Duration::from_millis(250);
        app.vault_transfer_tick(&ctx); // stage a unique temporary plaintext export
        app.vault_transfer_tick(&ctx); // write one 8 MiB plaintext chunk
        assert_eq!(app.vault_transfer.as_ref().unwrap().stream.as_ref().unwrap().done,
            ndr::vault::STREAM_CHUNK_SIZE as u64);
        app.vault_transfer.as_mut().unwrap().cancelled = true;
        app.vault_transfer_tick(&ctx);
        assert!(!out.join("stream-24mb.bin").exists());
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0,
            "cancelling midway must remove temporary plaintext output");
        app.vault_transfer = None;

        app.vault_export_to_other(1, false);
        finish_transfer(&mut app);
        assert_eq!(std::fs::metadata(out.join("stream-24mb.bin")).unwrap().len(),
            24 * 1024 * 1024 + 7);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cross_container_and_internal_transfers_each_play_one_completion_sound() {
        use super::super::tools_dialog::PLAYED;
        use super::super::vault_transfer_ui::VaultTransferKind;
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-cross-sound", "en");
        let left = root.join("left/audio-source.ndr");
        let right = root.join("right/audio-dest.ndr");
        let mut source = Vault::create(&left, "sound password left".into()).unwrap();
        source.add_file(1, "copy.txt", b"copy payload").unwrap();
        source.add_file(1, "move.txt", b"move payload").unwrap();
        let folder = source.add_folder(1, "subfolder").unwrap();
        let target = Vault::create(&right, "sound password right".into()).unwrap();
        app.attach_vault(0, PaneVault::new(left, source));
        app.attach_vault(1, PaneVault::new(right, target));
        app.active = 0;
        app.settings.set_flag("PlayCompletionSound", true);
        let expected = Some(app.resources.join("complete.wav"));
        PLAYED.with(|record| record.borrow_mut().clear());

        app.panes[0].cursor = app.panes[0].entries.iter()
            .position(|entry| entry.name == "copy.txt").unwrap();
        app.dispatch("copyToOther", &ctx);
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        finish_transfer(&mut app);
        assert_eq!(PLAYED.with(|record| record.borrow().len()), 1);

        app.panes[0].cursor = app.panes[0].entries.iter()
            .position(|entry| entry.name == "move.txt").unwrap();
        app.dispatch("moveToOther", &ctx);
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        finish_transfer(&mut app);
        assert!(app.panes[0].vault.as_mut().unwrap().database.list(1).unwrap()
            .iter().all(|entry| entry.name != "move.txt"));
        assert_eq!(PLAYED.with(|record| record.borrow().len()), 2);

        app.begin_vault_transfer(VaultTransferKind::Inside, 0, 0, false,
            vec!["copy.txt".into()], vec![], Some(folder));
        finish_transfer(&mut app);
        assert_eq!(PLAYED.with(|record| record.borrow().len()), 3);

        // Internal MOVE now encounters an occupied target. Skipping it is
        // deliberately silent and retains the original in the source folder.
        app.begin_vault_transfer(VaultTransferKind::Inside, 0, 0, true,
            vec!["copy.txt".into()], vec![], Some(folder));
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(
            Some((ConflictChoice::Skip, false))), &ctx);
        app.vault_transfer_tick(&ctx);
        assert_eq!(PLAYED.with(|record| record.borrow().len()), 3);
        app.vault_transfer = None;
        assert_eq!(PLAYED.with(|record| record.borrow().clone()),
            vec![expected.clone(), expected.clone(), expected]);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn finish_transfer(app: &mut App) {
        let ctx = app.egui_ctx.clone();
        if let Some(job) = app.vault_transfer.as_mut() {
            job.started = Instant::now() - Duration::from_millis(250);
        }
        for _ in 0..32 {
            app.vault_transfer_tick(&ctx);
            let Some(job) = &app.vault_transfer else { return; };
            if job.finished.is_some() {
                assert!(job.failed.is_none(), "vault transfer failed: {:?}", job.failed);
                assert!(!job.cancelled, "vault transfer was cancelled");
                app.vault_transfer = None;
                return;
            }
            assert!(job.waiting_conflict.is_none() && job.waiting_rename.is_none(),
                "unexpected conflict during the normal vault transfer test");
        }
        panic!("vault transfer did not finish");
    }

    fn advance_to_conflict(app: &mut App, ctx: &egui::Context) {
        app.vault_transfer.as_mut().expect("progress job").started = Instant::now() - Duration::from_millis(250);
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.vault_transfer_progress_ui(ctx));
        app.vault_transfer_tick(ctx);
        assert!(app.original_dialogs.last().is_some_and(|dialog| matches!(dialog.kind,
            super::super::original_dialogs::DialogKind::FileConflict(_))),
            "encrypted copy must show the existing file-conflict dialog");
    }

    #[test]
    fn cross_vault_duplicate_dialog_skip_overwrite_rename_cancel_and_move() {
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-conflict-cross", "ko");
        let left = root.join("left/source.ndr");
        let right = root.join("right/destination.ndr");
        let mut a = Vault::create(&left, "left password 123456".into()).unwrap();
        a.add_file(1, "same.txt", b"new encrypted version").unwrap();
        let mut b = Vault::create(&right, "right password 123456".into()).unwrap();
        b.add_file(1, "same.txt", b"old encrypted version").unwrap();
        app.attach_vault(0, PaneVault::new(left, a));
        app.attach_vault(1, PaneVault::new(right, b));
        app.active = 0;
        app.panes[0].cursor = app.panes[0].entries.iter().position(|entry| entry.name == "same.txt").unwrap();
        let target_bytes = |app: &mut App, name: &str| {
            let target = app.panes[1].vault.as_mut().unwrap();
            let id = target.database.list(1).unwrap().into_iter().find(|entry| entry.name == name).unwrap().id;
            target.database.read_file(id).unwrap()
        };

        app.dispatch("copyToOther", &ctx);
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        advance_to_conflict(&mut app, &ctx);
        let form = match &app.original_dialogs.last().unwrap().kind {
            super::super::original_dialogs::DialogKind::FileConflict(form) => form,
            _ => unreachable!(),
        };
        assert_eq!((form.source_size, form.destination_size), (Some(21), Some(21)));
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(Some((ConflictChoice::Skip, false))), &ctx);
        app.vault_transfer_tick(&ctx);
        assert_eq!(target_bytes(&mut app, "same.txt"), b"old encrypted version");
        app.vault_transfer = None;

        app.dispatch("copyToOther", &ctx);
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(Some((ConflictChoice::Overwrite, false))), &ctx);
        finish_transfer(&mut app);
        assert_eq!(target_bytes(&mut app, "same.txt"), b"new encrypted version");

        app.dispatch("copyToOther", &ctx);
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(Some((ConflictChoice::Rename, false))), &ctx);
        app.vault_transfer_tick(&ctx);
        assert!(app.original_dialogs.last().is_some_and(|dialog| matches!(dialog.kind,
            super::super::original_dialogs::DialogKind::Prompt { .. })));
        app.answer_dialog(super::super::original_dialogs::DialogResult::Text(Some("renamed.txt".into())), &ctx);
        finish_transfer(&mut app);
        assert_eq!(target_bytes(&mut app, "renamed.txt"), b"new encrypted version");

        app.dispatch("moveToOther", &ctx);
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(Some((ConflictChoice::Cancel, false))), &ctx);
        app.vault_transfer_tick(&ctx);
        assert!(app.panes[0].vault.as_mut().unwrap().database.list(1).unwrap().iter().any(|entry| entry.name == "same.txt"));
        app.vault_transfer = None;

        app.dispatch("moveToOther", &ctx);
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(Some((ConflictChoice::Overwrite, false))), &ctx);
        finish_transfer(&mut app);
        assert!(app.panes[0].vault.as_mut().unwrap().database.list(1).unwrap().is_empty());
        assert_eq!(target_bytes(&mut app, "same.txt"), b"new encrypted version");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn regular_to_vault_and_vault_to_regular_both_show_duplicate_dialog() {
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-conflict-regular", "ko");
        let file = root.join("right/safe.ndr");
        let mut vault = Vault::create(&file, "strong test password".into()).unwrap();
        vault.add_file(1, "alpha.txt", b"older secret").unwrap();
        vault.add_file(1, "export.txt", b"new plaintext contents").unwrap();
        app.attach_vault(1, PaneVault::new(file, vault));
        app.active = 0;
        app.panes[0].cursor = app.panes[0].entries.iter().position(|e| e.name == "alpha.txt").unwrap();
        app.dispatch("copyToOther", &ctx);
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(Some((ConflictChoice::Skip, false))), &ctx);
        app.vault_transfer_tick(&ctx);
        let items = app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap();
        let id = items.iter().find(|e| e.name == "alpha.txt").unwrap().id;
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.read_file(id).unwrap(), b"older secret");
        app.vault_transfer = None;

        app.dispatch("copyToOther", &ctx);
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(Some((ConflictChoice::Overwrite, false))), &ctx);
        finish_transfer(&mut app);
        let items = app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap();
        let id = items.iter().find(|e| e.name == "alpha.txt").unwrap().id;
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.read_file(id).unwrap(), b"alpha.txt");

        std::fs::write(root.join("left/export.txt"), b"existing plaintext").unwrap();
        app.refresh(0);
        app.active = 1;
        app.panes[1].cursor = app.panes[1].entries.iter().position(|e| e.name == "export.txt").unwrap();
        app.dispatch("copyToOther", &ctx);
        app.answer_message_box(ndr::msgbox::IDYES, &ctx);
        advance_to_conflict(&mut app, &ctx);
        app.answer_dialog(super::super::original_dialogs::DialogResult::FileConflict(Some((ConflictChoice::Overwrite, false))), &ctx);
        finish_transfer(&mut app);
        assert_eq!(std::fs::read(root.join("left/export.txt")).unwrap(), b"new plaintext contents");
        assert!(app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap().iter().any(|item| item.name == "export.txt"));
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn encrypted_dialog_strings_exist_in_every_bundled_language() {
        let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Resources/Languages");
        let languages = ["ca", "cs", "de", "en", "es", "fi", "fr", "hu", "it", "ja", "ko", "nl", "pl", "pt-BR", "ru", "sk", "sv", "tr", "uk", "zh-CN", "zh-TW"];
        let keyset: Vec<String> = std::fs::read_to_string(resources.join("en.lang")).unwrap()
            .split("[NDRVault]").nth(1).unwrap().lines()
            .filter_map(|line| line.split_once('=').map(|(key, _)| key.to_owned())).collect();
        for code in languages {
            let data = std::fs::read_to_string(resources.join(format!("{code}.lang"))).unwrap();
            let section = data.split("[NDRVault]").nth(1).unwrap();
            for key in &keyset {
                assert!(section.lines().any(|line| line.starts_with(&format!("{key}=")) && line.len() > key.len() + 1),
                    "missing NDRVault.{key} in {code}");
            }
            let language = compat::Language::load(&resources, code);
            assert_eq!(language.code, code);
            assert!(!language.text("NDRVault.Password", "").is_empty());
        }
        let korean = compat::Language::load(&resources, "ko");
        assert_eq!(korean.text("NDRVault.Password", ""), "비밀번호");
    }

    #[test]
    fn unlock_dialog_has_password_focus_and_compact_content() {
        let ctx = egui::Context::default();
        let mut dialog = VaultDialog::new(PathBuf::from("C:\\test\\secret.ndr"), false);
        let lang = compat::Language::default();
        let mut width = 0.0;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let before = ui.cursor().min;
                dialog.draw_unlock(ui, &lang);
                width = (ui.min_rect().right() - before.x).max(0.0);
            });
        });
        assert!(!dialog.name_select_pending, "password focus is requested on first draw");
        assert!(width >= 444.0, "unlock matches the 444px New Folder dialog content");
    }

    #[test]
    fn cross_vault_f3_f4_copy_move_and_cancel_are_safe() {
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-cross-panes", "en");
        let left = root.join("left/left.ndr");
        let right = root.join("right/right.ndr");
        let mut a = Vault::create(&left, "left secure password".into()).unwrap();
        let folder = a.add_folder(1, "documents").unwrap();
        a.add_file(folder, "nested.txt", b"encrypted contents").unwrap();
        a.add_file(1, "moving.txt", b"moving safely").unwrap();
        let b = Vault::create(&right, "right secure password".into()).unwrap();
        app.attach_vault(0, PaneVault::new(left, a));
        app.attach_vault(1, PaneVault::new(right, b));
        app.active = 0;
        app.panes[0].cursor = app.panes[0].entries.iter().position(|i| i.name == "documents").unwrap();
        app.dispatch("copyToOther", &ctx);
        assert!(app.msgboxes.is_empty(), "cross-vault F3 needs no security confirmation");
        finish_transfer(&mut app);
        let copied = app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap();
        let folder_id = copied.iter().find(|v| v.name == "documents").unwrap().id;
        let file_id = app.panes[1].vault.as_mut().unwrap().database.list(folder_id).unwrap()[0].id;
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.read_file(file_id).unwrap(), b"encrypted contents");
        assert!(app.panes[0].vault.as_mut().unwrap().database.list(1).unwrap().iter().any(|v| v.name == "documents"));
        app.panes[0].cursor = app.panes[0].entries.iter().position(|i| i.name == "moving.txt").unwrap();
        app.dispatch("moveToOther", &ctx);
        assert!(app.msgboxes.is_empty(), "cross-vault F4 needs no security confirmation");
        assert!(app.panes[0].vault.as_mut().unwrap().database.list(1).unwrap().iter().any(|v| v.name == "moving.txt"));
        finish_transfer(&mut app);
        assert!(!app.panes[0].vault.as_mut().unwrap().database.list(1).unwrap().iter().any(|v| v.name == "moving.txt"));
        assert!(app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap().iter().any(|v| v.name == "moving.txt"));
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn f4_moves_local_file_into_vault_only_after_encrypted_commit() {
        let (root, _ctx, mut app) = super::super::tools_tests::fixture("vault-move-in", "ko");
        let container = root.join("right/moving.ndr");
        let database = Vault::create(&container, "strong password 123456".into()).unwrap();
        app.attach_vault(1, PaneVault::new(container, database));
        let source = root.join("left/alpha.txt");
        assert!(source.is_file());
        app.active = 0;
        app.vault_transfer_from_local(0, 1, vec![source.clone()], true);
        assert!(app.msgboxes.is_empty(), "F4 must not prompt for import");
        assert!(source.is_file(), "move must not delete source before encrypted commit");
        finish_transfer(&mut app);
        assert!(!source.exists());
        let items = app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap();
        let id = items.iter().find(|i| i.name == "alpha.txt").unwrap().id;
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.read_file(id).unwrap(), b"alpha.txt");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn f4_moves_vault_file_to_local_only_after_plaintext_commit() {
        let (root, _ctx, mut app) = super::super::tools_tests::fixture("vault-move-out", "ko");
        let container = root.join("right/moving.ndr");
        let mut database = Vault::create(&container, "strong password 123456".into()).unwrap();
        database.add_file(1, "private.txt", b"original secret").unwrap();
        app.attach_vault(1, PaneVault::new(container, database));
        let destination = app.panes[0].path.clone();
        app.active = 1;
        let index = app.panes[1].entries.iter().position(|i| i.name == "private.txt").unwrap();
        app.panes[1].cursor = index;
        app.vault_export_to_other(1, true);
        assert!(app.msgboxes.is_empty(), "F4 move requires no security prompt");
        assert!(!destination.join("private.txt").exists(),
            "the source is not deleted or copied synchronously");
        finish_transfer(&mut app);
        assert_eq!(std::fs::read(destination.join("private.txt")).unwrap(), b"original secret");
        assert!(app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap().is_empty());
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn elapsed_session_is_reused_after_leaving_and_reentering_without_password() {
        let (root, _ctx, mut app) = super::super::tools_tests::fixture("vault-session-ten", "ko");
        let file = root.join("left/stay.ndr");
        let database = Vault::create(&file, "passphrase 123456".into()).unwrap();
        assert_eq!(SessionPolicy::from_settings(&app.settings),
            SessionPolicy::Elapsed(std::time::Duration::from_secs(600)));
        app.attach_vault(0, PaneVault::new(file.clone(), database));
        app.active = 0;
        app.parent();
        assert!(app.panes[0].vault.is_none());
        assert_eq!(app.vault_sessions.len(), 1, "leaving the folder caches authenticated SQLCipher");
        app.open_vault(file.clone(), false);
        assert!(app.vault_dialog.is_none(), "reentry must skip the password dialog");
        assert!(app.panes[0].vault.is_some());
        assert!(app.vault_sessions.is_empty(), "a reused session is now owned by the pane");
        app.parent();
        assert_eq!(app.vault_sessions.len(), 1);
        app.vault_sessions.clear();
        app.open_vault(file, false);
        assert!(app.vault_dialog.is_some(), "after the cached session is invalidated a password is required");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_until_exit_keeps_unlock_across_several_visits() {
        let (root, _ctx, mut app) = super::super::tools_tests::fixture("vault-session-exit", "ko");
        app.active = 1;
        app.settings.values.insert("VaultSessionPolicy".into(), serde_json::json!("exit"));
        assert_eq!(SessionPolicy::from_settings(&app.settings), SessionPolicy::UntilExit);
        let file = root.join("right/remember.ndr");
        let database = Vault::create(&file, "passphrase 123456".into()).unwrap();
        app.attach_vault(1, PaneVault::new(file.clone(), database));
        for _ in 0..3 {
            app.discard_vault(1);
            assert_eq!(app.vault_sessions.len(), 1);
            app.open_vault(file.clone(), false);
            assert!(app.vault_dialog.is_none());
            assert!(app.panes[1].vault.is_some());
        }
        app.discard_vault(1);
        app.vault_sessions.clear(); // Shutdown removes all retained auth.
        app.open_vault(file, false);
        assert!(app.vault_dialog.is_some());
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn f3_copies_regular_files_and_folders_into_vault_without_warning() {
        let (root, ctx, mut app) = super::super::tools_tests::fixture("vault-f5", "ko");
        let vault_file = root.join("right/safe.ndr");
        let database = Vault::create(&vault_file, "strong password 123456".into()).unwrap();
        app.attach_vault(1, PaneVault::new(vault_file.clone(), database));

        app.active = 0;
        let file_index = app.panes[0].entries.iter().position(|entry| entry.name == "alpha.txt").unwrap();
        app.panes[0].cursor = file_index;
        app.dispatch("copyToOther", &ctx);
        finish_transfer(&mut app);
        let listing = app.panes[1].vault.as_mut().unwrap().database.list(1).unwrap();
        let imported = listing.iter().find(|item| item.name == "alpha.txt").unwrap();
        assert_eq!(app.panes[1].vault.as_mut().unwrap().database.read_file(imported.id).unwrap(), b"alpha.txt");

        let folder_index = app.panes[0].entries.iter().position(|entry| entry.name == "folder1").unwrap();
        app.panes[0].cursor = folder_index;
        app.dispatch("copyToOther", &ctx);
        finish_transfer(&mut app);
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
        finish_transfer(&mut app);
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
        assert!(app.msgboxes.is_empty(), "F3 is the user's explicit copy request");
        finish_transfer(&mut app);
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
        assert!(app.panes[0].vault.is_none(), "leaving the container clears the pane view");
        assert_eq!(app.vault_sessions.len(), 1, "a valid login persists across navigation");
        assert!(file.is_file());
        assert!(!parent.join("스테고사우루스").exists(), "no plaintext folder was created");
        app.vault_sessions.clear();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn random_dinosaur_name_starts_focused_and_fully_selected() {
        let ctx = egui::Context::default();
        let mut dialog = VaultDialog::new(PathBuf::from("C:\\temp\\new.ndr"), true);
        let length = dialog.folder_name.chars().count();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                dialog.draw_create(ui, &compat::Language::default());
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
        // An exclusive authenticated session must end before testing a second unlock.
        drop(dialog);
        assert!(Vault::unlock(&root.join("비밀 문서.ndr"), "wrong password 123456".into()).is_err());
        assert!(Vault::unlock(&root.join("비밀 문서.ndr"), "long secure passphrase 12345!".into()).is_ok());
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
        Self { path, creating, folder_name, name_select_pending: true, created_path: None, password: String::new(), confirmation: String::new(), vault: None,
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

    fn localized_error(lang: &compat::Language, message: &str) -> String {
        let key = match message {
            "올바른 폴더 이름을 입력해 주세요." => "InvalidFolderName",
            "Windows에서 사용할 수 없는 폴더 이름입니다." => "ReservedFolderName",
            "비밀번호가 일치하지 않습니다." => "PasswordMismatch",
            "Wrong password, damaged container, or unsupported format" => "WrongPassword",
            "Unsupported encrypted container format" => "WrongFormat",
            "Unsupported encrypted container version" => "WrongVersion",
            "Encrypted container file does not exist" => "MissingContainer",
            "Password must contain at least 6 characters" => "ShortPassword",
            _ => return message.to_owned(),
        };
        lang.text(&format!("NDRVault.{key}"), message)
    }

    /// Same centered movable VCL-style dialog and 444px content width as
    /// New Folder / TfGetText. Only the additional password fields make it
    /// taller. No separate heading, grid, or native egui::Window chrome.
    fn draw_create(&mut self, ui: &mut egui::Ui, lang: &compat::Language) -> bool {
        let (area, _) = ui.allocate_exact_size(egui::vec2(444.0, 250.0), egui::Sense::hover());
        let at = |x: f32, y: f32, width: f32, height: f32| {
            egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(width, height))
        };

        ui.put(at(18.0, 8.0, 411.0, 25.0), egui::Label::new(lang.text("NDRVault.CreateHint", "Enter the name of the new encrypted folder.")));
        ui.put(at(18.0, 47.0, 100.0, 21.0), egui::Label::new(lang.text("NDRVault.FolderName", "Folder name")));
        let response = ui.put(at(122.0, 43.0, 307.0, 23.0),
            egui::TextEdit::singleline(&mut self.folder_name)
                .id(egui::Id::new("ndr-vault-folder-name")));
        if self.name_select_pending {
            response.request_focus();
            super::original_dialogs::select_all(ui, response.id, self.folder_name.chars().count());
            self.name_select_pending = false;
        }
        ui.put(at(18.0, 80.0, 100.0, 21.0), egui::Label::new(lang.text("NDRVault.Password", "Password")));
        ui.put(at(122.0, 76.0, 307.0, 23.0),
            egui::TextEdit::singleline(&mut self.password).password(true));
        ui.put(at(18.0, 113.0, 100.0, 21.0), egui::Label::new(lang.text("NDRVault.ConfirmPassword", "Confirm password")));
        ui.put(at(122.0, 109.0, 307.0, 23.0),
            egui::TextEdit::singleline(&mut self.confirmation).password(true));

        ui.put(at(18.0, 145.0, 411.0, 19.0), egui::Label::new(
            egui::RichText::new(lang.text("NDRVault.CreateNote", "Names and files are encrypted. Password: at least 6 characters.")).small().weak()));
        ui.put(at(18.0, 165.0, 411.0, 19.0), egui::Label::new(
            egui::RichText::new(lang.text("NDRVault.NoRecovery", "A forgotten password cannot be recovered.")).small().weak()));

        // Align exactly with the bottom-right controls of New Folder:
        // x=265 / 351, width=79, height=25; only Y moves for extra fields.
        let mut close = false;
        if ui.put(at(265.0, 215.0, 79.0, 25.0), egui::Button::new(lang.text("NDRVault.OK", "OK"))).clicked() {
            self.authenticate();
        }
        if ui.put(at(351.0, 215.0, 79.0, 25.0), egui::Button::new(lang.text("NDRVault.Cancel", "Cancel"))).clicked() {
            close = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) { close = true; }
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) && !close && self.created_path.is_none() {
            self.authenticate();
        }
        if !self.error.is_empty() {
            ui.painter().text(at(18.0, 189.0, 411.0, 18.0).left_top(),
                egui::Align2::LEFT_TOP, Self::localized_error(lang, &self.error),
                egui::FontId::proportional(11.0), ui.visuals().error_fg_color);
        }
        close || self.created_path.is_some()
    }

    /// Matches the centered movable original-dialog chrome and 444 px field
    /// alignment used by New Folder and encrypted-folder creation.
    fn draw_unlock(&mut self, ui: &mut egui::Ui, lang: &compat::Language) -> bool {
        let (area, _) = ui.allocate_exact_size(egui::vec2(444.0, 178.0), egui::Sense::hover());
        let at = |x: f32, y: f32, w: f32, h: f32| {
            egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h))
        };
        let display_name = self.path.file_stem().unwrap_or_default().to_string_lossy();
        ui.put(at(18.0, 8.0, 411.0, 24.0), egui::Label::new(display_name.as_ref()).truncate())
            .on_hover_text(display_name.as_ref());
        ui.put(at(18.0, 51.0, 100.0, 23.0),
            egui::Label::new(lang.text("NDRVault.Password", "Password")));
        let edit = ui.put(at(122.0, 47.0, 307.0, 23.0),
            egui::TextEdit::singleline(&mut self.password).password(true)
                .id(egui::Id::new("ndr-vault-unlock-password")));
        if self.name_select_pending {
            edit.request_focus();
            self.name_select_pending = false;
        }
        ui.put(at(18.0, 79.0, 411.0, 22.0), egui::Label::new(
            egui::RichText::new(lang.text("NDRVault.PasswordNotStored", "The password is not saved.")).small().weak()));
        if !self.error.is_empty() {
            ui.put(at(18.0, 106.0, 411.0, 35.0), egui::Label::new(
                egui::RichText::new(Self::localized_error(lang, &self.error)).small().color(ui.visuals().error_fg_color)));
        }
        let mut close = false;
        let submit = ui.put(at(265.0, 145.0, 79.0, 25.0),
            egui::Button::new(lang.text("NDRVault.OK", "OK"))).clicked();
        if ui.put(at(351.0, 145.0, 79.0, 25.0),
            egui::Button::new(lang.text("NDRVault.Cancel", "Cancel"))).clicked() {
            close = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) { close = true; }
        if !close && (submit || ui.input(|input| input.key_pressed(egui::Key::Enter))) {
            self.authenticate();
        }
        close
    }

}

impl App {
    pub(super) fn open_vault(&mut self, path: PathBuf, creating: bool) {
        if !creating {
            let key = std::fs::canonicalize(&path).unwrap_or(path.clone());
            if let Some(mut database) = self.vault_sessions.remove(&key) {
                if !database.expired(SessionPolicy::from_settings(&self.settings)) {
                    match database.reload_after_mount() {
                        Ok(()) => {
                            self.vault_dialog = None;
                            self.attach_vault(self.active, PaneVault::new(path, database));
                            return;
                        }
                        Err(error) => eprintln!("NDR cached encrypted session refresh failed: {error}"),
                    }
                }
            }
        }
        self.vault_dialog = Some(VaultDialog::new(path, creating));
    }

    pub(super) fn vault_window_ui(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.vault_dialog.take() else { return };
        if self.settings.flag("VaultLockOnMinimize", false)
            && ctx.input(|input| input.viewport().minimized.unwrap_or(false)) {
            dialog.lock();
        }
        let mut close = false;
        if dialog.creating {
            // The ordinary New Folder prompt uses the same modal and title
            // helpers. This places both dialogs at the same owner-window
            // center and gives them matching chrome and title dragging.
            let id = egui::Id::new("ndr-vault-create-modal");
            super::keep_opaque(ctx, id);
            super::movable_modal(ctx, id).show(ctx, |ui| {
                let title = super::modal_title_start(ui, &self.language_pack.text("NDRVault.CreateTitle", "New encrypted folder"));
                close = dialog.draw_create(ui, &self.language_pack);
                close |= super::modal_title_finish(ui, id, title);
            });
        } else {
            let id = egui::Id::new("ndr-vault-unlock-modal");
            super::keep_opaque(ctx, id);
            super::movable_modal(ctx, id).show(ctx, |ui| {
                let title = super::modal_title_start(ui, &self.language_pack.text("NDRVault.UnlockTitle", "Unlock encrypted folder"));
                close = dialog.draw_unlock(ui, &self.language_pack);
                close |= super::modal_title_finish(ui, id, title);
            });
        }
        if let Some(database) = dialog.vault.take() {
            self.attach_vault(self.active, PaneVault::new(dialog.path.clone(), database));
            return;
        }
        if !close { self.vault_dialog = Some(dialog); }
        // Otherwise the connection and all plaintext/password UI buffers drop.
    }
}
