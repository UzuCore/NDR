//! Experimental WinFsp transport: expose an authenticated vault item to
//! Windows applications without creating any decrypted disk file. Only a
//! single selected item is exposed per mount; the provider is deliberately
//! conservative about filesystem operations it cannot support safely.
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use ndr::vault::{Vault, VaultItem, STREAM_CHUNK_SIZE};
use zeroize::{Zeroize, Zeroizing};
use widestring::U16CStr;
use winfsp::{FspError, filesystem::{DirInfo, DirMarker, FileInfo, FileSecurity, FileSystemContext, OpenFileInfo, VolumeInfo, WideNameInfo}, host::{DebugMode, FileSystemHost, FileSystemParams, FineGuard, VolumeParams}};

const PAGE: u64 = STREAM_CHUNK_SIZE as u64;
const NOT_FOUND: i32 = 0xC0000034u32 as i32;
const ACCESS_DENIED: i32 = 0xC0000022u32 as i32;
const END_OF_FILE: i32 = 0xC0000011u32 as i32;
const INTERNAL_ERROR: i32 = 0xC00000E5u32 as i32;

fn fs_error(code: i32) -> FspError { FspError::NTSTATUS(code) }

/// UI notification polling must not wait for a large external file's
/// authenticated save. A busy lock defers the notice without consuming it.
fn poll_revision<T>(state: &Mutex<T>, observed: &mut u64,
    revision: impl FnOnce(&T) -> u64) -> Result<bool, String> {
    let current = match state.try_lock() {
        Ok(state) => revision(&state),
        Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    if current == *observed { return Ok(false); }
    *observed = current;
    Ok(true)
}

fn poll_error_notice<T>(state: &Mutex<T>, reported: &mut bool,
    failed: impl FnOnce(&T) -> bool) -> bool {
    let Ok(state) = state.try_lock() else { return false };
    if !failed(&state) { *reported = false; return false; }
    if *reported { return false; }
    *reported = true;
    true
}

#[test]
fn batch393_busy_notification_poll_does_not_block_or_consume_notice() {
    let state = Mutex::new((5u64,true));
    let mut observed = 3;
    let mut reported = false;
    let held = state.lock().unwrap();
    assert!(!poll_revision(&state,&mut observed,|s|s.0).unwrap());
    assert_eq!(observed,3);
    assert!(!poll_error_notice(&state,&mut reported,|s|s.1));
    assert!(!reported);
    drop(held);
    assert!(poll_revision(&state,&mut observed,|s|s.0).unwrap());
    assert!(poll_error_notice(&state,&mut reported,|s|s.1));
    let held = state.lock().unwrap();
    assert!(!poll_error_notice(&state,&mut reported,|s|s.1));
    assert!(reported,"lock contention may not reset an existing notice");
    drop(held);
    assert!(!poll_revision(&state,&mut observed,|s|s.0).unwrap());
    assert!(!poll_error_notice(&state,&mut reported,|s|s.1));
    state.lock().unwrap().1=false;
    assert!(!poll_error_notice(&state,&mut reported,|s|s.1));
    assert!(!reported);
    state.lock().unwrap().1=true;
    assert!(poll_error_notice(&state,&mut reported,|s|s.1));
}

fn load_installed_winfsp_dll() -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::LibraryLoader::LoadLibraryW;
    use windows::core::PCWSTR;
    for base in [
        std::env::var_os("ProgramFiles(x86)"),
        std::env::var_os("ProgramFiles"),
    ].into_iter().flatten() {
        let dll = std::path::PathBuf::from(base).join("WinFsp/bin/winfsp-x64.dll");
        if dll.is_file() {
            let mut wide: Vec<u16> = dll.as_os_str().encode_wide().collect();
            wide.push(0);
            // Load the signed runtime from its installed directory. The
            // winfsp-rs delay-loaded binding can then resolve its basename.
            return unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())).is_ok() };
        }
    }
    false
}

pub(super) fn runtime_available() -> bool {
    load_installed_winfsp_dll() && winfsp::winfsp_init().is_ok()
}

// Per open file: at most two dirty pages in plaintext. Cold dirty pages
// are appended as unpublished GCM records to the original encrypted file.
// A two-page read cache also prevents alternating-page read amplification.
const DIRTY_PAGE_LIMIT: usize = 2;
/// Grow a sensitive page without releasing an unwiped old allocation.
fn resize_plain_page(page: &mut Zeroizing<Vec<u8>>, len: usize) {
    debug_assert!(len <= STREAM_CHUNK_SIZE);
    if len < page.len() {
        page[len..].zeroize();
        page.truncate(len);
    } else if len > page.capacity() {
        let mut next = Zeroizing::new(Vec::with_capacity(len.next_power_of_two().min(STREAM_CHUNK_SIZE)));
        next.extend_from_slice(page);
        next.resize(len, 0);
        *page = next;
    } else {
        page.resize(len, 0);
    }
}
struct StagedPage {
    chunk: ndr::vault::StagedVaultChunk,
    valid: usize,
}

