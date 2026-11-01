//! Real-format operations matrix: selective NDR clipboard copy, add/copy into
//! an archive, cut/move out, cut/move in, delete inside and read-only refusal.
//! Only temporary copies are ever edited; downloaded originals are hashed.
use ndr::{archive, fsops::ConflictChoice};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}, sync::atomic::AtomicBool};

fn sha(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).expect("hash source")))
}

fn selected_copy(source: &Path, key: &str, destination: &Path, stop: &AtomicBool) -> Result<(), String> {
    archive::extract_entries_with_program(
        source, destination, &[key.to_owned()], None, stop,
        &mut |_, _| ConflictChoice::Cancel,
        &mut |_, _| None, None, &mut |_, _, _| {},
    )?;
    let basename = key.rsplit(['/', '\\']).next().unwrap_or("");
    if !destination.join(basename).is_file() {
        return Err(format!("Expected selected archive entry missing: {basename}"));
    }
    Ok(())
}

fn copy_archive_for_test(original: &Path, case: &Path, label: &str) -> PathBuf {
    let folder = case.join(label);
    fs::create_dir(&folder).expect("create isolated staging directory");
    let clone = folder.join(original.file_name().expect("archive name"));
    fs::copy(original, &clone).expect("clone downloaded original");
    clone
}

