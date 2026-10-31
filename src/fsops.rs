//! Worker filesystem operations. Conflicts are answered through the event reply channel.
use crate::compat::{self, DescriptionTransfer};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::SystemTime,
};
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub hidden: bool,
    /// Windows attribute bits (FILE_ATTRIBUTE_*). 0 for remote and archive items.
    pub attributes: u32,
    /// Original TNXFileItem kind 4 (attribute 0x40000, FUN_007d2c54): drive items appended at the end of the list (path = "C:\").
    /// Never selected or a job target; opening one goes to that drive.
    pub drive: bool,
}
impl Entry {
    /// ".." (kind 1, attribute 0x10) that original GetList (0x45a34c), archive (0x45ecd0) and SFTP (0x45bb0c) put first at non-root locations.
    pub fn is_parent(&self) -> bool {
        self.is_dir && !self.drive && self.name == ".."
    }
    /// The ".." item. Its path is the parent location (opening goes up) and its time the current folder's time (original GetFolderTime).
    pub fn parent_item(path: PathBuf, modified: Option<SystemTime>) -> Self {
        Self { path, name: "..".into(), is_dir: true, size: 0, modified, hidden: false, attributes: 0x10, drive: false }
    }
}
#[derive(Debug)]
pub enum Operation {
    Copy {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        move_files: bool,
    },
    Delete {
        sources: Vec<PathBuf>,
        recycle: bool,
    },
    /// C# DeleteWithSkippedPaths: permanent delete, keeping items under `skipped` and their parent folders.
    DeleteSkipping {
        sources: Vec<PathBuf>,
        skipped: Vec<PathBuf>,
    },
    /// C# FileOperationService.Shred: overwrites files three times with random values, 0 and 0xFF, then renames them to random names and deletes them.
    Shred {
        sources: Vec<PathBuf>,
    },
    /// The delete job of original miEmptyTempClick (0x434344) (mode 2, flags 0xF0): permanently deletes only the items inside the folder and keeps the folder (0x10),
    /// without asking about attributes (0x20), continuing without asking on errors (0x40) and without notices (0x80).
    EmptyFolder {
        folder: PathBuf,
    },
    Mkdir {
        path: PathBuf,
    },
    Rename {
        source: PathBuf,
        destination: PathBuf,
    },
}
/// C# FileConflictAction(Abort = Cancel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    Overwrite,
    /// Overwrites only when the source is newer (otherwise skips).
    OverwriteNewer,
    ReplaceFolder,
    Skip,
    /// C# GetAvailableTarget: the first free name from "name (2).ext".
    Rename,
    /// When the existing file equals the front of the source, writes only the rest (file to file, when the destination is smaller).
    Resume,
    Cancel,
}
#[derive(Debug, Clone)]
pub struct OperationOptions {
    pub remove_read_only: bool,
    pub use_descriptions: bool,

