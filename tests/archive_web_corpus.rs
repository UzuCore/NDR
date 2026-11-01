//! Optional independent archive-format audit using downloaded, real test
//! archives from libarchive and SharpCompress. Run `scripts/archive_web_corpus.ps1`
//! to materialize fixtures under target/ first; no external files are executed.

use ndr::archive;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

#[test]
fn actual_downloaded_archive_fixtures_are_checked_through_ndr() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/archive-web-corpus");
    let manifest = corpus.join("inventory.json");
    if !manifest.exists() {
        eprintln!("NDR_EXTERNAL_CORPUS_SKIPPED: run scripts/archive_web_corpus.ps1 to download real samples");
        return;
    }
    let data = fs::read(&manifest).expect("read archive corpus manifest");
    let json: Value = serde_json::from_str(
        std::str::from_utf8(&data).expect("UTF-8 corpus manifest").trim_start_matches('\u{feff}'),
    ).expect("parse archive corpus manifest");
    let items = json["fixtures"].as_array().expect("manifest fixture list");
    let mut results = Vec::new();
    let mut listed = 0usize;
    let mut refused = 0usize;
    let mut verified_extract = 0usize;
    let cancelled = AtomicBool::new(false);

    for item in items {
        let name = item["name"].as_str().expect("fixture name");
        let path = corpus.join("samples").join(name);
        if !path.is_file() { continue; }
        let result = archive::list(&path, None);
        let status = match &result {
            Ok(entries) => {
                listed += 1;
                format!("PASS:{} entries", entries.len())
            }
            Err(error) => {
                refused += 1;
                format!("REFUSED:{}", error.chars().take(190).collect::<String>())
            }
        };
        if name.ends_with("test_read_format_7zip_zstd.7z")
            || name.ends_with("test_read_format_7zip_solid_zstd.7z")
            || name.ends_with("7Zip.ZSTD.7z")
        {
            assert!(result.as_ref().err().is_some_and(|e| e.contains("04F71101")),
                "7z with unsupported Zstandard method must be refused early: {name} {status}");
        }
        if name.ends_with("test_read_format_zip_zstd.zipx")
            || name.ends_with("WinZip27_ZSTD.zipx")
            || name.ends_with("test_compat_zip_1.zip")
        {
            assert!(result.is_ok(), "ZIP/ZIPX codec supported in bundled engine: {name} {status}");
        }
        // Verify NDR's actual staged extraction on several small, ordinary
        // samples, not only the CLI's `7z t` command.
        if ["test_read_format_7zip_copy.7z", "test_read_format_zip_bzip2.zipx",
            "sharpcompress/Arj.store.arj"].contains(&name)
        {
            assert!(result.is_ok(), "sample should list: {name} {status}");
            let output = corpus.join("ndr-extraction-check").join(format!("{}-{}", std::process::id(), verified_extract));
            archive::extract(&path, &output, None, &cancelled)
                .unwrap_or_else(|e| panic!("NDR extraction failed on {name}: {e}"));
            assert!(output.is_dir() && fs::read_dir(&output).unwrap().next().is_some(),
                "NDR extraction of {name} must materialize actual content");
            fs::remove_dir_all(&output).expect("clean extraction staging");
            verified_extract += 1;
        }
        results.push(json!({
            "name": name,
            "format": archive::archive_format(&path),
            "editable": archive::can_modify(&path),
            "bundled_engine_test": item["sevenzip_test"],
            "ndr_listing": status
        }));
    }
    assert!(results.len() >= 40, "too few downloaded samples for a meaningful audit");
    fs::write(corpus.join("ndr-inventory.json"), serde_json::to_vec_pretty(&json!({
        "sample_count": results.len(),
        "ndr_listed": listed,
        "ndr_refused": refused,
        "ndr_extracted": verified_extract,
        "results": results,
    })).unwrap()).expect("write NDR archive audit results");
    eprintln!("NDR_EXTERNAL_CORPUS_DONE samples={} listed={} refused={} extracted={}",
        results.len(), listed, refused, verified_extract);
}
