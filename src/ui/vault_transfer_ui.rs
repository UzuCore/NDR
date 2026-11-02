//! Interactive encrypted-folder file operations. Uses the same conflict
//! dialog as ordinary file jobs, with a separate 441px progress window.
//! The unit of progress/cancellation is one selected root file/folder.
use super::*;
use std::io::{Read, Write};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VaultTransferKind { Import, Export, Between, Inside }

/// A single file is transferred cooperatively, one 8 MiB chunk per UI frame.
/// Incomplete SQLCipher transactions are rolled back on explicit cancellation
/// or database close; temporary plaintext exports are removed on drop.
pub(super) struct VaultStreamState {
    input: Option<std::fs::File>,
    output: Option<std::fs::File>,
    temp_path: Option<PathBuf>,
    destination_path: Option<PathBuf>,
    source_id: Option<i64>,
    target_id: Option<i64>,
    ordinal: i64,
    pub(super) done: u64,
    pub(super) total: u64,
}

impl Drop for VaultStreamState {
    fn drop(&mut self) {
        self.output.take();
        if let Some(path) = self.temp_path.take() { let _ = std::fs::remove_file(path); }
    }
}

pub(super) struct VaultTransferJob {
    kind: VaultTransferKind,
    pub(super) source: usize,
    pub(super) target: usize,
    from_folder: i64,
    to_folder: i64,
    names: Vec<String>,
    source_paths: Vec<PathBuf>,
    source_container: Option<PathBuf>,
    target_container: Option<PathBuf>,
    source_display: String,
    target_display: String,
    moving: bool,
    index: usize,
    copied: usize,
    skipped: usize,
    pub(super) started: Instant,
    pub(super) finished: Option<Instant>,
    pub(super) failed: Option<String>,
    pub(super) cancelled: bool,
    pub(super) waiting_conflict: Option<Receiver<ConflictChoice>>,
    pub(super) waiting_rename: Option<Receiver<Option<String>>>,
    destination_name: Option<String>,
    replace: bool,
    pub(super) stream: Option<VaultStreamState>,
}

impl App {
    pub(super) fn begin_vault_transfer(&mut self, kind: VaultTransferKind, source: usize,
        target: usize, moving: bool, names: Vec<String>, paths: Vec<PathBuf>,
        destination_folder: Option<i64>) {
        if names.is_empty() { return; }
        if self.vault_transfer.as_ref().is_some_and(|job| job.finished.is_some()) {
            self.vault_transfer = None;
        }
        if self.vault_transfer.is_some() || self.job.as_ref().is_some_and(|job| !job.finished) {
            self.status = self.language_pack.text("NDRVault.JobBusy", "Finish the current transfer first.");
            return;
        }
        self.conflict_all = None;
        let source_vault = self.panes[source].vault.as_ref();
        let target_vault = self.panes[target].vault.as_ref();
        let source_container = source_vault.map(|v| v.file.clone());
        let target_container = target_vault.map(|v| v.file.clone());
        let mut source_display = source_vault.map(|v| v.virtual_path())
            .unwrap_or_else(|| self.panes[source].path.clone()).to_string_lossy().into_owned();
        let mut target_display = target_vault.map(|v| v.virtual_path())
            .unwrap_or_else(|| self.panes[target].path.clone()).to_string_lossy().into_owned();
        let from_folder = source_vault.map_or(1, |v| v.folder());
        let to_folder = destination_folder.unwrap_or_else(|| target_vault.map_or(1, |v| v.folder()));
        if kind == VaultTransferKind::Import {
            if let Some(parent) = paths.first().and_then(|path| path.parent()) {
                source_display = parent.to_string_lossy().into_owned();
            }
        }
        if kind == VaultTransferKind::Inside {
            if let Some(state) = target_vault {
                if let Ok(relative) = state.database.relative_folder_path(to_folder) {
                    target_display = state.file.join(relative).to_string_lossy().into_owned();
                }
            }
        }
        self.vault_transfer = Some(VaultTransferJob {
            kind, source, target, from_folder, to_folder, names, source_paths: paths,
            source_container, target_container, source_display, target_display,
            moving, index: 0, copied: 0, skipped: 0, started: Instant::now(),
            finished: None, failed: None, cancelled: false, waiting_conflict: None,
            waiting_rename: None, destination_name: None, replace: false, stream: None,
        });
        self.egui_ctx.request_repaint();
    }

