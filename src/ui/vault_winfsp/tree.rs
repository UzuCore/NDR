//! One authenticated WinFsp volume per unlocked .ndr container. Unlike the
//! legacy single-file transport, this exposes the complete directory tree to
//! Explorer, ordinary Windows applications, and their copy/create/rename/delete
//! APIs. All bytes remain encrypted inside the physical .ndr file.
use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const NOT_EMPTY: i32 = 0xC0000101u32 as i32;
const NAME_COLLISION: i32 = 0xC0000035u32 as i32;
const FILE_IS_DIRECTORY: i32 = 0xC00000BAu32 as i32;
const SHARING_VIOLATION: i32 = 0xC0000043u32 as i32;

fn key_for(path: &str) -> String { path.replace('/', "\\").to_lowercase() }

/// A short-lived write reservation serializes an NDR multi-frame transfer
/// against the filesystem writer; it never keeps decrypted temp files.
pub(crate) struct TransferReservation(Arc<AtomicBool>);
impl Drop for TransferReservation {
    fn drop(&mut self) { self.0.store(false, Ordering::Release); }
}

struct TreeData {
    vault: Vault,
    gate: Arc<Mutex<()>>,
    transfer_active: Arc<AtomicBool>,
    revoked: Arc<AtomicBool>,
    files: HashMap<String, Arc<Mutex<MountData>>>,
    revision: u64,
    open_handles: usize,
    last_error: Option<String>,
}

impl TreeData {
    fn refresh(&mut self) -> winfsp::Result<()> {
        let result = {
            let _writer = self.gate.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
            self.vault.reload_after_mount()
        };
        result.map_err(|e| {
            eprintln!("NDR WinFsp tree refresh failed: {e}");
            fs_error(INTERNAL_ERROR)
        })
    }

    fn lookup(&self, path: &str) -> winfsp::Result<VaultItem> {
        if path.is_empty() || path == "\\" {
            return Ok(VaultItem { id: 1, name: String::new(), folder: true, size: 0,
                attributes: 0x10, creation_time: 0, last_access_time: 0,
                last_write_time: 0, change_time: 0 });
        }
        let mut current = 1;
        let mut last = None;
        for component in path.trim_matches('\\').split('\\') {
            if component.is_empty() || component == "." { continue; }
            if component == ".." { return Err(fs_error(ACCESS_DENIED)); }
            let node = self.vault.find_child(current, component)
                .map_err(|_| fs_error(NOT_FOUND))?
                .ok_or_else(|| fs_error(NOT_FOUND))?;
            current = node.id;
            last = Some(node);
        }
        last.ok_or_else(|| fs_error(NOT_FOUND))
    }

    fn parent_name(&self, path: &str) -> winfsp::Result<(i64, String)> {
        let parts: Vec<_> = path.trim_matches('\\').split('\\')
            .filter(|p| !p.is_empty()).collect();
        if parts.is_empty() { return Err(fs_error(ACCESS_DENIED)); }
        let name = parts.last().unwrap().to_string();
        if name == "." || name == ".." { return Err(fs_error(ACCESS_DENIED)); }
        let mut folder = 1;
        for part in &parts[..parts.len() - 1] {
            if *part == "." || *part == ".." { return Err(fs_error(ACCESS_DENIED)); }
            let item = self.vault.find_child(folder, part)
                .map_err(|_| fs_error(NOT_FOUND))?
                .ok_or_else(|| fs_error(NOT_FOUND))?;
            if !item.folder { return Err(fs_error(NOT_FOUND)); }
            folder = item.id;
        }
        Ok((folder, name))
    }

    fn create_file(&mut self, node: &VaultItem, key: String,
        parent: i64) -> winfsp::Result<Arc<Mutex<MountData>>> {
        if let Some(file) = self.files.get(&key) {
            file.lock().map_err(|_| fs_error(INTERNAL_ERROR))?.open_files += 1;
            return Ok(Arc::clone(file));
        }
        let vault = self.vault.fork_for_mount().map_err(|_| fs_error(INTERNAL_ERROR))?;
        let file = Arc::new(Mutex::new(MountData {
            vault, id: node.id, parent, name: node.name.clone(),
            size: node.size.max(0) as u64, original_size: node.size.max(0) as u64,
            pages: BTreeMap::new(), read_cache: None, truncate_floor: None,
            dirty: false, last_error: None, saved_revision: 0,
            open_files: 1, writer_gate: Arc::clone(&self.gate),
        }));
        self.files.insert(key, Arc::clone(&file));
        Ok(file)
    }

