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