    fn vault_task_target_exists(&mut self, job: &VaultTransferJob,
        name: &str) -> Result<Option<ndr::vault::VaultItem>, String> {
        if job.kind == VaultTransferKind::Export {
            let path = PathBuf::from(&job.target_display).join(name);
            return match std::fs::symlink_metadata(&path) {
                Ok(meta) => Ok(Some(ndr::vault::VaultItem {
                    id: 0, name: name.into(), folder: meta.is_dir(), size: meta.len() as i64,
                })),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.to_string()),
            };
        }
        let pane = self.panes[job.target].vault.as_mut().ok_or("Destination encrypted container closed")?;
        Ok(pane.database.list(job.to_folder)?.into_iter().find(|e| e.name.eq_ignore_ascii_case(name)))
    }

    fn vault_task_source_info(&mut self, job: &VaultTransferJob) -> Option<(bool, u64)> {
        let name = job.names.get(job.index)?;
        if job.kind == VaultTransferKind::Import {
            let meta = std::fs::symlink_metadata(job.source_paths.get(job.index)?).ok()?;
            return Some((meta.is_dir(), meta.len()));
        }
        self.panes[job.source].vault.as_mut()?.database.list(job.from_folder).ok()?
            .into_iter().find(|e| &e.name == name).map(|e| (e.folder, e.size.max(0) as u64))
    }

    fn vault_task_paths(job: &VaultTransferJob, name: &str) -> (PathBuf, PathBuf) {
        let source = if job.kind == VaultTransferKind::Import {
            job.source_paths.get(job.index).cloned().unwrap_or_default()
        } else { PathBuf::from(&job.source_display).join(&job.names[job.index]) };
        (source, PathBuf::from(&job.target_display).join(name))
    }

    fn vault_task_finish(&mut self, job: &mut VaultTransferJob) {
        // Match ordinary NDR transfers: one cue for a fully successful batch,
        // never for cancellation, failed work, or skipped-only operations.
        // This is called once per terminal transition, not per 8 MiB chunk.
        if job.failed.is_none() && !job.cancelled && job.skipped == 0 && job.copied > 0 {
            self.play_operation_completion_sound();
        }
        job.finished = Some(Instant::now());
        self.conflict_all = None;
        let summary = self.language_pack.text("NDRVault.ProgressSummary",
            "Completed: {done}; skipped: {skipped}; total: {total}")
            .replace("{done}", &job.index.saturating_sub(job.skipped).to_string())
            .replace("{skipped}", &job.skipped.to_string())
            .replace("{total}", &job.names.len().to_string());
        let label = if let Some(error) = &job.failed {
            format!("{}: {error}", self.language_pack.text("NDRVault.JobFailed", "Encrypted transfer interrupted"))
        } else if job.cancelled {
            self.language_pack.text("NDRVault.JobCancelled", "Encrypted transfer cancelled; completed items retained")
        } else { self.language_pack.text("NDRVault.JobDone", "Encrypted transfer completed") };
        self.status = format!("{label}. {summary}");
        self.egui_ctx.request_repaint_after(Duration::from_millis(650));
    }

    fn vault_task_changed(&self, job: &VaultTransferJob) -> bool {
        for (pane, expected) in [(job.source, job.source_container.as_ref()), (job.target, job.target_container.as_ref())] {
            if let Some(expected) = expected {
                let Some(actual) = self.panes[pane].vault.as_ref() else { return true; };
                match (std::fs::canonicalize(&actual.file), std::fs::canonicalize(expected)) {
                    (Ok(a), Ok(b)) if a == b => (),
                    _ => return true,
                }
                if pane == job.source && job.kind != VaultTransferKind::Import && actual.folder() != job.from_folder {
                    return true;
                }
                if pane == job.target && job.kind != VaultTransferKind::Inside && actual.folder() != job.to_folder {
                    return true;
                }
            }
        }
        false
    }
}

