//! Experimental WinFsp transport: expose an authenticated vault item to
//! Windows applications without creating any decrypted disk file. Only a
//! single selected item is exposed per mount; the provider is deliberately
//! conservative about filesystem operations it cannot support safely.
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use ndr::vault::{Vault, VaultItem, STREAM_CHUNK_SIZE};
use zeroize::Zeroizing;
use widestring::U16CStr;
use winfsp::{FspError, filesystem::{DirInfo, DirMarker, FileInfo, FileSecurity, FileSystemContext, OpenFileInfo, VolumeInfo, WideNameInfo}, host::{DebugMode, FileSystemHost, FileSystemParams, FineGuard, VolumeParams}};

const PAGE: u64 = STREAM_CHUNK_SIZE as u64;
const NOT_FOUND: i32 = 0xC0000034u32 as i32;
const ACCESS_DENIED: i32 = 0xC0000022u32 as i32;
const END_OF_FILE: i32 = 0xC0000011u32 as i32;
const INTERNAL_ERROR: i32 = 0xC00000E5u32 as i32;

fn fs_error(code: i32) -> FspError { FspError::NTSTATUS(code) }

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

struct MountData {
    vault: Vault,
    id: i64,
    parent: i64,
    name: String,
    size: u64,
    original_size: u64,
    pages: BTreeMap<u64, Zeroizing<Vec<u8>>>,
    read_cache: Option<(u64, Zeroizing<Vec<u8>>)>,
    truncate_floor: Option<u64>,
    dirty: bool,
    last_error: Option<String>,
}

impl MountData {
    fn read_at(&mut self, buffer: &mut [u8], offset: u64) -> Result<usize, String> {
        if offset >= self.size { return Ok(0); }
        let length = buffer.len().min(self.size.saturating_sub(offset) as usize);
        let mut done = 0;
        while done < length {
            let position = offset + done as u64;
            let page = position / PAGE;
            let within = (position % PAGE) as usize;
            let take = (length - done).min(STREAM_CHUNK_SIZE - within);
            buffer[done..done + take].fill(0);
            if let Some(staged) = self.pages.get(&page) {
                if within < staged.len() {
                    let available = take.min(staged.len() - within);
                    buffer[done..done + available].copy_from_slice(&staged[within..within + available]);
                }
            } else if position < self.original_size {
                // Repeated 64 KiB reads of a large video should not decrypt
                // the same 8 MiB GCM record on every filesystem callback.
                if self.read_cache.as_ref().is_none_or(|(cached, _)| *cached != page) {
                    self.read_cache = self.vault.stream_read_chunk(self.id, page as i64)?
                        .map(|data| (page, data));
                }
                if let Some((_, plaintext)) = self.read_cache.as_ref() {
                    if within < plaintext.len() {
                        let available = take.min(plaintext.len() - within)
                            .min(self.truncate_floor.unwrap_or(u64::MAX).saturating_sub(position) as usize);
                        buffer[done..done + available].copy_from_slice(&plaintext[within..within + available]);
                    }
                }
            }
            done += take;
        }
        Ok(done)
    }

    fn write_at(&mut self, buffer: &[u8], offset: u64) -> Result<usize, String> {
        if buffer.is_empty() { return Ok(0); }
        let end = offset.checked_add(buffer.len() as u64).ok_or("File offset overflow")?;
        // Guard against a malicious application requesting arbitrarily huge
        // sparse files and consuming unbounded resources during commit.
        if end > (1u64 << 40) { return Err("Virtual file exceeds 1 TiB safety limit".into()); }
        let mut done = 0;
        while done < buffer.len() {
            let position = offset + done as u64;
            let page = position / PAGE;
            let within = (position % PAGE) as usize;
            let take = (buffer.len() - done).min(STREAM_CHUNK_SIZE - within);
            if !self.pages.contains_key(&page) {
                let mut original = self.vault.stream_read_chunk(self.id, page as i64)?
                    .map_or_else(|| Zeroizing::new(Vec::new()), |v| v);
                original.resize(STREAM_CHUNK_SIZE, 0);
                if let Some(cutoff) = self.truncate_floor {
                    let from = cutoff.saturating_sub(page * PAGE).min(PAGE) as usize;
                    original[from..].fill(0);
                }
                self.pages.insert(page, original);
            }
            let staged = self.pages.get_mut(&page).unwrap();
            staged[within..within + take].copy_from_slice(&buffer[done..done + take]);
            done += take;
        }
        self.size = self.size.max(end);
        self.dirty = true;
        Ok(done)
    }

