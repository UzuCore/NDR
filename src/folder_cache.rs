//! Bounded automatic folder-size acceleration. No per-file persistent index:
//! disk records are fixed-size path hashes and last verified totals; live
//! subtree checkpoints own a bounded number of change notification handles.
use crate::{cache, native::{self, FolderSizeWatch}, tools::{FolderSizeSummary, folder_size_summary_known_directory}};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, io::{Read, Write}, os::windows::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}}, path::{Path, PathBuf}, sync::atomic::{AtomicBool, Ordering}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

const MAGIC: &[u8; 8] = b"NDRFSZ01";
const RECORD_BYTES: usize = 68;
pub const DISK_RECORD_LIMIT: usize = 4096;
pub const DISK_MAX_BYTES: usize = 12 + DISK_RECORD_LIMIT * RECORD_BYTES + 32;
const DISK_AGE: u64 = 30 * 24 * 3600;
const TOUCH_AGE: u64 = 24 * 3600;
pub const FULL_AUDIT_AGE: Duration = Duration::from_secs(600);
pub const CHECKPOINT_LIMIT: usize = 256;
const CHECKPOINTS_PER_SCAN: usize = 32;
const CHECKPOINT_DEPTH: usize = 2;
const MIN_CHECKPOINT_ITEMS: u64 = 128;
const CHECKPOINT_IDLE: Duration = Duration::from_secs(180);
#[derive(Clone, Copy)]
struct ScanPolicy { budget: usize, depth: usize, min_items: u64 }
impl ScanPolicy {
    fn for_media(media: native::FolderSizeMedia) -> Self {
        match media {
            native::FolderSizeMedia::Fast => Self { budget: CHECKPOINTS_PER_SCAN, depth: CHECKPOINT_DEPTH, min_items: MIN_CHECKPOINT_ITEMS },
            native::FolderSizeMedia::Seek => Self { budget: 16, depth: 1, min_items: MIN_CHECKPOINT_ITEMS },
            native::FolderSizeMedia::Unknown => Self { budget: 24, depth: 2, min_items: MIN_CHECKPOINT_ITEMS },
        }
    }
}
type Key = [u8; 32];
type Identity = (u32, u64, u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Record { identity: Identity, bytes: u64, seen: u64 }
type Records = HashMap<Key, Record>;

fn now_seconds() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }
fn key(path: &Path) -> Key {
    // Exact UTF-16 preserves case-sensitive Windows directories and avoids
    // a plaintext filename/path inventory on disk.
    let mut hash = Sha256::new();
    let mut block = [0u8; 256];
    let mut used = 0;
    for word in path.as_os_str().encode_wide() {
        block[used..used + 2].copy_from_slice(&word.to_le_bytes());
        used += 2;
        if used == block.len() { hash.update(block); used = 0; }
    }
    hash.update(&block[..used]);
    hash.finalize().into()
}
fn live(record: &Record, now: u64) -> bool { record.seen <= now && now - record.seen <= DISK_AGE }
fn trim(records: &mut Records, now: u64) {
    records.retain(|_, r| live(r, now));
    if records.len() > DISK_RECORD_LIMIT {
        let mut order: Vec<_> = records.iter().map(|(k, r)| (*k, r.seen)).collect();
        order.sort_unstable_by_key(|&(k, seen)| (seen, k));
        // Evict in batches, not one O(N log N) sort per new folder.
        for (k, _) in order.into_iter().take(records.len() - (DISK_RECORD_LIMIT - 256)) { records.remove(&k); }
    }
}
fn encode(records: &Records) -> Vec<u8> {
    debug_assert!(records.len() <= DISK_RECORD_LIMIT);
    let mut out = Vec::with_capacity(12 + records.len() * RECORD_BYTES + 32);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    let mut ordered: Vec<_> = records.iter().collect();
    ordered.sort_unstable_by_key(|(k, _)| **k);
    for (k, r) in ordered {
        out.extend_from_slice(k);
        out.extend_from_slice(&r.identity.0.to_le_bytes());
        for n in [r.identity.1, r.identity.2, r.bytes, r.seen] { out.extend_from_slice(&n.to_le_bytes()); }
    }
    let hash = Sha256::digest(&out);
    out.extend_from_slice(&hash);
    out
}
fn decode(data: &[u8], now: u64) -> Option<Records> {
    if data.len() < 44 || data.len() > DISK_MAX_BYTES || &data[..8] != MAGIC { return None; }
    let count = u32::from_le_bytes(data[8..12].try_into().ok()?) as usize;
    if count > DISK_RECORD_LIMIT || data.len() != 12 + count * RECORD_BYTES + 32 { return None; }
    let end = data.len() - 32;
    if Sha256::digest(&data[..end])[..] != data[end..] { return None; }
    let mut out = Records::with_capacity(count);
    for row in data[12..end].chunks_exact(RECORD_BYTES) {
        let k = row[..32].try_into().ok()?;
        let u64_at = |i: usize| u64::from_le_bytes(row[i..i+8].try_into().unwrap());
        let record = Record { identity: (u32::from_le_bytes(row[32..36].try_into().ok()?), u64_at(36), u64_at(44)), bytes: u64_at(52), seen: u64_at(60) };
        if live(&record, now) { out.insert(k, record); }
    }
    Some(out)
}
fn read_records(path: &Path) -> Records {
    if crate::fsops::ensure_plain_path(path).is_err() { return Records::new(); }
    let Ok(file) = fs::File::open(path) else { return Records::new(); };
    if !file.metadata().is_ok_and(|m| m.is_file() && m.len() <= DISK_MAX_BYTES as u64) { return Records::new(); }
    let mut data = Vec::new();
    if file.take(DISK_MAX_BYTES as u64 + 1).read_to_end(&mut data).is_err() { return Records::new(); }
    decode(&data, now_seconds()).unwrap_or_default()
}

