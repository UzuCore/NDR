//! Disk-backed, disposable NDR data lives below the active portable profile.
//! Runtime lookup/texture caches stay in memory; vault plaintext and
//! transactional file-operation staging must never enter this directory.
use std::{fs, path::{Path, PathBuf}, sync::OnceLock};

static ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn root() -> PathBuf {
    ROOT.get_or_init(|| {
        std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from(".")).join("cache")
    }).clone()
}

/// Called before starting any worker. An explicit --profile also isolates
/// disk caches, just as it isolates settings. No external fallback is used.
pub fn configure(profile: &Path) -> Result<(), String> {
    let _ = ROOT.set(profile.join("cache"));
    prepare(&root())
}

pub fn archive_entries() -> PathBuf { root().join("archive-entries") }
pub fn updates() -> PathBuf { root().join("updates") }

pub(crate) fn prepare(path: &Path) -> Result<(), String> {
    // Do not write into a junction/symlink substituted for the cache root.
    crate::fsops::ensure_plain_path(path)?;
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    crate::fsops::ensure_plain_path(path)
}

#[test]
fn disk_cache_children_share_one_root() {
    assert_eq!(archive_entries().parent(), Some(root().as_path()));
    assert_eq!(updates().parent(), Some(root().as_path()));
    assert_eq!(root().file_name().unwrap(), "cache");
}


// Disk limits are deliberately soft: an active preview or unresolved update
// must not be destroyed to satisfy a quota. Count only positively identified
// NDR-owned sessions; unknown/reparse/unreadable trees are never removed.
use std::{fs::{File, OpenOptions}, os::windows::fs::OpenOptionsExt,
    time::{Duration, Instant, SystemTime}, sync::Mutex};

pub const PREVIEW_SOFT_BYTES: u64 = 1024 * 1024 * 1024;
pub const PREVIEW_SOFT_COUNT: usize = 256;
pub const UPDATE_SOFT_BYTES: u64 = 512 * 1024 * 1024;
pub const UPDATE_KEEP: usize = 2;
const DAY: Duration = Duration::from_secs(86400);
const PREVIEW_AGE: Duration = Duration::from_secs(7 * 86400);

#[derive(Default, Debug, PartialEq, Eq)]
pub struct CleanupReport {
    pub removed: usize,
    pub trimmed: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub protected: usize,
    pub unknown: usize,
}
#[derive(Clone, Copy)]
struct TreeStats { bytes: u64, newest: SystemTime }

