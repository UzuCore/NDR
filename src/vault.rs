//! NDR's password-only SQLCipher encrypted virtual folder backend.
//! Uses SQLCipher 4 authenticated pages; never creates a plaintext staging directory.
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::{path::{Path, PathBuf}, time::{Duration, Instant}};
use std::io::Write;
use zeroize::{Zeroize, Zeroizing};

pub const EXTENSION: &str = "ndr";
pub const MAX_FILE_SIZE: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPolicy {
    CloseOnLeave,
    Inactive(Duration),
    Elapsed(Duration),
    UntilExit,
}

impl SessionPolicy {
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let minutes = settings.values.get("VaultSessionMinutes").and_then(|v| v.as_i64()).unwrap_or(15).clamp(1, 1440) as u64;
        match settings.text("VaultSessionPolicy", "close").as_str() {
            "inactive" => Self::Inactive(Duration::from_secs(minutes * 60)),
            "elapsed" => Self::Elapsed(Duration::from_secs(minutes * 60)),
            "exit" => Self::UntilExit,
            _ => Self::CloseOnLeave,
        }
    }

    pub fn expired(self, opened: Instant, last_used: Instant, now: Instant) -> bool {
        match self {
            Self::Inactive(duration) => now.saturating_duration_since(last_used) >= duration,
            Self::Elapsed(duration) => now.saturating_duration_since(opened) >= duration,
            Self::CloseOnLeave | Self::UntilExit => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultItem {
    pub id: i64,
    pub name: String,
    pub folder: bool,
    pub size: i64,
}

impl Drop for VaultItem {
    fn drop(&mut self) { self.name.zeroize(); }
}

pub struct Vault {
    connection: Connection,
    pub file: PathBuf,
    opened: Instant,
    last_used: Instant,
}

fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.len() > 255
        || name.trim() != name || name.chars().any(|c| c.is_control() || "/\\:*?\"<>|".contains(c)) {
        return Err("Invalid container entry name".into());
    }
    Ok(())
}

fn configure(conn: &Connection, password: Zeroizing<String>) -> Result<(), String> {
    if password.chars().count() < 6 { return Err("Password must contain at least 6 characters".into()); }
    // SQLITE key must be set before any access to the encrypted database.
    conn.pragma_update(None, "key", password.as_str()).map_err(|e| e.to_string())?;
    // SQLCipher exposes cipher_status as text on some bundled builds and as
    // an integer on others. Accept only the explicit enabled value `1`.
    let active: rusqlite::types::Value = conn.pragma_query_value(None, "cipher_status", |row| row.get(0))
        .map_err(|_| "SQLCipher encryption status could not be verified".to_owned())?;
    if !matches!(active, rusqlite::types::Value::Integer(1))
        && !matches!(active, rusqlite::types::Value::Text(ref value) if value == "1") {
        return Err("SQLCipher encryption is not active".into());
    }
    conn.pragma_update(None, "cipher_memory_security", "ON").map_err(|e| e.to_string())?;
    conn.pragma_update(None, "temp_store", "MEMORY").map_err(|e| e.to_string())?;
    conn.pragma_update(None, "secure_delete", "ON").map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "DELETE").map_err(|e| e.to_string())?;
    conn.pragma_update(None, "synchronous", "FULL").map_err(|e| e.to_string())?;
    Ok(())
}

