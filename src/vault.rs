//! NDR's password-only SQLCipher encrypted virtual folder backend.
//! Uses SQLCipher 4 authenticated pages; never creates a plaintext staging directory.
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::{path::{Path, PathBuf}, time::{Duration, Instant}};
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
        if id == 1 { return Err("Cannot delete the container root".into()); }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let deleted = transaction.execute("WITH RECURSIVE descendants(id) AS (SELECT id FROM items WHERE id=?1 UNION ALL SELECT i.id FROM items i JOIN descendants d ON i.parent=d.id) DELETE FROM items WHERE id IN (SELECT id FROM descendants)", [id])
            .map_err(|e| e.to_string())?;
        if deleted == 0 { return Err("Item not found".into()); }
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

#[cfg(test)]
mod tests {
    use super::*;
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