/// Owned only by the low-priority folder worker. Cache failure never
/// prevents calculation. A fixed staging name is protected by an exclusive
/// Windows file handle; concurrent instances merge rather than overwrite.
pub struct DiskStore {
    root: PathBuf,
    records: Records,
    dirty: Records,
    enabled: bool,
}
impl DiskStore {
    pub fn open(profile: &Path, enabled: bool) -> Self {
        let root = profile.join("cache");
        let records = if enabled { read_records(&root.join("folder_sizes.db")) } else { Records::new() };
        Self { root, records, dirty: Records::new(), enabled }
    }
    pub fn hint(&self, path: &Path) -> Option<u64> {
        if !self.enabled { return None; }
        let record = self.records.get(&key(path))?;
        if !live(record, now_seconds()) || native::folder_size_identity(path)? != record.identity { return None; }
        // The prior process cannot prove descendants are unchanged. This
        // is the last successful observation for the matching identity,
        // for display only until a live scan validates the current total.
        Some(record.bytes)
    }
    pub fn record(&mut self, path: &Path, watch: &FolderSizeWatch, bytes: u64) {
        if !self.enabled { return; }
        let k = key(path);
        let now = now_seconds();
        let identity = watch.identity();
        if self.records.get(&k).is_some_and(|r| r.identity == identity && r.bytes == bytes && live(r, now) && now - r.seen < TOUCH_AGE) { return; }
        let record = Record { identity, bytes, seen: now };
        self.records.insert(k, record);
        self.dirty.insert(k, record);
        if self.records.len() > DISK_RECORD_LIMIT { trim(&mut self.records, now); }
        if self.dirty.len() > DISK_RECORD_LIMIT { trim(&mut self.dirty, now); }
    }
    pub fn dirty(&self) -> bool { !self.dirty.is_empty() }
    pub fn flush(&mut self) -> Result<(), String> {
        if !self.dirty() { return Ok(()); }
        cache::prepare(&self.root)?;
        let db = self.root.join("folder_sizes.db");
        let lock = self.root.join("folder_sizes.lock");
        let temp = self.root.join("folder_sizes.tmp");
        for path in [&db, &lock, &temp] { crate::fsops::ensure_plain_path(path)?; }
        let _guard = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).share_mode(0).open(&lock).map_err(|e| e.to_string())?;
        let mut merged = read_records(&db);
        for (&k, &r) in &self.dirty {
            if merged.get(&k).is_none_or(|old| old.seen <= r.seen) { merged.insert(k, r); }
        }
        trim(&mut merged, now_seconds());
        let payload = encode(&merged);
        // Never truncate a pre-existing file (it could be a hard link).
        match fs::remove_file(&temp) {
            Ok(()) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.to_string()),
        }
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| e.to_string())?;
        file.write_all(&payload).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
        drop(file);
        // Windows atomic replacement on the same filesystem.
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
        let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = db.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        self.records = merged;
        self.dirty.clear();
        Ok(())
    }
}