impl Vault {
    pub fn create(path: &Path, password: String) -> Result<Self, String> {
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION)) {
            return Err("Encrypted containers must have the .ndr extension".into());
        }
        if path.exists() { return Err("The encrypted container already exists".into()); }
        // CREATE_NEW prevents accidentally replacing existing user data.
        let _reservation = std::fs::OpenOptions::new().write(true).create_new(true).open(path)
            .map_err(|e| e.to_string())?;
        let result = (|| -> Result<Self, String> {
            let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
                .map_err(|e| e.to_string())?;
            configure(&conn, Zeroizing::new(password))?;
            let cipher: String = conn.pragma_query_value(None, "cipher_version", |row| row.get(0))
                .map_err(|_| "The SQLCipher encryption engine was not loaded".to_owned())?;
            if cipher.is_empty() { return Err("SQLCipher engine unavailable".into()); }
            conn.execute_batch("PRAGMA user_version=1;
                CREATE TABLE vault_meta (id INTEGER PRIMARY KEY CHECK(id=1), signature TEXT NOT NULL);
                INSERT INTO vault_meta VALUES (1,'NDR_SQLCIPHER_V1');
                CREATE TABLE items (id INTEGER PRIMARY KEY, parent INTEGER NOT NULL,
                  name TEXT NOT NULL, folder INTEGER NOT NULL CHECK(folder IN(0,1)), contents BLOB,
                  CHECK(folder=0 OR contents IS NULL));
                CREATE UNIQUE INDEX items_parent_name ON items(parent,name);
                INSERT INTO items(id,parent,name,folder) VALUES(1,0,'',1);")
                .map_err(|e| e.to_string())?;
            Ok(Self { connection: conn, file: path.to_path_buf(), opened: Instant::now(), last_used: Instant::now() })
        })();
        if result.is_err() { let _ = std::fs::remove_file(path); }
        result
    }

    pub fn unlock(path: &Path, password: String) -> Result<Self, String> {
        if !path.is_file() { return Err("Encrypted container file does not exist".into()); }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| e.to_string())?;
        configure(&conn, Zeroizing::new(password))?;
        let signature: String = conn.query_row("SELECT signature FROM vault_meta WHERE id=1", [], |row| row.get(0))
            .map_err(|_| "Wrong password, damaged container, or unsupported format".to_owned())?;
        if signature != "NDR_SQLCIPHER_V1" { return Err("Unsupported encrypted container format".into()); }
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).map_err(|e| e.to_string())?;
        if version != 1 { return Err("Unsupported encrypted container version".into()); }
        Ok(Self { connection: conn, file: path.to_path_buf(), opened: Instant::now(), last_used: Instant::now() })
    }

    pub fn expired(&self, policy: SessionPolicy) -> bool { policy.expired(self.opened, self.last_used, Instant::now()) }
    pub fn touch(&mut self) { self.last_used = Instant::now(); }

    fn ensure_folder(&self, id: i64) -> Result<(), String> {
        let is_folder: Option<i64> = self.connection.query_row("SELECT folder FROM items WHERE id=?1", [id], |r| r.get(0))
            .optional().map_err(|e| e.to_string())?;
        if is_folder != Some(1) { return Err("Encrypted folder was not found".into()); }
        Ok(())
    }

    pub fn list(&mut self, parent: i64) -> Result<Vec<VaultItem>, String> {
        self.ensure_folder(parent)?;
        let mut statement = self.connection.prepare("SELECT id,name,folder,coalesce(length(contents),0) FROM items WHERE parent=?1 ORDER BY folder DESC,name COLLATE NOCASE")
            .map_err(|e| e.to_string())?;
        let values = statement.query_map([parent], |r| Ok(VaultItem { id: r.get(0)?, name: r.get(1)?, folder: r.get::<_, i64>(2)? == 1, size: r.get(3)? }))
            .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        Ok(values)
    }

    pub fn parent(&self, id: i64) -> Result<i64, String> {
        self.connection.query_row("SELECT parent FROM items WHERE id=?1 AND folder=1", [id], |r| r.get(0))
            .map_err(|e| e.to_string())
    }

    pub fn add_folder(&mut self, parent: i64, name: &str) -> Result<i64, String> {
        self.insert(parent, name, true, None)
    }
    pub fn add_file(&mut self, parent: i64, name: &str, data: &[u8]) -> Result<i64, String> {
        if data.len() > MAX_FILE_SIZE { return Err("File exceeds current container safety limit (64 MiB)".into()); }
        self.insert(parent, name, false, Some(data))
    }

    /// Import files and entire folders as a single atomic SQLCipher operation.
    /// Symlinks, junctions and other reparse points are not followed.
    /// If any item conflicts or fails validation, the whole import is rolled back.
    pub fn import_paths(&mut self, parent: i64, sources: &[PathBuf]) -> Result<usize, String> {
        if sources.is_empty() { return Ok(0); }
        self.ensure_folder(parent)?;
        let own_file = std::fs::canonicalize(&self.file).map_err(|e| e.to_string())?;
        for source in sources {
            let full = std::fs::canonicalize(source).map_err(|e| e.to_string())?;
            if own_file.starts_with(&full) {
                return Err("Cannot import the open encrypted container or its parent folder".into());
            }
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let mut items = 0usize;
        let mut total_bytes = 0u64;
        for source in sources {
            import_path(&transaction, parent, source, 0, &mut items, &mut total_bytes)?;
        }
        transaction.commit().map_err(|e| e.to_string())?;
        self.touch();
        Ok(items)
    }
    /// Export selected items to an existing real directory as plaintext.
    /// Do not overwrite or merge existing paths; on failure roll back the
    /// paths created by this operation only.
    pub fn export_items(&mut self, parent: i64, names: &[String], destination: &Path) -> Result<usize, String> {
        if names.is_empty() { return Ok(0); }
        let meta = std::fs::symlink_metadata(destination).map_err(|e| e.to_string())?;
        if !meta.is_dir() || is_reparse_point(&meta) {
            return Err("Export destination must be a real directory, not a link".into());
        }
        self.ensure_folder(parent)?;
        let selected = self.list(parent)?;
        let mut exports = Vec::new();
        for name in names {
            let item = selected.iter().find(|item| item.name == *name)
                .ok_or("Selected encrypted item does not exist")?;
            if exports.iter().any(|(id, _): &(i64, String)| *id == item.id) {
                return Err("Duplicate export selection".into());
            }
            exports.push((item.id, item.name.clone()));
        }
        let mut created = Vec::<PathBuf>::new();
        let mut count = 0usize;
        let mut total = 0u64;
        let result = (|| -> Result<(), String> {
            for (id, name) in exports {
                self.export_item(id, &name, destination, 0, &mut count, &mut total, &mut created)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            for path in created.iter().rev() {
                if path.is_dir() { let _ = std::fs::remove_dir(path); }
                else { let _ = std::fs::remove_file(path); }
            }
            return Err(error);
        }
        self.touch();
        Ok(count)
    }

    fn export_item(
        &mut self, id: i64, name: &str, destination: &Path, depth: usize,
        count: &mut usize, total: &mut u64, created: &mut Vec<PathBuf>,
    ) -> Result<(), String> {
        if depth > 64 || *count >= 10_000 { return Err("Encrypted export exceeds directory depth or item limit".into()); }
        valid_name(name)?;
        if is_windows_reserved_name(name) { return Err("Cannot export a Windows reserved filename".into()); }
        let path = destination.join(name);
        if path.exists() || std::fs::symlink_metadata(&path).is_ok() {
            return Err(format!("Export target already exists: {}", path.display()));
        }
        let folder: i64 = self.connection.query_row("SELECT folder FROM items WHERE id=?1", [id], |row| row.get(0))
            .map_err(|e| e.to_string())?;
        if folder == 1 {
            std::fs::create_dir(&path).map_err(|e| e.to_string())?;
            created.push(path.clone());
            *count += 1;
            for child in self.list(id)? {
                self.export_item(child.id, &child.name, &path, depth + 1, count, total, created)?;
            }
        } else {
            let data = Zeroizing::new(self.read_file(id)?);
            if data.len() > MAX_FILE_SIZE { return Err("Encrypted export file exceeds 64 MiB".into()); }
            *total = total.checked_add(data.len() as u64).ok_or("Export size overflow")?;
            if *total > 512 * 1024 * 1024 { return Err("Encrypted export batch exceeds 512 MiB".into()); }
            let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
                .map_err(|e| e.to_string())?;
            created.push(path);
            out.write_all(&data).map_err(|e| e.to_string())?;
            out.sync_all().map_err(|e| e.to_string())?;
            *count += 1;
        }
        Ok(())
    }

    fn insert(&mut self, parent: i64, name: &str, folder: bool, data: Option<&[u8]>) -> Result<i64, String> {
        valid_name(name)?;
        self.ensure_folder(parent)?;
        self.connection.execute("INSERT INTO items(parent,name,folder,contents) VALUES(?1,?2,?3,?4)", params![parent,name,folder as i64,data])
            .map_err(|e| e.to_string())?;
        self.touch();
        Ok(self.connection.last_insert_rowid())
    }
    pub fn read_file(&mut self, id: i64) -> Result<Vec<u8>, String> {
        let result: Vec<u8> = self.connection.query_row("SELECT contents FROM items WHERE id=?1 AND folder=0", [id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        self.touch();
        Ok(result)
    }
    pub fn update_file(&mut self, id: i64, data: &[u8]) -> Result<(), String> {
        if data.len() > MAX_FILE_SIZE { return Err("File exceeds current 64 MiB limit".into()); }
        let changed = self.connection.execute("UPDATE items SET contents=?1 WHERE id=?2 AND folder=0", params![data,id])
            .map_err(|e| e.to_string())?;
        if changed == 0 { return Err("File not found".into()); }
        self.touch();
        Ok(())
    }
    pub fn rename(&mut self, id: i64, name: &str) -> Result<(), String> {
        if id == 1 { return Err("Root folder cannot be renamed".into()); }
        valid_name(name)?;
        let changed = self.connection.execute("UPDATE items SET name=?1 WHERE id=?2", params![name,id]).map_err(|e| e.to_string())?;
        if changed == 0 { return Err("Item not found".into()); }
        self.touch();
        Ok(())
    }
    pub fn remove(&mut self, id: i64) -> Result<(), String> {
        self.remove_many(&[id])
    }

    /// Atomic multi-selection deletion, including nested folders.
    /// A nonexistent item aborts all deletions in the selection.
    pub fn remove_many(&mut self, ids: &[i64]) -> Result<(), String> {
        if ids.is_empty() { return Ok(()); }
        if ids.contains(&1) { return Err("Cannot delete the container root".into()); }
        let mut unique = std::collections::HashSet::new();
        if ids.iter().any(|id| !unique.insert(*id)) { return Err("Duplicate delete selection".into()); }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        for id in ids {
            let deleted = transaction.execute("WITH RECURSIVE descendants(id) AS (SELECT id FROM items WHERE id=?1 UNION ALL SELECT i.id FROM items i JOIN descendants d ON i.parent=d.id) DELETE FROM items WHERE id IN (SELECT id FROM descendants)", [id])
                .map_err(|e| e.to_string())?;
            if deleted == 0 { return Err("Item not found; selection was not deleted".into()); }
        }
        transaction.commit().map_err(|e| e.to_string())?;
        self.touch();
        Ok(())
    }
    pub fn verify_integrity(&self) -> Result<(), String> {
        let mut statement = self.connection.prepare("PRAGMA cipher_integrity_check").map_err(|e| e.to_string())?;
        let issues = statement.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        if issues.is_empty() { Ok(()) } else { Err(format!("Encrypted container integrity check failed: {}", issues.len())) }
    }
}

fn import_path(
    tx: &rusqlite::Transaction<'_>, parent: i64, source: &Path,
    depth: usize, items: &mut usize, total_bytes: &mut u64,
) -> Result<(), String> {
    const MAX_ITEMS: usize = 10_000;
    const MAX_DEPTH: usize = 64;
    const MAX_BATCH_BYTES: u64 = 512 * 1024 * 1024;
    if depth > MAX_DEPTH || *items >= MAX_ITEMS {
        return Err("Encrypted import exceeds the folder depth or item limit".into());
    }
    let meta = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if is_reparse_point(&meta) {
        return Err("Encrypted import does not follow symlinks, junctions or reparse points".into());
    }
    let name = source.file_name().and_then(|n| n.to_str())
        .ok_or("Encrypted import requires a valid Unicode filename")?;
    valid_name(name)?;
    if meta.is_dir() {
        tx.execute("INSERT INTO items(parent,name,folder,contents) VALUES(?1,?2,1,NULL)", params![parent, name])
            .map_err(|e| e.to_string())?;
        *items += 1;
        let id = tx.last_insert_rowid();
        let mut children = std::fs::read_dir(source).map_err(|e| e.to_string())?
            .map(|result| result.map(|entry| entry.path()).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        children.sort();
        for child in children {
            import_path(tx, id, &child, depth + 1, items, total_bytes)?;
        }
    } else if meta.is_file() {
        if meta.len() > MAX_FILE_SIZE as u64 {
            return Err(format!("Encrypted import file exceeds 64 MiB: {}", source.display()));
        }
        *total_bytes = total_bytes.checked_add(meta.len()).ok_or("Encrypted import is too large")?;
        if *total_bytes > MAX_BATCH_BYTES {
            return Err("Encrypted import batch exceeds 512 MiB".into());
        }
        let data = Zeroizing::new(std::fs::read(source).map_err(|e| e.to_string())?);
        // The source may grow after its metadata was checked.
        if data.len() > MAX_FILE_SIZE {
            return Err("Encrypted import source changed while reading".into());
        }
        tx.execute("INSERT INTO items(parent,name,folder,contents) VALUES(?1,?2,0,?3)", params![parent, name, data.as_slice()])
            .map_err(|e| e.to_string())?;
        *items += 1;
    } else {
        return Err("Encrypted import accepts ordinary files and folders only".into());
    }
    Ok(())
}

fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() { return true; }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 { return true; }
    }
    false
}

fn is_windows_reserved_name(name: &str) -> bool {
    if name.ends_with('.') || name.ends_with(' ') { return true; }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" | "COM6" | "COM7" | "COM8" | "COM9" | "LPT1" | "LPT2" | "LPT3" | "LPT4" | "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recursive_import_is_atomic_and_preserves_encrypted_folder_tree() {
        let root = std::env::temp_dir().join(format!("ndr-vault-import-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(root.join("source/deep/sub")).unwrap();
        std::fs::write(root.join("source/deep/sub/secret.txt"), b"hidden plaintext sample").unwrap();
        std::fs::write(root.join("single.bin"), [0u8, 255, 23, 4]).unwrap();
        let container = root.join("vault.ndr");
        let mut vault = Vault::create(&container, "safe password 123456".into()).unwrap();
        assert_eq!(vault.import_paths(1, &[root.join("source"), root.join("single.bin")]).unwrap(), 5);
        let top = vault.list(1).unwrap();
        assert_eq!(top.len(), 2);
        let folder = top.iter().find(|item| item.name == "source").unwrap();
        let deep = vault.list(folder.id).unwrap();
        let sub = vault.list(deep[0].id).unwrap();
        let file = vault.list(sub[0].id).unwrap();
        assert_eq!(vault.read_file(file[0].id).unwrap(), b"hidden plaintext sample");
        assert_eq!(vault.read_file(top.iter().find(|i| i.name == "single.bin").unwrap().id).unwrap(), [0, 255, 23, 4]);
        // Even if one source inserts successfully, a later conflict rolls back
        // the entire batch instead of leaving a partial import.
        std::fs::write(root.join("additional.txt"), b"additional").unwrap();
        assert!(vault.import_paths(1, &[root.join("additional.txt"), root.join("single.bin")]).is_err());
        assert_eq!(vault.list(1).unwrap().len(), 2);
        assert!(vault.import_paths(1, &[root.join("additional.txt")]).is_ok());
        assert_eq!(vault.list(1).unwrap().len(), 3);
        vault.verify_integrity().unwrap();
        drop(vault);
        let raw = std::fs::read(&container).unwrap();
        assert!(!raw.windows(b"hidden plaintext sample".len()).any(|w| w == b"hidden plaintext sample"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recursive_import_rejects_symlink_without_partial_commit() {
        let root = std::env::temp_dir().join(format!("ndr-vault-link-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("valid.txt"), b"safe").unwrap();
        let link = root.join("linked.txt");
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(root.join("valid.txt"), &link).is_ok();
        #[cfg(not(windows))]
        let linked = std::os::unix::fs::symlink(root.join("valid.txt"), &link).is_ok();
        if linked {
            let mut vault = Vault::create(&root.join("safe.ndr"), "password 123456".into()).unwrap();
            assert!(vault.import_paths(1, &[root.join("valid.txt"), link]).is_err());
            assert!(vault.list(1).unwrap().is_empty());
            drop(vault);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn explicit_export_preserves_tree_and_rolls_back_on_conflicts() {
        let root = std::env::temp_dir().join(format!("ndr-vault-export-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let output = root.join("destination");
        std::fs::create_dir_all(&output).unwrap();
        let mut vault = Vault::create(&root.join("source.ndr"), "long passphrase 123456".into()).unwrap();
        let folder = vault.add_folder(1, "docs").unwrap();
        vault.add_file(folder, "report.txt", b"report plaintext").unwrap();
        vault.add_file(1, "data.bin", &[0, 3, 255, 12]).unwrap();
        let names = vec!["docs".into(), "data.bin".into()];
        assert_eq!(vault.export_items(1, &names, &output).unwrap(), 3);
        assert_eq!(std::fs::read(output.join("docs/report.txt")).unwrap(), b"report plaintext");
        assert_eq!(std::fs::read(output.join("data.bin")).unwrap(), [0, 3, 255, 12]);
        assert!(vault.export_items(1, &names, &output).is_err());
        std::fs::remove_dir_all(output.join("docs")).unwrap();
        assert!(vault.export_items(1, &names, &output).is_err());
        assert!(!output.join("docs").exists());
        assert_eq!(std::fs::read(output.join("data.bin")).unwrap(), [0, 3, 255, 12]);
        vault.verify_integrity().unwrap();
        drop(vault);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn multi_delete_is_all_or_nothing() {
        let root = std::env::temp_dir().join(format!("ndr-vault-delete-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let mut vault = Vault::create(&root.join("safe.ndr"), "safe password 123456".into()).unwrap();
        let a = vault.add_file(1, "a.txt", b"a").unwrap();
        let b = vault.add_file(1, "b.txt", b"b").unwrap();
        assert!(vault.remove_many(&[a, i64::MAX]).is_err());
        assert_eq!(vault.list(1).unwrap().len(), 2);
        assert!(vault.remove_many(&[a, a]).is_err());
        assert_eq!(vault.list(1).unwrap().len(), 2);
        vault.remove_many(&[a, b]).unwrap();
        assert!(vault.list(1).unwrap().is_empty());
        drop(vault);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn import_refuses_its_own_container() {
        let root = std::env::temp_dir().join(format!("ndr-vault-self-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("inner.ndr");
        let mut vault = Vault::create(&file, "long password 123456".into()).unwrap();
        assert!(vault.import_paths(1, &[file.clone()]).is_err());
        assert!(vault.import_paths(1, &[root.clone()]).is_err());
        assert!(vault.list(1).unwrap().is_empty());
        drop(vault);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn password_minimum_is_six_unicode_characters() {
        let path = std::env::temp_dir().join(format!("ndr-password-six-{}.ndr", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(Vault::create(&path, "12345".into()).is_err());
        assert!(!path.exists(), "rejected password must not leave a container");
        let vault = Vault::create(&path, "123456".into()).unwrap();
        drop(vault);
        assert!(Vault::unlock(&path, "12345".into()).is_err());
        assert!(Vault::unlock(&path, "123456".into()).is_ok());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn encrypted_container_roundtrip_password_and_structure() {
        let path = std::env::temp_dir().join(format!("ndr-vault-{}.ndr", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let secret = "INTERNAL-SECRET-FILENAME-DO-NOT-LEAK.txt";
        let mut vault = Vault::create(&path, "high entropy password 1234!".into()).unwrap();
        let folder = vault.add_folder(1, "private").unwrap();
        let file = vault.add_file(folder, secret, b"secret document content 12345678").unwrap();
        assert_eq!(vault.read_file(file).unwrap(), b"secret document content 12345678");
        assert_eq!(vault.list(folder).unwrap()[0].name, secret);
        vault.verify_integrity().unwrap();
        drop(vault);
        let raw = std::fs::read(&path).unwrap();
        assert!(!raw.windows(secret.len()).any(|w| w == secret.as_bytes()));
        assert!(!raw.windows(13).any(|w| w == b"secret document"));
        assert!(!raw.starts_with(b"SQLite format 3"));
        assert!(Vault::unlock(&path, "wrong password 123456".into()).is_err());
        let mut opened = Vault::unlock(&path, "high entropy password 1234!".into()).unwrap();
        assert_eq!(opened.read_file(file).unwrap(), b"secret document content 12345678");
        opened.remove(folder).unwrap();
        assert!(opened.list(1).unwrap().is_empty());
        drop(opened);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn policy_expiration_and_required_reauthentication() {
        let now = Instant::now();
        assert!(!SessionPolicy::CloseOnLeave.expired(now, now, now + Duration::from_secs(1200)));
        assert!(!SessionPolicy::UntilExit.expired(now, now, now + Duration::from_secs(1200)));
        assert!(SessionPolicy::Elapsed(Duration::from_secs(900)).expired(now, now + Duration::from_secs(800), now + Duration::from_secs(900)));
        assert!(SessionPolicy::Inactive(Duration::from_secs(900)).expired(now, now, now + Duration::from_secs(900)));
    }
}