impl App {
    pub(super) fn vault_transfer_tick(&mut self, ctx: &egui::Context) {
        let Some(mut job) = self.vault_transfer.take() else { return; };
        if let Some(ended) = job.finished {
            if ended.elapsed() < Duration::from_millis(650) {
                self.vault_transfer = Some(job);
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            return;
        }
        if self.vault_task_changed(&job) { job.cancelled = true; }
        if job.cancelled {
            if let Some(stream) = job.stream.take() { self.vault_task_abort_stream(&job, stream); }
            self.vault_task_finish(&mut job);
            self.vault_transfer = Some(job);
            return;
        }
        if let Some(mut stream) = job.stream.take() {
            match self.vault_task_stream_step(&job, &mut stream) {
                Ok(false) => {
                    job.stream = Some(stream);
                    self.vault_transfer = Some(job);
                    ctx.request_repaint();
                    return;
                }
                Ok(true) => {
                    job.copied += 1;
                    job.index += 1;
                    job.destination_name = None;
                    job.replace = false;
                    self.vault_task_refresh_after_item(&job);
                }
                Err(error) => {
                    self.vault_task_abort_stream(&job, stream);
                    job.failed = Some(error);
                    self.vault_task_refresh_after_item(&job);
                }
            }
            if job.failed.is_some() || job.index >= job.names.len() { self.vault_task_finish(&mut job); }
            self.vault_transfer = Some(job);
            ctx.request_repaint();
            return;
        }
        if job.cancelled || job.index >= job.names.len() {
            self.vault_task_finish(&mut job);
            self.vault_transfer = Some(job);
            return;
        }
        if let Some(receiver) = job.waiting_conflict.as_ref() {
            let answer = match receiver.try_recv() {
                Ok(answer) => answer,
                Err(std::sync::mpsc::TryRecvError::Empty) => { self.vault_transfer = Some(job); return; },
                Err(std::sync::mpsc::TryRecvError::Disconnected) => ConflictChoice::Cancel,
            };
            job.waiting_conflict = None;
            match answer {
                ConflictChoice::Cancel => job.cancelled = true,
                ConflictChoice::Skip | ConflictChoice::OverwriteNewer | ConflictChoice::Resume => {
                    // Vault items lack comparable timestamps. Overwrite-newer
                    // conservatively skips instead of replacing unknown data.
                    job.skipped += 1;
                    job.index += 1;
                    job.destination_name = None;
                    job.replace = false;
                }
                ConflictChoice::Rename => {
                    let (sender, receiver) = mpsc::channel();
                    self.ask_rename_target(job.names[job.index].clone(), sender);
                    job.waiting_rename = Some(receiver);
                }
                ConflictChoice::Overwrite | ConflictChoice::ReplaceFolder => job.replace = true,
            }
        }
        if let Some(receiver) = job.waiting_rename.as_ref() {
            let name = match receiver.try_recv() {
                Ok(name) => name,
                Err(std::sync::mpsc::TryRecvError::Empty) => { self.vault_transfer = Some(job); return; },
                Err(std::sync::mpsc::TryRecvError::Disconnected) => None,
            };
            job.waiting_rename = None;
            match name {
                Some(name) if !name.trim().is_empty() => {
                    job.destination_name = Some(name);
                    job.replace = false;
                }
                _ => job.cancelled = true,
            }
        }
        if job.cancelled || job.index >= job.names.len() {
            self.vault_task_finish(&mut job);
            self.vault_transfer = Some(job);
            return;
        }
        // The first frame must show progress even for one small file.
        if job.started.elapsed() < Duration::from_millis(90) {
            self.vault_transfer = Some(job);
            ctx.request_repaint_after(Duration::from_millis(35));
            return;
        }
        let target_name = job.destination_name.clone().unwrap_or_else(|| job.names[job.index].clone());
        let occupied = match self.vault_task_target_exists(&job, &target_name) {
            Ok(occupied) => occupied,
            Err(error) => {
                job.failed = Some(error);
                self.vault_task_finish(&mut job);
                self.vault_transfer = Some(job);
                return;
            }
        };
        if let Some(item) = occupied.filter(|_| !job.replace) {
            let (source, target) = Self::vault_task_paths(&job, &target_name);
            if job.kind != VaultTransferKind::Import {
                if let Some((folder, size)) = self.vault_task_source_info(&job) {
                    super::file_operation::set_virtual_entry(&source, Some((folder, (!folder).then_some(size), None)));
                }
            }
            if job.kind != VaultTransferKind::Export {
                super::file_operation::set_virtual_entry(&target,
                    Some((item.folder, (!item.folder).then_some(item.size.max(0) as u64), None)));
            }
            let (sender, receiver) = mpsc::channel();
            self.ask_file_conflict(source.clone(), target.clone(), false, sender);
            super::file_operation::set_virtual_entry(&source, None);
            super::file_operation::set_virtual_entry(&target, None);
            job.waiting_conflict = Some(receiver);
            self.vault_transfer = Some(job);
            ctx.request_repaint();
            return;
        }
        let original = job.names[job.index].clone();
        match self.vault_task_start_stream(&job, &original, &target_name) {
            Ok(Some(stream)) => {
                job.stream = Some(stream);
                self.vault_transfer = Some(job);
                ctx.request_repaint();
                return;
            }
            Ok(None) => (), // Recursive folders still use the tree operation.
            Err(error) => {
                job.failed = Some(error);
                self.vault_task_finish(&mut job);
                self.vault_transfer = Some(job);
                return;
            }
        }
        match self.vault_task_copy_item(&job, &original, &target_name) {
            Ok(count) => {
                job.copied += count;
                job.index += 1;
                job.destination_name = None;
                job.replace = false;
                match job.kind {
                    VaultTransferKind::Import => {
                        self.refresh_vault(job.target);
                        if job.moving { self.refresh(job.source); }
                    }
                    VaultTransferKind::Export => {
                        self.refresh(job.target);
                        if job.moving { self.refresh_vault(job.source); }
                    }
                    VaultTransferKind::Between => {
                        self.refresh_vault(job.target);
                        if job.moving { self.refresh_vault(job.source); }
                    }
                    VaultTransferKind::Inside => self.refresh_vault(job.source),
                }
            }
            Err(error) => {
                // A destination may already have committed before a MOVE's
                // source deletion failed. Refresh both views so the retained
                // duplicate is visible to the user immediately.
                job.failed = Some(error);
                match job.kind {
                    VaultTransferKind::Import => { self.refresh_vault(job.target); self.refresh(job.source); }
                    VaultTransferKind::Export => { self.refresh(job.target); self.refresh_vault(job.source); }
                    VaultTransferKind::Between => { self.refresh_vault(job.source); self.refresh_vault(job.target); }
                    VaultTransferKind::Inside => self.refresh_vault(job.source),
                }
            }
        }
        if job.failed.is_some() || job.index >= job.names.len() { self.vault_task_finish(&mut job); }
        self.vault_transfer = Some(job);
        ctx.request_repaint();
    }

    fn vault_task_refresh_after_item(&mut self, job: &VaultTransferJob) {
        match job.kind {
            VaultTransferKind::Import => { self.refresh_vault(job.target); if job.moving { self.refresh(job.source); } }
            VaultTransferKind::Export => { self.refresh(job.target); if job.moving { self.refresh_vault(job.source); } }
            VaultTransferKind::Between => { self.refresh_vault(job.target); if job.moving { self.refresh_vault(job.source); } }
            VaultTransferKind::Inside => self.refresh_vault(job.source),
        }
    }

    fn vault_task_copy_item(&mut self, job: &VaultTransferJob, original: &str, name: &str) -> Result<usize, String> {
        let count = match job.kind {
            VaultTransferKind::Import => {
                let state = self.panes[job.target].vault.as_mut().ok_or("Encrypted destination closed")?;
                state.database.import_one(job.to_folder, &job.source_paths[job.index], name, job.replace)?
            }
            VaultTransferKind::Export => {
                let state = self.panes[job.source].vault.as_mut().ok_or("Encrypted source closed")?;
                state.database.export_one(job.from_folder, original, &PathBuf::from(&job.target_display), name, job.replace)?
            }
            VaultTransferKind::Between => {
                let (left, right) = self.panes.split_at_mut(1);
                let (source, target) = if job.source == 0 { (&mut left[0], &mut right[0]) }
                    else { (&mut right[0], &mut left[0]) };
                let source = source.vault.as_mut().ok_or("Encrypted source closed")?;
                let target = target.vault.as_mut().ok_or("Encrypted destination closed")?;
                target.database.copy_from_vault_item(&mut source.database, job.from_folder, original, job.to_folder, name, job.replace)?
            }
            VaultTransferKind::Inside => {
                let state = self.panes[job.source].vault.as_mut().ok_or("Encrypted container closed")?;
                return state.database.transfer_one(job.from_folder, original, job.to_folder, name, job.moving, job.replace);
            }
        };
        // A move removes the source only after the destination committed.
        // If removal fails, both copies are intentionally retained.
        if job.moving {
            match job.kind {
                VaultTransferKind::Import => {
                    let path = &job.source_paths[job.index];
                    let meta = std::fs::symlink_metadata(path)
                        .map_err(|e| format!("Encrypted copy saved; local original retained: {e}"))?;
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err("Encrypted copy saved; local path became a link, original retained".into());
                    }
                    let result = if meta.is_dir() { std::fs::remove_dir_all(path) }
                        else { std::fs::remove_file(path) };
                    result.map_err(|e| format!("Encrypted copy saved; local original not removed: {e}"))?;
                }
                VaultTransferKind::Export | VaultTransferKind::Between => {
                    let source = self.panes[job.source].vault.as_mut().ok_or("Encrypted source closed")?;
                    let item = source.database.list(job.from_folder)?.into_iter().find(|item| item.name == original)
                        .ok_or("Encrypted copy saved; source item changed before removal")?;
                    source.database.remove_many(&[item.id])
                        .map_err(|e| format!("Encrypted copy saved; original retained: {e}"))?;
                }
                VaultTransferKind::Inside => (),
            }
        }
        Ok(count)
    }