struct MountData {
    vault: Vault,
    id: i64,
    parent: i64,
    name: String,
    size: u64,
    original_size: u64,
    pages: BTreeMap<u64, Zeroizing<Vec<u8>>>,
    staged_pages: BTreeMap<u64, StagedPage>,
    read_cache: Option<(u64, Zeroizing<Vec<u8>>)>,
    previous_read: Option<(u64, Zeroizing<Vec<u8>>)>,
    truncate_floor: Option<u64>,
    dirty: bool,
    last_error: Option<String>,
    // Incremented only after the encrypted index was committed successfully.
    saved_revision: u64,
    open_files: usize,
    closing: bool,
    writer_gate: Arc<Mutex<()>>,
}

impl MountData {
    fn clear_read_cache(&mut self) {
        self.read_cache = None;
        self.previous_read = None;
    }

    fn original_visible_size(&self) -> u64 {
        self.original_size.min(self.truncate_floor.unwrap_or(u64::MAX))
    }

    /// Only immutable or unpublished encrypted records reach this cache.
    /// Dirty resident pages take precedence; logical truncation masks bytes
    /// of a staged record without changing the record's authenticated bytes.
    fn cached_page(&mut self, page: u64) -> Result<&[u8], String> {
        if self.read_cache.as_ref().is_some_and(|(index, _)| *index == page) {
            return Ok(&self.read_cache.as_ref().unwrap().1);
        }
        if self.previous_read.as_ref().is_some_and(|(index, _)| *index == page) {
            std::mem::swap(&mut self.read_cache, &mut self.previous_read);
            return Ok(&self.read_cache.as_ref().unwrap().1);
        }
        let plain = if let Some(staged) = self.staged_pages.get(&page) {
            self.vault.read_staged_chunk(&staged.chunk)?
        } else {
            self.vault.stream_read_chunk(self.id, page as i64)?
                .unwrap_or_else(|| Zeroizing::new(Vec::new()))
        };
        self.previous_read = self.read_cache.take();
        self.read_cache = Some((page, plain));
        Ok(&self.read_cache.as_ref().unwrap().1)
    }

    fn read_at(&mut self, buffer: &mut [u8], offset: u64) -> Result<usize, String> {
        if offset >= self.size { return Ok(0); }
        let length = buffer.len().min(self.size.saturating_sub(offset) as usize);
        let mut done = 0;
        while done < length {
            let position = offset + done as u64;
            let page = position / PAGE;
            let within = (position % PAGE) as usize;
            let take = (length - done).min(STREAM_CHUNK_SIZE - within);
            let out = &mut buffer[done..done + take];
            out.fill(0);
            if let Some(staged) = self.pages.get(&page) {
                let available = take.min(staged.len().saturating_sub(within));
                if available > 0 {
                    out[..available].copy_from_slice(&staged[within..within + available]);
                }
            } else {
                let visible = self.staged_pages.get(&page).map_or_else(
                    || self.original_visible_size().saturating_sub(page * PAGE).min(PAGE) as usize,
                    |staged| staged.valid);
                if within < visible {
                    let plaintext = self.cached_page(page)?;
                    let available = take.min(visible - within).min(plaintext.len().saturating_sub(within));
                    if available > 0 {
                        out[..available].copy_from_slice(&plaintext[within..within + available]);
                    }
                }
            }
            done += take;
        }
        Ok(done)
    }

    /// Append a cold dirty page before removing its only plaintext copy.
    /// A failed append leaves the page and all prior successful writes intact.
    fn evict_dirty_page(&mut self) -> Result<(), String> {
        if self.pages.len() < DIRTY_PAGE_LIMIT { return Ok(()); }
        let page = *self.pages.keys().next().unwrap();
        let bytes = &self.pages[&page];
        let chunk = self.vault.stage_chunk(bytes)?;
        let valid = bytes.len();
        self.staged_pages.insert(page, StagedPage { chunk, valid });
        self.pages.remove(&page);
        Ok(())
    }

    fn load_dirty_page(&mut self, page: u64, full_overwrite: bool) -> Result<(), String> {
        if self.pages.contains_key(&page) { return Ok(()); }
        self.evict_dirty_page()?;
        let visible = self.staged_pages.get(&page).map_or_else(
            || self.original_visible_size().saturating_sub(page * PAGE).min(PAGE) as usize,
            |staged| staged.valid);
        let mut plain = if full_overwrite || visible == 0 {
            Zeroizing::new(Vec::new())
        } else if self.read_cache.as_ref().is_some_and(|(index, _)| *index == page) {
            self.read_cache.take().unwrap().1
        } else if self.previous_read.as_ref().is_some_and(|(index, _)| *index == page) {
            self.previous_read.take().unwrap().1
        } else if let Some(staged) = self.staged_pages.get(&page) {
            self.vault.read_staged_chunk(&staged.chunk)?
        } else {
            self.vault.stream_read_chunk(self.id, page as i64)?
                .unwrap_or_else(|| Zeroizing::new(Vec::new()))
        };
        if plain.len() > visible {
            plain[visible..].zeroize();
            plain.truncate(visible);
        }
        self.clear_read_cache();
        self.pages.insert(page, plain);
        self.staged_pages.remove(&page);
        Ok(())
    }