fn tree_stats(path: &Path, check_open: bool) -> Option<TreeStats> {
    // Keep only pending directories, not a PathBuf for every file in a
    // wide directory. Accounting can use Windows' enumerated metadata;
    // the destructive recheck still obtains fresh metadata.
    let mut pending = vec![path.to_path_buf()];
    let mut result = TreeStats { bytes: 0, newest: SystemTime::UNIX_EPOCH };
    while let Some(p) = pending.pop() {
        // Recheck every directory before entering it, even in accounting
        // mode; enumerated metadata alone is not a reparse-point guard.
        let m = fs::symlink_metadata(&p).ok()?;
        if crate::fsops::is_reparse(&m) { return None; }
        result.newest = result.newest.max(m.modified().ok()?).max(m.created().ok()?);
        if m.is_dir() {
            for child in fs::read_dir(&p).ok()? {
                let child = child.ok()?;
                let meta = if check_open { fs::symlink_metadata(child.path()).ok()? } else { child.metadata().ok()? };
                if crate::fsops::is_reparse(&meta) { return None; }
                if meta.is_dir() {
                    pending.push(child.path());
                } else if meta.is_file() {
                    result.newest = result.newest.max(meta.modified().ok()?).max(meta.created().ok()?);
                    if check_open { let _guard = OpenOptions::new().read(true).share_mode(0).open(child.path()).ok()?; }
                    result.bytes = result.bytes.saturating_add(meta.len());
                } else { return None; }
            }
        } else if m.is_file() {
            // Even a reader that allows deletion can still need the path.
            // A zero-sharing probe refuses existing read/write handles.
            if check_open { let _guard = OpenOptions::new().read(true).share_mode(0).open(&p).ok()?; }
            result.bytes = result.bytes.saturating_add(m.len());
        } else { return None; }
    }
    Some(result)
}
fn age_at(now: SystemTime, time: SystemTime) -> Duration {
    now.duration_since(time).unwrap_or_default()
}
fn preview_owner(name: &str) -> Option<(u32, bool)> {
    let (rest, trash) = if let Some(s) = name.strip_prefix(".ndr-entry-") { (s, false) }
        else { (name.strip_prefix(".ndr-trash-")?, true) };
    let (pid, seq) = rest.split_once('-')?;
    if !pid.bytes().all(|b| b.is_ascii_digit()) || seq.is_empty() || !seq.bytes().all(|b| b.is_ascii_digit()) { return None; }
    Some((pid.parse().ok()?, trash))
}
fn update_owner(name: &str) -> Option<u32> {
    let mut parts = name.strip_prefix("ndr-update-build-")?.split('-');
    let build = parts.next()?.parse::<u64>().ok()?;
    let pid = parts.next()?.parse::<u32>().ok()?;
    let stamp = parts.next()?.parse::<u128>().ok()?;
    if build == 0 || pid == 0 || stamp == 0 || parts.next().is_some() { return None; }
    Some(pid)
}
fn owner_may_be_alive(pid: u32) -> bool {
    use windows_sys::Win32::{Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject}};
    if pid == std::process::id() || pid == 0 { return true; }
    // SYNCHRONIZE only: no privilege escalation, memory read, or termination.
    let handle = unsafe { OpenProcess(0x00100000, 0, pid) };
    if handle.is_null() {
        // PID not found is the only negative result; access denied is protected.
        return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
    }
    let alive = unsafe { WaitForSingleObject(handle, 0) != WAIT_OBJECT_0 };
    unsafe { CloseHandle(handle); }
    alive
}
// Real PIDs and filesystem timestamps are not deterministic test
// fixtures. Probe a nonexistent PID and derive the test clock from the
// actual on-disk timestamps instead of racing Windows' cached clock.
#[cfg(test)]
pub(crate) fn fixture_dead_owner() -> u32 {
    (0..1024).map(|n| u32::MAX - n).find(|&pid| !owner_may_be_alive(pid)).expect("nonexistent fixture PID")
}

#[cfg(test)]
pub(crate) fn fixture_clock(root: &Path) -> SystemTime {
    SystemTime::now().max(tree_stats(root, false).expect("fixture metadata").newest) + Duration::from_secs(1)
}

fn maintenance_lock(root: &Path) -> Option<File> {
    if !root.is_dir() || crate::fsops::ensure_plain_path(root).is_err() { return None; }
    let path = root.join(".ndr-maintenance.lock");
    crate::fsops::ensure_plain_path(&path).ok()?;
    // Never truncate: an existing file may be a hard link.
    OpenOptions::new().read(true).write(true).create(true).truncate(false).share_mode(0).open(path).ok()
}
fn remove_owned_tree(root: &Path, path: &Path, original: TreeStats, now: SystemTime) -> bool {
    if path.parent() != Some(root) || crate::fsops::ensure_plain_path(path).is_err() { return false; }
    let Some(current) = tree_stats(path, true) else { return false; };
    if current.bytes != original.bytes || current.newest != original.newest || current.newest > now { return false; }
    // Rename before deletion, as the previous preview cache did. Do not
    // overwrite an existing quarantine; failed deletions leave owned trash.
    let trash = crate::fsops::unique_path(root, "trash");
    if fs::rename(path, &trash).is_err() { return false; }
    if fs::remove_dir_all(&trash).is_ok() { true }
    else {
        // Best effort restore when a file became busy after the probe.
        let _ = fs::rename(&trash, path);
        false
    }
}
pub fn clean_previews(root: &Path, age: Duration, now: SystemTime, max_bytes: u64, max_count: usize) -> CleanupReport {
    let mut report = CleanupReport::default();
    let Some(_guard) = maintenance_lock(root) else { return report; };
    let Ok(items) = fs::read_dir(root) else { return report; };
    let mut candidates = Vec::new();
    let mut count = 0usize;
    for item in items.flatten() {
        let Some((pid, trash)) = item.file_name().to_str().and_then(preview_owner) else { continue; };
        let path = item.path();
        if !item.file_type().is_ok_and(|t| t.is_dir()) { report.unknown += 1; continue; }
        // Do not even enumerate the live owner's potentially changing tree.
        if owner_may_be_alive(pid) { report.protected += 1; continue; }
        let Some(stats) = tree_stats(&path, false) else { report.unknown += 1; continue; };
        count += 1;
        report.bytes_before = report.bytes_before.saturating_add(stats.bytes);
        candidates.push((stats.newest, trash, path, stats));
    }
    candidates.sort_unstable_by_key(|(newest, trash, _, _)| (!*trash, *newest));
    report.bytes_after = report.bytes_before;
    for (_, trash, path, stats) in candidates {
        let old = age_at(now, stats.newest);
        let over = report.bytes_after > max_bytes || count > max_count;
        if !trash && old < age && (!over || old < DAY) { report.protected += 1; continue; }
        if remove_owned_tree(root, &path, stats, now) {
            report.removed += 1;
            report.bytes_after = report.bytes_after.saturating_sub(stats.bytes);
            count = count.saturating_sub(1);
        } else { report.protected += 1; }
    }
    report
}