    fn metadata_write<T>(&mut self, f: impl FnOnce(&mut Vault) -> Result<T, String>) -> winfsp::Result<T> {
        let gate = Arc::clone(&self.gate);
        let transfer = Arc::clone(&self.transfer_active);
        let result = {
            let _writer = gate.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
            if self.revoked.load(Ordering::Acquire) {
                return Err(fs_error(ACCESS_DENIED));
            }
            // We already own the TreeData mutex here. Waiting on an NDR
            // multi-frame transfer would deadlock the GUI: it polls the
            // same tree for status before advancing to its next chunk.
            // Return a standard Windows sharing violation instead; ordinary
            // Explorer copy/move can retry after the short transfer ends.
            if transfer.load(Ordering::Acquire) {
                return Err(fs_error(SHARING_VIOLATION));
            }
            self.vault.reload_after_mount().and_then(|()| f(&mut self.vault))
        };
        let result = result.map_err(|error| {
            self.last_error = Some(error.clone());
            eprintln!("NDR WinFsp tree write failed: {error}");
            fs_error(INTERNAL_ERROR)
        })?;
        self.revision = self.revision.wrapping_add(1);
        Ok(result)
    }
}

enum TreeHandle {
    Directory(i64),
    File { state: Arc<Mutex<MountData>>, deleted: AtomicBool },
}

struct TreeFs { state: Arc<Mutex<TreeData>> }

impl TreeFs {
    fn info(id: i64, folder: bool, size: u64) -> FileInfo {
        FileInfo {
            file_attributes: if folder { 0x10 } else { 0x20 },
            file_size: if folder { 0 } else { size },
            allocation_size: if folder { 0 } else { size.div_ceil(4096) * 4096 },
            index_number: id as u64,
            ..FileInfo::default()
        }
    }

    fn info_item(item: &VaultItem, size: u64) -> FileInfo {
        let mut info=Self::info(item.id, item.folder, size);
        info.file_attributes=item.attributes;
        info.creation_time=item.creation_time;
        info.last_access_time=item.last_access_time;
        info.last_write_time=item.last_write_time;
        info.change_time=item.change_time;
        info
    }

    fn stat_handle(&self, handle: &TreeHandle) -> winfsp::Result<FileInfo> {
        let (id, size) = match handle {
            TreeHandle::Directory(id) => (*id, 0),
            TreeHandle::File { state, .. } => {
                let file=state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
                (file.id, file.size)
            }
        };
        let mut shared=self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
        // Flush/Close can commit a new authenticated index and replace a
        // file's internal id. Refresh this cached view before stat'ing that
        // open handle or Windows receives a false ERROR_FILE_NOT_FOUND on
        // FlushFileBuffers, even though the editor's save succeeded.
        shared.refresh()?;
        let item=shared.vault.entry_info(id).map_err(|_| fs_error(NOT_FOUND))?;
        Ok(Self::info_item(&item, size))
    }

    fn with_file<R>(&self, context: &TreeHandle,
        f: impl FnOnce(&mut MountData) -> winfsp::Result<R>) -> winfsp::Result<R> {
        let TreeHandle::File { state, deleted } = context else {
            return Err(fs_error(FILE_IS_DIRECTORY));
        };
        if deleted.load(Ordering::Acquire) { return Err(fs_error(NOT_FOUND)); }
        let mut state = state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
        f(&mut state)
    }

    fn flush_file(&self, context: &TreeHandle) -> winfsp::Result<()> {
        let TreeHandle::File { state, deleted } = context else { return Ok(()); };
        if deleted.load(Ordering::Acquire) { return Ok(()); }
        let shared = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
        // Do not hold tree mutex while executing a file commit: the driver
        // may issue concurrent metadata callbacks.
        let gate = Arc::clone(&shared.gate);
        let pending = Arc::clone(&shared.transfer_active);
        drop(shared);
        let _writer = loop {
            let lock = gate.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
            if !pending.load(Ordering::Acquire) { break lock; }
            drop(lock);
            std::thread::sleep(Duration::from_millis(12));
        };
        let result = {
            let mut file = state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
            let before = file.saved_revision;
            match file.commit_with_gate() {
                Ok(()) => Ok(file.saved_revision != before),
                Err(error) => {
                    eprintln!("NDR WinFsp encrypted save failed: {error}");
                    Err(error)
                }
            }
        };
        drop(_writer);
        let result = match result {
            Ok(changed) => changed,
            Err(error) => {
                if let Ok(mut tree) = self.state.lock() {
                    tree.last_error = Some(error);
                }
                return Err(fs_error(INTERNAL_ERROR));
            }
        };
        if result {
            let mut shared = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
            shared.revision = shared.revision.wrapping_add(1);
        }
        Ok(())
    }
}

impl FileSystemContext for TreeFs {
    type FileContext = TreeHandle;