#[test]
fn actual_web_formats_copy_move_delete_and_readonly_safety() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/archive-web-corpus");
    if !base.join("inventory.json").is_file() {
        eprintln!("NDR_OPS_CORPUS_SKIPPED: download upstream fixtures first");
        return;
    }
    let text = fs::read_to_string(base.join("inventory.json")).unwrap();
    let manifest: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap();
    let fixtures = manifest["fixtures"].as_array().expect("corpus fixtures");
    let sandbox = base.join(format!("ops-isolated-{}", std::process::id()));
    assert!(!sandbox.exists(), "existing sandbox must never be overwritten");
    fs::create_dir(&sandbox).unwrap();
    let stop = AtomicBool::new(false);
    let mut rows = Vec::new();
    let mut copy_ok = 0usize;
    let mut copy_error = 0usize;
    let mut listed_refused = 0usize;
    let mut add_ok = 0usize;
    let mut move_out_ok = 0usize;
    let mut move_in_ok = 0usize;
    let mut delete_ok = 0usize;
    let mut readonly_checked = 0usize;

    for (index, item) in fixtures.iter().enumerate() {
        let name = item["name"].as_str().expect("sample name");
        let original = base.join("samples").join(name);
        assert!(original.is_file(), "all fixture bytes must be present: {name}");
        let original_hash = sha(&original);
        let case = sandbox.join(format!("sample-{index:03}"));
        fs::create_dir(&case).unwrap();
        let writable = archive::can_modify(&original);
        let entries = archive::list(&original, None);
        let selected = entries.as_ref().ok()
            .and_then(|list| list.iter().find(|entry| !entry.is_dir).map(|entry| entry.key.clone()));
        let mut copy_out = String::from("BLOCKED_BY_LISTING");
        let mut copy_in = String::from("READ_ONLY");
        let mut delete = String::from("READ_ONLY");
        let mut move_out = String::from("READ_ONLY");
        let mut move_in = String::from("READ_ONLY");

        if let Some(key) = selected.as_deref() {
            match selected_copy(&original, key, &case.join("copy-out"), &stop) {
                Ok(()) => { copy_ok += 1; copy_out = "PASS".into(); },
                Err(e) => {
                    copy_error += 1;
                    copy_out = format!("FAIL: {e}");
                    assert!(!case.join("copy-out").exists(), "failed copy published content: {name}");
                }
            }
        } else if entries.is_err() {
            listed_refused += 1;
            assert!(selected_copy(&original, "test.txt", &case.join("blocked-copy"), &stop).is_err(),
                "listing refusal must block selective copy: {name}");
            assert!(!case.join("blocked-copy").exists(), "blocked copy created output: {name}");
        } else {
            copy_out = "SKIP_NO_FILE_ENTRY".into();
        }

        if writable && entries.is_ok() {
            let marker = format!("ndr-copy-probe-{index:03}.txt");
            let probe = case.join(&marker);
            fs::write(&probe, format!("NDR real-file operations case {index}\n")).unwrap();
            let added = copy_archive_for_test(&original, &case, "copy-in");
            let before = sha(&added);
            match archive::add_entries(&added, "", &[probe.clone()], None, &stop) {
                Ok(()) => match archive::list(&added, None) {
                    Ok(list) if list.iter().any(|e| e.key.replace('\\', "/") == marker) => {
                        add_ok += 1; copy_in = "PASS".into();
                    },
                    Ok(_) => copy_in = "FAIL: entry absent after add".into(),
                    Err(e) => copy_in = format!("FAIL: added archive cannot be listed: {e}"),
                },
                Err(e) => {
                    copy_in = format!("FAIL: {e}");
                    assert_eq!(sha(&added), before, "failed addition changed clone: {name}");
                }
            }

            let local_movable = case.join(format!("ndr-move-in-{index:03}.txt"));
            fs::write(&local_movable, b"Only remove after NDR archive add succeeds").unwrap();
            let target = copy_archive_for_test(&original, &case, "move-in");
            match archive::add_entries(&target, "", &[local_movable.clone()], None, &stop) {
                Ok(()) => {
                    let key = local_movable.file_name().unwrap().to_string_lossy();
                    if archive::list(&target, None).is_ok_and(|list| list.iter().any(|e| e.key == key)) {
                        fs::remove_file(&local_movable).expect("remove local source after verified archive add");
                        assert!(!local_movable.exists());
                        move_in_ok += 1; move_in = "PASS".into();
                    } else { move_in = "FAIL: moved entry missing from archive".into(); }
                }
                Err(e) => {
                    move_in = format!("FAIL: {e}");
                    assert!(local_movable.exists(), "failed move removed local source: {name}");
                }
            }

            if let Some(key) = selected.as_deref() {
                let deleted = copy_archive_for_test(&original, &case, "delete-entry");
                let before = sha(&deleted);
                let original_entries = entries.as_ref().unwrap();
                match archive::delete_entries(&deleted, &[key.to_owned()], None, &stop) {
                    Ok(()) => {
                        match archive::list(&deleted, None) {
                            Ok(after) if !after.iter().any(|e| e.key == key)
                                && original_entries.iter().filter(|e| e.key != key).all(|e| after.iter().any(|x| x.key == e.key)) => {
                                    delete_ok += 1; delete = "PASS".into();
                                },
                            Ok(_) => delete = "FAIL: entry not removed or sibling missing".into(),
                            Err(e) => delete = format!("FAIL: archive no longer listable: {e}"),
                        }
                    }
                    Err(e) => {
                        assert_eq!(sha(&deleted), before, "failed deletion corrupted clone: {name}");
                        delete = format!("FAIL: {e}");
                    }
                }

                // Move out: selective copy must succeed before the source is
                // deleted. The source is a *clone*, never the downloaded file.
                let moving = copy_archive_for_test(&original, &case, "move-out");
                match selected_copy(&moving, key, &case.join("moved-to"), &stop) {
                    Ok(()) => match archive::delete_entries(&moving, &[key.to_owned()], None, &stop) {
                        Ok(()) if archive::list(&moving, None).is_ok_and(|es| !es.iter().any(|e| e.key == key)) => {
                            move_out_ok += 1; move_out = "PASS".into();
                        },
                        Ok(()) => move_out = "FAIL: moved source still in archive".into(),
                        Err(e) => move_out = format!("FAIL: delete after copy: {e}"),
                    },
                    Err(e) => move_out = format!("FAIL: copy stage: {e}"),
                }
            }
        } else if !writable {
            let clone = case.join(original.file_name().unwrap());
            fs::copy(&original, &clone).unwrap();
            let before = sha(&clone);
            let key = selected.as_deref().unwrap_or("ndr-probe.txt");
            assert!(archive::delete_entries(&clone, &[key.to_owned()], None, &stop).is_err(),
                "read-only format allowed entry delete: {name}");
            let probe = case.join("readonly-probe.txt");
            fs::write(&probe, b"must not be inserted").unwrap();
            assert!(archive::add_entries(&clone, "", &[probe], None, &stop).is_err(),
                "read-only format allowed archive add: {name}");
            assert_eq!(sha(&clone), before, "read-only cloned archive mutated: {name}");
            readonly_checked += 1;
        }
        assert_eq!(sha(&original), original_hash, "downloaded source changed: {name}");
        rows.push(json!({ "name": name, "format": archive::archive_format(&original),
            "writable": writable, "copy_out": copy_out, "copy_in": copy_in,
            "move_out": move_out, "move_in": move_in, "delete_inside": delete,
            "original_preserved": true }));
    }

    fs::write(base.join("ndr-operations-inventory.json"), serde_json::to_vec_pretty(&json!({
        "total": rows.len(), "copy_out_pass": copy_ok, "copy_out_failed": copy_error,
        "copy_out_blocked": listed_refused, "copy_into_archive_pass": add_ok,
        "move_into_archive_pass": move_in_ok, "move_out_archive_pass": move_out_ok,
        "delete_inside_archive_pass": delete_ok, "readonly_formats_blocked": readonly_checked,
        "results": rows
    })).unwrap()).unwrap();
    fs::remove_dir_all(&sandbox).expect("clean isolated operations sandbox only");
    assert!(rows.len() >= 58, "expected all downloaded web archive samples");
    assert!(readonly_checked >= 30, "expected read-only archive mutation refusals");
    assert!(copy_ok >= 20, "expected successful actual selective archive copies");
    eprintln!("NDR_ARCHIVE_OPERATIONS_DONE total={} copy_ok={copy_ok} copy_failed={copy_error} blocked={listed_refused} add_ok={add_ok} move_in_ok={move_in_ok} move_out_ok={move_out_ok} delete_ok={delete_ok} read_only={readonly_checked}", rows.len());
}