    /// Prepare the current file without loading it into memory. Directories
    /// still use the recursive tree transfer, which streams each child file.
    fn vault_task_start_stream(&mut self, job: &VaultTransferJob,
        name: &str, destination_name: &str) -> Result<Option<VaultStreamState>, String> {
        if job.kind == VaultTransferKind::Inside && job.moving { return Ok(None); }
        let mut input = None;
        let mut source_id = None;
        let total;
        if job.kind == VaultTransferKind::Import {
            let path = &job.source_paths[job.index];
            let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 { return Err("Cannot stream symlink, junction or reparse point".into()); }
            if meta.is_dir() { return Ok(None); }
            if !meta.is_file() { return Err("Encrypted import requires an ordinary file".into()); }
            let target = self.panes[job.target].vault.as_ref().ok_or("Encrypted destination closed")?;
            let own = std::fs::canonicalize(&target.file).map_err(|e| e.to_string())?;
            let from = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
            if own == from { return Err("Cannot import the open encrypted container".into()); }
            input = Some(std::fs::File::open(path).map_err(|e| e.to_string())?);
            total = meta.len();
        } else {
            let source = self.panes[job.source].vault.as_mut().ok_or("Encrypted source closed")?;
            let selected = source.database.list(job.from_folder)?.into_iter()
                .find(|item| item.name == name).ok_or("Encrypted source item disappeared")?;
            if selected.folder { return Ok(None); }
            total = source.database.stream_file_size(selected.id)?;
            source_id = Some(selected.id);
        }
        let mut output = None;
        let mut temp_path = None;
        let mut destination_path = None;
        let mut target_id = None;
        if job.kind == VaultTransferKind::Export {
            let path = PathBuf::from(&job.target_display).join(destination_name);
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                use std::os::windows::fs::MetadataExt;
                if meta.is_dir() { return Err("Cannot replace a directory with a streamed file".into()); }
                if meta.file_attributes() & 0x400 != 0 { return Err("Cannot replace symbolic link or junction".into()); }
                if !job.replace { return Err("Destination already exists".into()); }
            }
            let mut staged = None;
            for attempt in 0..16u32 {
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?.as_nanos();
                let candidate = PathBuf::from(&job.target_display).join(format!(
                    ".ndr-transfer-{}-{stamp}-{attempt}.tmp", std::process::id()));
                match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
                    Ok(file) => { staged = Some((candidate, file)); break; },
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                    Err(error) => return Err(error.to_string()),
                }
            }
            let (path_temp, file) = staged.ok_or("Could not create a unique temporary export file")?;
            temp_path = Some(path_temp);
            output = Some(file);
            destination_path = Some(path);
        } else {
            let pane = self.panes[job.target].vault.as_mut().ok_or("Encrypted destination closed")?;
            target_id = Some(pane.database.begin_stream_file(job.to_folder, destination_name, job.replace)?);
        }
        Ok(Some(VaultStreamState { input, output, temp_path, destination_path,
            source_id, target_id, ordinal: 0, done: 0, total }))
    }

    fn vault_task_abort_stream(&mut self, job: &VaultTransferJob, stream: VaultStreamState) {
        if stream.target_id.is_some() {
            if let Some(vault) = self.panes[job.target].vault.as_mut() {
                let _ = vault.database.rollback_stream_file();
            }
        }
        // Drop removes the temporary plaintext export without modifying the
        // original destination. Never delete already committed items here.
        drop(stream);
    }

    fn vault_task_stream_step(&mut self, job: &VaultTransferJob,
        stream: &mut VaultStreamState) -> Result<bool, String> {
        if stream.done < stream.total {
            let data = if let Some(input) = stream.input.as_mut() {
                let mut buffer = Zeroizing::new(vec![0u8; ndr::vault::STREAM_CHUNK_SIZE]);
                let amount = input.read(&mut buffer).map_err(|e| e.to_string())?;
                if amount == 0 { return Err("Input file changed or ended unexpectedly; transaction rolled back".into()); }
                buffer.truncate(amount);
                buffer
            } else {
                let source = self.panes[job.source].vault.as_ref().ok_or("Encrypted source closed during transfer")?;
                source.database.stream_read_chunk(stream.source_id.ok_or("Missing encrypted source")?, stream.ordinal)?
                    .ok_or("Encrypted source ended before the expected byte count")?
            };
            if stream.done.checked_add(data.len() as u64).ok_or("Transfer byte count overflow")? > stream.total {
                return Err("Source size changed during transfer".into());
            }
            if let Some(output) = stream.output.as_mut() {
                output.write_all(&data).map_err(|e| e.to_string())?;
            } else {
                let target = self.panes[job.target].vault.as_mut().ok_or("Encrypted destination closed during transfer")?;
                target.database.stream_write_chunk(stream.target_id.ok_or("Missing encrypted destination")?,
                    stream.ordinal, &data)?;
            }
            stream.done += data.len() as u64;
            stream.ordinal += 1;
            return Ok(false);
        }
        if let Some(input) = stream.input.as_mut() {
            if input.metadata().map_err(|e| e.to_string())?.len() != stream.total {
                return Err("Source size changed during encryption; transaction rolled back".into());
            }
        }
        if let Some(output) = stream.output.as_mut() {
            output.sync_all().map_err(|e| e.to_string())?;
            stream.output.take();
            let final_path = stream.destination_path.as_ref().ok_or("Missing export destination")?;
            let staged = stream.temp_path.as_ref().ok_or("Missing staged export")?;
            let mut recovery = None;
            if let Ok(meta) = std::fs::symlink_metadata(final_path) {
                use std::os::windows::fs::MetadataExt;
                if meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
                    return Err("Export target became a directory or link; original preserved".into());
                }
                if !job.replace { return Err("Destination appeared during export; original preserved".into()); }
                let recovery_path = final_path.with_extension(format!("ndr-recovery-{}-{}",
                    std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                        .map_err(|e| e.to_string())?.as_nanos()));
                if std::fs::symlink_metadata(&recovery_path).is_ok() {
                    return Err("Export recovery name exists; original preserved".into());
                }
                std::fs::rename(final_path, &recovery_path).map_err(|e| e.to_string())?;
                recovery = Some(recovery_path);
            }
            if let Err(error) = std::fs::rename(staged, final_path) {
                if let Some(path) = recovery {
                    std::fs::rename(&path, final_path).map_err(|restore| format!(
                        "Export failed: {error}; original remains at {} (restoration error: {restore})", path.display()))?;
                }
                return Err(error.to_string());
            }
            stream.temp_path = None;
            if let Some(path) = recovery {
                std::fs::remove_file(&path).map_err(|e| format!(
                    "Export completed, but old destination backup remains at {}: {e}", path.display()))?;
            }
        } else {
            let target = self.panes[job.target].vault.as_mut().ok_or("Encrypted destination closed before commit")?;
            if stream.total == 0 {
                target.database.stream_write_chunk(stream.target_id.ok_or("Missing destination item")?, 0, &[])?;
            }
            target.database.commit_stream_file()?;
        }
        // Move the original only AFTER the complete encrypted/plaintext copy
        // has been successfully committed and renamed into place.
        if job.moving {
            match job.kind {
                VaultTransferKind::Import => {
                    let path = &job.source_paths[job.index];
                    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("Copy succeeded; original retained: {e}"))?;
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 { return Err("Copy succeeded, but source became a link; retained".into()); }
                    std::fs::remove_file(path).map_err(|e| format!("Copy succeeded; original retained: {e}"))?;
                }
                VaultTransferKind::Export | VaultTransferKind::Between => {
                    let source = self.panes[job.source].vault.as_mut().ok_or("Copy succeeded; encrypted source closed")?;
                    source.database.remove_many(&[stream.source_id.ok_or("Missing original item ID")?])
                        .map_err(|e| format!("Copy succeeded; original retained: {e}"))?;
                }
                VaultTransferKind::Inside => (),
            }
        }
        Ok(true)
    }

    /// Keep the NDR file-operation layout and popup alignment for encrypted
    /// work as well. The conflict dialog is the existing TfFileExists form.
    pub(super) fn vault_transfer_progress_ui(&mut self, ctx: &egui::Context) {
        let Some(job) = self.vault_transfer.as_mut() else { return; };
        let id = egui::Id::new("ndr-vault-progress");
        super::keep_opaque(ctx, id);
        let title = self.language_pack.text(if job.moving { "NDRVault.ProgressMove" } else { "NDRVault.ProgressCopy" },
            if job.moving { "Moving encrypted items..." } else { "Copying encrypted items..." });
        let cancel_text = self.language_pack.text("NDRVault.ProgressCancel", "Cancel");
        let count_template = self.language_pack.text("NDRVault.ProgressCount", "{done} / {total}");
        let covered = !self.original_dialogs.is_empty() || !self.msgboxes.is_empty();
        let mut cancel = false;
        super::movable_modal(ctx, id).show(ctx, |ui| {
            ui.set_width(441.0);
            if super::modal_title_bar(ui, id, &title, None) && !covered { cancel = true; }
            ui.separator();
            let (area, _) = ui.allocate_exact_size(egui::vec2(441.0, 143.0), egui::Sense::hover());
            let at = |x: f32, y: f32, w: f32, h: f32| {
                egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h))
            };
            let font = egui::FontId::proportional(13.0);
            let from = super::original_dialogs::path_ellipsis(ui, &job.source_display, 353.0, &font);
            let to = super::original_dialogs::path_ellipsis(ui, &job.target_display, 353.0, &font);
            ui.put(at(20.0, 23.0, 353.0, 16.0), egui::Label::new(from).truncate());
            ui.put(at(20.0, 42.0, 353.0, 15.0), egui::Label::new(to).truncate());
            let file = job.names.get(job.index).or_else(|| job.names.last()).map(String::as_str).unwrap_or("");
            ui.put(at(20.0, 71.0, 317.0, 17.0), egui::Label::new(file).truncate());
            let fraction = job.stream.as_ref().map_or(0.0, |stream| {
                if stream.total == 0 { 1.0 } else { stream.done as f64 / stream.total as f64 }
            });
            let ratio = if job.finished.is_some() && job.failed.is_none() && !job.cancelled { 1.0 }
                else { ((job.index as f64 + fraction) / job.names.len().max(1) as f64) as f32 };
            ui.put(at(18.0, 94.0, 327.0, 16.0), egui::ProgressBar::new(ratio).desired_height(16.0));
            if ui.put(at(355.0, 90.0, 78.0, 25.0), egui::Button::new(cancel_text)).clicked() && !covered {
                cancel = true;
            }
            let count = if let Some(stream) = job.stream.as_ref() {
                self.language_pack.text("NDRVault.ProgressBytes", "{done} / {total} MiB")
                    .replace("{done}", &format!("{:.1}", stream.done as f64 / 1048576.0))
                    .replace("{total}", &format!("{:.1}", stream.total as f64 / 1048576.0))
            } else {
                count_template.replace("{done}", &job.index.to_string())
                    .replace("{total}", &job.names.len().to_string())
            };
            ui.painter().text(area.min + egui::vec2(345.0, 116.0), egui::Align2::RIGHT_TOP,
                count, egui::FontId::proportional(12.0), ui.visuals().text_color());
        });
        if !covered && ctx.input(|input| input.key_pressed(egui::Key::Escape)) { cancel = true; }
        if cancel { job.cancelled = true; ctx.request_repaint(); }
    }
}
