//! Regression for the real 7z containing a Unix-side `nul` member which must
//! not break the Windows archive pane or expose a reserved device path.
//! Set NDR_LOCAL_BK3 to the existing source archive; it is never modified.
use ndr::{archive, fsops::ConflictChoice};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}, sync::atomic::AtomicBool};

fn hash(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).expect("read test archive")))
}

fn count_tree(root: &Path) -> (usize, usize, u64) {
    let mut pending = vec![root.to_path_buf()];
    let mut files = 0;
    let mut dirs = 0;
    let mut bytes = 0;
    while let Some(dir) = pending.pop() {
        for child in fs::read_dir(dir).unwrap() {
            let child = child.unwrap();
            let meta = child.metadata().unwrap();
            if meta.is_dir() { dirs += 1; pending.push(child.path()); }
            else if meta.is_file() { files += 1; bytes += meta.len(); }
            else { panic!("unexpected extracted member type") }
            assert!(!child.file_name().to_string_lossy().eq_ignore_ascii_case("nul"), "Windows reserved device entry must never be created");
        }
    }
    (files, dirs, bytes)
}

#[test]
fn actual_bk3_7z_can_browse_select_copy_and_safely_extract_all() {
    let Ok(absolute) = std::env::var("NDR_LOCAL_BK3") else {
        eprintln!("BK3_REAL_ARCHIVE_SKIPPED: NDR_LOCAL_BK3 is not set");
        return;
    };
    let source = PathBuf::from(absolute);
    assert!(source.is_file());
    let before = hash(&source);
    let (entries, omitted) = archive::list_with_skipped_devices(&source, None).unwrap();
    assert_eq!(omitted, ["nul"]);
    assert_eq!(entries.iter().filter(|entry| !entry.is_dir).count(), 124);
    assert_eq!(entries.iter().filter(|entry| entry.is_dir).count(), 15);
    assert!(entries.iter().any(|entry| entry.key == ".gitignore"));
    assert!(!entries.iter().any(|entry| entry.key.eq_ignore_ascii_case("nul")));

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/bk3-qa");
    fs::create_dir_all(&root).unwrap();
    let folder = root.join(format!("ndr-bk3-isolated-{}", std::process::id()));
    assert!(!folder.exists(), "never overwrite a pre-existing test directory");
    fs::create_dir(&folder).unwrap();
    let stop = AtomicBool::new(false);
    let selected = folder.join("selected");
    archive::extract_entries_with_program(&source, &selected, &[".gitignore".into()], None, &stop,
        &mut |_, _| ConflictChoice::Cancel, &mut |_, _| None, None, &mut |_, _, _| {})
        .expect("valid selected entry must be copied to normal folder");
    assert_eq!(fs::metadata(selected.join(".gitignore")).unwrap().len(), 311);
    assert!(archive::extract_entry_to_temp(&source, None, "nul", &stop).is_err(), "never open Windows reserved device member");
    let destination = folder.join("full");
    archive::extract(&source, &destination, None, &stop).expect("full extract must skip the reserved name without losing safe files");
    let (files, dirs, bytes) = count_tree(&destination);
    assert_eq!(files, 124);
    assert_eq!(dirs, 15);
    assert_eq!(bytes, 60_789_073);
    let clone = folder.join("edit-test.7z");
    fs::copy(&source, &clone).unwrap();
    let clone_before = hash(&clone);
    assert!(archive::delete_entries(&clone, &[".gitignore".into()], None, &stop).is_err(), "editing a mixed-safety archive must not discard unsafe entries");
    assert!(archive::rename_entry(&clone, ".gitignore", "safe-renamed.txt", None, &stop).is_err());
    assert!(archive::add_entries(&clone, "", &[selected.join(".gitignore")], None, &stop).is_err());
    assert_eq!(hash(&clone), clone_before, "all blocked edits must preserve the archive bytes");
    assert_eq!(hash(&source), before, "the original user archive must be byte-for-byte unchanged");

    let report = json!({"archive": source.display().to_string(), "archive_sha256_unchanged": before,
        "visible_members": entries.len(), "hidden_device_members": omitted,
        "selective_copy": "PASS", "whole_extraction": "PASS", "extract_files": files,
        "extract_dirs": dirs, "extract_bytes": bytes, "unsafe_member_open": "BLOCKED",
        "archive_edit_preservation": "PASS"});
    fs::write(root.join("ndr-bk3-results.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    fs::remove_dir_all(&folder).unwrap();
    eprintln!("NDR_BK3_REAL_ARCHIVE_PASS visible={} hidden=1 extracted_files={} extracted_bytes={} original_sha256_unchanged", entries.len(), files, bytes);
}