    fn write_at(&mut self, buffer: &[u8], offset: u64) -> Result<usize, String> {
        if buffer.is_empty() { return Ok(0); }
        let end = offset.checked_add(buffer.len() as u64).ok_or("File offset overflow")?;
        // Preserve the existing sparse-file safety limit.
        if end > (1u64 << 40) { return Err("Virtual file exceeds 1 TiB safety limit".into()); }
        let mut done = 0;
        while done < buffer.len() {
            let position = offset + done as u64;
            let page = position / PAGE;
            let within = (position % PAGE) as usize;
            let take = (buffer.len() - done).min(STREAM_CHUNK_SIZE - within);
            if let Err(error) = self.load_dirty_page(page, within == 0 && take == STREAM_CHUNK_SIZE) {
                // Return a short successful write if earlier pages were
                // accepted. The OS can retry the rest without losing them.
                if done > 0 { return Ok(done); }
                return Err(error);
            }
            let staged = self.pages.get_mut(&page).unwrap();
            if staged.len() < within + take { resize_plain_page(staged, within + take); }
            staged[within..within + take].copy_from_slice(&buffer[done..done + take]);
            done += take;
            self.size = self.size.max(offset + done as u64);
            self.dirty = true;
        }
        Ok(done)
    }

    fn resize(&mut self, new_size: u64) {
        if new_size < self.size {
            self.truncate_floor = Some(self.truncate_floor.map_or(new_size, |old| old.min(new_size)));
            // The historical floor applies only to original data. Later
            // writes below a new EOF must survive a second shrink/grow.
            self.pages.retain(|index, bytes| {
                let valid = new_size.saturating_sub(index * PAGE).min(PAGE) as usize;
                if valid < bytes.len() {
                    bytes[valid..].zeroize();
                    bytes.truncate(valid);
                }
                valid > 0
            });
            self.staged_pages.retain(|index, staged| {
                staged.valid = staged.valid.min(new_size.saturating_sub(index * PAGE).min(PAGE) as usize);
                staged.valid > 0
            });
            self.clear_read_cache();
        }
        if new_size != self.size {
            self.size = new_size;
            self.dirty = true;
        }
    }

    fn overwrite(&mut self) {
        self.resize(0);
        self.truncate_floor = Some(0);
        self.pages.clear();
        self.staged_pages.clear();
        self.clear_read_cache();
        self.dirty = true;
    }

    fn commit(&mut self) -> Result<(), String> {
        if !self.dirty { return Ok(()); }
        let gate = Arc::clone(&self.writer_gate);
        let _writer = gate.lock().map_err(|e| e.to_string())?;
        self.commit_with_gate()
    }