fn successful_update(path: &Path) -> bool {
    // New installers hold this file with zero sharing until they exit.
    // Legacy sessions have no lock; the day-long grace still applies.
    let lock = path.join(".ndr-install.lock");
    match fs::symlink_metadata(&lock) {
        Ok(m) if m.is_file() && !crate::fsops::is_reparse(&m) => {
            if OpenOptions::new().read(true).share_mode(0).open(&lock).is_err() { return false; }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => return false,
    }
    // Recognize old successful installers as well. A failure, missing log,
    // incomplete backup or unknown directory is not disposable.
    let log = path.join("update.log");
    let Ok(meta) = fs::symlink_metadata(&log) else { return false; };
    if crate::fsops::is_reparse(&meta) || !meta.is_file() || meta.len() > 4096 { return false; }
    let Ok(raw) = fs::read(&log) else { return false; };
    let text = String::from_utf8_lossy(&raw);
    let Some(n) = text.trim_start_matches('\u{feff}').trim().strip_prefix("Update installed: ").and_then(|s| s.strip_suffix(" files")) else { return false; };
    n.parse::<u32>().is_ok_and(|n| n > 0 && n <= 1500)
        && fs::symlink_metadata(path.join("backup/NDR.exe")).is_ok_and(|m| m.is_file() && !crate::fsops::is_reparse(&m) && m.len() > 0)
}
fn trim_success_payload(path: &Path, now: SystemTime) -> (usize, u64) {
    let mut trimmed = 0;
    let mut bytes = 0;
    // Backups/logs are sufficient for rollback/diagnosis; keep neither an
    // extra extracted copy nor a second copy of the publicly available ZIP.
    for name in ["extracted", "NDR-windows-x64.zip"] {
        let child = path.join(name);
        let Ok(meta) = fs::symlink_metadata(&child) else { continue; };
        let Some(stats) = tree_stats(&child, true) else { continue; };
        if stats.newest > now || crate::fsops::ensure_plain_path(&child).is_err() { continue; }
        let removed = if meta.is_dir() { remove_owned_tree(path, &child, stats, now) }
            else if meta.is_file() && !crate::fsops::is_reparse(&meta) { fs::remove_file(&child).is_ok() }
            else { false };
        if removed { trimmed += 1; bytes += stats.bytes; }
    }
    (trimmed, bytes)
}
pub fn clean_updates(root: &Path, now: SystemTime, max_bytes: u64, keep: usize) -> CleanupReport {
    let mut report = CleanupReport::default();
    let Some(_guard) = maintenance_lock(root) else { return report; };
    let Ok(items) = fs::read_dir(root) else { return report; };
    let mut completed = Vec::new();
    for item in items.flatten() {
        let Some(pid) = item.file_name().to_str().and_then(update_owner) else { continue; };
        let path = item.path();
        if owner_may_be_alive(pid) { report.protected += 1; continue; }
        if !item.file_type().is_ok_and(|t| t.is_dir()) { report.unknown += 1; continue; }
        let Some(stats) = tree_stats(&path, false) else { report.unknown += 1; continue; };
        report.bytes_before = report.bytes_before.saturating_add(stats.bytes);
        if successful_update(&path) {
            // Use the success log timestamp, not extraction timestamps.
            let finished = fs::metadata(path.join("update.log")).and_then(|m| m.modified()).unwrap_or(stats.newest);
            completed.push((finished, path, stats));
        } else { report.protected += 1; }
    }
    // The newest completed rollback is always retained, even if oversized.
    completed.sort_unstable_by(|a,b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    report.bytes_after = report.bytes_before;
    for (index, (finished, path, stats)) in completed.into_iter().enumerate() {
        // The exclusive lock protocol makes completed new sessions
        // immutable; old versions need a longer grace period.
        let grace = if path.join(".ndr-install.lock").is_file() { Duration::from_secs(600) } else { DAY };
        if age_at(now, finished) < grace || age_at(now, stats.newest) < grace {
            report.protected += 1;
            continue;
        }
        if !successful_update(&path) { report.protected += 1; continue; }
        let known_members = fs::read_dir(&path).ok().is_some_and(|items| items.into_iter().all(|item|
            item.ok().is_some_and(|item| matches!(item.file_name().to_str(),
                Some("backup" | "extracted" | "NDR-windows-x64.zip" | "update.log" |
                    "apply-update.ps1" | "installer-locale.json" | "installer-host.log" | ".ndr-install.lock")))));
        let discard = known_members && (index >= keep.max(1) || (index > 0 && report.bytes_after > max_bytes));
        if discard && remove_owned_tree(root, &path, stats, now) {
            report.removed += 1;
            report.bytes_after = report.bytes_after.saturating_sub(stats.bytes);
        } else {
            let (n, bytes) = trim_success_payload(&path, now);
            report.trimmed += n;
            report.bytes_after = report.bytes_after.saturating_sub(bytes);
            report.protected += 1;
        }
    }
    report
}
static MAINTENANCE: Mutex<Option<Instant>> = Mutex::new(None);
/// The normal application calls this off the UI thread at startup. Preview
/// opens can schedule another sweep, at most once per ten minutes.
pub fn schedule_maintenance() {
    let mut last = MAINTENANCE.lock().unwrap_or_else(|e| e.into_inner());
    if last.is_some_and(|t| t.elapsed() < Duration::from_secs(600)) { return; }
    *last = Some(Instant::now());
    drop(last);
    std::thread::spawn(|| {
        unsafe {
            use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
            SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN as i32);
        }
        let now = SystemTime::now();
        let previews = clean_previews(&archive_entries(), PREVIEW_AGE, now, PREVIEW_SOFT_BYTES, PREVIEW_SOFT_COUNT);
        let updates = clean_updates(&updates(), now, UPDATE_SOFT_BYTES, UPDATE_KEEP);
        log::debug!("NDR cache maintenance: previews={previews:?}; updates={updates:?}");
    });
}

#[cfg(test)]
mod maintenance_tests {
    use super::*;
    fn fixture(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ndr-cache-maintenance-{}-{}-{label}", std::process::id(),
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(&path).unwrap(); path
    }
    fn preview(root: &Path, name: &str) -> PathBuf {
        let p = root.join(name); fs::create_dir_all(p.join("sub")).unwrap();
        fs::write(p.join("sub/file.txt"), b"preview").unwrap(); p
    }
    #[test]
    fn batch406_future_timestamp_remains_protected() {
        let root = fixture("future");
        let p = preview(&root, &format!(".ndr-trash-{}-7", fixture_dead_owner()));
        let now = fixture_clock(&root);
        filetime::set_file_mtime(p.join("sub/file.txt"), filetime::FileTime::from_system_time(now + DAY)).unwrap();
        let report = clean_previews(&root, PREVIEW_AGE, now, 0, 0);
        assert_eq!(report.removed, 0);
        assert_eq!(report.protected, 1);
        assert_eq!(clean_previews(&root, PREVIEW_AGE, now + DAY * 2, 0, 0).removed, 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn preview_budget_protects_live_locked_young_and_unknown_sessions() {
        let root = fixture("previews");
        let pid = fixture_dead_owner();
        let old = preview(&root, &format!(".ndr-entry-{pid}-10"));
        let locked = preview(&root, &format!(".ndr-entry-{pid}-11"));
        let live = preview(&root, &format!(".ndr-entry-{}-12", std::process::id()));
        let unknown = preview(&root, ".ndr-entry-unrecognized");
        let guard = OpenOptions::new().read(true).share_mode(7).open(locked.join("sub/file.txt")).unwrap();
        let now = fixture_clock(&root);
        let r = clean_previews(&root, PREVIEW_AGE, now, 0, 0);
        assert_eq!(r.removed, 0, "minimum age");
        let r = clean_previews(&root, PREVIEW_AGE, now + DAY * 2, 0, 0);
        assert_eq!(r.removed, 1);
        assert!(!old.exists());
        for p in [&locked,&live,&unknown] { assert_eq!(fs::read(p.join("sub/file.txt")).unwrap(), b"preview"); }
        drop(guard);
        assert_eq!(clean_previews(&root, PREVIEW_AGE, now + DAY * 2, 0, 0).removed, 1);
        fs::remove_dir_all(root).unwrap();
    }
    fn update(root: &Path, name: &str, success: bool) -> PathBuf {
        let p = root.join(name); fs::create_dir_all(p.join("backup")).unwrap(); fs::create_dir_all(p.join("extracted")).unwrap();
        fs::write(p.join("backup/NDR.exe"), b"rollback").unwrap(); fs::write(p.join("extracted/NDR.exe"), [1u8; 32]).unwrap();
        fs::write(p.join("NDR-windows-x64.zip"), [2u8; 64]).unwrap();
        fs::write(p.join("update.log"), if success { "Update installed: 132 files" } else { "Update failed: interrupted" }).unwrap();
        p
    }
    #[test]
    fn update_retention_preserves_two_backups_failed_and_active_work() {
        let root = fixture("updates");
        let pid = fixture_dead_owner();
        let a = update(&root,&format!("ndr-update-build-21-{pid}-100"),true);
        fs::write(a.join("installer-host.log"), b"diagnostic").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let b = update(&root,&format!("ndr-update-build-22-{pid}-101"),true);
        std::thread::sleep(Duration::from_millis(20));
        let c = update(&root,&format!("ndr-update-build-23-{pid}-102"),true);
        let failed = update(&root,&format!("ndr-update-build-24-{pid}-103"),false);
        let live = update(&root,&format!("ndr-update-build-25-{}-104",std::process::id()),true);
        let unknown = update(&root,"other",true);
        let installing = update(&root,&format!("ndr-update-build-26-{pid}-105"),true);
        let install_guard = OpenOptions::new().read(true).write(true).create_new(true).share_mode(0)
            .open(installing.join(".ndr-install.lock")).unwrap();
        let now = fixture_clock(&root);
        assert_eq!(clean_updates(&root,now,0,2).removed,0);
        let r = clean_updates(&root,now + DAY * 2,u64::MAX,2);
        assert_eq!(r.removed,1);
        assert_eq!(r.trimmed,4);
        assert!(!a.exists());
        for p in [&b,&c] { assert_eq!(fs::read(p.join("backup/NDR.exe")).unwrap(),b"rollback"); assert!(!p.join("extracted").exists()); assert!(!p.join("NDR-windows-x64.zip").exists()); }
        for p in [&failed,&live,&unknown,&installing] { assert!(p.join("extracted/NDR.exe").is_file()); }
        // The newest rollback is protected even under a zero-byte target.
        clean_updates(&root,now + DAY * 2,0,2);
        assert!(c.join("backup/NDR.exe").is_file());
        assert!(failed.join("backup/NDR.exe").is_file());
        assert!(installing.join("extracted/NDR.exe").is_file());
        drop(install_guard);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cleanup_never_traverses_a_junction_or_runs_concurrently() {
        let root = fixture("reparse");
        let outside = fixture("outside");
        fs::write(outside.join("personal.txt"), b"keep").unwrap();
        let p = root.join(format!(".ndr-entry-{}-1",fixture_dead_owner())); fs::create_dir_all(&p).unwrap();
        let status = std::process::Command::new("cmd.exe").args(["/c","mklink","/J"])
            .arg(p.join("link")).arg(&outside).output().unwrap();
        assert!(status.status.success(), "{:?}", status);
        let r = clean_previews(&root, PREVIEW_AGE, SystemTime::now()+DAY*10,0,0);
        assert_eq!(r.unknown,1);
        assert_eq!(fs::read(outside.join("personal.txt")).unwrap(),b"keep");
        let guard = maintenance_lock(&root).unwrap();
        assert!(maintenance_lock(&root).is_none());
        assert_eq!(clean_previews(&root, PREVIEW_AGE,SystemTime::now()+DAY*10,0,0),CleanupReport::default());
        drop(guard);
        fs::remove_dir(p.join("link")).unwrap();
        fs::remove_dir_all(root).unwrap();fs::remove_dir_all(outside).unwrap();
    }
}
