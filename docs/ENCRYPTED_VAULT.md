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

- 6-character *minimum*, but **length alone is not entropy**; prefer unique
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

## Dinosaur names for new encrypted folders

The new-folder field starts with one randomly proposed dinosaur genus name,
selected from 1,605 distinct non-avian genera and 179 curated Korean display
names. Korean names are favoured, while the full scientific-name pool remains
available. The proposed name is fully selected on opening, so typing replaces
it immediately. Existing `.ndr` files are skipped. The extension is added
silently on disk; manual user names are always permitted.

The scientific names are taken from the Paleobiology Database's valid-genus
dataset as presented by https://dinopages.com/dinosaurs/ (retrieved on
2026-11-02, 1,606 records, 1,605 distinct genus names). DinoPages documents
the dataset and filtering method at https://dinopages.com/about/. Original
taxonomy data is CC BY 4.0, credited to the Paleobiology Database
(https://paleobiodb.org/). Korean names are curated local transliterations;
they are not the source database's Korean taxonomic names.

The suggestion is a convenience feature, **not a security source of entropy**:
the name generator is independent from SQLCipher keys and passwords.

## In-pane folder behavior (2026-11-02)

- The encrypted container is physically one `.ndr` SQLCipher file, but NDR's
  ordinary file panels present it with a folder icon, a folder type and a name
  without the `.ndr` extension. Windows Explorer still sees an `.ndr` file.
- After creation, NDR enters the new folder immediately with the already
  authenticated session. Opening an existing one asks for its password once,
  then the regular file panel displays its encrypted directory contents.
- Double-click subfolders to enter. The `..` entry returns to the parent;
  leaving the container closes the SQLCipher session. File names and content
  are not extracted to an unencrypted working directory.
- Supported in-pane operations: create internal folders/files, rename, atomic
  multi-item delete, paste local files and full directory trees directly into
  SQLCipher, and edit UTF-8 text files up to 1 MiB in a memory-only editor.
- F3 (Copy to Other Pane) copies selected regular files/directories from the other local pane into
  an open encrypted folder, preserving subfolder structure. Import runs in a
  single SQLCipher transaction: a conflict, invalid name or failed read rolls
  back the entire batch. No plaintext staging files are created. Reparse
  points, symbolic links and junctions are rejected.
- F3 in the reverse direction exports selected items to the other local pane.
  It always requests confirmation because this writes **unencrypted plaintext**.
  Export never overwrites existing targets; on failure it attempts to remove
  only the entries created by that export, preserving prior user files.
- Ctrl+V accepts regular files or whole folder trees from the filesystem. A
  cut/move clipboard is rejected: use copy instead. F4 move, shell drag-and-drop
  and external program opening remain unsupported for vault contents to avoid
  unsafe operations on synthetic filesystem paths.
- The per-file size limit is 64 MiB; an import/export batch is limited to
  512 MiB, 10,000 entries and 64 nested levels. These synchronous operations
  are not intended for huge datasets or external editing workflows.
- Navigating away or switching tabs clears decrypted pane text, filenames and
  editor buffers, while the authenticated SQLCipher connection is retained
  only in process memory until the selected session deadline.
- Expiration, NDR exit or the optional minimize lock drops the connection and
  all retained authentication. No password is saved to settings or disk.

## Session policy

Configured in Options > NDR > Encrypted Folder:

- **Until NDR exits:** one authentication per container is valid throughout
  the current NDR process, including after leaving the folder and returning.
- **For a specified duration:** one authentication is valid for a fixed number
  of minutes measured from login. The default is **10 minutes**, editable as
  an integer between 1 and 1440 minutes. Reopening does not reset the timer.
- The default is the timed 10-minute mode. Existing `close`/`inactive` policies
  are migrated to elapsed-time behavior, instead of immediately locking on
  folder leave. Expired sessions always ask for the password again.
- Optional minimize lock is disabled by default, but when enabled it also
  invalidates sessions retained from previously closed folders.

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
- The pane integration now supports folder-tree import/export, atomic deletion,
  and F3 panel-to-panel copy. It does **not** yet integrate every native pane
  operation, shell drag-and-drop, multi-gigabyte streaming, or password changes.
- Imported file limit is 64 MiB and in-memory text editing limit is 1 MiB.

## Release readiness

**Experimental feature. Not independently security-audited.** Before treating
it as storage for irreplaceable secrets, test corruption recovery, power-loss
simulation, concurrent access, backup/restore, SQLCipher upgrades and password
strength UX. Preserve separate encrypted backups. Do not deploy as a final
security product based only on passing unit tests.