    /// Called by the complete-tree filesystem while its writer reservation
    /// is already held. Never lock the same non-reentrant Mutex twice.
    fn commit_with_gate(&mut self) -> Result<(), String> {
        if !self.dirty { return Ok(()); }
        self.vault.reload_after_mount()?;
        // Do not replace a different file that happens to occupy this name.
        if self.vault.find_child(self.parent, &self.name)?.is_none_or(|item| item.id != self.id) {
            return Err("Encrypted file changed while open; unsaved edits retained".into());
        }
        // The transaction's old authenticated index supplies original
        // references. No full index fork or second writer cursor is needed.
        let new_id = self.vault.begin_stream_file(self.parent, &self.name, true)?;
        let result = (|| -> Result<(), String> {
            let blocks = self.size.div_ceil(PAGE);
            let original_visible = self.original_visible_size();
            self.clear_read_cache();
            for index in 0..blocks {
                let length = (self.size - index * PAGE).min(PAGE) as usize;
                if let Some(page) = self.pages.get_mut(&index) {
                    // Borrow dirty bytes directly. Do not copy another 8 MiB.
                    resize_plain_page(page, length);
                    self.vault.stream_write_chunk(new_id, index as i64, page)?;
                } else if let Some(staged) = self.staged_pages.get(&index) {
                    if staged.valid == length && staged.chunk.len() == length {
                        self.vault.stream_use_staged_chunk(new_id, index as i64, &staged.chunk)?;
                    } else {
                        let mut plain = self.vault.read_staged_chunk(&staged.chunk)?;
                        let keep = staged.valid.min(length);
                        if keep < plain.len() {
                            plain[keep..].zeroize();
                            plain.truncate(keep);
                        }
                        resize_plain_page(&mut plain, length);
                        self.vault.stream_write_chunk(new_id, index as i64, &plain)?;
                    }
                } else if index * PAGE + length as u64 <= original_visible
                    && self.vault.stream_reuse_previous_chunk(new_id,index as i64,self.id,length)? {
                    // Authenticate and reference, rather than re-encrypt,
                    // the unchanged original record at its immutable offset.
                } else {
                    let keep = original_visible.saturating_sub(index * PAGE).min(length as u64) as usize;
                    let mut plain = if keep > 0 {
                        self.vault.stream_read_previous_chunk(self.id, index as i64)?
                            .unwrap_or_else(|| Zeroizing::new(Vec::new()))
                    } else { Zeroizing::new(Vec::new()) };
                    if keep < plain.len() {
                        plain[keep..].zeroize();
                        plain.truncate(keep);
                    }
                    resize_plain_page(&mut plain, length);
                    self.vault.stream_write_chunk(new_id, index as i64, &plain)?;
                }
            }
            self.vault.commit_stream_file()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = self.vault.rollback_stream_file();
            self.last_error = Some(error.clone());
            return Err(error);
        }
        self.id = new_id;
        self.original_size = self.size;
        self.pages.clear();
        self.staged_pages.clear();
        self.clear_read_cache();
        self.truncate_floor = None;
        self.dirty = false;
        self.last_error = None;
        self.saved_revision = self.saved_revision.wrapping_add(1);
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Handle { Root, File }

struct SingleFileFs { state: Arc<Mutex<MountData>> }

impl SingleFileFs {
    fn is_file(&self, path: &U16CStr) -> bool {
        let name = path.to_string_lossy();
        let Some(stripped) = name.strip_prefix('\\') else { return false };
        !stripped.contains('\\') && ndr::vault::names_equal(stripped, &self.state.lock().unwrap().name)
    }
    fn info(&self, file: bool) -> FileInfo {
        let size = if file { self.state.lock().unwrap().size } else { 0 };
        FileInfo {
            file_attributes: if file { 0x20 } else { 0x10 },
            file_size: size,
            allocation_size: size.div_ceil(4096) * 4096,
            index_number: if file { 2 } else { 1 },
            ..FileInfo::default()
        }
    }
}

impl FileSystemContext for SingleFileFs {
    type FileContext = Handle;
    fn get_security_by_name(&self, file_name: &U16CStr,
        _descriptor: Option<&mut [c_void]>,
        _resolve: impl FnOnce(&U16CStr) -> Option<FileSecurity>) -> winfsp::Result<FileSecurity> {
        let file = if file_name.to_string_lossy() == "\\" { false }
            else if self.is_file(file_name) { true }
            else { return Err(fs_error(NOT_FOUND)); };
        Ok(FileSecurity { reparse: false, sz_security_descriptor: 0,
            attributes: if file { 0x20 } else { 0x10 } })
    }
    fn open(&self, file_name: &U16CStr, _options: u32,
        _access: u32,
        file_info: &mut OpenFileInfo) -> winfsp::Result<Handle> {
        let handle = if file_name.to_string_lossy() == "\\" { Handle::Root }
            else if self.is_file(file_name) { Handle::File }
            else { return Err(fs_error(NOT_FOUND)); };
        let mut state = self.state.lock().map_err(|_| fs_error(INTERNAL_ERROR))?;
        if state.closing { return Err(fs_error(ACCESS_DENIED)); }
        if matches!(handle, Handle::File) {
            state.open_files = state.open_files.saturating_add(1);
        }
        drop(state);
        *file_info.as_mut() = self.info(matches!(handle, Handle::File));
        Ok(handle)
    }
    fn close(&self, handle: Handle) {
        if matches!(handle, Handle::File) {
            let mut state = self.state.lock().unwrap();
            if let Err(error) = state.commit() { state.last_error = Some(error); }
            state.open_files = state.open_files.saturating_sub(1);
        }
    }
    fn get_file_info(&self, context: &Handle, output: &mut FileInfo) -> winfsp::Result<()> {
        *output = self.info(matches!(context, Handle::File)); Ok(())
    }
    fn read(&self, context: &Handle, buffer: &mut [u8], offset: u64) -> winfsp::Result<u32> {
        if !matches!(context, Handle::File) { return Err(fs_error(ACCESS_DENIED)); }
        let done = self.state.lock().unwrap().read_at(buffer, offset)
            .map_err(|_| fs_error(INTERNAL_ERROR))?;
        if done == 0 { Err(fs_error(END_OF_FILE)) } else { Ok(done as u32) }
    }
    fn write(&self, context: &Handle, buffer: &[u8], offset: u64,
        write_to_eof: bool, constrained_io: bool,
        info: &mut FileInfo) -> winfsp::Result<u32> {
        if !matches!(context, Handle::File) { return Err(fs_error(ACCESS_DENIED)); }
        let mut state = self.state.lock().unwrap();
        let start = if write_to_eof { state.size } else { offset };
        let max_length = if constrained_io {
            state.size.saturating_sub(start).min(buffer.len() as u64) as usize
        } else { buffer.len() };
        let written = state.write_at(&buffer[..max_length], start)
            .map_err(|_| fs_error(INTERNAL_ERROR))?;
        drop(state);
        *info = self.info(true);
        Ok(written as u32)
    }
    fn set_file_size(&self, context: &Handle, new_size: u64,
        allocation_only: bool, info: &mut FileInfo) -> winfsp::Result<()> {
        if !matches!(context, Handle::File) { return Err(fs_error(ACCESS_DENIED)); }
        if new_size > (1u64 << 40) { return Err(fs_error(ACCESS_DENIED)); }
        if allocation_only { *info = self.info(true); return Ok(()); }
        let mut state = self.state.lock().unwrap();
        state.resize(new_size);
        drop(state);
        *info = self.info(true); Ok(())
    }
    fn overwrite(&self, context: &Handle,
        _attributes: u32, _replace_attributes: bool, _allocation_size: u64,
        _extra: Option<&[u8]>, info: &mut FileInfo) -> winfsp::Result<()> {
        if !matches!(context, Handle::File) { return Err(fs_error(ACCESS_DENIED)); }
        let mut state = self.state.lock().unwrap();
        state.overwrite();
        drop(state);
        *info = self.info(true); Ok(())
    }
    fn set_basic_info(&self, context: &Handle, _attributes: u32,
        _created: u64, _accessed: u64, _written: u64, _changed: u64,
        info: &mut FileInfo) -> winfsp::Result<()> {
        *info = self.info(matches!(context, Handle::File)); Ok(())
    }
    fn flush(&self, context: Option<&Handle>, info: &mut FileInfo) -> winfsp::Result<()> {
        if context.is_some_and(|h| matches!(h, Handle::File)) {
            self.state.lock().unwrap().commit().map_err(|_| fs_error(INTERNAL_ERROR))?;
        }
        *info = self.info(true); Ok(())
    }
    fn read_directory(&self, context: &Handle, _pattern: Option<&U16CStr>,
        marker: DirMarker, buffer: &mut [u8]) -> winfsp::Result<u32> {
        if !matches!(context, Handle::Root) { return Err(fs_error(ACCESS_DENIED)); }
        let state = self.state.lock().unwrap();
        let name = &state.name;
        if marker.inner_as_cstr().is_some_and(|cursor| cursor.to_string_lossy().eq_ignore_ascii_case(name)) {
            return Ok(0);
        }
        let mut entry = DirInfo::<255>::new();
        *entry.file_info_mut() = FileInfo { file_attributes: 0x20, file_size: state.size,
            allocation_size: state.size.div_ceil(4096)*4096, index_number: 2,
            ..FileInfo::default() };
        let wide: Vec<u16> = name.encode_utf16().collect();
        entry.set_name_raw(wide.as_slice())?;
        let mut cursor = 0u32;
        let _ = entry.append_to_buffer(buffer, &mut cursor);
        Ok(cursor)
    }
    fn get_volume_info(&self, info: &mut VolumeInfo) -> winfsp::Result<()> {
        info.total_size = 1u64 << 40;
        info.free_size = (1u64 << 40).saturating_sub(self.state.lock().unwrap().size);
        info.set_volume_label("NDR Vault"); Ok(())
    }
}

/// RAII mount. The original pane owns the exclusive gate while this
/// authenticated in-process fork is active. Drop stops kernel callbacks.
pub(super) struct VaultMount {
    host: FileSystemHost<SingleFileFs, FineGuard>,
    state: Arc<Mutex<MountData>>,
    pub file_path: std::path::PathBuf,
    pub vault_path: std::path::PathBuf,
    pub pane: usize,
    pub item_name: String,
    pub parent_id: i64,
    observed_revision: u64,
    error_reported: bool,
}

impl VaultMount {
    pub fn start(pane: usize, vault_path: std::path::PathBuf,
        original: &Vault, parent: i64, item: VaultItem,
        writer_gate: Arc<Mutex<()>>, occupied: &[char]) -> Result<Self, String> {
        if !runtime_available() {
            return Err("WinFsp 2.1 runtime/driver is not installed".to_owned());
        }
        let drives = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        let letter = (b'T'..=b'Z').rev().find(|&c|
            drives & (1u32 << (c - b'A')) == 0 && !occupied.contains(&(c as char)))
            .ok_or("No free drive letter for WinFsp encrypted view")?;
        let mountpoint = format!("{}:", letter as char);
        let file_path = std::path::PathBuf::from(format!("{}\\{}", mountpoint, item.name));
        let state = Arc::new(Mutex::new(MountData {
            vault: original.fork_for_mount()?, id: item.id, parent, name: item.name.clone(),
            size: item.size.max(0) as u64, original_size: item.size.max(0) as u64,
            pages: BTreeMap::new(), staged_pages: BTreeMap::new(), read_cache: None, previous_read: None, truncate_floor: None, dirty: false, last_error: None, saved_revision: 0, open_files: 0, closing: false, writer_gate,
        }));
        let mut params = VolumeParams::new();
        params.filesystem_name("NDRVAULT").sector_size(512)
            .sectors_per_allocation_unit(8).case_sensitive_search(false)
            .case_preserved_names(true).unicode_on_disk(true)
            .persistent_acls(false).file_info_timeout(0)
            .flush_and_purge_on_cleanup(true);
        let config = FileSystemParams { use_dir_info_by_name: false,
            volume_params: params, debug_mode: DebugMode::none() };
        let mut host = FileSystemHost::<SingleFileFs, FineGuard>::new_with_options(
            config, SingleFileFs { state: Arc::clone(&state) })
            .map_err(|e| format!("WinFsp create failed: {e}"))?;
        host.mount(&mountpoint).map_err(|e| format!("WinFsp mount failed: {e}"))?;
        host.start().map_err(|e| format!("WinFsp start failed: {e}"))?;
        Ok(Self { host, state, file_path, vault_path, pane, item_name: item.name.clone(), parent_id: parent, observed_revision: 0, error_reported: false })
    }
    /// The driver flushes edited bytes to the encrypted container on close
    /// and flush, without a separate UI confirmation. The pane may refresh
    /// after each successful commit while the virtual file remains mounted.
    pub fn take_saved_revision(&mut self) -> Result<bool, String> {
        poll_revision(&self.state, &mut self.observed_revision, |state| state.saved_revision)
    }

    pub fn take_save_error_notice(&mut self) -> bool {
        poll_error_notice(&self.state, &mut self.error_reported, |state| state.last_error.is_some())
    }

    pub fn writer_gate(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.state.lock().unwrap().writer_gate)
    }

    pub fn drive_letter(&self) -> Option<char> {
        self.file_path.to_string_lossy().chars().next()
    }

    #[cfg(test)]
    pub fn state_saved_revision_for_test(&self) -> u64 {
        self.state.lock().map_or(0, |state| state.saved_revision)
    }

    pub fn backing_removed(&self) -> bool {
        self.state.try_lock().is_ok_and(|state| state.vault.backing_removed())
    }
    pub fn revoke(&self) {
        if let Ok(mut state) = self.state.lock() { state.closing = true; }
    }
    pub fn is_revoked(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.closing)
    }
    pub fn has_open_files(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.open_files > 0)
    }

