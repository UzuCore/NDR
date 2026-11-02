//! NDR's password-only SQLCipher encrypted virtual folder backend.
//! Uses SQLCipher 4 authenticated pages; never creates a plaintext staging directory.
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::{path::{Path, PathBuf}, time::{Duration, Instant}};
use std::io::{Read, Write};
use zeroize::{Zeroize, Zeroizing};

pub const EXTENSION: &str = "ndr";
pub const MAX_FILE_SIZE: usize = 64 * 1024 * 1024;
pub const STREAM_CHUNK_SIZE: usize = 8 * 1024 * 1024;

/// V1 containers remain readable; large imported files use a supplemental
/// chunk table without changing the authenticated SQLCipher file format.
fn ensure_chunks(conn: &Connection) -> Result<(), String> {
    // r201 computed the size of each encrypted file by aggregating every
    // chunk on EVERY folder refresh. Cache exact byte sizes once; the table is
    // only created when the first newer build opens an existing container.
    // Existing SQLCipher v1 single-BLOB files remain fully compatible.
    let has_sizes: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='item_sizes')",
        [], |row| row.get(0)).map_err(|e| e.to_string())?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS item_chunks (
        item_id INTEGER NOT NULL, ordinal INTEGER NOT NULL,
        data BLOB NOT NULL, PRIMARY KEY(item_id,ordinal)
    ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS item_sizes (
        item_id INTEGER PRIMARY KEY, byte_size INTEGER NOT NULL CHECK(byte_size>=0)
    );
    CREATE TRIGGER IF NOT EXISTS item_chunks_insert_size AFTER INSERT ON item_chunks
       BEGIN INSERT INTO item_sizes(item_id,byte_size) VALUES(NEW.item_id,length(NEW.data))
       ON CONFLICT(item_id) DO UPDATE SET byte_size=byte_size+excluded.byte_size; END;
    CREATE TRIGGER IF NOT EXISTS item_chunks_delete_size AFTER DELETE ON item_chunks
       BEGIN UPDATE item_sizes SET byte_size=byte_size-length(OLD.data)
       WHERE item_id=OLD.item_id; END;
    CREATE TRIGGER IF NOT EXISTS items_delete_chunks AFTER DELETE ON items
       BEGIN DELETE FROM item_chunks WHERE item_id=OLD.id; END;
    CREATE TRIGGER IF NOT EXISTS items_delete_sizes AFTER DELETE ON items
       BEGIN DELETE FROM item_sizes WHERE item_id=OLD.id; END;")
       .map_err(|e| e.to_string())?;
    if !has_sizes {
        // Backfill once for containers written by r201-r203. Reopening an
        // already migrated vault never rescans large encrypted BLOB rows.
        conn.execute("INSERT INTO item_sizes(item_id,byte_size)
            SELECT item_id,SUM(length(data)) FROM item_chunks GROUP BY item_id", [])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPolicy {
    CloseOnLeave,
    Inactive(Duration),
    Elapsed(Duration),
    UntilExit,
}

impl SessionPolicy {
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let minutes = settings.values.get("VaultSessionMinutes").and_then(|v| v.as_i64()).unwrap_or(10).clamp(1, 1440) as u64;
        match settings.text("VaultSessionPolicy", "elapsed").as_str() {
            "exit" => Self::UntilExit,
            // Legacy close/inactivity settings migrate to the time-limited
            // policy. Sessions no longer expire simply by leaving the folder.
            _ => Self::Elapsed(Duration::from_secs(minutes * 60)),
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
            ensure_chunks(&conn)?;
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
        ensure_chunks(&conn)?;
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
        let mut statement = self.connection.prepare("SELECT items.id,items.name,items.folder,
            coalesce(item_sizes.byte_size,length(items.contents),0)
            FROM items LEFT JOIN item_sizes ON item_sizes.item_id=items.id
            WHERE items.parent=?1 ORDER BY items.folder DESC,items.name COLLATE NOCASE")
            .map_err(|e| e.to_string())?;
        let values = statement.query_map([parent], |r| Ok(VaultItem { id: r.get(0)?, name: r.get(1)?, folder: r.get::<_, i64>(2)? == 1, size: r.get(3)? }))
            .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        Ok(values)
    }

    pub fn parent(&self, id: i64) -> Result<i64, String> {
        self.connection.query_row("SELECT parent FROM items WHERE id=?1 AND folder=1", [id], |r| r.get(0))
            .map_err(|e| e.to_string())
    }

    /// Display path for an internal encrypted folder; never accesses the
    /// ordinary filesystem outside the container.
    pub fn relative_folder_path(&self, mut folder: i64) -> Result<PathBuf, String> {
        let mut parts = Vec::new();
        while folder != 1 {
            if parts.len() >= 64 { return Err("Encrypted folder depth exceeded".into()); }
            let (parent, name): (i64, String) = self.connection.query_row(
                "SELECT parent,name FROM items WHERE id=?1 AND folder=1", [folder],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).map_err(|e| e.to_string())?;
            if parent == folder { return Err("Invalid encrypted folder parent".into()); }
            parts.push(name);
            folder = parent;
        }
        let mut result = PathBuf::new();
        for component in parts.into_iter().rev() { result.push(component); }
        Ok(result)
    }

    /// Resolve a folder path relative to the encrypted container root.
    pub fn resolve_folder(&mut self, path: &str) -> Result<i64, String> {
        let mut folder = 1;
        for part in path.trim_matches('/').split('/') {
            if part.is_empty() { continue; }
            valid_name(part)?;
            folder = self.connection.query_row(
                "SELECT id FROM items WHERE parent=?1 AND folder=1 AND name=?2 COLLATE NOCASE",
                params![folder,part], |row| row.get(0))
                .optional().map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Encrypted destination folder not found: {part}"))?;
        }
        Ok(folder)
    }

    /// Move/copy selected children within one encrypted container atomically.
    pub fn transfer_items(&mut self, from: i64, names: &[String], to: i64, moving: bool) -> Result<usize, String> {
        if names.is_empty() { return Ok(0); }
        self.ensure_folder(from)?;
        self.ensure_folder(to)?;
        if from == to && moving { return Ok(0); }
        let items = self.list(from)?;
        let mut selected = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for name in names {
            let item = items.iter().find(|item| &item.name == name)
                .ok_or_else(|| format!("Encrypted source item is missing: {name}"))?;
            if !seen.insert(item.id) { return Err("Duplicate encrypted selection".into()); }
            selected.push((item.id, item.name.clone(), item.folder));
        }
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        for (id, name, is_folder) in &selected {
            // Copies into descendants would otherwise traverse the newly
            // created copy recursively and can grow without bound.
            if *is_folder {
                let mut ancestor = to;
                loop {
                    if ancestor == *id { return Err("Cannot move a folder inside itself".into()); }
                    if ancestor == 1 { break; }
                    ancestor = tx.query_row("SELECT parent FROM items WHERE id=?1 AND folder=1", [ancestor], |r| r.get(0))
                        .map_err(|e| e.to_string())?;
                }
            }
            let clash: i64 = tx.query_row("SELECT COUNT(*) FROM items WHERE parent=?1 AND name=?2 COLLATE NOCASE", params![to,name], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if clash != 0 { return Err(format!("Encrypted destination already contains {name}")); }
        }
        let mut copied = 0;
        let mut bytes = 0u64;
        for (id, _, _) in &selected {
            if moving {
                tx.execute("UPDATE items SET parent=?1 WHERE id=?2", params![to,id]).map_err(|e| e.to_string())?;
                copied += 1;
            } else {
                copy_encrypted_item(&tx, *id, to, 0, &mut copied, &mut bytes)?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.touch();
        Ok(copied)
    }

    /// Copy from a different encrypted database without plaintext staging.
    /// The destination is atomic; source deletion for a move is a separate
    /// caller operation after this transaction has committed successfully.
    pub fn copy_from_vault(&mut self, source: &mut Vault, from: i64, names: &[String], to: i64) -> Result<usize, String> {
        if names.is_empty() { return Ok(0); }
        let a = std::fs::canonicalize(&self.file).map_err(|e| e.to_string())?;
        let b = std::fs::canonicalize(&source.file).map_err(|e| e.to_string())?;
        if a == b { return Err("Use internal transfer for the same encrypted container".into()); }
        self.ensure_folder(to)?;
        source.ensure_folder(from)?;
        let available = source.list(from)?;
        let mut selected = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for name in names {
            let item = available.iter().find(|item| item.name == *name)
                .ok_or_else(|| format!("Encrypted source item is missing: {name}"))?;
            if !seen.insert(item.id) { return Err("Duplicate encrypted selection".into()); }
            selected.push(item.id);
        }
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        let mut count = 0usize;
        let mut bytes = 0u64;
        for id in selected {
            copy_from_other_vault(&tx, source, id, to, 0, &mut count, &mut bytes)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.touch();
        Ok(count)
    }

    /// Copy one item with optional destination rename/replacement. Replacing
    /// the existing destination and copying the new tree is one transaction.
    pub fn copy_from_vault_item(&mut self, source: &mut Vault, from: i64, name: &str,
        to: i64, target_name: &str, replace: bool) -> Result<usize, String> {
        valid_name(target_name)?;
        self.ensure_folder(to)?;
        source.ensure_folder(from)?;
        if std::fs::canonicalize(&self.file).map_err(|e| e.to_string())?
            == std::fs::canonicalize(&source.file).map_err(|e| e.to_string())? {
            return Err("Use same-container transfer for the same encrypted file".into());
        }
        let item = source.list(from)?.into_iter().find(|v| v.name == name)
            .ok_or("Encrypted source item is missing")?;
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        replace_destination(&tx, to, target_name, replace)?;
        let mut count = 0;
        let mut bytes = 0;
        copy_from_other_vault_named(&tx, source, item.id, to, Some(target_name), 0, &mut count, &mut bytes)?;
        tx.commit().map_err(|e| e.to_string())?;
        self.touch();
        Ok(count)
    }

    /// Import a single filesystem root as a different name or replace an
    /// existing vault entry atomically. Never removes the source path.
    pub fn import_one(&mut self, to: i64, path: &Path, name: &str, replace: bool) -> Result<usize, String> {
        valid_name(name)?;
        self.ensure_folder(to)?;
        let own = std::fs::canonicalize(&self.file).map_err(|e| e.to_string())?;
        let source = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        if own.starts_with(&source) { return Err("Cannot import the open encrypted container or its parent".into()); }
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        replace_destination(&tx, to, name, replace)?;
        let mut count = 0;
        let mut bytes = 0;
        import_path_named(&tx, to, path, Some(name), 0, &mut count, &mut bytes)?;
        tx.commit().map_err(|e| e.to_string())?;
        self.touch();
        Ok(count)
    }

    /// One safe internal copy/move; overwrite approval replaces the whole
    /// destination entry in the same transaction as the new data.
    pub fn transfer_one(&mut self, from: i64, name: &str, to: i64,
        target_name: &str, moving: bool, replace: bool) -> Result<usize, String> {
        valid_name(target_name)?;
        self.ensure_folder(from)?;
        self.ensure_folder(to)?;
        let item = self.list(from)?.into_iter().find(|v| v.name == name)
            .ok_or("Encrypted source item is missing")?;
        if item.folder {
            let mut at = to;
            loop {
                if at == item.id { return Err("Cannot transfer a folder into its descendants".into()); }
                if at == 1 { break; }
                at = self.parent(at)?;
            }
        }
        if from == to && item.name.eq_ignore_ascii_case(target_name) {
            if moving { return Ok(0); }
            return Err("Copying an item over itself is not permitted".into());
        }
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        replace_destination(&tx, to, target_name, replace)?;
        let mut count = 0;
        if moving {
            tx.execute("UPDATE items SET parent=?1,name=?2 WHERE id=?3", params![to,target_name,item.id])
                .map_err(|e| e.to_string())?;
            count = 1;
        } else {
            let mut bytes = 0;
            copy_encrypted_item_named(&tx, item.id, to, Some(target_name), 0, &mut count, &mut bytes)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        self.touch();
        Ok(count)
    }

    pub fn add_folder(&mut self, parent: i64, name: &str) -> Result<i64, String> {
        self.insert(parent, name, true, None)
    }
    pub fn add_file(&mut self, parent: i64, name: &str, data: &[u8]) -> Result<i64, String> {
        if data.len() > MAX_FILE_SIZE { return Err("File exceeds current container safety limit (64 MiB)".into()); }
        self.insert(parent, name, false, Some(data))
    }

    /// Begin a file transfer whose lifetime extends across UI frames. The
    /// caller must call commit_stream_file or rollback_stream_file, including
    /// on cancellation. An uncommitted transaction is rolled back on close.
    pub fn begin_stream_file(&mut self, parent: i64, name: &str, replace: bool) -> Result<i64, String> {
        valid_name(name)?;
        self.ensure_folder(parent)?;
        self.connection.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
        let prepare = (|| -> Result<i64, String> {
            replace_destination(&self.connection, parent, name, replace)?;
            self.connection.execute("INSERT INTO items(parent,name,folder,contents) VALUES(?1,?2,0,NULL)",
                params![parent,name]).map_err(|e| e.to_string())?;
            Ok(self.connection.last_insert_rowid())
        })();
        if prepare.is_err() { let _ = self.connection.execute_batch("ROLLBACK"); }
        prepare
    }

    pub fn stream_write_chunk(&mut self, id: i64, ordinal: i64, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > STREAM_CHUNK_SIZE || ordinal < 0 { return Err("Invalid streaming chunk".into()); }
        self.connection.execute("INSERT INTO item_chunks(item_id,ordinal,data) VALUES(?1,?2,?3)",
            params![id,ordinal,bytes]).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn stream_file_size(&self, id: i64) -> Result<u64, String> {
        let size: i64 = self.connection.query_row("SELECT coalesce(
                (SELECT byte_size FROM item_sizes WHERE item_id=?1),length(contents),0)
            FROM items WHERE id=?1 AND folder=0",
            [id], |r| r.get(0)).map_err(|e| e.to_string())?;
        u64::try_from(size).map_err(|e| e.to_string())
    }

    pub fn stream_read_chunk(&self, id: i64, ordinal: i64) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        if ordinal < 0 { return Err("Invalid streaming chunk offset".into()); }
        let has_chunks: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM item_chunks WHERE item_id=?1)", [id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if has_chunks {
            return self.connection.query_row("SELECT data FROM item_chunks WHERE item_id=?1 AND ordinal=?2",
                params![id,ordinal], |row| row.get::<_,Vec<u8>>(0))
                .optional().map(|v| v.map(Zeroizing::new)).map_err(|e| e.to_string());
        }
        let mut legacy = self.connection.blob_open(rusqlite::MAIN_DB, "items", "contents", id, true)
            .map_err(|e| e.to_string())?;
        let start = (ordinal as u64).checked_mul(STREAM_CHUNK_SIZE as u64)
            .ok_or("Encrypted chunk offset overflow")?;
        let length = legacy.len() as u64;
        if start >= length { return Ok(None); }
        use std::io::{Seek, SeekFrom};
        legacy.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let mut bytes = Zeroizing::new(vec![0u8; ((length-start) as usize).min(STREAM_CHUNK_SIZE)]);
        legacy.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        Ok(Some(bytes))
    }

    pub fn commit_stream_file(&mut self) -> Result<(), String> {
        self.connection.execute_batch("COMMIT").map_err(|e| e.to_string())?;
        self.touch();
        Ok(())
    }

    pub fn rollback_stream_file(&mut self) -> Result<(), String> {
        self.connection.execute_batch("ROLLBACK").map_err(|e| e.to_string())?;
        self.touch();
        Ok(())
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

    /// Export one root under a new name, optionally replacing an existing
    /// non-reparse destination. Existing content is temporarily moved aside
    /// and restored if writing the replacement fails.
    pub fn export_one(&mut self, from: i64, name: &str, destination: &Path,
        target_name: &str, replace: bool) -> Result<usize, String> {
        valid_name(target_name)?;
        if is_windows_reserved_name(target_name) { return Err("Reserved Windows export name".into()); }
        let meta = std::fs::symlink_metadata(destination).map_err(|e| e.to_string())?;
        if !meta.is_dir() || is_reparse_point(&meta) { return Err("Export destination must be a real directory".into()); }
        let item = self.list(from)?.into_iter().find(|item| item.name == name)
            .ok_or("Encrypted source item does not exist")?;
        let path = destination.join(target_name);
        let occupied = std::fs::symlink_metadata(&path).ok();
        if occupied.is_some() && !replace { return Err("Destination already exists".into()); }
        if occupied.as_ref().is_some_and(is_reparse_point) { return Err("Cannot replace a symbolic link or junction".into()); }
        let mut backup = None;
        if occupied.is_some() {
            let mut candidate = None;
            for attempt in 0..16u32 {
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| e.to_string())?.as_nanos();
                let alternative = destination.join(format!(".ndr-recovery-{}-{stamp}-{attempt}", std::process::id()));
                if std::fs::symlink_metadata(&alternative).is_err() { candidate = Some(alternative); break; }
            }
            let alternative = candidate.ok_or("Unable to reserve a unique replacement backup path")?;
            std::fs::rename(&path, &alternative).map_err(|e| e.to_string())?;
            backup = Some(alternative);
        }
        let mut created = Vec::new();
        let mut count = 0;
        let mut bytes = 0;
        let result = self.export_item(item.id, target_name, destination, 0, &mut count, &mut bytes, &mut created);
        if let Err(error) = result {
            for file in created.iter().rev() {
                if file.is_dir() { let _ = std::fs::remove_dir(file); }
                else { let _ = std::fs::remove_file(file); }
            }
            if let Some(backup) = backup {
                std::fs::rename(&backup, &path).map_err(|e| format!(
                    "Export failed: {error}. Original preserved at {} but restoration failed: {e}", backup.display()))?;
            }
            return Err(error);
        }
        if let Some(backup) = backup {
            let meta = std::fs::symlink_metadata(&backup).map_err(|e| e.to_string())?;
            let removed = if meta.is_dir() { std::fs::remove_dir_all(&backup) } else { std::fs::remove_file(&backup) };
            removed.map_err(|e| format!("Export finished but old target backup remains at {}: {e}", backup.display()))?;
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
            let mut out = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
                .map_err(|e| e.to_string())?;
            created.push(path);
            let bytes = write_stored_chunks(&self.connection, id, &mut out)?;
            *total = total.checked_add(bytes).ok_or("Export size overflow")?;
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
        let valid: i64 = self.connection.query_row("SELECT folder FROM items WHERE id=?1", [id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if valid != 0 { return Err("Encrypted item is a folder".into()); }
        // In-memory text editing still has an explicit safety bound; large
        // transfers use write_stored_chunks and never invoke read_file.
        let size = self.stream_file_size(id)?;
        if size > MAX_FILE_SIZE as u64 { return Err("File is too large for in-memory editing; export to edit".into()); }
        let mut result = Vec::with_capacity(size as usize);
        write_stored_chunks(&self.connection, id, &mut result)?;
        self.touch();
        Ok(result)
    }
    pub fn update_file(&mut self, id: i64, data: &[u8]) -> Result<(), String> {
        if data.len() > MAX_FILE_SIZE { return Err("File exceeds current 64 MiB limit".into()); }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let changed = transaction.execute("UPDATE items SET contents=?1 WHERE id=?2 AND folder=0", params![data,id])
            .map_err(|e| e.to_string())?;
        if changed == 0 { return Err("File not found".into()); }
        transaction.execute("DELETE FROM item_chunks WHERE item_id=?1", [id]).map_err(|e| e.to_string())?;
        transaction.execute("DELETE FROM item_sizes WHERE item_id=?1", [id]).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
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

fn replace_destination(tx: &Connection, parent: i64, name: &str, replace: bool) -> Result<(), String> {
    let existing: Option<i64> = tx.query_row(
        "SELECT id FROM items WHERE parent=?1 AND name=?2 COLLATE NOCASE", params![parent,name],
        |r| r.get(0),
    ).optional().map_err(|e| e.to_string())?;
    if let Some(id) = existing {
        if !replace { return Err(format!("Destination already exists: {name}")); }
        tx.execute("WITH RECURSIVE descendants(id) AS (SELECT id FROM items WHERE id=?1 UNION ALL SELECT i.id FROM items i JOIN descendants d ON i.parent=d.id) DELETE FROM items WHERE id IN (SELECT id FROM descendants)", [id])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Process encrypted data with a bounded 8 MiB plaintext buffer. A failed
/// read or database write rolls back the caller's SQLCipher transaction.
fn append_chunks<R: Read>(tx: &rusqlite::Transaction<'_>, id: i64, source: &mut R) -> Result<u64, String> {
    let mut buffer = Zeroizing::new(vec![0u8; STREAM_CHUNK_SIZE]);
    let mut ordinal = 0i64;
    let mut total = 0u64;
    loop {
        let size = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 { break; }
        tx.execute("INSERT INTO item_chunks(item_id,ordinal,data) VALUES(?1,?2,?3)",
            params![id, ordinal, &buffer[..size]]).map_err(|e| e.to_string())?;
        ordinal += 1;
        total = total.checked_add(size as u64).ok_or("Encrypted item size overflow")?;
    }
    if ordinal == 0 {
        tx.execute("INSERT INTO item_chunks(item_id,ordinal,data) VALUES(?1,0,x'')", [id])
            .map_err(|e| e.to_string())?;
    }
    Ok(total)
}

fn copy_stored_chunks(tx: &rusqlite::Transaction<'_>, source: &Connection,
    source_id: i64, target_id: i64) -> Result<u64, String> {
    let mut total = 0u64;
    let count: i64 = source.query_row("SELECT COUNT(*) FROM item_chunks WHERE item_id=?1",
        [source_id], |r| r.get(0)).map_err(|e| e.to_string())?;
    if count > 0 {
        let mut query = source.prepare("SELECT data FROM item_chunks WHERE item_id=?1 ORDER BY ordinal")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([source_id]).map_err(|e| e.to_string())?;
        let mut ordinal = 0i64;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let bytes = Zeroizing::new(row.get::<_, Vec<u8>>(0).map_err(|e| e.to_string())?);
            if bytes.len() > STREAM_CHUNK_SIZE { return Err("Invalid encrypted chunk size".into()); }
            tx.execute("INSERT INTO item_chunks(item_id,ordinal,data) VALUES(?1,?2,?3)",
                params![target_id, ordinal, bytes.as_slice()]).map_err(|e| e.to_string())?;
            total = total.checked_add(bytes.len() as u64).ok_or("Encrypted item size overflow")?;
            ordinal += 1;
        }
    } else {
        let mut legacy = source.blob_open(rusqlite::MAIN_DB, "items", "contents", source_id, true)
            .map_err(|e| e.to_string())?;
        total = append_chunks(tx, target_id, &mut legacy)?;
    }
    Ok(total)
}

fn write_stored_chunks<W: Write>(connection: &Connection, id: i64, destination: &mut W) -> Result<u64, String> {
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM item_chunks WHERE item_id=?1",
        [id], |r| r.get(0)).map_err(|e| e.to_string())?;
    let mut total = 0u64;
    if count > 0 {
        let mut query = connection.prepare("SELECT data FROM item_chunks WHERE item_id=?1 ORDER BY ordinal")
            .map_err(|e| e.to_string())?;
        let mut rows = query.query([id]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let bytes = Zeroizing::new(row.get::<_, Vec<u8>>(0).map_err(|e| e.to_string())?);
            if bytes.len() > STREAM_CHUNK_SIZE { return Err("Invalid encrypted chunk size".into()); }
            destination.write_all(&bytes).map_err(|e| e.to_string())?;
            total = total.checked_add(bytes.len() as u64).ok_or("Export size overflow")?;
        }
    } else {
        let mut legacy = connection.blob_open(rusqlite::MAIN_DB, "items", "contents", id, true)
            .map_err(|e| e.to_string())?;
        let mut buffer = Zeroizing::new(vec![0u8; STREAM_CHUNK_SIZE]);
        loop {
            let size = legacy.read(&mut buffer).map_err(|e| e.to_string())?;
            if size == 0 { break; }
            destination.write_all(&buffer[..size]).map_err(|e| e.to_string())?;
            total = total.checked_add(size as u64).ok_or("Export size overflow")?;
        }
    }
    Ok(total)
}

fn copy_from_other_vault_named(
    tx: &rusqlite::Transaction<'_>, source: &mut Vault, id: i64, parent: i64,
    renamed: Option<&str>, depth: usize, count: &mut usize, total: &mut u64,
) -> Result<(), String> {
    if depth > 64 || *count >= 10_000 { return Err("Cross-vault copy exceeds depth or item limit".into()); }
    let (original, folder): (String, bool) = source.connection.query_row(
        "SELECT name,folder FROM items WHERE id=?1", [id],
        |r| Ok((r.get(0)?, r.get::<_, i64>(1)? == 1)),
    ).map_err(|e| e.to_string())?;
    let name = renamed.unwrap_or(&original);
    valid_name(name)?;
    let collision: i64 = tx.query_row(
        "SELECT COUNT(*) FROM items WHERE parent=?1 AND name=?2 COLLATE NOCASE",
        params![parent, name], |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    if collision != 0 { return Err(format!("Encrypted destination already contains {name}")); }
    if folder {
        tx.execute("INSERT INTO items(parent,name,folder,contents) VALUES(?1,?2,1,NULL)", params![parent,name])
            .map_err(|e| e.to_string())?;
        let new_parent = tx.last_insert_rowid();
        *count += 1;
        for child in source.list(id)? {
            copy_from_other_vault_named(tx, source, child.id, new_parent, None, depth + 1, count, total)?;
        }
    } else {
        tx.execute("INSERT INTO items(parent,name,folder,contents) VALUES(?1,?2,0,NULL)",
            params![parent,name]).map_err(|e| e.to_string())?;
        let target_id = tx.last_insert_rowid();
        let size = copy_stored_chunks(tx, &source.connection, id, target_id)?;
        *total = total.checked_add(size).ok_or("Cross-vault transfer size overflow")?;
        *count += 1;
    }
    Ok(())
}

fn copy_from_other_vault(
    tx: &rusqlite::Transaction<'_>, source: &mut Vault, id: i64, parent: i64,
    depth: usize, count: &mut usize, total: &mut u64,
) -> Result<(), String> {
    copy_from_other_vault_named(tx, source, id, parent, None, depth, count, total)
}

fn copy_encrypted_item_named(
    tx: &rusqlite::Transaction<'_>, id: i64, parent: i64, renamed: Option<&str>, depth: usize,
    count: &mut usize, total_bytes: &mut u64,
) -> Result<(), String> {
    if depth > 64 || *count >= 10_000 { return Err("Encrypted copy exceeds depth or item limit".into()); }
    let (original, folder): (String, i64) = tx.query_row(
        "SELECT name,folder FROM items WHERE id=?1", [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).map_err(|e| e.to_string())?;
    let name = renamed.unwrap_or(&original);
    valid_name(name)?;
    tx.execute("INSERT INTO items(parent,name,folder,contents) VALUES(?1,?2,?3,NULL)",
        params![parent, name, folder]).map_err(|e| e.to_string())?;
    let target = tx.last_insert_rowid();
    if folder == 0 {
        // Read from the same encrypted connection while the destination
        // transaction is open. Iterating the source chunk query would keep a
        // SQLite read statement alive during writes, so copy via indexed fetch.
        let n: i64 = tx.query_row("SELECT COUNT(*) FROM item_chunks WHERE item_id=?1",
            [id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if n > 0 {
            for ordinal in 0..n {
                let bytes = Zeroizing::new(tx.query_row("SELECT data FROM item_chunks WHERE item_id=?1 AND ordinal=?2",
                    params![id,ordinal], |r| r.get::<_, Vec<u8>>(0)).map_err(|e| e.to_string())?);
                tx.execute("INSERT INTO item_chunks(item_id,ordinal,data) VALUES(?1,?2,?3)",
                    params![target,ordinal,bytes.as_slice()]).map_err(|e| e.to_string())?;
                *total_bytes = total_bytes.checked_add(bytes.len() as u64).ok_or("Encrypted copy size overflow")?;
            }
        } else {
            let contents = Zeroizing::new(tx.query_row("SELECT contents FROM items WHERE id=?1",
                [id], |r| r.get::<_, Vec<u8>>(0)).map_err(|e| e.to_string())?);
            let mut reader = contents.as_slice();
            *total_bytes = total_bytes.checked_add(append_chunks(tx, target, &mut reader)?)
                .ok_or("Encrypted copy size overflow")?;
        }
    }
    *count += 1;
    if folder == 1 {
        let mut statement = tx.prepare("SELECT id FROM items WHERE parent=?1 ORDER BY id")
            .map_err(|e| e.to_string())?;
        let children: Vec<i64> = statement.query_map([id], |r| r.get(0))
            .map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        drop(statement);
        for child in children {
            copy_encrypted_item_named(tx, child, target, None, depth + 1, count, total_bytes)?;
        }
    }
    Ok(())
}

fn copy_encrypted_item(
    tx: &rusqlite::Transaction<'_>, id: i64, parent: i64, depth: usize,
    count: &mut usize, total_bytes: &mut u64,
) -> Result<(), String> {
    copy_encrypted_item_named(tx, id, parent, None, depth, count, total_bytes)
}

fn import_path_named(
    tx: &rusqlite::Transaction<'_>, parent: i64, source: &Path, renamed: Option<&str>,
    depth: usize, items: &mut usize, total_bytes: &mut u64,
) -> Result<(), String> {
    const MAX_ITEMS: usize = 10_000;
    const MAX_DEPTH: usize = 64;
    if depth > MAX_DEPTH || *items >= MAX_ITEMS {
        return Err("Encrypted import exceeds the folder depth or item limit".into());
    }
    let meta = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if is_reparse_point(&meta) {
        return Err("Encrypted import does not follow symlinks, junctions or reparse points".into());
    }
    let default_name = source.file_name().and_then(|n| n.to_str())
        .ok_or("Encrypted import requires a valid Unicode filename")?;
    let name = renamed.unwrap_or(default_name);
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
            import_path_named(tx, id, &child, None, depth + 1, items, total_bytes)?;
        }
    } else if meta.is_file() {
        // Only one 8 MiB plaintext buffer is used regardless of source size.
        // The enclosing transaction rolls back the entire folder on error.
        tx.execute("INSERT INTO items(parent,name,folder,contents) VALUES(?1,?2,0,NULL)", params![parent, name])
            .map_err(|e| e.to_string())?;
        let id = tx.last_insert_rowid();
        let mut input = std::fs::File::open(source).map_err(|e| e.to_string())?;
        let bytes = append_chunks(tx, id, &mut input)?;
        if bytes != meta.len() {
            return Err("Import source changed size during encryption; rolled back".into());
        }
        *total_bytes = total_bytes.checked_add(bytes).ok_or("Encrypted import size overflow")?;
        *items += 1;
    } else {
        return Err("Encrypted import accepts ordinary files and folders only".into());
    }
    Ok(())
}

fn import_path(
    tx: &rusqlite::Transaction<'_>, parent: i64, source: &Path,
    depth: usize, items: &mut usize, total_bytes: &mut u64,
) -> Result<(), String> {
    import_path_named(tx, parent, source, None, depth, items, total_bytes)
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
    fn cached_file_sizes_survive_legacy_migration_copy_edit_delete_and_reopen() {
        let root = std::env::temp_dir().join(format!("ndr-fast-list-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("fast.ndr");
        let mut vault = Vault::create(&path, "fast list password".into()).unwrap();
        vault.add_file(1, "legacy.txt", b"old").unwrap();
        let source = root.join("large.bin");
        std::fs::write(&source, vec![42u8; STREAM_CHUNK_SIZE + 17]).unwrap();
        vault.import_one(1, &source, "large.bin", false).unwrap();
        let large = vault.list(1).unwrap().into_iter().find(|i| i.name == "large.bin").unwrap();
        assert_eq!(large.size as usize, STREAM_CHUNK_SIZE + 17);
        let cached: i64 = vault.connection.query_row("SELECT byte_size FROM item_sizes WHERE item_id=?1",
            [large.id], |r| r.get(0)).unwrap();
        assert_eq!(cached, large.size);
        // Emulate a container created by r201-r203 before the metadata cache.
        vault.connection.execute_batch("DROP TRIGGER item_chunks_insert_size;
            DROP TRIGGER item_chunks_delete_size; DROP TRIGGER items_delete_sizes;
            DROP TABLE item_sizes;").unwrap();
        drop(vault);
        let mut vault = Vault::unlock(&path, "fast list password".into()).unwrap();
        assert_eq!(vault.list(1).unwrap().into_iter().find(|i| i.name == "large.bin").unwrap().size,
            (STREAM_CHUNK_SIZE + 17) as i64);
        let cache_rebuilt: i64 = vault.connection.query_row("SELECT byte_size FROM item_sizes WHERE item_id=?1",
            [large.id], |r| r.get(0)).unwrap();
        assert_eq!(cache_rebuilt, large.size);
        let destination = vault.add_folder(1, "copies").unwrap();
        vault.transfer_items(1, &["large.bin".into()], destination, false).unwrap();
        let duplicated = vault.list(destination).unwrap().into_iter().find(|i| i.name == "large.bin").unwrap();
        assert_eq!(duplicated.size, large.size);
        vault.update_file(duplicated.id, b"edited").unwrap();
        assert_eq!(vault.list(destination).unwrap().into_iter().find(|i| i.name == "large.bin").unwrap().size, 6);
        assert_eq!(vault.stream_file_size(duplicated.id).unwrap(), 6);
        vault.remove_many(&[large.id]).unwrap();
        let old_cache: Option<i64> = vault.connection.query_row(
            "SELECT byte_size FROM item_sizes WHERE item_id=?1", [large.id], |r| r.get(0)).optional().unwrap();
        assert_eq!(old_cache, None, "deletion must clear the metadata cache");
        vault.verify_integrity().unwrap();
        drop(vault);
        let mut reopened = Vault::unlock(&path, "fast list password".into()).unwrap();
        assert_eq!(reopened.list(destination).unwrap().into_iter().find(|i| i.name == "large.bin").unwrap().size, 6);
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unlock_old_v1_container_and_mix_legacy_blob_with_streamed_chunks() {
        let root = std::env::temp_dir().join(format!("ndr-stream-compat-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("legacy.ndr");
        let mut old = Vault::create(&path, "legacy password 123".into()).unwrap();
        old.add_file(1, "legacy.txt", b"original 2026 format").unwrap();
        old.connection.execute_batch("DROP TRIGGER items_delete_chunks; DROP TABLE item_chunks;").unwrap();
        drop(old);
        let mut opened = Vault::unlock(&path, "legacy password 123".into()).unwrap();
        let legacy_id = opened.list(1).unwrap()[0].id;
        assert_eq!(opened.read_file(legacy_id).unwrap(), b"original 2026 format");
        let input = root.join("new.bin");
        std::fs::write(&input, b"new chunked format data").unwrap();
        opened.import_one(1, &input, "new.bin", false).unwrap();
        let ids = opened.list(1).unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(opened.read_file(ids.iter().find(|i| i.name == "new.bin").unwrap().id).unwrap(), b"new chunked format data");
        opened.export_one(1, "legacy.txt", &root, "legacy-output.txt", false).unwrap();
        assert_eq!(std::fs::read(root.join("legacy-output.txt")).unwrap(), b"original 2026 format");
        opened.verify_integrity().unwrap();
        drop(opened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "Run explicitly for a >512 MiB streaming integration check"]
    fn streams_550_mib_file_without_the_previous_batch_limit() {
        let root = std::env::temp_dir().join(format!("ndr-stream-550-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(root.join("out")).unwrap();
        let input = root.join("source.bin");
        let mut file = std::fs::File::create(&input).unwrap();
        let size = 550 * 1024 * 1024 + 13;
        file.set_len(size).unwrap();
        use std::io::{Seek, SeekFrom};
        file.write_all(b"FIRST-550").unwrap();
        file.seek(SeekFrom::End(-8)).unwrap();
        file.write_all(b"LAST-550").unwrap();
        drop(file);
        let mut source = Vault::create(&root.join("first.ndr"), "long password 123456".into()).unwrap();
        let mut target = Vault::create(&root.join("second.ndr"), "other password 123456".into()).unwrap();
        source.import_one(1, &input, "large.bin", false).unwrap();
        assert_eq!(source.list(1).unwrap()[0].size as u64, size);
        target.copy_from_vault_item(&mut source, 1, "large.bin", 1, "copied.bin", false).unwrap();
        assert_eq!(target.list(1).unwrap()[0].size as u64, size);
        target.export_one(1, "copied.bin", &root.join("out"), "roundtrip.bin", false).unwrap();
        let mut result = std::fs::File::open(root.join("out/roundtrip.bin")).unwrap();
        assert_eq!(result.metadata().unwrap().len(), size);
        let mut first = [0u8; 9];
        result.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"FIRST-550");
        result.seek(SeekFrom::End(-8)).unwrap();
        let mut last = [0u8; 8];
        result.read_exact(&mut last).unwrap();
        assert_eq!(&last, b"LAST-550");
        drop(result);
        source.verify_integrity().unwrap();
        target.verify_integrity().unwrap();
        drop((source,target));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn large_encrypted_files_stream_through_import_export_and_cross_container_copy() {
        let folder = std::env::temp_dir().join(format!("ndr-stream-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(folder.join("out")).unwrap();
        let input = folder.join("input.bin");
        let mut writer = std::fs::File::create(&input).unwrap();
        writer.set_len(80 * 1024 * 1024 + 7).unwrap();
        use std::io::{Seek, SeekFrom};
        writer.write_all(b"START").unwrap();
        writer.seek(SeekFrom::End(-7)).unwrap();
        writer.write_all(b"THE-END").unwrap();
        drop(writer);
        let mut a = Vault::create(&folder.join("one.ndr"), "long password alpha".into()).unwrap();
        let mut b = Vault::create(&folder.join("two.ndr"), "long password beta".into()).unwrap();
        assert_eq!(a.import_one(1, &input, "large.bin", false).unwrap(), 1);
        let entry = &a.list(1).unwrap()[0];
        assert_eq!(entry.size, 80 * 1024 * 1024 + 7);
        assert!(a.read_file(entry.id).is_err(), "large files must not be loaded for text editing");
        assert_eq!(b.copy_from_vault_item(&mut a, 1, "large.bin", 1, "copied.bin", false).unwrap(), 1);
        assert_eq!(b.list(1).unwrap()[0].size, entry.size);
        b.export_one(1, "copied.bin", &folder.join("out"), "out.bin", false).unwrap();
        let output = std::fs::read(folder.join("out/out.bin")).unwrap();
        assert_eq!(output.len() as i64, entry.size);
        assert_eq!(&output[..5], b"START");
        assert_eq!(&output[output.len()-7..], b"THE-END");
        drop(output);
        a.verify_integrity().unwrap();
        b.verify_integrity().unwrap();
        drop((a,b));
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn encrypted_conflict_replacement_is_atomic_and_export_restores_on_failure() {
        let root = std::env::temp_dir().join(format!("ndr-conflict-recovery-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(root.join("local")).unwrap();
        let local = root.join("local");
        let mut source = Vault::create(&root.join("source.ndr"), "test source password".into()).unwrap();
        let mut target = Vault::create(&root.join("target.ndr"), "test target password".into()).unwrap();
        source.add_file(1, "same.txt", b"updated content").unwrap();
        target.add_file(1, "same.txt", b"old content").unwrap();
        assert!(target.copy_from_vault_item(&mut source, 1, "same.txt", 1, "same.txt", false).is_err());
        let id = target.list(1).unwrap()[0].id;
        assert_eq!(target.read_file(id).unwrap(), b"old content");
        target.copy_from_vault_item(&mut source, 1, "same.txt", 1, "same.txt", true).unwrap();
        let id = target.list(1).unwrap()[0].id;
        assert_eq!(target.read_file(id).unwrap(), b"updated content");
        target.copy_from_vault_item(&mut source, 1, "same.txt", 1, "renamed.txt", false).unwrap();
        assert_eq!(target.list(1).unwrap().len(), 2);

        std::fs::write(local.join("same.txt"), b"filesystem version").unwrap();
        assert!(target.import_one(1, &local.join("same.txt"), "same.txt", false).is_err());
        target.import_one(1, &local.join("same.txt"), "same.txt", true).unwrap();
        let id = target.list(1).unwrap().into_iter().find(|i| i.name == "same.txt").unwrap().id;
        assert_eq!(target.read_file(id).unwrap(), b"filesystem version");
        assert_eq!(std::fs::read(local.join("same.txt")).unwrap(), b"filesystem version");

        std::fs::write(local.join("renamed.txt"), b"old plaintext target").unwrap();
        assert!(target.export_one(1, "renamed.txt", &local, "renamed.txt", false).is_err());
        assert_eq!(std::fs::read(local.join("renamed.txt")).unwrap(), b"old plaintext target");
        target.export_one(1, "renamed.txt", &local, "renamed.txt", true).unwrap();
        assert_eq!(std::fs::read(local.join("renamed.txt")).unwrap(), b"updated content");
        assert!(!std::fs::read_dir(&local).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with(".ndr-recovery-")));
        source.verify_integrity().unwrap();
        target.verify_integrity().unwrap();
        drop((source, target));
        std::fs::remove_dir_all(root).unwrap();
    }
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
    fn cross_container_copy_is_encrypted_recursive_atomic_and_keeps_source() {
        let root = std::env::temp_dir().join(format!("ndr-cross-vault-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let mut source = Vault::create(&root.join("source.ndr"), "source password 123456".into()).unwrap();
        let mut target = Vault::create(&root.join("target.ndr"), "target password 654321".into()).unwrap();
        let folder = source.add_folder(1, "documents").unwrap();
        let subfolder = source.add_folder(folder, "nested").unwrap();
        source.add_file(subfolder, "private.dat", &[0, 255, 48, 17, 99]).unwrap();
        source.add_file(1, "other.txt", b"second file").unwrap();
        let names = ["documents".to_string(), "other.txt".to_string()];
        assert_eq!(target.copy_from_vault(&mut source, 1, &names, 1).unwrap(), 4);
        assert_eq!(source.list(1).unwrap().len(), 2, "copy must preserve source");
        let copied = target.list(1).unwrap();
        let folder_id = copied.iter().find(|i| i.name == "documents").unwrap().id;
        let nested_id = target.list(folder_id).unwrap()[0].id;
        let file_id = target.list(nested_id).unwrap()[0].id;
        assert_eq!(target.read_file(file_id).unwrap(), [0, 255, 48, 17, 99]);
        assert!(target.copy_from_vault(&mut source, 1, &names, 1).is_err(), "conflict must abort whole batch");
        assert_eq!(target.list(1).unwrap().len(), 2);
        source.add_file(1, "new-only.txt", b"must rollback").unwrap();
        assert!(target.copy_from_vault(&mut source, 1, &["new-only.txt".into(), "other.txt".into()], 1).is_err());
        assert_eq!(target.list(1).unwrap().len(), 2, "a late destination collision rolls back earlier inserts");
        let ids = source.list(1).unwrap().into_iter().map(|i| i.id).collect::<Vec<_>>();
        source.remove_many(&ids).unwrap();
        assert!(source.list(1).unwrap().is_empty(), "move deletes source only after committed copy");
        assert_eq!(target.list(1).unwrap().len(), 2);
        source.verify_integrity().unwrap();
        target.verify_integrity().unwrap();
        assert!(std::fs::read(root.join("target.ndr")).unwrap().windows(12)
            .all(|bytes| bytes != b"private.dat\0"), "filename must not be a plaintext database string");
        drop((source, target));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn encrypted_internal_copy_move_preserves_tree_and_rolls_back() {
        let root = std::env::temp_dir().join(format!("ndr-vault-internal-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&root).unwrap();
        let mut vault = Vault::create(&root.join("safe.ndr"), "long passphrase 123456".into()).unwrap();
        let source = vault.add_folder(1, "source").unwrap();
        let destination = vault.add_folder(1, "destination").unwrap();
        let child = vault.add_folder(source, "nested").unwrap();
        vault.add_file(child, "secret.txt", b"secret bytes").unwrap();
        assert_eq!(vault.resolve_folder("destination").unwrap(), destination);
        assert_eq!(vault.transfer_items(1, &["source".into()], destination, true).unwrap(), 1);
        assert!(vault.list(1).unwrap().iter().all(|item| item.name != "source"));
        let moved = vault.list(destination).unwrap()[0].id;
        assert!(vault.transfer_items(destination, &["source".into()], child, true).is_err(),
            "moving to descendant must fail");
        let copied = vault.transfer_items(destination, &["source".into()], 1, false).unwrap();
        assert_eq!(copied, 3);
        let copied_folder = vault.list(1).unwrap().into_iter().find(|i| i.name == "source").unwrap();
        let copied_nested = vault.list(copied_folder.id).unwrap()[0].id;
        let copied_file = vault.list(copied_nested).unwrap()[0].id;
        assert_eq!(vault.read_file(copied_file).unwrap(), b"secret bytes");
        assert_eq!(vault.list(destination).unwrap()[0].id, moved);
        assert!(vault.transfer_items(destination, &["source".into(), "missing".into()], 1, false).is_err());
        assert_eq!(vault.list(1).unwrap().len(), 2);
        vault.verify_integrity().unwrap();
        drop(vault);
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
