//! Bounded, per-pane browsing index. Never use a browsing snapshot for
//! extraction, mutation, password validation or conflict decisions.
use super::{ArchiveLocation, Entry};
use ndr::archive::ArchiveEntry;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    time::SystemTime,
};

const CACHE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Stamp {
    path: PathBuf,
    volume: u32,
    file_id: u64,
    size: u64,
    creation: i64,
    write: i64,
    change: i64,
}

/// Keep the archive stable during lookup/listing, not for the lifetime of
/// the pane. Unsupported filesystems, multi-volume or encrypted sessions
/// take the original cancellable decoder path instead.
fn guarded_stamp(path: &Path) -> Option<(File, Stamp)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FileBasicInfo,
        GetFileInformationByHandle, GetFileInformationByHandleEx,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
    };
    if !ndr::listing_cache::supported_local_path(path) { return None; }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "7z" | "zip" | "tar" | "gz" | "bz2" | "xz") { return None; }
    ndr::fsops::ensure_plain_path(path).ok()?;
    let path = fs::canonicalize(path).ok()?;
    if ext == "zip" {
        // Split ZIP archives may have an unchanged central directory but
        // changed earlier volumes. Don't cache them.
        for extension in ["z01", "Z01"] {
            match fs::symlink_metadata(path.with_extension(extension)) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                _ => return None,
            }
        }
    }
    let file = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || ndr::fsops::is_reparse(&metadata) { return None; }
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let mut times: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
        || unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), FileBasicInfo,
            (&mut times as *mut FILE_BASIC_INFO).cast(), std::mem::size_of::<FILE_BASIC_INFO>() as u32) } == 0 {
        return None;
    }
    let id = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    if id == 0 || times.ChangeTime == 0 { return None; }
    let stamp = Stamp { path, volume: info.dwVolumeSerialNumber, file_id: id,
        size: metadata.len(), creation: times.CreationTime, write: times.LastWriteTime, change: times.ChangeTime };
    Some((file, stamp))
}

pub(super) struct ArchiveBrowseSnapshot {
    stamp: Option<Stamp>,
    entries: Vec<ArchiveEntry>,
    /// Sorted offsets, not duplicate paths. The original enumeration
    /// order is used to preserve implicit folder timestamps.
    order: Vec<u32>,
    skipped: usize,
}

impl ArchiveBrowseSnapshot {
    fn new(mut entries: Vec<ArchiveEntry>, skipped: usize, stamp: Option<Stamp>) -> Self {
        for entry in &mut entries {
            if entry.key.contains('\\') { entry.key = entry.key.replace('\\', "/"); }
        }
        let bytes = entries.capacity().saturating_mul(std::mem::size_of::<ArchiveEntry>() + 4)
            .saturating_add(entries.iter().map(|e| e.key.capacity()).sum::<usize>());
        let stamp = stamp.filter(|_| bytes <= CACHE_BYTES && u32::try_from(entries.len()).is_ok());
        let mut order = if stamp.is_some() { (0..entries.len() as u32).collect::<Vec<_>>() } else { Vec::new() };
        order.sort_unstable_by(|&a, &b| entries[a as usize].key.cmp(&entries[b as usize].key));
        Self { stamp, entries, order, skipped }
    }

    fn project(&self, prefix: &str, cancel: &AtomicBool) -> Result<Vec<Entry>, String> {
        if prefix.is_empty() || self.order.is_empty() {
            return project_rows(self.entries.iter(), prefix, None, cancel);
        }
        let current = prefix.trim_end_matches('/');
        let key = |i: u32| self.entries[i as usize].key.as_str();
        let own = self.order.binary_search_by(|&i| key(i).cmp(current)).ok()
            .map(|i| self.entries[self.order[i] as usize].modified);
        let start = self.order.partition_point(|&i| key(i) < prefix);
        let count = self.order[start..].partition_point(|&i| key(i).starts_with(prefix));
        // Deep folders normally match only a tiny fraction of the full
        // archive. For large subtrees a linear scan is cheaper than sorting.
        if count > self.entries.len() / 3 {
            return project_rows(self.entries.iter(), prefix, None, cancel);
        }
        let mut indices = self.order[start..start + count].to_vec();
        indices.sort_unstable();
        project_rows(indices.into_iter().map(|i| &self.entries[i as usize]), prefix, own, cancel)
    }
}

