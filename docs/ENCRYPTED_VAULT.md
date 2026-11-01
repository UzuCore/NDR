# NDR encrypted virtual folder (.ndr) — security and implementation

## Dependency evaluation (November 2026)

The .ndr file is a single password-encrypted SQLCipher database. The UI treats
it as an encrypted virtual folder, not a ZIP archive or an OS drive mount.

| Project | Strength | Reason for selecting/rejecting |
| --- | --- | --- |
| SQLCipher (Zetetic) | AES-256-CBC, per-page HMAC-SHA512, PBKDF2-HMAC-SHA512 (256,000 iterations by default), page integrity checking | **Selected**: strong, maintained, single-file database, atomic writes |
| rusqlite | Maintained Rust wrapper; bundled SQLCipher/OpenSSL option | **Selected**: avoids requiring a locally installed SQLCipher DLL |
| zeroize | Wipes the password entry buffer on drop and explicit session lock | **Selected**; memory allocations outside its control remain a risk |
| Cryptomator | AES-GCM file encryption, AES-SIV filenames | Not selected: storage consists of many ciphertext files |
| VeraCrypt | Mature encrypted container/volume | Not selected: normal use requires filesystem mounting, which the in-process UI does not need |
| AnyFS-SQLite | Proposed SQLite-backed virtual file storage | Not selected: published crate explicitly marked non-functional prerelease |
| rencfs | Rust/FUSE encrypted filesystem | Not selected: project warns unaudited, under development, not for sensitive data |

References:
- https://www.zetetic.net/sqlcipher/design/
- https://www.zetetic.net/sqlcipher/sqlcipher-api/
- https://github.com/sqlcipher/sqlcipher
- https://github.com/rusqlite/rusqlite
- https://docs.cryptomator.org/security/architecture/
- https://www.sqlite.org/tempfiles.html

Dependencies are pinned in Cargo.lock. The initial tested version of
`sqlcipher-src` is SQLCipher 4.19.0 / SQLite 3.53.4. This must be reviewed for
future upstream security patches. SQLCipher 5 beta is not used because its
format is intentionally incompatible with SQLCipher 4.

## Password and authentication

- 16-character *minimum*, but **length alone is not entropy**; prefer unique
  randomly generated passwords/passphrases. SQLCipher 4 defaults to PBKDF2-
  HMAC-SHA512 with 256,000 iterations and a unique salt per container.
- There is **no master key, backdoor, recovery password, cleartext key file,
  unattended unlock, or key stored in settings.json**. Forgetting the password
  means losing access to the content. NDR cannot guarantee mathematical
  impossibility of password guessing or resist a compromised operating system.
- The library fails closed unless its `cipher_status` confirms active
  SQLCipher encryption. Opening requires the correct password, schema version,
  and encrypted metadata signature. Page MAC verification can be requested.
- The container file has an encrypted header rather than the plaintext
  `SQLite format 3` header. Filenames and directory structure are database
  columns encrypted along with the contents.

## Session policy

Configured in Options > NDR > Encrypted Folder:

- Default policy: lock immediately when the encrypted folder browser closes.
- Default period: 15 minutes (used only for inactivity/timed policies).
- Always lock on NDR exit: enabled; minimize lock: disabled by default.
- Expiration or explicit lock drops the SQLCipher connection and destroys
  the displayed plaintext editor buffer. **Every subsequent unlock requires
  the password again**, regardless of earlier session settings.
- Optional modes: lock after inactivity, lock after elapsed time, or on NDR
  exit. The 15-minute value is configurable when one of the timed modes is
  selected.

## Durability and plaintext exposure

- SQLite uses DELETE rollback journaling with FULL synchronous transactions,
  not WAL, to avoid persistent `-wal`/`-shm` companions. A temporary **encrypted**
  rollback journal may exist during writes or after a crash until recovery.
  Disabling journaling for cosmetic single-file behavior risks corruption.
- SQLCipher works on encrypted database pages. It is not a substitute for
  protection of memory, Windows pagefile, crash dumps, malicious keyloggers,
  user-installed extensions, local backups, or malware acting as the user.
- Importing a pre-existing plaintext file into the vault **does not delete the
  original**. Exporting deliberately writes plaintext to the specified path.
  External editors are intentionally not launched automatically because they
  commonly leave sensitive plaintext on disk.
- The first UI integration supports creating vaults, opening via double-click,
  browsing nested folders in an NDR vault window, creating folders/files,
  importing/exporting explicitly, renaming/deleting and editing UTF-8 text in
  memory. It does **not** yet integrate every native pane file operation,
  shell drag-and-drop, multi-gigabyte streaming, or password changes.
- Imported file limit is 64 MiB and in-memory text editing limit is 1 MiB.

## Release readiness

**Experimental feature. Not independently security-audited.** Before treating
it as storage for irreplaceable secrets, test corruption recovery, power-loss
simulation, concurrent access, backup/restore, SQLCipher upgrades and password
strength UX. Preserve separate encrypted backups. Do not deploy as a final
security product based only on passing unit tests.
