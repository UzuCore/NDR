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
  an open encrypted folder, preserving subfolder structure. Each top-level
  item is imported in its own SQLCipher transaction. A failed item rolls back
  its complete subtree, while earlier completed items remain. No plaintext
  staging files are created. Reparse points, symbolic links and junctions
  are rejected.
- F3 in the reverse direction exports selected items to the other local pane.
  It always requests confirmation because this writes **unencrypted plaintext**.
  Existing targets invoke NDR's normal duplicate-file dialog. When overwrite
  is explicitly approved, the old target is moved to a recovery name and
  restored if the new export fails. Skipping, renaming or cancelling never
  overwrites existing files silently.
- F4 between a regular pane and an encrypted pane moves selected items by
  copying each selected root successfully first and only then deleting that
  root's source.
  A failed source deletion retains the completed destination copy and reports
  the failure. Moving decrypted content into a regular pane requires explicit
  confirmation, and exports are plaintext. This is not a cross-filesystem
  atomic move.
- Copy to Folder / Move to Folder inside an unlocked vault accepts a destination
  path relative to the vault root, such as `documents/archive` or `/`. These
  actions use one SQLCipher transaction per top-level item; folder cycles,
  name conflicts and failed copies roll back that item.
- When each panel displays a **different unlocked** `.ndr` container, F3 copies
  selected files or complete folder trees directly between the two SQLCipher
  databases without writing plaintext to a temporary filesystem location.
  The destination database commits one transaction for each selected root,
  so a copy error cannot leave a partial folder tree. Earlier completed
  roots are preserved. F4 subsequently deletes each source after its copy.
  Transactions across two separate databases cannot be atomic together: if
  source deletion fails after destination commit, NDR deliberately retains
  both encrypted copies and reports the incomplete move. Existing target
  entries are replaced only after the user approves NDR's conflict dialog;
  deletion and reinsertion are atomic in the target SQLCipher transaction.
- Every encrypted transfer displays the normal-size 441 px NDR progress
  window and opens the original TfFileExists dialog on a duplicate name.
  While an unlocked `.ndr` container is displayed, its pane frame is pink
  (`#FF69B4`) instead of the normal local/archive frame color. The inactive
  pane uses the original 40% background / 60% frame blend, and leaving or
  locking the container immediately restores the ordinary skin border.
  On a fully successful encrypted copy or move, it also plays the same
  `complete.wav` completion sound as ordinary NDR file operations when
  `PlayCompletionSound` is enabled. Skipped, cancelled and failed transfers
  remain silent, and a streaming transfer plays once at the end rather
  than once for every 8 MiB chunk. A successful encrypted-item deletion
  also uses this shared completion cue.
  The dialog supports overwrite, skip, rename, cancel and apply-to-all.
  Overwrite-newer conservatively skips when vault timestamps cannot be
  compared; resume is unavailable. Cancel stops before the next selected
  root and retains already completed copies. For individual files, the progress
  window counts bytes, processing one 8 MiB chunk per UI frame; cancellation
  rolls back uncommitted SQLCipher chunks or removes an unfinished plaintext
  export. Recursive directory roots currently update progress when the entire
  root completes, and cannot be interrupted partway through a directory.
- Ctrl+V accepts regular files or whole folder trees from the filesystem. A
  cut/move clipboard is rejected: use copy instead. Shell drag-and-drop
  and external program opening remain unsupported for vault contents to avoid
  unsafe operations on synthetic filesystem paths.
- The former 64 MiB file limit and 512 MiB batch-size limit are removed from
  file transfers. New imported files use 8 MiB SQLCipher-encrypted chunks,
  so the entire plaintext is not buffered in memory. Root-tree operations
  retain their 10,000-entry and 64-nested-level safety bounds. Actual limits
  depend on free disk space, SQLite file-size limits and database journaling.
- Existing single-BLOB v1 files remain readable and exportable. Opening an
  older container creates the supplemental `item_chunks` table and a cleanup
  trigger without changing its encryption key or SQLCipher signature. New
  chunked files require an updated NDR to read; **older NDR versions do not
  understand the new chunk table**. Preserve backups before downgrading.
- Folder listing speed: NDR caches the byte count of each chunked file in
  `item_sizes` instead of calculating the sum of all its chunks on every
  directory navigation. Containers written before this optimization are
  migrated once on unlock; writes, file deletion and transaction rollback
  maintain the cache within the same SQLCipher transaction. The migration
  does not decrypt data into a plaintext temporary folder.
- After unlocking, individual file/folder navigation and encrypted transfer
  conflict checks use a case-insensitive indexed lookup, rather than reading
  the full directory. File transfer panes refresh their contents at most once
  per 64 completed files and again at job completion, instead of re-sorting
  the entire folder for each copied item. Imported files reuse an 8 MiB
  memory buffer, and a direct indexed query retrieves each encrypted chunk.
- Internal copy within the same `.ndr` uses bounded SQLCipher `INSERT ... SELECT`
  groups (up to four 8 MiB rows at a time) for chunked data (and an internal
  SQL BLOB copy for legacy entries) without round-tripping plaintext file
  contents through the Rust application. SQLCipher
  continues to encrypt pages, use authenticated storage, and commit changes
  atomically. A bounded 16 MiB page cache per unlocked container avoids
  re-reading database pages during common operations.
- Explicit plaintext COPY exports use the Windows write cache (like regular
  NDR file copies) to avoid an unnecessary forced disk flush. MOVE exports
  still force a durable flush before deleting the encrypted original. For a
  COPY, abrupt power loss may leave the newest exported bytes unwritten,
  though the original encrypted item remains intact.
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
  F3 copy, F4 move, cross-container encrypted transfer, and transactional
  copy/move between folders inside a single container. It does **not** yet integrate every native pane
  operation, shell drag-and-drop, or password changes.
- The built-in memory-only text editor still supports UTF-8 files of at most
  1 MiB. The 64 MiB bound remains on internal in-memory APIs used for small
  edits; it is not a limit on encrypted import/copy/export operations.

## Dialog appearance and languages

- Opening an existing container uses the same centered, draggable title-row
  modal chrome and 444 px content width as New Folder / New Encrypted Folder.
  The password is focused automatically, and the dialog has aligned OK/Cancel
  buttons, Enter to submit, and Escape to close.
- The NDRVault section of all 21 bundled language files contains the new
  encrypted-folder strings, including transfer confirmations/errors, session
  configuration and the automatic folder-size option. Korean and English have
  reviewed full text; other languages use local translations for common dialog
  controls and English fallbacks for longer security-related messages that do
  not yet have verified translations.

## Release readiness

**Experimental feature. Not independently security-audited.** Before treating
it as storage for irreplaceable secrets, test corruption recovery, power-loss
simulation, concurrent access, backup/restore, SQLCipher upgrades and password
strength UX. Preserve separate encrypted backups. Do not deploy as a final
security product based only on passing unit tests.