fn project_rows<'a>(
    entries: impl Iterator<Item = &'a ArchiveEntry>,
    prefix: &str,
    mut own_time: Option<Option<SystemTime>>,
    cancel: &AtomicBool,
) -> Result<Vec<Entry>, String> {
    let current = prefix.trim_end_matches('/');
    let mut first_below = None;
    let mut rows = BTreeMap::<&str, Entry>::new();
    for (i, entry) in entries.enumerate() {
        if i % 128 == 0 && cancel.load(Ordering::Relaxed) { return Err("Cancelled".into()); }
        if !current.is_empty() && entry.key == current { own_time = Some(entry.modified); }
        let Some(rest) = entry.key.strip_prefix(prefix) else { continue; };
        if !current.is_empty() && first_below.is_none() { first_below = Some(entry.modified); }
        let (name, nested) = rest.split_once('/').map_or((rest, false), |(name, _)| (name, true));
        if name.is_empty() { continue; }
        let folder = nested || entry.is_dir;
        let slot = rows.entry(name).or_insert_with(|| Entry {
            path: PathBuf::from(format!("{prefix}{name}")),
            name: name.to_owned(), is_dir: folder,
            size: if folder { 0 } else { entry.size },
            modified: entry.modified, hidden: false,
            attributes: if folder { 0x10 } else { 0 }, drive: false,
        });
        if !nested { slot.modified = entry.modified; }
    }
    let parent = own_time.or(first_below);
    let mut listed = Vec::with_capacity(rows.len() + usize::from(parent.is_some()));
    if let Some(time) = parent { listed.push(Entry::parent_item(PathBuf::from(".."), time)); }
    listed.extend(rows.into_values());
    Ok(listed)
}

pub(super) fn read(
    location: &ArchiveLocation,
    cache: &Mutex<Option<Arc<ArchiveBrowseSnapshot>>>,
    allow_cache: bool,
    reuse: bool,
    cancel: &AtomicBool,
) -> Result<(Vec<Entry>, usize), String> {
    let guard = (allow_cache && location.password.is_empty()).then(|| guarded_stamp(&location.file)).flatten();
    if cancel.load(Ordering::Relaxed) { return Err("Cancelled".into()); }
    if reuse {
        let hit = cache.lock().ok().and_then(|current| {
            let current = current.as_ref()?;
            let stamp = &guard.as_ref()?.1;
            (current.stamp.as_ref() == Some(stamp)).then(|| current.clone())
        });
        if let Some(hit) = hit { return hit.project(&location.prefix, cancel).map(|rows| (rows, hit.skipped)); }
    }
    if let Ok(mut current) = cache.lock() {
        if !cancel.load(Ordering::Relaxed) { *current = None; }
    }
    let password = (!location.password.is_empty()).then_some(location.password.as_str());
    // When a guard is available, pass that exact canonical object to the
    // decoder. Do not associate an alias that changed with the old stamp.
    let decode_path = guard.as_ref().map_or(location.file.as_path(), |(_, stamp)| stamp.path.as_path());
    let (entries, skipped) = ndr::archive::list_with_skipped_devices_cancel(decode_path, password, cancel)?;
    if cancel.load(Ordering::Relaxed) { return Err("Cancelled".into()); }
    let stamp = guard.as_ref().and_then(|(_, before)| {
        guarded_stamp(&location.file).and_then(|(_, after)| (after == *before).then(|| before.clone()))
    });
    let snapshot = Arc::new(ArchiveBrowseSnapshot::new(entries, skipped.len(), stamp));
    let rows = snapshot.project(&location.prefix, cancel)?;
    if snapshot.stamp.is_some() {
        if let Ok(mut current) = cache.lock() {
            if !cancel.load(Ordering::Relaxed) { *current = Some(snapshot.clone()); }
        }
    }
    Ok((rows, snapshot.skipped))
}