    pub fn finish(&mut self) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|e| e.to_string())?;
        if state.open_files != 0 {
            return Err("Close the external editor before disconnecting the encrypted virtual file".into());
        }
        state.commit()?;
        if let Some(error) = &state.last_error { return Err(error.clone()); }
        state.closing = true;
        drop(state);
        self.host.stop();
        Ok(())
    }
}

/// The official WinFsp installer is embedded in the NDR executable. Users
/// explicitly approve the system-driver installation through Windows UAC.
pub(super) fn launch_embedded_installer() -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    const INSTALLER: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/winfsp-2.1.25156.msi"));
    const CHECKSUM: &str = "073A70E00F77423E34BED98B86E600DEF93393BA5822204FAC57A29324DB9F7A";
    let actual = Sha256::digest(INSTALLER);
    let actual = actual.iter().map(|v| format!("{v:02X}")).collect::<String>();
    if actual != CHECKSUM { return Err("Embedded WinFsp installer hash mismatch".into()); }
    let mut random = [0u8; 16];
    openssl::rand::rand_bytes(&mut random).map_err(|e| e.to_string())?;
    let nonce = random.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let root = std::env::temp_dir().join(format!("ndr-winfsp-setup-{nonce}"));
    std::fs::create_dir(&root).map_err(|e| e.to_string())?;
    let path = root.join("winfsp-2.1.25156.msi");
    let mut output = std::fs::OpenOptions::new().create_new(true).write(true)
        .open(&path).map_err(|e| e.to_string())?;
    output.write_all(INSTALLER).map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    drop(output);
    let args = format!("/i \"{}\" /passive /norestart", path.display());
    ndr::native::shell_open(std::path::Path::new("msiexec.exe"), Some(&args), "runas")
        .map_err(|e| format!("WinFsp installer was cancelled or elevation failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires installed WinFsp driver and a free drive letter; opt in for real mount smoke"]
    fn mounted_virtual_file_round_trips_through_windows_io_without_a_close_dialog() {
        let root = std::env::temp_dir().join(format!("ndr-winfsp-real-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("test.ndr");
        let mut original = Vault::create(&path, "real mount disposable password".into()).unwrap();
        let id = original.add_file(1, "edit.txt", b"before").unwrap();
        let item = original.find_child(1, "edit.txt").unwrap().unwrap();
        assert_eq!(item.id, id);
        let mut mount = VaultMount::start(0, path.clone(), &original, 1, item,
            Arc::new(Mutex::new(())), &[]).unwrap();
        assert_eq!(std::fs::read(&mount.file_path).unwrap(), b"before");
        std::fs::write(&mount.file_path, b"after-save").unwrap();
        // Kernel Close callbacks may finish *after* the user-mode write
        // handle has been dropped. Never assume a synchronous close.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while mount.has_open_files() || mount.state.lock().unwrap().saved_revision == 0 {
            assert!(std::time::Instant::now() < until,
                "WinFsp close/commit did not complete within 10s (handles={}, revision={})",
                mount.state.lock().unwrap().open_files,
                mount.state.lock().unwrap().saved_revision);
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert_eq!(mount.state.lock().unwrap().saved_revision, 1);
        assert!(mount.take_saved_revision().unwrap());
        assert!(!mount.take_saved_revision().unwrap());
        mount.finish().unwrap();
        drop(mount);
        original.reload_after_mount().unwrap();
        let updated = original.find_child(1, "edit.txt").unwrap().unwrap();
        assert_eq!(original.read_file(updated.id).unwrap(), b"after-save");
        drop(original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires installed WinFsp driver and two free letters; opt-in real mount test"]
    fn two_simultaneously_mounted_files_preserve_both_encrypted_commits() {
        let root = std::env::temp_dir().join(format!("ndr-winfsp-dual-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("dual.ndr");
        let mut original = Vault::create(&path, "dual mount disposable password".into()).unwrap();
        original.add_file(1, "first.txt", b"initial first").unwrap();
        original.add_file(1, "second.txt", b"initial second").unwrap();
        let gate = Arc::new(Mutex::new(()));
        let first = original.find_child(1, "first.txt").unwrap().unwrap();
        let mut mount_first = VaultMount::start(0, path.clone(), &original, 1, first,
            Arc::clone(&gate), &[]).unwrap();
        let second = original.find_child(1, "second.txt").unwrap().unwrap();
        let mut mount_second = VaultMount::start(0, path.clone(), &original, 1, second,
            Arc::clone(&gate), &[mount_first.drive_letter().unwrap()]).unwrap();
        assert_ne!(mount_first.drive_letter(), mount_second.drive_letter());
        assert_eq!(std::fs::read(&mount_first.file_path).unwrap(), b"initial first");
        assert_eq!(std::fs::read(&mount_second.file_path).unwrap(), b"initial second");
        for (mount, replacement) in [(&mut mount_second, b"second changed".as_slice()),
                                      (&mut mount_first, b"first changed".as_slice())] {
            std::fs::write(&mount.file_path, replacement).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while mount.has_open_files() || mount.state.lock().unwrap().saved_revision == 0 {
                assert!(std::time::Instant::now() < deadline, "WinFsp close/commit timeout");
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            assert!(mount.take_saved_revision().unwrap());
        }
        mount_second.finish().unwrap();
        mount_first.finish().unwrap();
        drop(mount_first);
        drop(mount_second);
        original.reload_after_mount().unwrap();
        for (name, contents) in [("first.txt", b"first changed".as_slice()),
                                 ("second.txt", b"second changed".as_slice())] {
            let item = original.find_child(1, name).unwrap().unwrap();
            assert_eq!(original.read_file(item.id).unwrap(), contents,
                "one mount must not overwrite the other file's metadata");
        }
        drop(original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn virtual_file_commits_without_plaintext_disk_copy() {
        let root = std::env::temp_dir().join(format!("ndr-winfsp-pages-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("private.ndr");
        let mut original = Vault::create(&path, "a safe test password".into()).unwrap();
        let plain = b"original picture bytes";
        let id = original.add_file(1, "picture.png", plain).unwrap();
        let mut mounted = MountData {
            vault: original.fork_for_mount().unwrap(), id, parent: 1, name: "picture.png".into(),
            size: plain.len() as u64, original_size: plain.len() as u64,
            pages: BTreeMap::new(), staged_pages: BTreeMap::new(), read_cache: None, previous_read: None, truncate_floor: None, dirty: false, last_error: None, saved_revision: 0, open_files: 0, closing: false, writer_gate: Arc::new(Mutex::new(())),
        };
        let mut before = vec![0; plain.len()];
        mounted.read_at(&mut before, 0).unwrap();
        assert_eq!(&before, plain);
        assert_eq!(mounted.read_cache.as_ref().map(|(index, _)| *index), Some(0));
        let mut repeated = vec![0; plain.len()];
        mounted.read_at(&mut repeated, 0).unwrap();
        assert_eq!(repeated, before);
        mounted.write_at(b"replaced", 0).unwrap();
        assert_eq!(mounted.saved_revision, 0, "unsaved writes must not notify the pane");
        mounted.commit().unwrap();
        assert_eq!(mounted.saved_revision, 1, "successful encrypted commit notifies the pane");
        mounted.commit().unwrap();
        assert_eq!(mounted.saved_revision, 1, "no new writes means no redundant UI refresh");
        original.reload_after_mount().unwrap();
        let updated = original.find_child(1, "picture.png").unwrap().unwrap();
        assert_eq!(original.read_file(updated.id).unwrap(), b"replaced picture bytes");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        drop(mounted);
        drop(original);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn batch384_dirty_pages_are_bounded_and_truncation_retains_later_writes() {
        let root = std::env::temp_dir().join(format!("ndr-winfsp-b384-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("test.ndr");
        let mut vault = Vault::create(&path, "test bounded staging password".into()).unwrap();
        let id = vault.add_file(1,"large.bin",&[]).unwrap();
        let mut file = MountData {
            vault:vault.fork_for_mount().unwrap(),id,parent:1,name:"large.bin".into(),
            size:0,original_size:0,pages:BTreeMap::new(),staged_pages:BTreeMap::new(),
            read_cache:None,previous_read:None,truncate_floor:None,dirty:false,last_error:None,
            saved_revision:0,open_files:0,closing:false,writer_gate:Arc::new(Mutex::new(())),
        };
        let page=vec![71;STREAM_CHUNK_SIZE];
        for i in 0..5u64 {
            assert_eq!(file.write_at(&page,i*PAGE).unwrap(),page.len());
            assert!(file.pages.len()<=DIRTY_PAGE_LIMIT);
        }
        assert_eq!(file.staged_pages.len(),3);
        vault.reload_after_mount().unwrap();
        assert_eq!(vault.stream_file_size(id).unwrap(),0,"uncommitted spill must not publish metadata");
        file.resize(PAGE+8);
        file.resize(3*PAGE);
        file.write_at(b"preserved",2*PAGE+4).unwrap();
        file.resize(2*PAGE+32);
        file.resize(5*PAGE);
        let mut read=vec![0;64];
        file.read_at(&mut read,PAGE).unwrap();
        assert_eq!(&read[..8],&[71;8]);assert_eq!(&read[8..],&[0;56]);
        file.read_at(&mut read,2*PAGE).unwrap();
        assert_eq!(&read[4..13],b"preserved");assert_eq!(&read[13..],&[0;51]);
        file.commit().unwrap();
        assert!(file.pages.is_empty()&&file.staged_pages.is_empty());
        vault.reload_after_mount().unwrap();vault.verify_integrity().unwrap();
        assert_eq!(vault.stream_file_size(file.id).unwrap(),5*PAGE);
        let third=vault.stream_read_chunk(file.id,2).unwrap().unwrap();
        assert_eq!(&third[4..13],b"preserved");assert!(third[13..].iter().all(|&b|b==0));
        drop((file,vault));std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shrinking_then_growing_file_does_not_restore_truncated_plaintext() {
        let root = std::env::temp_dir().join(format!("ndr-winfsp-truncate-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("private.ndr");
        let mut original = Vault::create(&path, "safe test password".into()).unwrap();
        let id = original.add_file(1, "video.bin", b"ABCDEFGHIJKLMNOP").unwrap();
        let state = Arc::new(Mutex::new(MountData {
            vault: original.fork_for_mount().unwrap(), id, parent: 1, name: "video.bin".into(),
            size: 16, original_size: 16, pages: BTreeMap::new(), staged_pages: BTreeMap::new(), read_cache: None, previous_read: None,
            truncate_floor: None, dirty: false, last_error: None, saved_revision: 0, open_files: 0, closing: false, writer_gate: Arc::new(Mutex::new(())),
        }));
        let fs = SingleFileFs { state: Arc::clone(&state) };
        let mut info = FileInfo::default();
        fs.set_file_size(&Handle::File, 3, false, &mut info).unwrap();
        fs.set_file_size(&Handle::File, 16, false, &mut info).unwrap();
        let mut bytes = [0u8;16];
        state.lock().unwrap().read_at(&mut bytes, 0).unwrap();
        assert_eq!(&bytes[..3], b"ABC");
        assert_eq!(&bytes[3..], &[0u8;13]);
        state.lock().unwrap().commit().unwrap();
        original.reload_after_mount().unwrap();
        let found = original.find_child(1, "video.bin").unwrap().unwrap();
        assert_eq!(original.read_file(found.id).unwrap(), bytes);
        drop(fs);
        drop(state);
        drop(original);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn overwritten_virtual_file_does_not_retain_old_content() {
        let root = std::env::temp_dir().join(format!("ndr-winfsp-overwrite-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("private.ndr");
        let mut vault = Vault::create(&path, "safe test password".into()).unwrap();
        let original = b"private original plaintext";
        let id = vault.add_file(1, "photo.png", original).unwrap();
        let state = Arc::new(Mutex::new(MountData {
            vault: vault.fork_for_mount().unwrap(), id, parent: 1, name: "photo.png".into(),
            size: original.len() as u64, original_size: original.len() as u64,
            pages: BTreeMap::new(), staged_pages: BTreeMap::new(), read_cache: None, previous_read: None, truncate_floor: None, dirty: false, last_error: None, saved_revision: 0, open_files: 0, closing: false, writer_gate: Arc::new(Mutex::new(())),
        }));
        let fs = SingleFileFs { state: Arc::clone(&state) };
        let mut info = FileInfo::default();
        fs.overwrite(&Handle::File, 0, false, 0, None, &mut info).unwrap();
        fs.write(&Handle::File, b"new", 0, false, false, &mut info).unwrap();
        state.lock().unwrap().commit().unwrap();
        vault.reload_after_mount().unwrap();
        let updated = vault.find_child(1, "photo.png").unwrap().unwrap();
        assert_eq!(vault.read_file(updated.id).unwrap(), b"new");
        drop(fs);
        drop(state);
        drop(vault);
        std::fs::remove_dir_all(root).unwrap();
    }
}

mod tree;
pub(super) use tree::{VaultTreeMount, TransferReservation};