    fn get_security_by_name(&self, file_name: &U16CStr,
        _descriptor: Option<&mut [c_void]>,
        _resolve: impl FnOnce(&U16CStr) -> Option<FileSecurity>) -> winfsp::Result<FileSecurity> {
        let mut shared = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
        if shared.revoked.load(Ordering::Acquire) { return Err(fs_error(ACCESS_DENIED)); }
        shared.refresh()?;
        let node = shared.lookup(&file_name.to_string_lossy())?;
        Ok(FileSecurity { reparse: false, sz_security_descriptor: 0,
            attributes: node.attributes })
    }

    fn open(&self, file_name: &U16CStr, _options: u32,
        _access: u32, file_info: &mut OpenFileInfo) -> winfsp::Result<TreeHandle> {
        let path = file_name.to_string_lossy();
        let mut shared = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
        if shared.revoked.load(Ordering::Acquire) { return Err(fs_error(ACCESS_DENIED)); }
        shared.refresh()?;
        let node = shared.lookup(&path)?;
        let handle = if node.folder { TreeHandle::Directory(node.id) } else {
            let (parent, _) = shared.parent_name(&path)?;
            let file = shared.create_file(&node, key_for(&path), parent)?;
            TreeHandle::File { state: file, deleted: AtomicBool::new(false) }
        };
        let info = match &handle {
            TreeHandle::Directory(_) => Self::info_item(&node, 0),
            TreeHandle::File { state, .. } => {
                let file = state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
                Self::info_item(&node, file.size)
            }
        };
        shared.open_handles += 1;
        *file_info.as_mut() = info;
        Ok(handle)
    }

    fn create(&self, file_name: &U16CStr, create_options: u32,
        _access: u32, attrs: u32, _security: Option<&[c_void]>,
        _allocation_size: u64, _extra: Option<&[u8]>, reparse: bool,
        file_info: &mut OpenFileInfo) -> winfsp::Result<TreeHandle> {
        if reparse { return Err(fs_error(ACCESS_DENIED)); }
        let path = file_name.to_string_lossy();
        let folder = create_options & 1 != 0 || attrs & 0x10 != 0;
        let mut shared = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
        if shared.revoked.load(Ordering::Acquire) { return Err(fs_error(ACCESS_DENIED)); }
        let (parent, name) = {
            shared.refresh()?;
            shared.parent_name(&path)?
        };
        if shared.vault.find_child(parent, &name).map_err(|_| fs_error(INTERNAL_ERROR))?.is_some() {
            return Err(fs_error(NAME_COLLISION));
        }
        let id = shared.metadata_write(|vault|
            vault.add_mounted_entry(parent, &name, folder, attrs))?;
        let handle = if folder {
            TreeHandle::Directory(id)
        } else {
            let item = shared.vault.find_child(parent, &name)
                .map_err(|_| fs_error(INTERNAL_ERROR))?
                .ok_or_else(|| fs_error(INTERNAL_ERROR))?;
            let data = shared.create_file(&item, key_for(&path), parent)?;
            TreeHandle::File { state: data, deleted: AtomicBool::new(false) }
        };
        shared.open_handles += 1;
        let item = shared.vault.entry_info(id).map_err(|_| fs_error(INTERNAL_ERROR))?;
        *file_info.as_mut() = Self::info_item(&item, 0);
        Ok(handle)
    }

    fn close(&self, handle: TreeHandle) {
        let mut closed: Option<Arc<Mutex<MountData>>> = None;
        match &handle {
            TreeHandle::File { state, deleted } => {
                if !deleted.load(Ordering::Acquire) {
                    let _ = self.flush_file(&handle);
                }
                if let Ok(mut file) = state.lock() {
                    file.open_files = file.open_files.saturating_sub(1);
                    if file.open_files == 0 && (file.dirty || file.last_error.is_some()) {
                        eprintln!("NDR WinFsp preserving unsaved virtual file state: {}", file.name);
                    } else if file.open_files == 0 { closed = Some(Arc::clone(state)); }
                }
            }
            TreeHandle::Directory(_) => {}
        }
        if let Ok(mut shared) = self.state.lock() {
            shared.open_handles = shared.open_handles.saturating_sub(1);
            if let Some(state) = closed {
                shared.files.retain(|_, v| !Arc::ptr_eq(v, &state));
            }
        }
    }

    fn get_file_info(&self, handle: &TreeHandle, out: &mut FileInfo) -> winfsp::Result<()> {
        *out = self.stat_handle(handle)?;
        Ok(())
    }

    fn read(&self, handle: &TreeHandle, out: &mut [u8], offset: u64) -> winfsp::Result<u32> {
        let done = self.with_file(handle, |file|
            file.read_at(out, offset).map_err(|_| fs_error(INTERNAL_ERROR)))?;
        if done == 0 { Err(fs_error(END_OF_FILE)) } else { Ok(done as u32) }
    }