    /// C# FileOperationControl.ErrorResolver: asks on each error with Event::Failure (otherwise Event::Error).
    pub interactive_errors: bool,
    /// Original conflict 0x10 (rename): asks for a new name with Event::RenameTarget instead of an automatic name (C# GetAvailableTarget).
    pub prompt_rename: bool,
}
impl Default for OperationOptions {
    fn default() -> Self {
        Self {
            remove_read_only: false,
            use_descriptions: true,
            interactive_errors: false,
            prompt_rename: false,
        }
    }
}
pub fn run(operation: Operation, events: Sender<Event>, cancel: Arc<AtomicBool>) {
    run_with_options(operation, events, cancel, OperationOptions::default())
}
#[derive(Debug)]
pub enum Event {
    Progress {
        path: PathBuf,
        bytes: u64,
        files: u64,
    },
    /// Original progress window lTo (0x412308): the real destination path of the file being copied (source) (including names changed by conflicts).
    Target {
        source: PathBuf,
        destination: PathBuf,
    },
    Conflict {
        source: PathBuf,
        destination: PathBuf,
        /// C# FileConflictInfo.AllowResume (false for extraction and remote).
        allow_resume: bool,
        reply: Sender<ConflictChoice>,
    },
    /// An error that stopped the whole job (C# FatalError). Not sent when stopped by cancel.
    Fatal(String),
    Password {
        archive: PathBuf,
        reply: Sender<Option<String>>,
    },
    Error {
        path: PathBuf,
        message: String,
    },
    /// C# FileOperationControl.ReportError (with an ErrorResolver): answer true = continue. With can_continue false it only notifies.
    Failure {
        path: PathBuf,
        message: String,
        can_continue: bool,
        reply: Sender<bool>,
    },
    /// Items skipped at conflicts (C# SkippedItems).
    Skipped {
        path: PathBuf,
    },
    /// Total counts and bytes learned during the job (extraction learns them after reading the listing). Used by the progress bar and lCount.
    Totals {
        files: Option<u64>,
        bytes: Option<u64>,
    },
    /// Original FUN_00411bb4 ("rename" at a conflict): asks for a new name (TITLE_RENAME_AND_COPY). None = cancel.
    RenameTarget {
        name: String,
        reply: Sender<Option<String>>,
    },
    /// Original 0x439593: the next volume (name) is missing -> TfMsgBox(TITLE_ERROR, ARC_CHANGEVOL %s, MB_OKCANCEL | MB_ICONEXCLAMATION). Answer true (OK) = retry.
    ChangeVolume {
        volume: String,
        reply: Sender<bool>,
    },
    Finished {
        cancelled: bool,
        files: u64,
        bytes: u64,
    },
}
pub fn run_with_options(
    operation: Operation,
    events: Sender<Event>,
    cancel: Arc<AtomicBool>,
    options: OperationOptions,
) {
    let mut j = Job {
        events,
        cancel,
        files: 0,
        bytes: 0,
        descriptions: std::collections::BTreeMap::new(),
        transfers: Vec::new(),
        options,
        quiet: false,
    };
    let result = (|| -> Result<(), String> {
        check_cancel(&j.cancel)?;
        match operation {
            Operation::Copy {
                sources,
                destination,
                move_files,
            } => {
                ensure_plain_path(&destination)?;
                fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
                // Original GetFileListToWork (FUN_00413ac4, +0x868 0 copy / 1 move): before the work, folders are walked (FUN_004138d4)
                // so the progress window knows every file and byte. A move inside one drive renames, so the original counts it as one item
                // (4000 bytes) unless the destination folder already exists. NDR always counts the files below (the rename is quick anyway).
                let (files, bytes) = tree_totals(&sources, &destination, move_files, j.options.use_descriptions, &j.cancel);
                let _ = j.events.send(Event::Totals { files: Some(files), bytes: Some(bytes) });
                let count = sources.len();
                for (index, s) in sources.into_iter().enumerate() {
                    j.check()?;
                    // C# CanContinueAfterError: asks "continue?" only when sources remain after this one.
                    let boundary = index + 1 < count;
                    let result = (|| {
                        let name = s.file_name().ok_or("A filesystem root cannot be copied")?;
                        let mut target = destination.join(name);
                        if absolute(&s)?.to_string_lossy().eq_ignore_ascii_case(&absolute(&target)?.to_string_lossy()) {
                            // A move in place has nothing to do.
                            if move_files {
                                return Ok(());
                            }
                            // Original FUN_004134e0: copying into the same folder finds the destination (itself), so it asks TfFileExists, and for rename (0x10)
                            // gets a new name from FUN_00411bb4 and copies under it. Fixed: other answers such as overwrite could destroy the source, so they
                            // skip (the original tries CopyFileW onto itself).
                            let is_dir = fs::symlink_metadata(&s).map(|m| m.is_dir()).map_err(|e| e.to_string())?;
                            match j.conflict(&s, &target)? {
                                ConflictChoice::Rename if j.options.prompt_rename => match j.ask_rename(name.to_string_lossy().into_owned())? {
                                    Some(new) if !new.trim().is_empty() && !new.contains(['\\', '/']) && !new.eq_ignore_ascii_case(&name.to_string_lossy()) => {
                                        target = target.with_file_name(new);
                                        if fs::symlink_metadata(&target).is_ok() {
                                            target = available_target(&target, is_dir)?;
                                        }
                                    }
                                    _ => {
                                        j.skipped(&s);
                                        return Ok(());
                                    }
                                },
                                ConflictChoice::Rename => target = available_target(&target, is_dir)?,
                                _ => {
                                    j.skipped(&s);
                                    return Ok(());
                                }
                            }
                        }
                        if inside(&s, &target)? {
                            return Err("Destination is the source or lies inside it".into());
                        }
                        j.copy(&s, &target, move_files, boundary).map(|_| ())
                    })();
                    if let Err(message) = result {
                        if j.cancel.load(Ordering::Relaxed) {
                            return Err(message);
                        }
                        j.report(s, message, boundary)?;
                    }
                }
            }
            Operation::Delete {
                sources,
                recycle: use_bin,
            } => {
                // C# DeleteWithResult: deleted item count (files) = the chosen items for the Recycle Bin, every deleted file and folder for permanent delete.
                let count = sources.len();
                for (index, p) in sources.into_iter().enumerate() {
                    j.check()?;
                    let boundary = index + 1 < count;
                    let result = if use_bin {
                        if fs::symlink_metadata(&p).is_err() {
                            continue;
                        }
                        recycle(&p).map(|_| j.files += 1)
                    } else {
                        j.delete_filtered(&p, &[], 0, boundary).map(|_| ())
                    };
                    match result {
                        Ok(()) => j.remove_description(&p),
                        Err(message) => {
                            if j.cancel.load(Ordering::Relaxed) {
                                return Err(message);
                            }
                            j.report(p, message, boundary)?;
                        }
                    }
                }
            }
            Operation::DeleteSkipping { sources, skipped } => {
                let count = sources.len();
                for (index, p) in sources.into_iter().enumerate() {
                    j.check()?;
                    let boundary = index + 1 < count;
                    match j.delete_filtered(&p, &skipped, 0, boundary) {
                        Ok(Disposition::Deleted) => j.remove_description(&p),
                        Ok(_) => {}
                        Err(message) => {
                            if j.cancel.load(Ordering::Relaxed) {
                                return Err(message);
                            }
                            j.report(p, message, boundary)?;
                        }
                    }
                }
            }
            Operation::EmptyFolder { folder } => {
                j.quiet = true;
                let children: Vec<PathBuf> = match fs::read_dir(&folder) {
                    Ok(items) => items.filter_map(|item| item.ok().map(|i| i.path())).collect(),
                    Err(_) => Vec::new(),
                };
                for child in children {
                    j.check()?;
                    if j.delete_filtered(&child, &[], 0, true).is_err() && j.cancel.load(Ordering::Relaxed) {
                        return Err("Cancelled".into());
                    }
                }
            }
            Operation::Shred { sources } => {
                for p in sources {
                    check_cancel(&j.cancel)?;
                    if fs::symlink_metadata(&p).is_err() {
                        continue;
                    }
                    match j.shred(&p, 0) {
                        Ok(()) => j.remove_description(&p),
                        Err(message) => {
                            let _ = j.events.send(Event::Error { path: p, message });
                        }
                    }
                }
            }
            Operation::Mkdir { path } => {
                ensure_plain_path(&path)?;
                fs::create_dir(&path).map_err(|e| e.to_string())?;
                j.progress(&path);
            }
            Operation::Rename {
                source,
                destination,
            } => {
                ensure_plain_path(&source)?;
                ensure_plain_path(&destination)?;
                if destination.exists() {
                    return Err("Rename destination already exists".into());
                }
                if inside(&source, &destination)? {
                    return Err("Invalid rename destination".into());
                }
                let description = j.description(&source)?;
                rename_new(&source, &destination)?;
                j.record_description(&source, &destination, description, true);
                j.files += 1;
                j.progress(&destination);
            }
        }
        Ok(())
    })();
    if let Err(message) = result {
        if !j.cancel.load(Ordering::Relaxed) {
            let _ = j.events.send(Event::Fatal(message));
        }
    }
    j.flush_descriptions();
    let _ = j.events.send(Event::Finished {
        cancelled: j.cancel.load(Ordering::Relaxed),
        files: j.files,
        bytes: j.bytes,
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    pub(super) fn fixture() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "ndr-core-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    /// Original GetFileListToWork (FUN_00413ac4) + FUN_004138d4: the totals of a folder copy are known before the first file.
    /// A move inside the same drive whose destination name is free counts as one item (renamed as a whole).
    #[test]
    fn copy_reports_tree_totals_before_the_first_file() {
        let p = fixture();
        fs::create_dir_all(p.join("in/a/sub")).unwrap();
        fs::write(p.join("in/a/x.txt"), [0u8; 5]).unwrap();
        fs::write(p.join("in/a/sub/y.txt"), [0u8; 7]).unwrap();
        let events = copy(&p, false, false, ConflictChoice::Overwrite);
        let totals = events.iter().position(|e| matches!(e, Event::Totals { .. })).expect("totals");
        let first_progress = events.iter().position(|e| matches!(e, Event::Progress { .. })).expect("progress");
        assert!(totals < first_progress, "totals come first");
        assert!(matches!(events[totals], Event::Totals { files: Some(2), bytes: Some(12) }));
        let cancel = AtomicBool::new(false);
        assert_eq!(tree_totals(&[p.join("out/a")], &p.join("other"), true, false, &cancel), (1, 0), "a same-drive move renames the folder");
        assert_eq!(tree_totals(&[p.join("out/a")], &p.join("in"), true, false, &cancel), (2, 12), "the destination name exists, so its files are moved one by one");
        fs::remove_dir_all(&p).unwrap();
    }
    #[test]
    fn unique_path_skips_names_left_by_an_earlier_process_with_the_same_pid() {
        let p = fixture();
        let first = unique_path(&p, "probe");
        let id: u64 = first.file_name().unwrap().to_string_lossy().rsplit('-').next().unwrap().parse().unwrap();
        // Next-number names left by an earlier process with the same PID.
        let left: Vec<PathBuf> = (1..=3).map(|n| p.join(format!(".ndr-probe-{}-{}", std::process::id(), id + n))).collect();
        for path in &left {
            fs::create_dir(path).unwrap();
        }
        let next = unique_path(&p, "probe");
        assert!(!next.exists(), "{}", next.display());
        assert!(!left.contains(&next));
        fs::create_dir(&next).unwrap();
        fs::remove_dir_all(&p).unwrap();
    }
    pub(super) fn copy(p: &Path, moving: bool, cancel: bool, choice: ConflictChoice) -> Vec<Event> {
        let (tx, rx) = mpsc::channel();
        let s = p.join("in/a");
        let d = p.join("out");
        let flag = Arc::new(AtomicBool::new(cancel));
        let h = thread::spawn(move || {
            run(
                Operation::Copy {
                    sources: vec![s],
                    destination: d,
                    move_files: moving,
                },
                tx,
                flag,
            )
        });
        let mut all = Vec::new();
        for e in rx {
            if let Event::Conflict { reply, .. } = &e {
                reply.send(choice).unwrap();
            }
            all.push(e);
        }
        h.join().unwrap();
        all
    }
    #[test]
    fn cancelled_resume_restores_the_existing_part() {
        let p = fixture();
        fs::create_dir(p.join("in")).unwrap();
        fs::create_dir(p.join("out")).unwrap();
        let data: Vec<u8> = (0..48u32 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        fs::write(p.join("in/a"), &data).unwrap();
        let existing = 1024 * 1024;
        fs::write(p.join("out/a"), &data[..existing]).unwrap();
        let (tx, rx) = mpsc::channel();
        let flag = Arc::new(AtomicBool::new(false));
        let (s, d, worker_flag) = (p.join("in/a"), p.join("out"), flag.clone());
        let h = thread::spawn(move || run(Operation::Copy { sources: vec![s], destination: d, move_files: false }, tx, worker_flag));
        let mut resumed = false;
        for e in rx {
            match e {
                Event::Conflict { reply, .. } => {
                    reply.send(ConflictChoice::Resume).unwrap();
                    resumed = true;
                }
                // Cancel after the first resumed chunk.
                Event::Progress { bytes, .. } if resumed && bytes > 0 => flag.store(true, Ordering::SeqCst),
                _ => {}
            }
        }
        h.join().unwrap();
        assert!(resumed);
        assert_eq!(fs::metadata(p.join("out/a")).unwrap().len(), existing as u64, "the partly appended bytes are cut off again");
        assert_eq!(fs::read(p.join("out/a")).unwrap(), &data[..existing]);
        fs::remove_dir_all(p).unwrap();
    }
    pub(super) fn prepared() -> PathBuf {
        let p = fixture();
        fs::create_dir(p.join("in")).unwrap();
        fs::create_dir(p.join("out")).unwrap();
        fs::write(p.join("in/a"), b"new").unwrap();
        fs::write(p.join("out/a"), b"old").unwrap();
        p
    }
    #[test]
    fn overwrite_requires_reply_and_replaces_only_on_success() {
        let p = prepared();
        let events = copy(&p, false, false, ConflictChoice::Overwrite);
        assert!(events.iter().any(|e| matches!(e, Event::Conflict { .. })));
        assert_eq!(fs::read(p.join("out/a")).unwrap(), b"new");
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn cancellation_preserves_existing_destination() {
        let p = prepared();
        let events = copy(&p, true, true, ConflictChoice::Overwrite);
        assert_eq!(fs::read(p.join("out/a")).unwrap(), b"old");
        assert!(p.join("in/a").exists());
        assert!(events.iter().any(|e| matches!(
            e,
            Event::Finished {
                cancelled: true,
                ..
            }
        )));
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn move_only_removes_successful_source() {
        let p = prepared();
        copy(&p, true, false, ConflictChoice::Skip);
        assert!(p.join("in/a").exists());
        copy(&p, true, false, ConflictChoice::Overwrite);
        assert!(!p.join("in/a").exists());
        assert_eq!(fs::read(p.join("out/a")).unwrap(), b"new");
        fs::remove_dir_all(p).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn junction_is_never_followed() {
        let p = fixture();
        fs::create_dir(p.join("outside")).unwrap();
        fs::write(p.join("outside/secret"), b"safe").unwrap();
        let result = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(p.join("link"))
            .arg(p.join("outside"))
            .output()
            .unwrap();
        assert!(result.status.success(), "junction fixture failed");
        let (tx, rx) = mpsc::channel();
        run(
            Operation::Delete {
                sources: vec![p.join("link")],
                recycle: false,
            },
            tx,
            Arc::new(AtomicBool::new(false)),
        );
        let events: Vec<_> = rx.into_iter().collect();
        // C# DeleteFilteredPath: the junction itself is removed, its target is never traversed.
        assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })), "{events:?}");
        assert!(fs::symlink_metadata(p.join("link")).is_err());
        assert_eq!(fs::read(p.join("outside/secret")).unwrap(), b"safe");
        fs::remove_dir_all(p).unwrap();
    }
    fn run_events(operation: Operation) -> Vec<Event> {
        let (tx, rx) = mpsc::channel();
        run(operation, tx, Arc::new(AtomicBool::new(false)));
        rx.into_iter().collect()
    }
    #[test]
    fn permanent_delete_skips_kept_items_and_clears_read_only() {
        let p = fixture();
        fs::create_dir_all(p.join("tree/keep")).unwrap();
        fs::write(p.join("tree/keep/protected.txt"), b"keep").unwrap();
        fs::write(p.join("tree/gone.txt"), b"gone").unwrap();
        fs::write(p.join("readonly.txt"), b"ro").unwrap();
        let mut permissions = fs::metadata(p.join("readonly.txt")).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(p.join("readonly.txt"), permissions).unwrap();
        let protected = find_protected_delete_items(&[p.join("tree"), p.join("readonly.txt")]);
        assert_eq!(protected.len(), 1);
        assert!(protected[0].0.ends_with("readonly.txt") && protected[0].1 & 1 == 1);
        let events = run_events(Operation::DeleteSkipping { sources: vec![p.join("tree"), p.join("readonly.txt")], skipped: vec![p.join("tree/keep/protected.txt")] });
        assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })), "{events:?}");
        assert!(p.join("tree/keep/protected.txt").exists(), "skipped item kept");
        assert!(p.join("tree").exists() && !p.join("tree/gone.txt").exists(), "parents of skipped items stay, the rest goes");
        assert!(!p.join("readonly.txt").exists(), "read-only cleared before delete");
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn shred_overwrites_and_removes_files_and_folders() {
        let p = fixture();
        fs::create_dir_all(p.join("folder/sub")).unwrap();
        fs::write(p.join("folder/sub/a.bin"), vec![7u8; 3 * 1024 * 1024 + 5]).unwrap();
        fs::write(p.join("b.txt"), b"secret").unwrap();
        let events = run_events(Operation::Shred { sources: vec![p.join("folder"), p.join("b.txt"), p.join("missing")] });
        assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })), "{events:?}");
        assert!(!p.join("folder").exists() && !p.join("b.txt").exists());
        assert!(fs::read_dir(&p).unwrap().next().is_none(), "no temporary shred names remain");
        fs::remove_dir_all(p).unwrap();
    }
}
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, Write},
    sync::mpsc,
    time::Duration,
};
/// Original FUN_004138d4 (inside GetFileListToWork FUN_00413ac4): files and bytes below the chosen items, so the progress window
/// knows the totals before copying. Reparse points and (with descriptions on) description files are not counted because the copy
/// does not take them. For a move, an item that can be renamed as a whole (same root, destination name free) counts as one item,
/// like the original's same-drive branch.
fn tree_totals(sources: &[PathBuf], destination: &Path, moving: bool, descriptions: bool, cancel: &AtomicBool) -> (u64, u64) {
    let root = |p: &Path| p.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase());
    let (mut files, mut bytes) = (0u64, 0u64);
    for source in sources {
        let Ok(meta) = fs::symlink_metadata(source) else {
            files += 1;
            continue;
        };
        let target = source.file_name().map(|name| destination.join(name));
        if moving && root(source) == root(destination) && target.as_ref().is_some_and(|t| fs::symlink_metadata(t).is_err()) {
            files += 1;
            continue;
        }
        if !meta.is_dir() || is_reparse(&meta) {
            files += 1;
            bytes = bytes.saturating_add(meta.len());
            continue;
        }
        let mut pending = vec![source.clone()];
        while let Some(directory) = pending.pop() {
            if cancel.load(Ordering::Relaxed) {
                return (files, bytes);
            }
            let Ok(read) = fs::read_dir(&directory) else { continue };
            for item in read.flatten() {
                // DirEntry::metadata on Windows does not follow links (FindNextFileW values).
                let Ok(m) = item.metadata() else { continue };
                if is_reparse(&m) || (descriptions && compat::is_description_file(&item.file_name().to_string_lossy())) {
                    continue;
                }
                if m.is_dir() {
                    pending.push(item.path());
                } else {
                    files += 1;
                    bytes = bytes.saturating_add(m.len());
                }
            }
        }
    }
    (files, bytes)
}
/// Reject reparse points in every existing component, including destination ancestors.
pub fn ensure_plain_path(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) if is_reparse(&m) => {
                return Err(format!("Reparse point is not allowed: {}", p.display()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
pub fn is_reparse(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        m.file_type().is_symlink()
    }
}
pub fn list_dir(path: &Path, show_hidden: bool) -> Result<Vec<Entry>, String> {
    ensure_plain_path(path)?;
    let mut entries = Vec::new();
    for item in fs::read_dir(path).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        // On Windows DirEntry::metadata uses the values FindNextFileW gave (attributes, size, times) as is and does not follow links
        // (same meaning as symlink_metadata). Much faster on large folders than symlink_metadata, which opens each file.
        let m = item.metadata().map_err(|e| e.to_string())?;
        let name = item.file_name().to_string_lossy().into_owned();
        #[cfg(windows)]
        let attributes = {
            use std::os::windows::fs::MetadataExt;
            m.file_attributes()
        };
        #[cfg(not(windows))]
        let attributes = if name.starts_with('.') { 2 } else { 0 };
        let hidden = attributes & 2 != 0;
        // C# ListLocal: with hidden display off, items with hidden or system attributes are skipped (AttributesToSkip Hidden | System).
        if show_hidden || attributes & (2 | 4) == 0 {
            entries.push(Entry {
                path: item.path(),
                name,
                is_dir: m.is_dir(),
                size: if m.is_dir() { 0 } else { m.len() },
                modified: m.modified().ok(),
                hidden,
                attributes,
                drive: false,
            });
        }
    }
    Ok(entries)
}
pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Cancelled".into())
    } else {
        Ok(())
    }
}
/// Keep ordinary source content stable against concurrent Windows writers.
pub(crate) fn open_read(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    options.open(path).map_err(|e| e.to_string())
}
pub(crate) fn unique_path(parent: &Path, prefix: &str) -> PathBuf {
    use std::sync::atomic::AtomicU64;
    static ID: AtomicU64 = AtomicU64::new(0);
    // Skip names left by an earlier process that used the same number (PID) (e.g. %TEMP%\NDR\.ndr-entry-PID-0).
    // Numbers never collide within one process, and two processes with the same PID cannot exist at once.
    loop {
        let candidate = parent.join(format!(
            ".ndr-{}-{}-{}",
            prefix,
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
}
/// Atomically replace a file; an existing destination is never truncated first.
pub(crate) fn commit_file(stage: &Path, target: &Path, overwrite: bool) -> Result<(), String> {
    ensure_plain_path(target)?;
    if !overwrite {
        #[cfg(windows)]
        {
            return rename_new(stage, target);
        }
        #[cfg(not(windows))]
        {
            fs::hard_link(stage, target).map_err(|e| e.to_string())?;
            fs::remove_file(stage).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
        }
        let a: Vec<_> = stage.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<_> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 1 | 8) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(stage, target).map_err(|e| e.to_string())
    }
}
/// Rename without replacing a destination that appeared after the initial check.
pub(crate) fn rename_new(source: &Path, target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
        }
        let a: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let b: Vec<_> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { MoveFileExW(a.as_ptr(), b.as_ptr(), 8) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if target.exists() {
            return Err("Destination exists".into());
        }
        fs::rename(source, target).map_err(|e| e.to_string())
    }
}

pub(crate) fn preserve_times(file: &File, metadata: &fs::Metadata) -> Result<(), String> {
    let mut times = fs::FileTimes::new();
    if let Ok(t) = metadata.modified() {
        times = times.set_modified(t);
    }
    if let Ok(t) = metadata.accessed() {
        times = times.set_accessed(t);
    }
    file.set_times(times).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
        #[repr(C)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetFileTime(
                handle: *mut std::ffi::c_void,
                created: *const FileTime,
                accessed: *const FileTime,
                modified: *const FileTime,
            ) -> i32;
        }
        let time = metadata.creation_time();
        let created = FileTime {
            low: time as u32,
            high: (time >> 32) as u32,
        };
        if unsafe {
            SetFileTime(
                file.as_raw_handle(),
                &created,
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}
struct Job {
    events: Sender<Event>,
    cancel: Arc<AtomicBool>,
    files: u64,
    bytes: u64,
    descriptions: std::collections::BTreeMap<PathBuf, std::collections::BTreeMap<String, String>>,
    transfers: Vec<DescriptionTransfer>,
    options: OperationOptions,
    /// Original flags 0x40|0x80 (EmptyFolder): errors are neither asked about nor reported.
    quiet: bool,
}
impl Job {
    /// Original FUN_00412b00: the read-only flag of a copied destination is cleared only when the setting (remreadonly) is on and the source is on a CD/DVD.
    /// Fixed: previously, with the setting on, read-only was cleared when copying from any drive.
    fn clear_copied_read_only(&self, source: &Path) -> bool {
        self.options.remove_read_only && on_optical_drive(source)
    }
    fn description(&mut self, path: &Path) -> Result<Option<String>, String> {
        if !self.options.use_descriptions {
            return Ok(None);
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            return Ok(None);
        };
        if compat::is_description_file(name) {
            return Ok(None);
        }
        let parent = path.parent().unwrap_or(Path::new("."));
        if !self.descriptions.contains_key(parent) {
            self.descriptions
                .insert(parent.to_owned(), compat::read_descriptions(parent, true)?);
        }
        Ok(self.descriptions[parent]
            .iter()
            .find(|(n, _)| n.to_lowercase() == name.to_lowercase())
            .map(|(_, s)| s.clone()))
    }
    fn record_description(
        &mut self,
        source: &Path,
        destination: &Path,
        description: Option<String>,
        move_entry: bool,
    ) {
        if !self.options.use_descriptions {
            return;
        }
        if let (Some(source_name), Some(destination_name)) = (
            source.file_name().and_then(|s| s.to_str()),
            destination.file_name().and_then(|s| s.to_str()),
        ) {
            if !compat::is_description_file(source_name) {
                self.transfers.push(DescriptionTransfer {
                    source_directory: source.parent().unwrap_or(Path::new(".")).to_owned(),
                    source_name: source_name.into(),
                    destination_directory: destination
                        .parent()
                        .unwrap_or(Path::new("."))
                        .to_owned(),
                    destination_name: destination_name.into(),
                    description,
                    move_entry,
                });
            }
        }
    }
    fn flush_descriptions(&mut self) -> bool {
        let errors = compat::apply_description_transfers(&std::mem::take(&mut self.transfers));
        let success = errors.is_empty();
        for message in errors {
            let _ = self.events.send(Event::Error {
                path: PathBuf::new(),
                message,
            });
        }
        success
    }
    fn remove_description(&self, path: &Path) {
        if !self.options.use_descriptions {
            return;
        }
        if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
            if !compat::is_description_file(name) {
                if let Err(message) = compat::remove_descriptions(
                    path.parent().unwrap_or(Path::new(".")),
                    &[name.into()],
                ) {
                    let _ = self.events.send(Event::Error {
                        path: path.to_owned(),
                        message,
                    });
                }
            }
        }
    }
    fn progress(&self, p: &Path) {
        let _ = self.events.send(Event::Progress {
            path: p.to_path_buf(),
            bytes: self.bytes,
            files: self.files,
        });
    }
    /// Check point: Err on cancel.
    fn check(&self) -> Result<(), String> {
        check_cancel(&self.cancel)
    }
    /// C# FileOperationControl.ReportError: reports the error. In ask mode waits for the answer; not continuing cancels (Err).
    fn report(&self, path: PathBuf, message: String, can_continue: bool) -> Result<(), String> {
        if self.quiet {
            return Ok(());
        }
        if !self.options.interactive_errors {
            let _ = self.events.send(Event::Error { path, message });
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        if self.events.send(Event::Failure { path, message, can_continue, reply: tx }).is_err() {
            return Err("UI disconnected".into());
        }
        let resume = loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".into());
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => break answer,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break true,
            }
        };
        if !resume && can_continue {
            self.cancel.store(true, Ordering::Relaxed);
            return Err("Cancelled".into());
        }
        Ok(())
    }
    fn skipped(&self, path: &Path) {
        let _ = self.events.send(Event::Skipped { path: path.to_owned() });
    }
    fn ask_rename(&self, name: String) -> Result<Option<String>, String> {
        let (tx, rx) = mpsc::channel();
        self.events.send(Event::RenameTarget { name, reply: tx }).map_err(|_| "UI disconnected".to_string())?;
        loop {
            check_cancel(&self.cancel)?;
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(answer) => return Ok(answer),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Ok(None),
            }
        }
    }
    fn conflict(&self, s: &Path, d: &Path) -> Result<ConflictChoice, String> {
        let (tx, rx) = mpsc::channel();
        let plain = |p: &Path| fs::symlink_metadata(p).map(|m| !is_reparse(&m)).unwrap_or(false);
        self.events
            .send(Event::Conflict {
                source: s.to_owned(),
                destination: d.to_owned(),
                allow_resume: plain(s) && plain(d),
                reply: tx,
            })
            .map_err(|_| "UI disconnected".to_string())?;
        loop {
            check_cancel(&self.cancel)?;
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ConflictChoice::Cancel) => {
                    self.cancel.store(true, Ordering::Relaxed);
                    return Err("Cancelled".into());
                }
                Ok(c) => return Ok(c),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err("Conflict reply disconnected".into()),
            }
        }
    }
    fn copy(&mut self, s: &Path, d: &Path, moving: bool, boundary: bool) -> Result<bool, String> {
        self.copy_depth(s, d, moving, 0, boundary)
    }
    /// `boundary`: true when work remains after this item (at any level) (C# CanContinueAfterError).
    /// Original FUN_00412b00 (file) asks MSG_WANTTOCONTINUE when top-level items remain after it; FUN_00412fbc (folder item) also asks when the
    /// last item's folder still has files after the failed one. Together that is "work remains after", as here. FUN_00412fbc first asks
    /// TfFileExists for an existing destination folder (skip 0x20 skips the whole folder, rename 0x10 repeats until the name is free), creates
    /// the folders and copies each file; the progress form's cancel stops with ERROR_CANCELLED (0x4d3).
    fn copy_depth(
        &mut self,
        s: &Path,
        d: &Path,
        moving: bool,
        depth: usize,
        boundary: bool,
    ) -> Result<bool, String> {
        if depth > 256 {
            return Err("Directory nesting exceeds the safe copy limit (256)".into());
        }
        self.check()?;
        ensure_plain_path(s)?;
        ensure_plain_path(d)?;
        let m = fs::symlink_metadata(s).map_err(|e| e.to_string())?;
        let description = self.description(s)?;
        let mut d = d.to_owned();
        let mut overwrite = false;
        if let Ok(existing) = fs::symlink_metadata(&d) {
            match self.conflict(s, &d)? {
                ConflictChoice::Skip => {
                    self.skipped(s);
                    return Ok(false);
                }
                ConflictChoice::Rename if self.options.prompt_rename => {
                    // Original FUN_00411bb4: only the file name of the destination path becomes the typed name. Cancel skips (the original proceeds as is).
                    let name = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    match self.ask_rename(name)? {
                        Some(new) if !new.trim().is_empty() && !new.contains(['\\', '/']) => d = d.with_file_name(new),
                        _ => {
                            self.skipped(s);
                            return Ok(false);
                        }
                    }
                    // If the new name exists too, use a free C#-style name instead of overwriting (difference: the original overwrites).
                    if fs::symlink_metadata(&d).is_ok() {
                        d = available_target(&d, m.is_dir())?;
                    }
                }
                ConflictChoice::Rename => d = available_target(&d, m.is_dir())?,
                ConflictChoice::OverwriteNewer => {
                    // C#: skip when File.GetLastWriteTimeUtc(source) <= destination.
                    if m.modified().ok() <= existing.modified().ok() {
                        self.skipped(s);
                        return Ok(false);
                    }
                    overwrite = true;
                }
                ConflictChoice::Resume => {
                    if !m.is_file() || !existing.is_file() {
                        return Err("Only two ordinary files can resume a partial copy".into());
                    }
                    return self.resume_file(s, &d, &m, moving, description);
                }
                ConflictChoice::Overwrite => overwrite = true,
                ConflictChoice::ReplaceFolder => {
                    if !m.is_dir() || !d.is_dir() {
                        return Err("ReplaceFolder requires two ordinary directories".into());
                    }
                    return self.replace_folder(s, &d, moving, description, depth, boundary);
                }
                ConflictChoice::Cancel => unreachable!(),
            }
        } else if moving && depth == 0 {
            // C# MoveAsync/MoveFileAsync: without a destination, first try a rename only (same volume). If that fails, copy then delete the source.
            if rename_new(s, &d).is_ok() {
                let _ = self.events.send(Event::Target { source: s.to_path_buf(), destination: d.to_path_buf() });
                self.files += 1;
                self.progress(s);
                self.record_description(s, &d, description, true);
                return Ok(true);
            }
        }
        if m.is_dir() {
            if d.exists() && !d.is_dir() {
                return Err("Cannot replace a file with a directory".into());
            }
            fs::create_dir_all(&d).map_err(|e| e.to_string())?;
            let mut all = true;
            // C# CopyDirectoryAsync: subfolders first, then files.
            let mut children = Vec::new();
            for item in fs::read_dir(s).map_err(|e| e.to_string())? {
                let item = item.map_err(|e| e.to_string())?;
                if self.options.use_descriptions
                    && compat::is_description_file(&item.file_name().to_string_lossy())
                {
                    continue;
                }
                let directory = item.metadata().map(|m| attribute_directory(&m)).unwrap_or(false);
                children.push((!directory, item.path(), item.file_name()));
            }
            children.sort_by_key(|(file, _, _)| *file);
            let count = children.len();
            for (index, (_, path, name)) in children.into_iter().enumerate() {
                let child_boundary = boundary || index + 1 < count;
                match self.copy_depth(&path, &d.join(name), moving, depth + 1, child_boundary) {
                    Ok(done) => all &= done,
                    Err(e) => {
                        if self.cancel.load(Ordering::Relaxed) {
                            return Err(e);
                        }
                        self.report(path, e, child_boundary)?;
                        all = false;
                    }
                }
            }
            if all {
                let mut options = OpenOptions::new();
                options.write(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    options.custom_flags(0x02000000);
                }
                let directory = options.open(&d).map_err(|e| e.to_string())?;
                preserve_times(&directory, &m)?;
                drop(directory);
                let mut permissions = m.permissions();
                if self.clear_copied_read_only(s) {
                    permissions.set_readonly(false)
                }
                fs::set_permissions(&d, permissions).map_err(|e| e.to_string())?;
            }
            if moving && all {
                // Original FUN_004153a4 (move, +0x868 = 1): after moving a folder's contents, RemoveDirectoryW(source) only when
                // FUN_0041110c finds it empty, so skipped or failed items stay in the source folder.
                if !self.flush_descriptions() {
                    return Err("File data copied, but source directory retained because description metadata could not be saved".into());
                }
                for item in fs::read_dir(s).map_err(|e| e.to_string())? {
                    let item = item.map_err(|e| e.to_string())?;
                    if self.options.use_descriptions
                        && compat::is_description_file(&item.file_name().to_string_lossy())
                    {
                        fs::remove_file(item.path()).map_err(|e| e.to_string())?;
                    }
                }
                fs::remove_dir(s).map_err(|e| e.to_string())?;
            }
            self.record_description(s, &d, description, moving && all);
            return Ok(all);
        }
        if !m.is_file() {
            return Err("Only ordinary files and directories can be copied".into());
        }
        if d.is_dir() {
            return Err("Cannot replace a directory with a file".into());
        }
        // Original 0x412308: progress window lTo = folder of the current destination file. Report the destination as settled by conflict handling, including changed names.
        let _ = self.events.send(Event::Target { source: s.to_path_buf(), destination: d.to_path_buf() });
        let stage = unique_path(d.parent().ok_or("No destination parent")?, "copy");
        let result = (|| {
            let mut input = open_read(s)?;
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage)
                .map_err(|e| e.to_string())?;
            let mut buf = vec![0; 1024 * 1024];
            let mut copied = 0u64;
            loop {
                self.check()?;
                let n = input.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                self.bytes += n as u64;
                copied += n as u64;
                self.progress(s);
            }
            let final_source = input.metadata().map_err(|e| e.to_string())?;
            if copied != m.len()
                || final_source.len() != m.len()
                || final_source.modified().ok() != m.modified().ok()
            {
                return Err("Source changed during copy; existing destination retained".into());
            }
            preserve_times(&out, &m)?;
            let mut permissions = m.permissions();
            if self.clear_copied_read_only(s) {
                permissions.set_readonly(false)
            }
            out.set_permissions(permissions)
                .map_err(|e| e.to_string())?;
            out.sync_all().map_err(|e| e.to_string())?;
            drop(out);
            check_cancel(&self.cancel)?;
            // Original FUN_004134e0 (overwrite): clears the existing destination's attributes (SetFileAttributesW 0x20) and replaces it regardless of the setting.
            // Fixed: previously a read-only destination could not be overwritten when the read-only clearing setting was off.
            let original_permissions = if overwrite {
                fs::metadata(&d)
                    .ok()
                    .map(|m| m.permissions())
                    .filter(|p| p.readonly())
            } else {
                None
            };
            if let Some(permissions) = &original_permissions {
                ensure_plain_path(&d)?;
                let mut writable = permissions.clone();
                writable.set_readonly(false);
                fs::set_permissions(&d, writable).map_err(|e| e.to_string())?;
            }
            let committed =
                check_cancel(&self.cancel).and_then(|_| commit_file(&stage, &d, overwrite));
            if let Err(error) = committed {
                if let Some(permissions) = original_permissions {
                    ensure_plain_path(&d)?;
                    if let Err(restore) = fs::set_permissions(&d, permissions) {
                        return Err(format!(
                            "{error}; original destination attributes could not be restored: {restore}"
                        ));
                    }
                }
                return Err(error);
            }
            drop(input);
            self.files += 1;
            self.progress(s);
            self.record_description(s, &d, description, false);
            if moving {
                check_cancel(&self.cancel)?;
                fs::remove_file(s).map_err(|e| e.to_string())?;
                if let Some(transfer) = self.transfers.last_mut() {
                    if transfer.source_directory.join(&transfer.source_name) == s {
                        transfer.move_entry = true;
                    }
                }
            }
            Ok(true)
        })();
        if stage.exists() {
            let _ = fs::remove_file(&stage);
        }
        result
    }
    /// C# DeleteFilteredPath: keeps items under skipped paths and does not delete their parent folders either. Links (reparse points) are not followed;
    /// only the link itself is deleted. The read-only attribute is cleared before deleting. A missing item counts as deleted.
    /// Each deleted file, folder or link increments files (C# deletedItems). Inner errors are reported at once (boundary = work remaining after).
    /// Same rules as the original delete engine: FUN_004156c4 (loop over the chosen items), FUN_00414884 (one file: SetFileAttributesW 0x20 when R/H/S, then DeleteFileW),
    /// FUN_00414dfc (a chosen folder) and FUN_00414b50 (its contents): a reparse point is removed with RemoveDirectoryW without entering it, otherwise the contents first,
    /// then SetFileAttributesW 0x10 and RemoveDirectoryW). The protected-item question (FUN_004150ac) is asked before the job here.
    fn delete_filtered(&mut self, p: &Path, skipped: &[PathBuf], depth: usize, boundary: bool) -> Result<Disposition, String> {
        if depth > 256 {
            return Err("Directory nesting exceeds the safe delete limit (256)".into());
        }
        self.check()?;
        if skipped.iter().any(|root| same_or_inside(p, root)) {
            return Ok(Disposition::Skipped);
        }
        if let Some(parent) = p.parent() {
            ensure_plain_path(parent)?;
        }
        let m = match fs::symlink_metadata(p) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Disposition::Deleted),
            Err(e) => return Err(e.to_string()),
        };
        if !is_reparse(&m) {
            clear_read_only(p, &m);
        }
        if is_reparse(&m) {
            remove_link(p, attribute_directory(&m))?;
            self.files += 1;
            self.progress(p);
            return Ok(Disposition::Deleted);
        }
        if m.is_dir() {
            let mut kept = None;
            let children: Vec<PathBuf> = fs::read_dir(p)
                .map_err(|e| e.to_string())?
                .map(|item| item.map(|i| i.path()).map_err(|e| e.to_string()))
                .collect::<Result<_, _>>()?;
            let count = children.len();
            for (index, child) in children.into_iter().enumerate() {
                let child_boundary = boundary || index + 1 < count;
                match self.delete_filtered(&child, skipped, depth + 1, child_boundary) {
                    Ok(Disposition::Deleted) => {}
                    Ok(other) => kept = Some(kept.map_or(other, |k| if k == Disposition::Failed { k } else { other })),
                    Err(message) => {
                        if self.cancel.load(Ordering::Relaxed) {
                            return Err(message);
                        }
                        self.report(child, message, child_boundary)?;
                        kept = Some(Disposition::Failed);
                    }
                }
            }
            if let Some(disposition) = kept {
                return Ok(disposition);
            }
            self.check()?;
            fs::remove_dir(p).map_err(|e| e.to_string())?;
            self.files += 1;
        } else {
            fs::remove_file(p).map_err(|e| e.to_string())?;
            self.files += 1;
            self.bytes += m.len();
        }
        self.progress(p);
        Ok(Disposition::Deleted)
    }
    /// C# ShredEntry.
    fn shred(&mut self, p: &Path, depth: usize) -> Result<(), String> {
        if depth > 256 {
            return Err("Directory nesting exceeds the safe delete limit (256)".into());
        }
        check_cancel(&self.cancel)?;
        let m = fs::symlink_metadata(p).map_err(|e| e.to_string())?;
        if is_reparse(&m) {
            return remove_link(p, attribute_directory(&m));
        }
        if m.is_dir() {
            for item in fs::read_dir(p).map_err(|e| e.to_string())? {
                self.shred(&item.map_err(|e| e.to_string())?.path(), depth + 1)?;
            }
            return fs::remove_dir(p).map_err(|e| e.to_string());
        }
        clear_read_only(p, &m);
        {
            let mut file = OpenOptions::new().write(true).open(p).map_err(|e| e.to_string())?;
            let length = m.len();
            let mut buffer = vec![0u8; 1024 * 1024];
            for pass in 0..3 {
                use std::io::{Seek, SeekFrom};
                file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
                let mut remaining = length;
                while remaining > 0 {
                    check_cancel(&self.cancel)?;
                    let count = remaining.min(buffer.len() as u64) as usize;
                    match pass {
                        0 => crate::native::random_bytes(&mut buffer[..count])?,
                        1 => buffer[..count].fill(0),
                        _ => buffer[..count].fill(0xFF),
                    }
                    file.write_all(&buffer[..count]).map_err(|e| e.to_string())?;
                    remaining -= count as u64;
                }
                file.sync_all().map_err(|e| e.to_string())?;
            }
            buffer.fill(0);
        }
        let parent = p.parent().ok_or("Cannot resolve the shredded file's parent folder")?;
        let mut randomized = None;
        for _ in 0..32 {
            let mut bytes = [0u8; 16];
            crate::native::random_bytes(&mut bytes)?;
            let name = format!(".ndr-shred-{}.tmp", bytes.iter().map(|b| format!("{b:02X}")).collect::<String>());
            let candidate = parent.join(name);
            if fs::symlink_metadata(&candidate).is_err() {
                randomized = Some(candidate);
                break;
            }
        }
        let randomized = randomized.ok_or("Could not create a unique temporary name for the shredded file")?;
        fs::rename(p, &randomized).map_err(|e| e.to_string())?;
        if let Err(delete) = fs::remove_file(&randomized) {
            return match fs::rename(&randomized, p) {
                Ok(()) => Err(delete.to_string()),
                Err(restore) => Err(format!(
                    "The shredded file could not be deleted or restored to its original name. It remains at: {} ({delete}; {restore})",
                    randomized.display()
                )),
            };
        }
        self.files += 1;
        self.bytes += m.len();
        self.progress(p);
        Ok(())
    }
    /// C# ResumeFileAsync: an error when the existing file is larger than the source or its front differs; otherwise writes the rest and matches times and attributes.
    /// A move deletes the source afterwards.
    fn resume_file(&mut self, s: &Path, d: &Path, m: &fs::Metadata, moving: bool, description: Option<String>) -> Result<bool, String> {
        let existing = fs::metadata(d).map_err(|e| e.to_string())?.len();
        if existing > m.len() {
            return Err("The existing file is larger than the source and cannot be resumed".into());
        }
        let mut input = open_read(s)?;
        {
            let mut current = open_read(d)?;
            let mut left = vec![0; 1024 * 1024];
            let mut right = vec![0; 1024 * 1024];
            let mut remaining = existing;
            while remaining > 0 {
                self.check()?;
                let n = remaining.min(left.len() as u64) as usize;
                input.read_exact(&mut left[..n]).map_err(|e| e.to_string())?;
                current.read_exact(&mut right[..n]).map_err(|e| e.to_string())?;
                if left[..n] != right[..n] {
                    return Err("The existing file does not match the start of the source; it was not resumed".into());
                }
                remaining -= n as u64;
            }
        }
        let original = fs::metadata(d).map_err(|e| e.to_string())?.permissions();
        if original.readonly() {
            let mut writable = original.clone();
            writable.set_readonly(false);
            fs::set_permissions(d, writable).map_err(|e| e.to_string())?;
        }
        let mut out = OpenOptions::new().write(true).open(d).map_err(|e| e.to_string())?;
        let written = (|| -> Result<(), String> {
            out.seek(std::io::SeekFrom::Start(existing)).map_err(|e| e.to_string())?;
            let mut buf = vec![0; 1024 * 1024];
            loop {
                self.check()?;
                let n = input.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                self.bytes += n as u64;
                self.progress(s);
            }
            out.sync_all().map_err(|e| e.to_string())
        })();
        if let Err(error) = written {
            // Fixed: cancelling or failing during a resume left the destination at an intermediate size. It is truncated back to its size before resuming.
            let restored = out.set_len(existing).and_then(|()| out.sync_all());
            drop(out);
            if original.readonly() {
                let _ = fs::set_permissions(d, original);
            }
            return Err(match restored {
                Ok(()) => error,
                Err(truncate) => format!("{error} (the partly resumed file could not be restored: {truncate})"),
            });
        }
        preserve_times(&out, m)?;
        drop(out);
        let mut permissions = m.permissions();
        if self.clear_copied_read_only(s) {
            permissions.set_readonly(false);
        }
        fs::set_permissions(d, permissions).map_err(|e| e.to_string())?;
        drop(input);
        self.files += 1;
        self.progress(s);
        self.record_description(s, d, description, false);
        if moving {
            self.check()?;
            fs::remove_file(s).map_err(|e| e.to_string())?;
            if let Some(transfer) = self.transfers.last_mut() {
                if transfer.source_directory.join(&transfer.source_name) == s {
                    transfer.move_entry = true;
                }
            }
        }
        Ok(true)
    }
    fn replace_folder(
        &mut self,
        source: &Path,
        target: &Path,
        moving: bool,
        description: Option<String>,
        depth: usize,
        boundary: bool,
    ) -> Result<bool, String> {
        let parent = target
            .parent()
            .ok_or("Replacement destination has no parent")?;
        let stage = unique_path(parent, "replace-stage");
        let backup = unique_path(parent, "replace-backup");
        let checkpoint = self.transfers.len();
        let files_before = self.files;
        let copied = self.copy_depth(source, &stage, false, depth + 1, boundary);
        let staged_transfers = self.transfers.split_off(checkpoint);
        let staged_transfers: Vec<_> = staged_transfers
            .into_iter()
            .filter(|t| t.destination_directory.starts_with(&stage))
            .collect();
        let mut swapped = false;
        let result = (|| {
            if !copied? {
                return Err(
                    "Replacement staging had errors; existing destination was retained".into(),
                );
            }
            let errors = compat::apply_description_transfers(&staged_transfers);
            if !errors.is_empty() {
                return Err(errors.join("; "));
            }
            check_cancel(&self.cancel)?;
            ensure_plain_path(target)?;
            rename_new(target, &backup)?;
            if let Err(error) = rename_new(&stage, target) {
                if let Err(restore) = rename_new(&backup, target) {
                    return Err(format!(
                        "{error}; original folder retained at {} because rollback failed: {restore}",
                        backup.display()
                    ));
                }
                return Err(error);
            }
            swapped = true;
            self.record_description(source, target, description, false);
            if let Err(e) = fs::remove_dir_all(&backup) {
                return Err(format!(
                    "Replacement committed; original folder retained at {}: {e}",
                    backup.display()
                ));
            }
            if moving {
                self.remove_verified_source(source, target, 0)?;
                if let Some(t) = self.transfers.last_mut() {
                    if t.source_directory.join(&t.source_name) == source {
                        t.move_entry = true;
                    }
                }
            }
            Ok(true)
        })();
        if stage.exists() {
            let _ = fs::remove_dir_all(&stage);
        }
        if result.is_err() && !swapped {
            self.files = files_before;
        }
        result
    }
    fn remove_verified_source(
        &self,
        source: &Path,
        target: &Path,
        depth: usize,
    ) -> Result<(), String> {
        check_cancel(&self.cancel)?;
        if depth > 256 {
            return Err("Source nesting exceeds safe move limit".into());
        }
        ensure_plain_path(source)?;
        ensure_plain_path(target)?;
        let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            if !target.is_dir() {
                return Err("Committed replacement no longer matches source directory".into());
            }
            let mut descriptions = Vec::new();
            for e in fs::read_dir(source).map_err(|e| e.to_string())? {
                let e = e.map_err(|e| e.to_string())?;
                if self.options.use_descriptions
                    && compat::is_description_file(&e.file_name().to_string_lossy())
                {
                    descriptions.push(e.path());
                } else {
                    self.remove_verified_source(&e.path(), &target.join(e.file_name()), depth + 1)?;
                }
            }
            for p in descriptions {
                ensure_plain_path(&p)?;
                fs::remove_file(p).map_err(|e| e.to_string())?;
            }
            fs::remove_dir(source).map_err(|e| e.to_string())?;
        } else {
            let mut a = File::open(source).map_err(|e| e.to_string())?;
            let mut b = File::open(target).map_err(|e| e.to_string())?;
            let mut left = vec![0; 1024 * 1024];
            let mut right = vec![0; 1024 * 1024];
            let mut total = 0;
            loop {
                check_cancel(&self.cancel)?;
                let n = a.read(&mut left).map_err(|e| e.to_string())?;
                if n == 0 {
                    if b.read(&mut [0; 1]).map_err(|e| e.to_string())? != 0 {
                        return Err("Source changed while replacing folder; source retained".into());
                    }
                    break;
                }
                b.read_exact(&mut right[..n]).map_err(|e| e.to_string())?;
                if left[..n] != right[..n] {
                    return Err("Source changed while replacing folder; source retained".into());
                }
                total += n as u64;
            }
            let now = fs::metadata(source).map_err(|e| e.to_string())?;
            if total != metadata.len()
                || now.len() != metadata.len()
                || now.modified().ok() != metadata.modified().ok()
            {
                return Err("Source changed while replacing folder; source retained".into());
            }
            drop(a);
            drop(b);
            check_cancel(&self.cancel)?;
            fs::remove_file(source).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
/// C# GetAvailableTarget: the first free name from "name (2).ext" for files and "name (2)" for folders.
fn available_target(path: &Path, directory: bool) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("No destination parent")?;
    let name = path.file_name().ok_or("Invalid target name")?.to_string_lossy().into_owned();
    let extension = if directory {
        ""
    } else {
        // Path.GetExtension: from the last dot (if the only dot is at the start, the whole name is the extension).
        name.rfind('.').map_or("", |i| &name[i..])
    };
    let stem = &name[..name.len() - extension.len()];
    for suffix in 2u64.. {
        let candidate = parent.join(format!("{stem} ({suffix}){extension}"));
        if fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    unreachable!()
}
fn absolute(path: &Path) -> Result<PathBuf, String> {
    let p = std::path::absolute(path).map_err(|e| e.to_string())?;
    let mut clean = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                clean.pop();
            }
            std::path::Component::CurDir => {}
            _ => clean.push(c.as_os_str()),
        }
    }
    Ok(clean)
}
fn inside(source: &Path, target: &Path) -> Result<bool, String> {
    let s = absolute(source)?;
    let t = absolute(target)?;
    #[cfg(windows)]
    {
        Ok(t.to_string_lossy()
            .to_lowercase()
            .starts_with(&(s.to_string_lossy().to_lowercase() + "\\"))
            || s.to_string_lossy().to_lowercase() == t.to_string_lossy().to_lowercase())
    }
    #[cfg(not(windows))]
    {
        Ok(t.starts_with(s))
    }
}
fn recycle(path: &Path) -> Result<(), String> {
    ensure_plain_path(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[repr(C)]
        struct Op {
            hwnd: *mut std::ffi::c_void,
            func: u32,
            from: *const u16,
            to: *const u16,
            flags: u16,
            aborted: i32,
            mappings: *mut std::ffi::c_void,
            title: *const u16,
        }
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn SHFileOperationW(op: *mut Op) -> i32;
        }
        let path = absolute(path)?;
        let from: Vec<_> = path.as_os_str().encode_wide().chain([0, 0]).collect();
        let mut op = Op {
            hwnd: std::ptr::null_mut(),
            func: 3,
            from: from.as_ptr(),
            to: std::ptr::null(),
            flags: 0x40 | 0x10 | 0x4 | 0x400,
            aborted: 0,
            mappings: std::ptr::null_mut(),
            title: std::ptr::null(),
        };
        let result = unsafe { SHFileOperationW(&mut op) };
        if result != 0 || op.aborted != 0 {
            Err(format!("Recycle failed or cancelled ({result})"))
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        Err("Recycle Bin is only available on Windows".into())
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    #[test]
    fn same_volume_move_preserves_readonly_files_without_copy_delete_failure() {
        let p = tests::fixture();
        fs::create_dir(p.join("source")).unwrap();
        fs::write(p.join("source/a"), b"payload").unwrap();
        let mut permissions = fs::metadata(p.join("source/a")).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(p.join("source/a"), permissions).unwrap();
        let (tx, rx) = mpsc::channel();
        run(
            Operation::Copy {
                sources: vec![p.join("source")],
                destination: p.join("out"),
                move_files: true,
            },
            tx,
            Arc::new(AtomicBool::new(false)),
        );
        let events: Vec<_> = rx.into_iter().collect();
        assert!(
            !events.iter().any(|e| matches!(e, Event::Error { .. })),
            "{events:?}"
        );
        assert!(!p.join("source").exists());
        assert_eq!(fs::read(p.join("out/source/a")).unwrap(), b"payload");
        assert!(
            fs::metadata(p.join("out/source/a"))
                .unwrap()
                .permissions()
                .readonly()
        );
        let mut permissions = fs::metadata(p.join("out/source/a")).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(p.join("out/source/a"), permissions).unwrap();
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn configured_copy_can_replace_readonly_target() {
        // Original FUN_004134e0: overwrite clears the destination attributes (0x20) and replaces it regardless of the setting (remreadonly).
        let p = tests::prepared();
        let mut permissions = fs::metadata(p.join("out/a")).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(p.join("out/a"), permissions).unwrap();
        let (tx, rx) = mpsc::channel();
        let source = p.join("in/a");
        let destination = p.join("out");
        let worker = std::thread::spawn(move || {
            run_with_options(
                Operation::Copy {
                    sources: vec![source],
                    destination,
                    move_files: false,
                },
                tx,
                Arc::new(AtomicBool::new(false)),
                OperationOptions {
                    remove_read_only: false,
                    use_descriptions: true,
                    ..Default::default()
                },
            )
        });
        let mut errors = Vec::new();
        for event in rx {
            match event {
                Event::Conflict { reply, .. } => reply.send(ConflictChoice::Overwrite).unwrap(),
                Event::Error { message, .. } => errors.push(message),
                _ => {}
            }
        }
        worker.join().unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(fs::read(p.join("out/a")).unwrap(), b"new");
        assert!(
            !fs::metadata(p.join("out/a"))
                .unwrap()
                .permissions()
                .readonly()
        );
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn operation_options_clear_copied_readonly_and_disable_comment_writes() {
        let p = tests::prepared();
        fs::remove_file(p.join("out/a")).unwrap();
        compat::set_description(&p.join("in"), "a", "source note", true).unwrap();
        let mut permissions = fs::metadata(p.join("in/a")).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(p.join("in/a"), permissions).unwrap();
        let (tx, rx) = mpsc::channel();
        run_with_options(
            Operation::Copy {
                sources: vec![p.join("in/a")],
                destination: p.join("out"),
                move_files: false,
            },
            tx,
            Arc::new(AtomicBool::new(false)),
            OperationOptions {
                remove_read_only: true,
                use_descriptions: false,
                ..Default::default()
            },
        );
        let events: Vec<_> = rx.into_iter().collect();
        assert!(
            !events.iter().any(|e| matches!(e, Event::Error { .. })),
            "{events:?}"
        );
        // Original FUN_00412b00: read-only clearing applies only when the source is a CD/DVD (GetDriveTypeW 5), so copying from a fixed disk keeps it.
        assert!(!on_optical_drive(&p.join("in/a")));
        assert!(
            fs::metadata(p.join("out/a"))
                .unwrap()
                .permissions()
                .readonly(),
            "a copy from a fixed disk keeps the read-only attribute"
        );
        let mut out_permissions = fs::metadata(p.join("out/a")).unwrap().permissions();
        out_permissions.set_readonly(false);
        fs::set_permissions(p.join("out/a"), out_permissions).unwrap();
        assert!(!p.join("out/descript.dat").exists());
        let mut permissions = fs::metadata(p.join("in/a")).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(p.join("in/a"), permissions).unwrap();
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn copy_move_delete_transfer_real_description_metadata() {
        let p = tests::prepared();
        compat::set_description(&p.join("in"), "a", "source note", true).unwrap();
        compat::set_description(&p.join("out"), "a", "old note", true).unwrap();
        tests::copy(&p, false, false, ConflictChoice::Overwrite);
        assert_eq!(
            compat::read_descriptions(&p.join("out"), true)
                .unwrap()
                .get("a")
                .unwrap(),
            "source note"
        );
        assert_eq!(
            compat::read_descriptions(&p.join("in"), true)
                .unwrap()
                .get("a")
                .unwrap(),
            "source note"
        );
        tests::copy(&p, true, false, ConflictChoice::Overwrite);
        assert!(
            !compat::read_descriptions(&p.join("in"), true)
                .unwrap()
                .contains_key("a")
        );
        let (tx, rx) = mpsc::channel();
        run(
            Operation::Delete {
                sources: vec![p.join("out/a")],
                recycle: false,
            },
            tx,
            Arc::new(AtomicBool::new(false)),
        );
        let events: Vec<_> = rx.into_iter().collect();
        assert!(
            !events.iter().any(|e| matches!(e, Event::Error { .. })),
            "{events:?}"
        );
        assert!(
            !compat::read_descriptions(&p.join("out"), true)
                .unwrap()
                .contains_key("a")
        );
        fs::remove_dir_all(p).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn failed_replacement_with_junction_retains_original_tree_and_source() {
        let p = tests::fixture();
        fs::create_dir_all(p.join("source")).unwrap();
        fs::create_dir_all(p.join("outside")).unwrap();
        fs::create_dir_all(p.join("out/source")).unwrap();
        fs::write(p.join("outside/secret"), b"safe").unwrap();
        fs::write(p.join("out/source/old"), b"old").unwrap();
        let result = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(p.join("source").join("link"))
            .arg(p.join("outside"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let (tx, rx) = mpsc::channel();
        let source = p.join("source");
        let destination = p.join("out");
        let worker = std::thread::spawn(move || {
            run(
                Operation::Copy {
                    sources: vec![source],
                    destination,
                    move_files: true,
                },
                tx,
                Arc::new(AtomicBool::new(false)),
            )
        });
        let mut error = false;
        for event in rx {
            match event {
                Event::Conflict { reply, .. } => reply.send(ConflictChoice::ReplaceFolder).unwrap(),
                Event::Error { .. } => error = true,
                _ => {}
            }
        }
        worker.join().unwrap();
        assert!(error);
        assert_eq!(fs::read(p.join("out/source/old")).unwrap(), b"old");
        assert_eq!(fs::read(p.join("outside/secret")).unwrap(), b"safe");
        assert!(p.join("source").exists());
        fs::remove_dir(p.join("source/link")).unwrap();
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn replace_folder_stages_all_content_before_swapping_old_tree() {
        let p = tests::fixture();
        fs::create_dir_all(p.join("source/nested")).unwrap();
        fs::create_dir_all(p.join("out/source")).unwrap();
        fs::write(p.join("source/nested/new"), b"new").unwrap();
        fs::write(p.join("out/source/old"), b"old").unwrap();
        let (tx, rx) = mpsc::channel();
        let source = p.join("source");
        let destination = p.join("out");
        let worker = std::thread::spawn(move || {
            run(
                Operation::Copy {
                    sources: vec![source],
                    destination,
                    move_files: true,
                },
                tx,
                Arc::new(AtomicBool::new(false)),
            )
        });
        let mut errors = Vec::new();
        for event in rx {
            match event {
                Event::Conflict { reply, .. } => reply.send(ConflictChoice::ReplaceFolder).unwrap(),
                Event::Error { message, .. } => errors.push(message),
                _ => {}
            }
        }
        worker.join().unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert!(!p.join("source").exists());
        assert!(!p.join("out/source/old").exists());
        assert_eq!(fs::read(p.join("out/source/nested/new")).unwrap(), b"new");
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn directory_copy_preserves_timestamps_and_children() {
        let p = tests::fixture();
        fs::create_dir_all(p.join("source/nested")).unwrap();
        fs::write(p.join("source/nested/data"), b"payload").unwrap();
        let before = fs::metadata(p.join("source")).unwrap();
        let (tx, rx) = mpsc::channel();
        run(
            Operation::Copy {
                sources: vec![p.join("source")],
                destination: p.join("out"),
                move_files: false,
            },
            tx,
            Arc::new(AtomicBool::new(false)),
        );
        let events: Vec<_> = rx.into_iter().collect();
        assert!(
            !events.iter().any(|e| matches!(e, Event::Error { .. })),
            "{events:?}"
        );
        let after = fs::metadata(p.join("out/source")).unwrap();
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert_eq!(before.created().unwrap(), after.created().unwrap());
        assert_eq!(
            fs::read(p.join("out/source/nested/data")).unwrap(),
            b"payload"
        );
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn cancellation_during_conflict_wait_preserves_both_files() {
        let p = tests::fixture();
        fs::create_dir(p.join("in")).unwrap();
        fs::create_dir(p.join("out")).unwrap();
        fs::write(p.join("in/a"), b"new").unwrap();
        fs::write(p.join("out/a"), b"old").unwrap();
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let source = p.join("in/a");
        let destination = p.join("out");
        let worker = std::thread::spawn(move || {
            run(
                Operation::Copy {
                    sources: vec![source],
                    destination,
                    move_files: true,
                },
                tx,
                worker_cancel,
            )
        });
        // The totals (GetFileListToWork) come before the conflict question.
        let first = std::iter::from_fn(|| rx.recv_timeout(Duration::from_secs(2)).ok()).find(|e| !matches!(e, Event::Totals { .. })).unwrap();
        assert!(matches!(first, Event::Conflict { .. }));
        cancel.store(true, Ordering::Relaxed);
        let mut finished = false;
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(2)) {
            if matches!(
                event,
                Event::Finished {
                    cancelled: true,
                    ..
                }
            ) {
                finished = true;
                break;
            }
        }
        worker.join().unwrap();
        assert!(finished);
        assert_eq!(fs::read(p.join("out/a")).unwrap(), b"old");
        assert_eq!(fs::read(p.join("in/a")).unwrap(), b"new");
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn copy_preserves_hidden_and_modified_metadata() {
        let p = tests::fixture();
        fs::create_dir(p.join("in")).unwrap();
        fs::create_dir(p.join("out")).unwrap();
        let source = p.join("in/a");
        fs::write(&source, b"new").unwrap();
        let file = File::options().write(true).open(&source).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000))
            .unwrap();
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn SetFileAttributesW(p: *const u16, attr: u32) -> i32;
            }
            let wide: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
            assert_ne!(unsafe { SetFileAttributesW(wide.as_ptr(), 2 | 32) }, 0);
        }
        tests::copy(&p, false, false, ConflictChoice::Overwrite);
        let a = fs::metadata(source).unwrap();
        let b = fs::metadata(p.join("out/a")).unwrap();
        assert_eq!(a.modified().unwrap(), b.modified().unwrap());
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(a.file_attributes() & 2, b.file_attributes() & 2);
        }
        fs::remove_dir_all(p).unwrap();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    Deleted,
    Skipped,
    Failed,
}

/// C# IsUnderSkippedPath: case-insensitive comparison after normalizing with Path.GetFullPath.
pub fn same_or_inside(path: &Path, root: &Path) -> bool {
    let normal = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let (path, root) = (normal(path), normal(root));
    path == root || path.starts_with(&(root + "\\"))
}

/// Original FUN_00412b00: after copy or move, clears the destination's read-only flag (0x1) only when "clear read-only when copying from CD/DVD" (remreadonly, +0xd4) is on and the source root
/// (PathStripToRootW) is a CD-ROM drive (GetDriveTypeW == 5).
#[cfg(windows)]
pub fn on_optical_drive(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    let Ok(full) = std::path::absolute(path) else { return false };
    let Some(root) = full.components().next().map(|c| {
        let mut r = PathBuf::from(c.as_os_str());
        r.push("\\");
        r
    }) else {
        return false;
    };
    let wide: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: NUL-terminated root path for the duration of the call.
    unsafe { GetDriveTypeW(wide.as_ptr()) == 5 }
}
#[cfg(not(windows))]
pub fn on_optical_drive(_path: &Path) -> bool {
    false
}

/// RemoveDirectoryW / DeleteFileW: deletes a link (reparse point) itself without following it (C# Directory.Delete / File.Delete).
fn remove_link(path: &Path, is_dir: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{DeleteFileW, RemoveDirectoryW};
    let wide: Vec<u16> = std::path::absolute(path).map_err(|e| e.to_string())?.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: NUL-terminated path for the duration of the call.
    let ok = unsafe { if is_dir { RemoveDirectoryW(wide.as_ptr()) } else { DeleteFileW(wide.as_ptr()) } };
    if ok != 0 { Ok(()) } else { Err(std::io::Error::last_os_error().to_string()) }
}

fn clear_read_only(path: &Path, metadata: &fs::Metadata) {
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
}

/// C# FindProtectedDeleteItems: finds items with read-only, system or hidden attributes depth first (selection order).
/// Links are not followed. Results are (path, those attribute bits).
pub fn find_protected_delete_items(paths: &[PathBuf]) -> Vec<(PathBuf, u32)> {
    #[cfg(windows)]
    use std::os::windows::fs::MetadataExt;
    const PROTECTED: u32 = 0x1 | 0x2 | 0x4;
    let mut result = Vec::new();
    let mut visited = std::collections::HashSet::new();
    fn walk(path: &Path, result: &mut Vec<(PathBuf, u32)>, visited: &mut std::collections::HashSet<String>, depth: usize) {
        if depth > 256 {
            return;
        }
        let Ok(full) = std::path::absolute(path) else { return };
        let Ok(m) = fs::symlink_metadata(&full) else { return };
        #[cfg(windows)]
        let attributes = m.file_attributes();
        #[cfg(not(windows))]
        let attributes = 0u32;
        let important = attributes & PROTECTED;
        if (m.is_dir() || important != 0) && !visited.insert(full.to_string_lossy().to_lowercase()) {
            return;
        }
        if important != 0 {
            result.push((full.clone(), important));
        }
        if !m.is_dir() || is_reparse(&m) {
            return;
        }
        if let Ok(children) = fs::read_dir(&full) {
            for child in children.flatten() {
                walk(&child.path(), result, visited, depth + 1);
            }
        }
    }
    for path in paths {
        walk(path, &mut result, &mut visited, 0);
    }
    result
}

/// Metadata::is_dir() of a link is false, so whether a link is a directory is decided by FILE_ATTRIBUTE_DIRECTORY.
fn attribute_directory(m: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x10 != 0
    }
    #[cfg(not(windows))]
    {
        m.is_dir()
    }
}
