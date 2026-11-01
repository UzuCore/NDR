//! Optional tests against user-provided large ISO9660/UDF hybrid files.
//! NDR_LOCAL_ISO_FILES is a semicolon-separated list of absolute ISO paths.
//! No test ever modifies, moves, or deletes the supplied original images.
use ndr::{archive, fsops::ConflictChoice};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, io::{self, Read}, path::{Path, PathBuf}, sync::atomic::AtomicBool};

fn file_sha256(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let amount = file.read(&mut buffer)?;
        if amount == 0 { break }
        hasher.update(&buffer[..amount]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[test]
fn real_local_iso_files_list_copy_preview_and_refuse_mutation() {
    let Ok(value) = std::env::var("NDR_LOCAL_ISO_FILES") else {
        eprintln!("NDR_LOCAL_ISO_SKIPPED: set NDR_LOCAL_ISO_FILES to absolute ISO path(s)");
        return;
    };
    let files: Vec<PathBuf> = value.split(';').filter(|s| !s.is_empty()).map(PathBuf::from).collect();
    assert!(!files.is_empty(), "at least one ISO must be supplied");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/iso-real-qa");
    fs::create_dir_all(&root).unwrap();
    let cancel = AtomicBool::new(false);
    let mut results = Vec::new();
    for (index, iso) in files.iter().enumerate() {
        assert!(iso.is_file() && iso.extension().is_some_and(|e| e.eq_ignore_ascii_case("iso")), "expected actual ISO image: {}", iso.display());
        let original_metadata = fs::metadata(iso).unwrap();
        // Full streaming hashes prove that even the multi-gigabyte original
        // was not modified by the NDR archive operation tests.
        let before_hash = file_sha256(iso).expect("hash original ISO before test");
        let entries = archive::list(iso, None).unwrap_or_else(|e| panic!("{}: {e}", iso.display()));
        assert!(!entries.is_empty(), "large ISO must have a readable entry listing");
        assert!(!archive::can_modify(iso), "ISO archive contents must remain read-only");
        let case = root.join(format!("local-iso-test-{index}"));
        assert!(!case.exists(), "do not overwrite existing test output: {}", case.display());
        fs::create_dir(&case).unwrap();
        let mut copied = Vec::new();
        for (label, entry) in ["PS3_DISC.SFB", "ICON0.PNG"].iter().enumerate() {
            let selected = entries.iter().find(|e| !e.is_dir
                && e.key.replace('\\', "/").split('/').next_back().is_some_and(|name| name.eq_ignore_ascii_case(entry)))
                .unwrap_or_else(|| panic!("{}: missing {entry} in listing", iso.display()));
            let destination = case.join(format!("copy-{label}"));
            archive::extract_entries_with_program(iso, &destination, &[selected.key.clone()], None, &cancel,
                &mut |_, _| ConflictChoice::Cancel, &mut |_, _| None, None,
                &mut |_, _, _| {},
            ).unwrap_or_else(|e| panic!("{}: selective copy {} failed: {e}", iso.display(), selected.key));
            let extracted = destination.join(entry);
            assert!(extracted.is_file(), "selected file was not copied: {}", extracted.display());
            let metadata = fs::metadata(&extracted).unwrap();
            assert!(metadata.len() > 0);
            if selected.size_known { assert_eq!(metadata.len(), selected.size); }
            copied.push(json!({ "key": selected.key, "length": metadata.len(), "sha256": file_sha256(&extracted).unwrap(), "status": "PASS" }));
        }
        // The preview/open path uses its own extraction command and must
        // share the same handler selection as selective clipboard copy.
        let icon_key = entries.iter().find(|e| !e.is_dir && e.key.replace('\\', "/").ends_with("/ICON0.PNG"))
            .expect("expected icon within the image").key.clone();
        let preview = archive::extract_entry_to_temp(iso, None, &icon_key, &cancel)
            .unwrap_or_else(|e| panic!("{}: opening a file from ISO failed: {e}", iso.display()));
        assert!(preview.is_file(), "preview path must exist");
        let temp_root = archive::temp_entry_root();
        let relative = preview.strip_prefix(&temp_root).expect("NDR preview must stay under its own temp directory");
        let staged = relative.components().next().expect("preview staging folder");
        let staging_root = temp_root.join(staged.as_os_str());
        assert!(staging_root.is_dir(), "preview staging root should exist");
        fs::remove_dir_all(&staging_root).expect("remove only the isolated preview staging folder");
        let after_hash = file_sha256(iso).expect("hash original ISO after test");
        assert_eq!(after_hash, before_hash, "user ISO was modified: {}", iso.display());
        let final_metadata = fs::metadata(iso).unwrap();
        assert_eq!(final_metadata.len(), original_metadata.len());
        assert_eq!(final_metadata.modified().ok(), original_metadata.modified().ok());
        results.push(json!({"file":iso.display().to_string(),"bytes":original_metadata.len(),"entries":entries.len(),
            "iso_contents_read_only":true,"selective_copies":copied,"preview_pass":true,"sha256_original_unchanged":before_hash}));
        fs::remove_dir_all(&case).expect("remove temporary copies, never the source ISO");
    }
    fs::write(root.join("ndr-local-iso-results.json"), serde_json::to_vec_pretty(&json!({"tested":files.len(),"results":results})).unwrap()).unwrap();
    eprintln!("NDR_LARGE_ISO_TEST_PASS count={} readonly=true files_copied={} previews={}",files.len(),files.len()*2,files.len());
}

#[test]
fn malformed_iso_never_becomes_a_browsable_archive() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/iso-real-qa");
    fs::create_dir_all(&root).unwrap();
    let path = root.join(format!("corrupt-image-{}.iso", std::process::id()));
    assert!(!path.exists());
    fs::write(&path, b"Not a real ISO9660/UDF archive").unwrap();
    let outcome = archive::list(&path, None);
    fs::remove_file(&path).unwrap();
    assert!(outcome.is_err(), "bad ISO should be rejected even after fallback");
}