#[cfg(test)]
#[test]
fn batch429_archive_index_matches_original_order_and_folder_times() {
    let make = |name: &str, dir: bool, time: u64| ArchiveEntry {
        key: name.into(), is_dir: dir, size: 7, size_known: true,
        modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(time)),
    };
    let entries = vec![make("a\\z\\later", false, 1), make("b/q", false, 2),
        make("a/z", true, 3), make("a/z/earlier", false, 4), make("a/one", false, 5),
        make("c/x", false, 6), make("d/x", false, 7), make("e/x", false, 8)];
    let stamp = Stamp { path: "a.7z".into(), volume: 1, file_id: 1, size: 1, creation: 1, write: 1, change: 1 };
    let plain = ArchiveBrowseSnapshot::new(entries.clone(), 0, None);
    let indexed = ArchiveBrowseSnapshot::new(entries, 0, Some(stamp));
    let cancel = AtomicBool::new(false);
    for prefix in ["", "a/", "a/z/", "b/", "missing/"] {
        let a = plain.project(prefix, &cancel).unwrap();
        let b = indexed.project(prefix, &cancel).unwrap();
        assert_eq!(a.len(), b.len());
        for (a, b) in a.iter().zip(b) {
            assert_eq!((&a.name, &a.path, a.is_dir, a.size, a.modified), (&b.name, &b.path, b.is_dir, b.size, b.modified));
        }
    }
}

#[cfg(test)]
#[test]
fn batch428_archive_browsing_stamp_tracks_replacement_and_refresh() {
    let root = std::env::temp_dir().join(format!("ndr-archive-browse-{}-{}", std::process::id(),
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(root.join("source/inner")).unwrap();
    fs::write(root.join("source/inner/a.txt"), b"first").unwrap();
    let file = root.join("test.7z");
    let cancel = AtomicBool::new(false);
    ndr::archive::create(&[root.join("source")], &file, None, &cancel).unwrap();
    let mut location = ArchiveLocation { file: file.clone(), prefix: String::new(), password: String::new() };
    let cache = Mutex::new(None);
    assert_eq!(read(&location, &cache, true, false, &cancel).unwrap().0[0].name, "source");
    let before = cache.lock().unwrap().clone();
    location.prefix = "source/inner/".into();
    let rows = read(&location, &cache, true, true, &cancel).unwrap().0;
    assert!(rows.iter().any(|row| row.name == "a.txt"));
    if let (Some(old), Some((_, stamp))) = (before.as_ref(), guarded_stamp(&file)) {
        if old.stamp.as_ref() == Some(&stamp) {
            assert!(Arc::ptr_eq(old, cache.lock().unwrap().as_ref().unwrap()));
        }
    }
    fs::remove_file(root.join("source/inner/a.txt")).unwrap();
    fs::write(root.join("source/inner/b.txt"), b"other").unwrap();
    let replacement = root.join("replacement.7z");
    ndr::archive::create(&[root.join("source")], &replacement, None, &cancel).unwrap();
    fs::remove_file(&file).unwrap();
    fs::rename(&replacement, &file).unwrap();
    let rows = read(&location, &cache, true, true, &cancel).unwrap().0;
    assert!(rows.iter().any(|row| row.name == "b.txt"));
    assert!(!rows.iter().any(|row| row.name == "a.txt"));
    let rows = read(&location, &cache, true, false, &cancel).unwrap().0;
    assert!(rows.iter().any(|row| row.name == "b.txt"));
    drop(cache);
    fs::remove_dir_all(root).unwrap();
}