    fn commit(&mut self) -> Result<(), String> {
        if !self.dirty { return Ok(()); }
        // The original authenticated chunks must remain available even after
        // begin_stream_file logically replaces the old index entry.
        let snapshot = self.vault.fork_for_mount()?;
        let new_id = self.vault.begin_stream_file(self.parent, &self.name, true)?;
        let result = (|| -> Result<(), String> {
            let blocks = self.size.div_ceil(PAGE);
            for index in 0..blocks {
                let length = (self.size - index * PAGE).min(PAGE) as usize;
                let mut plain = if let Some(page) = self.pages.get(&index) {
                    Zeroizing::new(page[..length].to_vec())
                } else if index * PAGE < self.original_size {
                    snapshot.stream_read_chunk(self.id, index as i64)?
                        .unwrap_or_else(|| Zeroizing::new(Vec::new()))
                } else { Zeroizing::new(Vec::new()) };
                plain.resize(length, 0);
                if let Some(cutoff) = self.truncate_floor {
                    if !self.pages.contains_key(&index) {
                        let from = cutoff.saturating_sub(index * PAGE).min(length as u64) as usize;
                        plain[from..].fill(0);
                    }
                }
                self.vault.stream_write_chunk(new_id, index as i64, &plain)?;
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
        self.read_cache = None;
        self.truncate_floor = None;
        self.dirty = false;
        self.last_error = None;
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
        !stripped.contains('\\') && stripped.eq_ignore_ascii_case(&self.state.lock().unwrap().name)
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
        *file_info.as_mut() = self.info(matches!(handle, Handle::File));
        Ok(handle)
    }
    fn close(&self, handle: Handle) {
        if matches!(handle, Handle::File) {
            let mut state = self.state.lock().unwrap();
            if let Err(error) = state.commit() { state.last_error = Some(error); }
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
        if new_size < state.size {
            let floor = state.truncate_floor.map_or(new_size, |old| old.min(new_size));
            state.truncate_floor = Some(floor);
            for (index, data) in &mut state.pages {
                let from = floor.saturating_sub(index * PAGE).min(PAGE) as usize;
                data[from..].fill(0);
            }
        }
        state.size = new_size;
        state.pages.retain(|index, _| index.saturating_mul(PAGE) < new_size);
        state.dirty = true;
        drop(state);
        *info = self.info(true); Ok(())
    }
    fn overwrite(&self, context: &Handle,
        _attributes: u32, _replace_attributes: bool, _allocation_size: u64,
        _extra: Option<&[u8]>, info: &mut FileInfo) -> winfsp::Result<()> {
        if !matches!(context, Handle::File) { return Err(fs_error(ACCESS_DENIED)); }
        let mut state = self.state.lock().unwrap();
        state.size = 0;
        state.truncate_floor = Some(0);
        state.pages.clear();
        state.dirty = true;
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
}

impl VaultMount {
    pub fn start(pane: usize, vault_path: std::path::PathBuf,
        original: &Vault, parent: i64, item: VaultItem) -> Result<Self, String> {
        if !runtime_available() {
            return Err("WinFsp 2.1 runtime/driver is not installed".to_owned());
        }
        let drives = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        let letter = (b'T'..=b'Z').rev().find(|&c| drives & (1u32 << (c - b'A')) == 0)
            .ok_or("No free drive letter for WinFsp encrypted view")?;
        let mountpoint = format!("{}:", letter as char);
        let file_path = std::path::PathBuf::from(format!("{}\\{}", mountpoint, item.name));
        let state = Arc::new(Mutex::new(MountData {
            vault: original.fork_for_mount()?, id: item.id, parent, name: item.name.clone(),
            size: item.size.max(0) as u64, original_size: item.size.max(0) as u64,
            pages: BTreeMap::new(), read_cache: None, truncate_floor: None, dirty: false, last_error: None,
        }));
        let mut params = VolumeParams::new();
        params.filesystem_name("NDRVAULT").sector_size(512)
            .sectors_per_allocation_unit(8).case_sensitive_search(false)
            .case_preserved_names(true).unicode_on_disk(true)
            .persistent_acls(false).file_info_timeout(0);
        let config = FileSystemParams { use_dir_info_by_name: false,
            volume_params: params, debug_mode: DebugMode::none() };
        let mut host = FileSystemHost::<SingleFileFs, FineGuard>::new_with_options(
            config, SingleFileFs { state: Arc::clone(&state) })
            .map_err(|e| format!("WinFsp create failed: {e}"))?;
        host.mount(&mountpoint).map_err(|e| format!("WinFsp mount failed: {e}"))?;
        host.start().map_err(|e| format!("WinFsp start failed: {e}"))?;
        Ok(Self { host, state, file_path, vault_path, pane })
    }
    pub fn finish(&mut self) -> Result<(), String> {
        self.host.stop();
        let mut state = self.state.lock().map_err(|e| e.to_string())?;
        state.commit()?;
        if let Some(error) = &state.last_error { return Err(error.clone()); }
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
            pages: BTreeMap::new(), read_cache: None, truncate_floor: None, dirty: false, last_error: None,
        };
        let mut before = vec![0; plain.len()];
        mounted.read_at(&mut before, 0).unwrap();
        assert_eq!(&before, plain);
        assert_eq!(mounted.read_cache.as_ref().map(|(index, _)| *index), Some(0));
        let mut repeated = vec![0; plain.len()];
        mounted.read_at(&mut repeated, 0).unwrap();
        assert_eq!(repeated, before);
        mounted.write_at(b"replaced", 0).unwrap();
        mounted.commit().unwrap();
        original.reload_after_mount().unwrap();
        let updated = original.find_child(1, "picture.png").unwrap().unwrap();
        assert_eq!(original.read_file(updated.id).unwrap(), b"replaced picture bytes");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        drop(mounted);
        drop(original);
        std::fs::remove_dir_all(root).unwrap();
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
            size: 16, original_size: 16, pages: BTreeMap::new(), read_cache: None,
            truncate_floor: None, dirty: false, last_error: None,
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
            pages: BTreeMap::new(), read_cache: None, truncate_floor: None, dirty: false, last_error: None,
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