    fn write(&self, handle: &TreeHandle, data: &[u8], offset: u64,
        to_eof: bool, constrained: bool, info: &mut FileInfo) -> winfsp::Result<u32> {
        let done = self.with_file(handle, |file| {
            let offset = if to_eof { file.size } else { offset };
            let size = if constrained {
                file.size.saturating_sub(offset).min(data.len() as u64) as usize
            } else { data.len() };
            file.write_at(&data[..size], offset).map_err(|_| fs_error(INTERNAL_ERROR))
        })?;
        *info = self.stat_handle(handle)?;
        Ok(done as u32)
    }

    fn set_file_size(&self, handle: &TreeHandle, new_size: u64,
        allocation_only: bool, info: &mut FileInfo) -> winfsp::Result<()> {
        if new_size > 1u64 << 40 { return Err(fs_error(ACCESS_DENIED)); }
        if !allocation_only {
            self.with_file(handle, |file| {
                if new_size < file.size {
                    let floor = file.truncate_floor.map_or(new_size, |old| old.min(new_size));
                    file.truncate_floor = Some(floor);
                    for (index, bytes) in &mut file.pages {
                        let from = floor.saturating_sub(index * PAGE).min(PAGE) as usize;
                        bytes[from..].fill(0);
                    }
                }
                file.size = new_size;
                file.pages.retain(|index, _| index.saturating_mul(PAGE) < new_size);
                file.dirty = true;
                Ok(())
            })?;
        }
        *info = self.stat_handle(handle)?;
        Ok(())
    }

    fn overwrite(&self, handle: &TreeHandle, _attributes: u32,
        _replace: bool, _allocation_size: u64, _extra: Option<&[u8]>,
        info: &mut FileInfo) -> winfsp::Result<()> {
        self.with_file(handle, |file| {
            file.size = 0;
            file.truncate_floor = Some(0);
            file.pages.clear();
            file.read_cache = None;
            file.dirty = true;
            Ok(())
        })?;
        *info = self.stat_handle(handle)?;
        Ok(())
    }

    fn flush(&self, handle: Option<&TreeHandle>, out: &mut FileInfo) -> winfsp::Result<()> {
        if let Some(handle) = handle {
            self.flush_file(handle)?;
            *out = self.stat_handle(handle)?;
        } else { *out = Self::info(1, true, 0); }
        Ok(())
    }

    fn set_basic_info(&self, handle: &TreeHandle, attributes: u32,
        created: u64, accessed: u64, written: u64, changed: u64,
        out: &mut FileInfo) -> winfsp::Result<()> {
        let id = match handle {
            TreeHandle::Directory(id) => *id,
            TreeHandle::File { state, deleted } => {
                if deleted.load(Ordering::Acquire) { return Err(fs_error(NOT_FOUND)); }
                state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?.id
            }
        };
        // FileInfo refresh is OUTSIDE the state lock: other driver callbacks
        // need to read the same authenticated directory metadata.
        {
            let mut shared=self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
            shared.metadata_write(|vault|
                vault.set_basic_info(id, attributes, created, accessed, written, changed))?;
        }
        *out=self.stat_handle(handle)?;
        Ok(())
    }

    fn read_directory(&self, handle: &TreeHandle, _pattern: Option<&U16CStr>,
        marker: DirMarker, buffer: &mut [u8]) -> winfsp::Result<u32> {
        let TreeHandle::Directory(id) = handle else { return Err(fs_error(ACCESS_DENIED)); };
        let mut shared = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
        if shared.revoked.load(Ordering::Acquire) { return Err(fs_error(ACCESS_DENIED)); }
        shared.refresh()?;
        let mut entries: Vec<(String, FileInfo)> = Vec::new();
        if *id != 1 {
            entries.push((".".into(), Self::info(*id, true, 0)));
            let parent = shared.vault.parent(*id).map_err(|_| fs_error(INTERNAL_ERROR))?;
            entries.push(("..".into(), Self::info(parent, true, 0)));
        }
        let children = shared.vault.list(*id).map_err(|_| fs_error(INTERNAL_ERROR))?;
        for item in children {
            let name = item.name.clone();
            let key = shared.vault.relative_folder_path(*id).unwrap_or_default()
                .join(&name).to_string_lossy().replace('/', "\\").to_lowercase();
            let size = if !item.folder {
                shared.files.get(&format!("\\{key}")).and_then(|file|
                    file.lock().ok().map(|f| f.size)).unwrap_or(item.size.max(0) as u64)
            } else { 0 };
            entries.push((name, Self::info_item(&item, size)));
        }
        let marker = marker.inner_as_cstr().map(|s| s.to_string_lossy());
        let start = marker.as_ref().map_or(0, |mark| {
            entries.iter().position(|(name,_)| name.eq_ignore_ascii_case(mark))
                .map_or(entries.len(), |n| n + 1)
        });
        let mut cursor = 0u32;
        for (name, info) in entries.into_iter().skip(start) {
            let mut entry = DirInfo::<255>::new();
            *entry.file_info_mut() = info;
            let wide: Vec<u16> = name.encode_utf16().collect();
            entry.set_name_raw(wide.as_slice())?;
            if !entry.append_to_buffer(buffer, &mut cursor) { break; }
        }
        DirInfo::<255>::finalize_buffer(buffer, &mut cursor);
        Ok(cursor)
    }