struct Checkpoint {
    watch: FolderSizeWatch,
    summary: FolderSizeSummary,
    measured: Instant,
    used: Instant,
}
#[derive(Default, Clone, Copy, Debug)]
pub struct ScanWork { pub visited: u64, pub reused: u64, pub root_stable: bool }

/// At most 256 useful branches globally and 32 new watches per root scan.
/// Cache only directory totals (not every file); small and very deep branches
/// use the existing streaming enumerator. A periodic full audit bounds the
/// impact of notifications missed through alternate hard-link paths.
#[derive(Default)]
pub struct ScanEngine { checkpoints: HashMap<PathBuf, Checkpoint> }
impl ScanEngine {
    pub fn scan(&mut self, path: &Path, cancel: &AtomicBool, force_full: bool, root_watch: &FolderSizeWatch) -> Option<(FolderSizeSummary, ScanWork)> {
        self.scan_for_media(path, cancel, force_full, root_watch, native::FolderSizeMedia::Fast)
    }
    pub fn prune_idle(&mut self) {
        self.checkpoints.retain(|_, c| c.used.elapsed() < CHECKPOINT_IDLE && c.measured.elapsed() < FULL_AUDIT_AGE && c.watch.unchanged());
    }
    pub fn scan_for_media(&mut self, path: &Path, cancel: &AtomicBool, force_full: bool, root_watch: &FolderSizeWatch, media: native::FolderSizeMedia) -> Option<(FolderSizeSummary, ScanWork)> {
        let policy = ScanPolicy::for_media(media);
        if force_full {
            // A full audit must also retire descendant observations it
            // may not revisit separately because of the checkpoint limit.
            self.checkpoints.retain(|p, _| !p.starts_with(path));
        }
        let mut staged = Vec::new();
        let mut work = ScanWork::default();
        let summary = self.branch(path, 0, false, &mut staged, cancel, force_full, &mut work, policy)?;
        // The root watch was armed BEFORE enumeration. This lets us arm
        // child watches only AFTER finding useful branches, with no
        // unmonitored gap: discard all staged checkpoints on root changes.
        work.root_stable = !cancel.load(Ordering::Relaxed) && root_watch.matches_path(path);
        if summary.errors.is_empty() && work.root_stable {
            for (p, checkpoint) in staged { self.insert(&p, checkpoint); }
        }
        Some((summary, work))
    }
    fn insert(&mut self, path: &Path, checkpoint: Checkpoint) {
        if self.checkpoints.len() >= CHECKPOINT_LIMIT && !self.checkpoints.contains_key(path) {
            if let Some(old) = self.checkpoints.iter().min_by_key(|(_, c)| c.used).map(|(p, _)| p.clone()) { self.checkpoints.remove(&old); }
        }
        self.checkpoints.insert(path.to_path_buf(), checkpoint);
    }
    fn branch(&mut self, path: &Path, depth: usize, known_directory: bool, staged: &mut Vec<(PathBuf, Checkpoint)>, cancel: &AtomicBool, force_full: bool, work: &mut ScanWork, policy: ScanPolicy) -> Option<FolderSizeSummary> {
        if cancel.load(Ordering::Relaxed) { return None; }
        if depth > 0 && !force_full {
            if let Some(previous) = self.checkpoints.get_mut(path) {
                if previous.measured.elapsed() < FULL_AUDIT_AGE && previous.watch.matches_path(path) {
                    previous.used = Instant::now();
                    work.reused += 1;
                    return Some(previous.summary.clone());
                }
            }
            self.checkpoints.remove(path);
        }
        let eligible = depth > 0 && depth <= policy.depth
            && staged.len() < policy.budget && path.as_os_str().len() <= 2048;
        let started = Instant::now();
        // Root and checkpoint candidates can split at their immediate child
        // directories. Everything else uses the original iterative scan.
        let summary = if depth > 0 && (!eligible || depth == policy.depth) {
            let summary = folder_size_summary_known_directory(path, cancel)?;
            work.visited += summary.files + summary.folders;
            summary
        } else {
            let mut summary = FolderSizeSummary::default();
            let error = |s: &mut FolderSizeSummary, p: &Path, e: std::io::Error| {
                if s.errors.len() < 100 { s.errors.push(format!("{}: {e}", p.display())); }
            };
            if !known_directory {
                let meta = match fs::symlink_metadata(path) {
                    Ok(m) => m,
                    Err(e) => { error(&mut summary, path, e); return Some(summary); }
                };
                if meta.file_attributes() & 0x400 != 0 { return Some(summary); }
                if !meta.is_dir() {
                    work.visited += 1;
                    summary.total_bytes = meta.len(); summary.files = 1; return Some(summary);
                }
            }
            work.visited += 1;
            summary.folders = 1;
            let entries = match fs::read_dir(path) {
                Ok(e) => e,
                Err(e) => { error(&mut summary, path, e); return Some(summary); }
            };
            for entry in entries {
                if cancel.load(Ordering::Relaxed) { return None; }
                let entry = match entry { Ok(e) => e, Err(e) => { error(&mut summary, path, e); break; } };
                let meta = match entry.metadata() { Ok(m) => m, Err(e) => { error(&mut summary, &entry.path(), e); continue; } };
                if meta.file_attributes() & 0x400 != 0 { continue; }
                if meta.is_dir() {
                    let child = self.branch(&entry.path(), depth + 1, true, staged, cancel, force_full, work, policy)?;
                    summary.total_bytes += child.total_bytes;
                    summary.files += child.files;
                    summary.folders += child.folders;
                    let remaining = 100usize.saturating_sub(summary.errors.len());
                    summary.errors.extend(child.errors.into_iter().take(remaining));
                } else {
                    work.visited += 1;
                    summary.total_bytes += meta.len();
                    summary.files += 1;
                }
            }
            summary
        };
        if eligible && staged.len() < policy.budget && summary.errors.is_empty()
            && summary.files + summary.folders >= policy.min_items && !cancel.load(Ordering::Relaxed) {
            // Avoid opening a redundant outer watch when its children
            // already cover nearly all of its enumeration work.
            let child_items = staged.iter().filter(|(p, _)| p.parent() == Some(path))
                .map(|(_, c)| c.summary.files + c.summary.folders).sum::<u64>();
            if child_items < (summary.files + summary.folders).saturating_mul(9) / 10 {
                if let Some(watch) = FolderSizeWatch::open(path) {
                    if watch.unchanged() {
                        staged.push((path.to_path_buf(), Checkpoint { watch, summary: summary.clone(), measured: started, used: Instant::now() }));
                    }
                }
            }
        }
        Some(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::folder_size_summary;
    fn fixture(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ndr-folder-cache-{}-{}-{label}", std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn wait_changed(watch: &FolderSizeWatch) {
        let end = Instant::now() + Duration::from_secs(10);
        while watch.unchanged() {
            assert!(Instant::now() < end, "subtree change not signalled");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn fixed_records_are_bounded_checksummed_and_expire() {
        let now = now_seconds();
        let mut records = Records::new();
        for i in 0..DISK_RECORD_LIMIT + 1000 {
            let k = Sha256::digest(i.to_le_bytes()).into();
            records.insert(k, Record { identity: (1, i as u64, 2), bytes: i as u64 * 17, seen: now - (i % 500) as u64 });
        }
        trim(&mut records, now);
        assert!(records.len() <= DISK_RECORD_LIMIT);
        let data = encode(&records);
        assert!(data.len() <= DISK_MAX_BYTES);
        assert_eq!(decode(&data, now).unwrap(), records);
        assert!(decode(&data[..data.len()-1], now).is_none());
        let mut corrupt = data.clone();
        corrupt[24] ^= 1;
        assert!(decode(&corrupt, now).is_none());
        assert!(decode(&data, now + DISK_AGE + 1).unwrap().is_empty());
        assert!(decode(&data, now - 1000).unwrap().is_empty());
        let mut too_large = data;
        too_large.resize(DISK_MAX_BYTES + 1, 0);
        assert!(decode(&too_large, now).is_none());
        assert!(DISK_MAX_BYTES * 2 < 600 * 1024, "snapshot plus staging bound");
    }
    #[test]
    fn persistence_merges_instances_and_checks_identity_before_hint() {
        let profile = fixture("disk");
        let a = profile.join("a");
        let b = profile.join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let wa = FolderSizeWatch::open(&a).unwrap();
        let wb = FolderSizeWatch::open(&b).unwrap();
        let mut one = DiskStore::open(&profile, true);
        let mut two = DiskStore::open(&profile, true);
        one.record(&a, &wa, 4096);
        two.record(&b, &wb, 2048);
        one.flush().unwrap();
        two.flush().unwrap();
        let mut reopened = DiskStore::open(&profile, true);
        assert_eq!(reopened.hint(&a), Some(4096));
        assert_eq!(reopened.hint(&b), Some(2048));
        reopened.record(&a, &wa, 4096);
        assert!(!reopened.dirty(), "unchanged daily total must not rewrite disk");
        // Replacement at the same path is not a valid startup hint.
        fs::rename(&a, profile.join("old-a")).unwrap();
        fs::create_dir(&a).unwrap();
        assert_eq!(reopened.hint(&a), None);
        // A crashed staging file must not overwrite a hard-linked target.
        let sentinel = profile.join("sentinel");
        fs::write(&sentinel, b"preserve me").unwrap();
        fs::hard_link(&sentinel, profile.join("cache/folder_sizes.tmp")).unwrap();
        reopened.record(&b, &wb, 8192);
        reopened.flush().unwrap();
        assert_eq!(fs::read(&sentinel).unwrap(), b"preserve me");
        assert_eq!(DiskStore::open(&profile, true).hint(&b), Some(8192));
        // Locked cache is non-fatal, and dirty data survives for a retry.
        let lock = fs::OpenOptions::new().read(true).write(true).share_mode(0).open(profile.join("cache/folder_sizes.lock")).unwrap();
        reopened.record(&b, &wb, 16384);
        assert!(reopened.flush().is_err() && reopened.dirty());
        drop(lock);
        reopened.flush().unwrap();
        let db = profile.join("cache/folder_sizes.db");
        fs::write(&db, b"corrupt").unwrap();
        assert_eq!(DiskStore::open(&profile, true).hint(&b), None);
        drop((wa, wb, one, two, reopened));
        fs::remove_dir_all(profile).unwrap();
    }
    #[test]
    fn incremental_scan_reuses_only_clean_branches_and_audit_is_full() {
        let root = fixture("incremental");
        for i in 0..8 {
            let d = root.join(format!("branch-{i}"));
            fs::create_dir_all(&d).unwrap();
            for j in 0..128 { fs::write(d.join(format!("{j}.bin")), [1u8; 64]).unwrap(); }
        }
        let cancel = AtomicBool::new(false);
        let mut engine = ScanEngine::default();
        let (first, first_work) = engine.scan(&root, &cancel, true, &FolderSizeWatch::open(&root).unwrap()).unwrap();
        assert_eq!(first, folder_size_summary(&[root.clone()], &cancel).unwrap());
        assert_eq!(first_work.reused, 0);
        assert_eq!(first_work.visited, 1033);
        let changed = root.join("branch-3");
        fs::write(changed.join("17.bin"), [2u8; 512]).unwrap();
        wait_changed(&engine.checkpoints[&changed].watch);
        let (next, work) = engine.scan(&root, &cancel, false, &FolderSizeWatch::open(&root).unwrap()).unwrap();
        assert_eq!(next, folder_size_summary(&[root.clone()], &cancel).unwrap());
        assert_eq!(next.total_bytes, first.total_bytes + 448);
        assert_eq!(work.reused, 7);
        assert_eq!(work.visited, 130);
        println!("incremental visited {} -> {}, reused {}", first_work.visited, work.visited, work.reused);
        // New/deleted/replaced branches and deep changes cannot carry over
        // a total from a stale directory object.
        fs::remove_file(changed.join("1.bin")).unwrap();
        wait_changed(&engine.checkpoints[&changed].watch);
        fs::rename(root.join("branch-2"), root.join("moved")).unwrap();
        fs::create_dir(root.join("branch-2")).unwrap();
        fs::write(root.join("branch-2/new"), b"new").unwrap();
        let (next, _) = engine.scan(&root, &cancel, false, &FolderSizeWatch::open(&root).unwrap()).unwrap();
        assert_eq!(next, folder_size_summary(&[root.clone()], &cancel).unwrap());
        let (audit, audit_work) = engine.scan(&root, &cancel, true, &FolderSizeWatch::open(&root).unwrap()).unwrap();
        assert_eq!(audit, next);
        assert_eq!(audit_work.reused, 0);
        cancel.store(true, Ordering::Relaxed);
        assert!(engine.scan(&root, &cancel, false, &FolderSizeWatch::open(&root).unwrap()).is_none());
        drop(engine);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn checkpoint_count_stays_bounded() {
        let root = fixture("handles");
        let mut engine = ScanEngine::default();
        for i in 0..CHECKPOINT_LIMIT + 25 {
            let watch = FolderSizeWatch::open(&root).unwrap();
            engine.insert(&root.join(i.to_string()), Checkpoint { watch, summary: FolderSizeSummary::default(), measured: Instant::now(), used: Instant::now() });
        }
        assert_eq!(engine.checkpoints.len(), CHECKPOINT_LIMIT);
        drop(engine);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delta_move_delete_rename_and_idle_retirement_match_full_scan() {
        let root = fixture("mutation-matrix");
        let left = root.join("left");
        let right = root.join("right");
        let other = root.join("other");
        for d in [&left, &right, &other] {
            fs::create_dir_all(d).unwrap();
            for j in 0..140 { fs::write(d.join(format!("{j}.bin")), [1u8; 32]).unwrap(); }
        }
        let stop = AtomicBool::new(false);
        let mut engine = ScanEngine::default();
        let initial = FolderSizeWatch::open(&root).unwrap();
        let (first, work) = engine.scan(&root, &stop, true, &initial).unwrap();
        assert!(work.root_stable);
        assert_eq!(engine.checkpoints.len(), 3);
        fs::rename(left.join("0.bin"), right.join("moved.bin")).unwrap();
        fs::rename(left.join("1.bin"), left.join("renamed.bin")).unwrap();
        fs::remove_file(right.join("2.bin")).unwrap();
        fs::write(right.join("3.bin"), [3u8; 128]).unwrap();
        fs::write(left.join("new.bin"), [4u8; 16]).unwrap();
        wait_changed(&engine.checkpoints[&left].watch);
        wait_changed(&engine.checkpoints[&right].watch);
        let (next, work) = engine.scan(&root, &stop, false, &FolderSizeWatch::open(&root).unwrap()).unwrap();
        assert!(work.root_stable);
        assert_eq!(work.reused, 1);
        assert_eq!(next.total_bytes, first.total_bytes - 32 + 96 + 16);
        assert_eq!(next, folder_size_summary(&[root.clone()], &stop).unwrap());
        engine.checkpoints.get_mut(&other).unwrap().used = Instant::now() - CHECKPOINT_IDLE;
        engine.prune_idle();
        assert!(!engine.checkpoints.contains_key(&other));
        drop((engine, initial));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn small_and_redundant_subtrees_do_not_allocate_extra_watches() {
        let root = fixture("adaptive-watches");
        let large = root.join("outer/inner");
        fs::create_dir_all(&large).unwrap();
        for j in 0..160 { fs::write(large.join(format!("{j}.bin")), b"x").unwrap(); }
        for j in 0..40 {
            let small = root.join(format!("small-{j}"));
            fs::create_dir_all(&small).unwrap();
            fs::write(small.join("x"), b"x").unwrap();
        }
        let mut engine = ScanEngine::default();
        let stop = AtomicBool::new(false);
        let root_watch = FolderSizeWatch::open(&root).unwrap();
        let (summary, _) = engine.scan_for_media(&root, &stop, true, &root_watch, native::FolderSizeMedia::Fast).unwrap();
        assert_eq!(summary, folder_size_summary(&[root.clone()], &stop).unwrap());
        assert_eq!(engine.checkpoints.len(), 1);
        assert!(engine.checkpoints.contains_key(&large));
        let (summary, _) = engine.scan_for_media(&root, &stop, true, &root_watch, native::FolderSizeMedia::Seek).unwrap();
        assert_eq!(summary, folder_size_summary(&[root.clone()], &stop).unwrap());
        assert_eq!(engine.checkpoints.len(), 1);
        assert!(engine.checkpoints.contains_key(&root.join("outer")));
        drop((root_watch, engine));
        fs::remove_dir_all(root).unwrap();
    }
}
