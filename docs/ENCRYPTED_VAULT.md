# NDR encrypted folder format v3 - single-file AES-GCM filesystem

## November 2026 replacement

Current .ndr containers use the NDR v3 format. Files in the earlier v1 SQLCipher and v2 formats require a compatible older build. Older v3 indexes that lack Windows attributes and timestamps are read with defaults for those fields.

Storage design is inspired by CoverFS's single-file, dynamically growing index and block layout (https://github.com/s-macke/CoverFS). NDR has its own on-disk format and relies on the maintained OpenSSL AES-256-GCM implementation, not CoverFS's legacy CBC implementation. It is not compatible with CoverFS, VeraCrypt, or Cryptomator.

## Storage and security

- Each container is stored in one .ndr file that grows on demand. WinFsp provides the Windows drive view for unlocked containers in the default build.
- 128-byte fixed header: magic NDRV3FS!, 16-byte random PBKDF2 salt, KDF parameters, two generation-indexed metadata pointers protected by keyed HMAC-SHA256 (128-bit truncated MAC). Header fields and overall file length are not secret; names and data are encrypted.
- Password: >=6 characters for newly created containers (user-selectable security tradeoff), PBKDF2-HMAC-SHA256 with 600,000 iterations, 256-bit key. A short or predictable password can be guessed offline even when AES-256-GCM encryption is used. A unique random password of 16+ characters is strongly recommended. No password or key stored in settings.
- All file content is written in AES-256-GCM authenticated chunks up to 8 MiB, with independent random 96-bit nonces and 128-bit tags. AAD authenticates record kind, payload length, and physical block offset.
- Directory names, file sizes, block offsets, parent relationships and metadata are stored in a compact AES-256-GCM authenticated JSON index record. The whole index is loaded once on unlock for fast navigation.
- Windows file sharing rejects a second writer to an unlocked .ndr even across NDR processes, preventing stale concurrent metadata commits.
- The two header pointers are authenticated with keyed HMAC-SHA256. Both must validate; corrupted pointers now fail closed, trading some crash-recovery availability for protection against silent rollback to deleted content.
- On a mutation, new encrypted data blocks and a full encrypted metadata snapshot are appended. Data is flushed before flipping the generation-indexed metadata pointer, followed by sync; an altered/torn metadata pointer causes a clear failure instead of silently restoring a deleted prior state.
- This v3 format is experimental and has not been independently security-audited. Hardware/OS failures, malicious modifications, rollbacks, and backup workflows need more validation before use with real confidential data. Slot authentication prevents arbitrary changes and silent fallback, but cannot prevent replay of an entire previously authenticated container or saved valid header slots; no entirely self-contained file can provide guaranteed freshness against an attacker with rollback control.

## Performance characteristics

- Same-container copy shares immutable ciphertext block references (copy-on-write semantics); avoids copying or re-encrypting contents.
- Move, rename, delete update only the metadata index; no per-file zeroing or SQL transaction work.
- Import, export, cross-container copy must encrypt/decrypt actual bytes and cannot reach native raw copy speeds for large files.
- Deleting files unlinks them from the current index. Use **Compact Encrypted Folder** to reclaim space occupied by old records in the current file. SSD remnants, snapshots and backups can retain earlier bytes after compaction.
- The current architecture processes the 8 MiB chunk inside the egui repaint tick for some transfers, and manual compaction runs synchronously in the confirmation callback; UI responsiveness may degrade for large maintenance operations. Future improvement: bounded background worker with cancellable maintenance.

## UI / behavior

- Ctrl+K opens Create Encrypted Folder in the Folder menu immediately after New Folder. Normal new-folder Alt+K remains.
- Existing .ndr extension is displayed as an encrypted folder in NDR panes; unlocked panels have the pink border.
- Password on first unlock; same process-based or minute-based session policies; regular folder navigation, clipboard copy/paste, create, rename, export, delete.
- Double-click a file to open it from the WinFsp drive. NDR uses Notepad for recognized text extensions and the Windows default application for other files. The default build offers its signed, bundled WinFsp installer if the driver is missing. Windows requests administrator approval for installation.
- Save and close the file in the external application. WinFsp flush and close operations commit changes to the encrypted container. Windows applications can open the unlocked drive and use ordinary file operations. NDR checks for open handles or uncommitted edits before exiting.
- Session expiry or minimize locking can revoke new opens while allowing existing handles to finish saving. Close the external application before trying to exit NDR.
- NDR exposes plaintext through the virtual drive and writes plaintext files when explicitly requested to export. External applications and Windows may retain plaintext in backups, caches, thumbnails, recovery files or the pagefile.

## Known QA requirements

- Verify password failure; ciphertext and metadata corruption rejection; true GCM tag integrity; aborted stream rollback; 24 MiB transfer; cross-vault copy; recursive folders; conflict handling; deletion timing vs previous SQLCipher implementation; reopen after crash; restart NDR UI; ensure user settings unchanged.
- Always keep real external encrypted data out of automated deletion. Any benchmark file must be created inside a unique temporary test root.

## Encrypted-only storage compaction (r209)

- Use Folder > Compact Encrypted Folder while browsing inside an unlocked .ndr. Confirmation is required. This maintenance operation deliberately runs outside the fast copy/delete path because it may rewrite gigabytes of ciphertext.
- The implementation reads and authenticates every live data record, encrypts surviving records under fresh random AES-GCM nonces into a same-directory, encrypted `.ndr-compaction-*.tmp` file, and preserves shared immutable block references instead of multiplying internal copies.
- The original container is never edited or truncated during staging. Every rebuilt record and the new authenticated metadata index are verified before atomic same-volume replacement. On Windows, a named semaphore holds the writer lease across the handle swap, and the staging file is kept open with deny-write sharing until the rename.
- Windows DACL permissions are copied from the original container to the staging file; failure to preserve the ACL prevents replacement. A temporary stage file is encrypted, never plaintext. A clean operation leaves exactly one .ndr file and no staging file. A process crash during staging can leave an encrypted `.tmp` file; manual clean-up may be required.
- Error-injection tests prove that a failed rewrite before replacement leaves the original byte-for-byte intact and cleans temporary files. Reopen, wrong-password, restored in-container shared copies, copied file content and reduced container size are verified.
- **This is not a secure physical erase.** Old sectors, SSD wear-leveling remnants, OS snapshots, hard-linked copies, backup media, and old valid encrypted containers can survive. If an attacker retains an old authenticated copy, no self-contained format can prevent its replay without trusted external freshness state. To retire old secrets, ensure older backups are removed according to policy and rotate credentials and keys in a separately designed workflow.
- The in-process decrypted directory index and transaction snapshots now zeroize filename allocations on drop. New container creation zeroizes the password string, including rejected too-short passwords.