    fn set_delete(&self, handle: &TreeHandle, _name: &U16CStr,
        delete: bool) -> winfsp::Result<()> {
        if !delete { return Ok(()); }
        match handle {
            TreeHandle::Directory(id) => {
                if *id == 1 { return Err(fs_error(ACCESS_DENIED)); }
                let mut shared = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
                shared.refresh()?;
                if !shared.vault.list(*id).map_err(|_| fs_error(INTERNAL_ERROR))?.is_empty() {
                    return Err(fs_error(NOT_EMPTY));
                }
            }
            TreeHandle::File { state, deleted } => {
                let file = state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
                if file.open_files > 1 { return Err(fs_error(SHARING_VIOLATION)); }
                deleted.store(false, Ordering::Release);
            }
        }
        Ok(())
    }

    fn cleanup(&self, handle: &TreeHandle, _file_name: Option<&U16CStr>, flags: u32) {
        if flags & 1 == 0 { return; }
        let result: winfsp::Result<()> = (|| {
            let mut shared = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
            match handle {
                TreeHandle::Directory(id) => {
                    if *id == 1 { return Err(fs_error(ACCESS_DENIED)); }
                    shared.refresh()?;
                    if !shared.vault.list(*id).map_err(|_| fs_error(INTERNAL_ERROR))?.is_empty() {
                        return Err(fs_error(NOT_EMPTY));
                    }
                    shared.metadata_write(|v| v.remove(*id))?;
                }
                TreeHandle::File { state, deleted } => {
                    let file = state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
                    if file.open_files > 1 { return Err(fs_error(SHARING_VIOLATION)); }
                    let parent = file.parent;
                    let name = file.name.clone();
                    drop(file);
                    shared.refresh()?;
                    let node = shared.vault.find_child(parent, &name)
                        .map_err(|_| fs_error(INTERNAL_ERROR))?
                        .ok_or_else(|| fs_error(NOT_FOUND))?;
                    shared.metadata_write(|v| v.remove(node.id))?;
                    deleted.store(true, Ordering::Release);
                    shared.files.retain(|_,f| !Arc::ptr_eq(f,state));
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("NDR WinFsp tree cleanup failed: {error}");
            if let Ok(mut shared) = self.state.lock() {
                shared.last_error = Some(format!("Cannot delete encrypted item: {error}"));
            }
        }
    }

    fn rename(&self, handle: &TreeHandle, from: &U16CStr,
        target: &U16CStr, replace: bool) -> winfsp::Result<()> {
        let old = from.to_string_lossy();
        let new = target.to_string_lossy();
        if old.eq_ignore_ascii_case("\\") { return Err(fs_error(ACCESS_DENIED)); }
        if let TreeHandle::File { .. } = handle {
            self.flush_file(handle)?;
        }
        let mut shared = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
        shared.refresh()?;
        let original = shared.lookup(&old)?;
        let (from_parent, from_name) = shared.parent_name(&old)?;
        let (to_parent, to_name) = shared.parent_name(&new)?;
        if from_parent == to_parent && from_name.eq_ignore_ascii_case(&to_name) {
            shared.metadata_write(|v| v.rename(original.id, &to_name))?;
        } else {
            if let Some(existing) = shared.vault.find_child(to_parent, &to_name)
                .map_err(|_| fs_error(INTERNAL_ERROR))? {
                if !replace { return Err(fs_error(NAME_COLLISION)); }
                if shared.files.values().any(|item| item.lock().is_ok_and(|file|
                    file.id == existing.id && file.open_files > 0)) {
                    return Err(fs_error(SHARING_VIOLATION));
                }
            }
            if original.folder && shared.files.keys().any(|key|
                key.starts_with(&(key_for(&old) + "\\"))) {
                return Err(fs_error(SHARING_VIOLATION));
            }
            shared.metadata_write(|v| v.transfer_one(from_parent, &from_name,
                to_parent, &to_name, true, replace).map(|_| ()))?;
        }
        if let TreeHandle::File { state, .. } = handle {
            if let Ok(mut file) = state.lock() {
                file.name = to_name;
                file.parent = to_parent;
            }
            shared.files.remove(&key_for(&old));
            shared.files.insert(key_for(&new), Arc::clone(state));
        }
        Ok(())
    }

    fn get_volume_info(&self, info: &mut VolumeInfo) -> winfsp::Result<()> {
        info.total_size = 1u64 << 40;
        info.free_size = 1u64 << 40;
        info.set_volume_label("NDR Vault");
        Ok(())
    }
}

pub(crate) struct VaultTreeMount {
    host: FileSystemHost<TreeFs, FineGuard>,
    state: Arc<Mutex<TreeData>>,
    pub vault_path: std::path::PathBuf,
    pub mount_root: std::path::PathBuf,
    observed_revision: u64,
}

impl VaultTreeMount {
    pub fn start(path: std::path::PathBuf, source: &Vault,
        gate: Arc<Mutex<()>>, occupied: &[char]) -> Result<Self, String> {
        if !runtime_available() { return Err("WinFsp is not installed".into()); }
        let drives = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        let letter = (b'T'..=b'Z').rev().find(|&c|
            drives & (1u32 << (c - b'A')) == 0 && !occupied.contains(&(c as char)))
            .ok_or("No free drive letter for NDR encrypted volume")?;
        let mountpoint = format!("{}:", letter as char);
        let state = Arc::new(Mutex::new(TreeData {
            vault: source.fork_for_mount()?, gate,
            transfer_active: Arc::new(AtomicBool::new(false)),
            revoked: Arc::new(AtomicBool::new(false)),
            files: HashMap::new(), revision: 0, open_handles: 0,
            last_error: None,
        }));
        let mut params = VolumeParams::new();
        params.filesystem_name("NDRVAULT").sector_size(512)
            .sectors_per_allocation_unit(8).case_sensitive_search(false)
            .case_preserved_names(true).unicode_on_disk(true)
            .persistent_acls(false).file_info_timeout(0);
        let config = FileSystemParams {
            use_dir_info_by_name: false, volume_params: params,
            debug_mode: DebugMode::none(),
        };
        let mut host = FileSystemHost::<TreeFs, FineGuard>::new_with_options(
            config, TreeFs { state: Arc::clone(&state) })
            .map_err(|e| format!("WinFsp create failed: {e}"))?;
        host.mount(&mountpoint).map_err(|e| format!("WinFsp mount failed: {e}"))?;
        host.start().map_err(|e| format!("WinFsp start failed: {e}"))?;
        Ok(Self {
            host, state, vault_path: path,
            mount_root: std::path::PathBuf::from(format!("{mountpoint}\\")),
            observed_revision: 0,
        })
    }
    /// Expired/minimized sessions deny all NEW Windows file opens while
    /// retaining existing handles long enough to flush authenticated edits.
    pub fn revoke(&self) {
        if let Ok(state) = self.state.lock() {
            state.revoked.store(true, Ordering::Release);
        }
    }
    pub fn is_revoked(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.revoked.load(Ordering::Acquire))
    }
    pub fn drive_letter(&self) -> Option<char> {
        self.mount_root.to_string_lossy().chars().next()
    }
    pub fn writer_gate(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.state.lock().unwrap().gate)
    }
    pub fn reserve_transfer(&self) -> Result<TransferReservation, String> {
        let state = self.state.lock().map_err(|e| e.to_string())?;
        let busy = Arc::clone(&state.transfer_active);
        let gate = Arc::clone(&state.gate);
        drop(state);
        let _g = gate.lock().map_err(|e| e.to_string())?;
        if busy.swap(true, Ordering::AcqRel) {
            return Err("An encrypted file transfer is already underway".into());
        }
        Ok(TransferReservation(busy))
    }
    pub fn has_open_files(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.open_handles != 0)
    }
    pub fn can_disconnect(&self) -> bool {
        self.state.lock().is_ok_and(|tree|
            tree.open_handles == 0 && tree.files.values().all(|entry|
                entry.lock().is_ok_and(|file| !file.dirty && file.last_error.is_none())))
    }
    /// An individual file cannot be moved/deleted in NDR while another
    /// Windows program has it open; unrelated files remain writable.
    pub fn is_open_item(&self, parent: i64, name: &str) -> bool {
        self.state.lock().is_ok_and(|tree| {
            let prefix = tree.vault.relative_folder_path(parent).unwrap_or_default()
                .join(name).to_string_lossy().replace('/', "\\").to_lowercase();
            let prefix = format!("\\{prefix}");
            tree.files.iter().any(|(path, entry)| {
                (path == &prefix || path.starts_with(&(prefix.clone() + "\\")))
                    && entry.lock().is_ok_and(|file| file.open_files > 0)
            })
        })
    }
    pub fn take_saved_revision(&mut self) -> bool {
        let revision = self.state.lock().map_or(0, |s| s.revision);
        if revision == self.observed_revision { return false; }
        self.observed_revision = revision;
        true
    }
    pub fn take_save_error(&mut self) -> Option<String> {
        self.state.lock().ok().and_then(|mut s| s.last_error.take())
    }
    pub fn finish(&mut self) -> Result<(), String> {
        if self.has_open_files() {
            return Err("Close external applications using the encrypted drive".into());
        }
        let state = self.state.lock().map_err(|e| e.to_string())?;
        if state.files.values().any(|f|
            f.lock().is_ok_and(|f| f.dirty || f.last_error.is_some())) {
            return Err("An encrypted file has uncommitted edits".into());
        }
        drop(state);
        self.host.stop();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires installed WinFsp and unused drive letter; disposable encrypted test data"]
    fn full_tree_mount_supports_explorer_create_copy_rename_and_delete() {
        use std::os::windows::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!(
            "ndr-winfsp-tree-real-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tree.ndr");
        let mut original = Vault::create(&path, "tree mount test password 2026".into()).unwrap();
        let sub = original.add_folder(1,"Subfolder").unwrap();
        original.add_file(sub,"Existing.txt",b"previous").unwrap();
        let mut mount = VaultTreeMount::start(path.clone(), &original,
            Arc::new(Mutex::new(())), &[]).unwrap();
        let root = mount.mount_root.clone();
        assert!(std::fs::read_dir(&root).unwrap().any(|r|
            r.unwrap().file_name().to_string_lossy().eq_ignore_ascii_case("Subfolder")));
        assert_eq!(std::fs::read(root.join("Subfolder/Existing.txt")).unwrap(),b"previous");
        std::fs::create_dir(root.join("Newfolder")).unwrap();
        std::fs::write(root.join("Newfolder/new.bin"),b"secure imported payload").unwrap();
        // Both copy directions must use actual Win32 file I/O, with
        // authenticated storage at the virtual destination and ordinary
        // plaintext only at the explicitly requested local destination.
        let incoming = dir.join("from-explorer.bin");
        let outgoing = dir.join("to-explorer.bin");
        std::fs::write(&incoming,b"external source 2026").unwrap();
        std::fs::copy(&incoming,root.join("Subfolder/imported.bin")).unwrap();
        std::fs::copy(root.join("Subfolder/imported.bin"),&outgoing).unwrap();
        assert_eq!(std::fs::read(&outgoing).unwrap(),b"external source 2026");
        assert_eq!(
            std::fs::metadata(root.join("Subfolder/Existing.txt")).unwrap()
                .file_attributes() & 0x7, 0,
            "source is a normal file without readonly, hidden or system flags");
        std::fs::copy(root.join("Subfolder/Existing.txt"),root.join("copy.txt")).unwrap();
        assert_eq!(
            std::fs::metadata(root.join("copy.txt")).unwrap().file_attributes() & 0x7, 0,
            "CopyFileW must not treat WinFsp's 0xffffffff sentinel as real attributes");
        std::fs::rename(root.join("copy.txt"),root.join("renamed.txt")).unwrap();
        assert_eq!(std::fs::read(root.join("renamed.txt")).unwrap(),b"previous");
        let renamed=root.join("renamed.txt");
        assert_eq!(std::fs::metadata(&renamed).unwrap().file_attributes() & 0x7, 0,
            "renaming must preserve the ordinary removable-file attributes");
        std::fs::remove_file(&renamed).unwrap();
        std::fs::remove_file(root.join("Newfolder/new.bin")).unwrap();
        std::fs::remove_dir(root.join("Newfolder")).unwrap();
        let closed = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while mount.has_open_files() {
            assert!(std::time::Instant::now() < closed,
                "Explorer file handles were not released after normal close");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        mount.finish().unwrap();
        drop(mount);
        original.reload_after_mount().unwrap();
        assert!(original.find_child(1,"Newfolder").unwrap().is_none());
        assert!(original.find_child(1,"renamed.txt").unwrap().is_none());
        let folder = original.find_child(1,"Subfolder").unwrap().unwrap().id;
        let file = original.find_child(folder,"Existing.txt").unwrap().unwrap();
        assert_eq!(original.read_file(file.id).unwrap(),b"previous");
        let imported = original.find_child(folder,"imported.bin").unwrap().unwrap();
        assert_eq!(original.read_file(imported.id).unwrap(),b"external source 2026");
        drop(original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "requires installed WinFsp and unused drive letter; disposable encrypted test data"]
    fn encrypted_file_properties_persist_readonly_hidden_and_modified_time() {
        use std::os::windows::fs::MetadataExt;
        let root = std::env::temp_dir().join(format!(
            "ndr-winfsp-props-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let path=root.join("properties.ndr");
        let mut vault=Vault::create(&path,"property test password 2026".into()).unwrap();
        vault.add_file(1,"file.txt",b"contents").unwrap();
        vault.add_folder(1,"folder").unwrap();
        let mut mount=VaultTreeMount::start(path.clone(),&vault,
            Arc::new(Mutex::new(())),&[]).unwrap();
        let mounted=mount.mount_root.clone();
        let file=mounted.join("file.txt");
        let folder=mounted.join("folder");
        // filetime opens FILE_WRITE_DATA in some versions, which Windows
        // disallows after READONLY is set. Apply time first, just like an
        // ordinary filesystem API sequence, then set READONLY/HIDDEN.
        let time=filetime::FileTime::from_unix_time(1_577_934_245,0);
        filetime::set_file_mtime(&file,time).unwrap();
        ndr::native::set_attributes(&file,true,true).unwrap();
        ndr::native::set_attributes(&folder,false,true).unwrap();
        let m=std::fs::metadata(&file).unwrap();
        assert_ne!(m.file_attributes() & 0x1,0,"readonly attribute from virtual drive");
        assert_ne!(m.file_attributes() & 0x2,0,"hidden attribute from virtual drive");
        assert_eq!(m.modified().unwrap(), std::time::SystemTime::from(time),
            "WinFsp file modified time must roundtrip");
        assert_ne!(std::fs::metadata(&folder).unwrap().file_attributes() & 0x2,0,
            "directory hidden attribute must roundtrip");
        let limit=std::time::Instant::now()+std::time::Duration::from_secs(10);
        while mount.has_open_files() {
            assert!(std::time::Instant::now()<limit,"open driver handles after SetFileTime");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        mount.finish().unwrap();
        drop(mount);
        vault.reload_after_mount().unwrap();
        let item=vault.find_child(1,"file.txt").unwrap().unwrap();
        assert_eq!(item.attributes & 0x3,0x3,
            "attributes persist in authenticated .ndr metadata");
        assert_eq!(item.last_write_time,
            116_444_736_000_000_000+1_577_934_245u64*10_000_000);
        assert_eq!(vault.find_child(1,"folder").unwrap().unwrap().attributes & 0x2,0x2);
        drop(vault);
        let opened=Vault::unlock(&path,"property test password 2026".into()).unwrap();
        let item=opened.find_child(1,"file.txt").unwrap().unwrap();
        assert_eq!(item.attributes & 0x3,0x3,"unlock must keep original attributes");
        drop(opened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires installed WinFsp and unused drive letter; disposable encrypted test data"]
    fn mounted_tree_respects_transfer_reservation_and_session_revocation() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "ndr-winfsp-revocation-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("revoked.ndr");
        let mut original = Vault::create(&path, "test session expiry 2026".into()).unwrap();
        original.add_file(1,"already-open.txt",b"before").unwrap();
        let mut mount = VaultTreeMount::start(path, &original,
            Arc::new(Mutex::new(())), &[]).unwrap();
        let root = mount.mount_root.clone();
        let mut opened = std::fs::OpenOptions::new().read(true).write(true)
            .open(root.join("already-open.txt")).unwrap();
        assert!(mount.has_open_files());
        let reservation = mount.reserve_transfer().unwrap();
        assert!(std::fs::write(root.join("blocked-during-import.txt"), b"not committed").is_err(),
            "Windows file creation must fail quickly, not deadlock while NDR is transferring");
        drop(reservation);
        std::fs::write(root.join("allowed.txt"),b"after transfer").unwrap();
        mount.revoke();
        assert!(mount.is_revoked());
        assert!(std::fs::File::open(root.join("already-open.txt")).is_err(),
            "session expiry must block new plaintext read handles");
        opened.write_all(b"after!").unwrap();
        opened.sync_all().unwrap();
        drop(opened);
        let closed = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while mount.has_open_files() {
            assert!(std::time::Instant::now() < closed,
                "the last external editor handle did not close");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        mount.finish().unwrap();
        drop(mount);
        original.reload_after_mount().unwrap();
        let item = original.find_child(1,"already-open.txt").unwrap().unwrap();
        assert_eq!(original.read_file(item.id).unwrap(),b"after!",
            "an existing editor must be able to save before the revoked mount disconnects");
        let allowed = original.find_child(1,"allowed.txt").unwrap().unwrap();
        assert_eq!(original.read_file(allowed.id).unwrap(),b"after transfer");
        assert!(original.find_child(1,"blocked-during-import.txt").unwrap().is_none());
        drop(original);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
