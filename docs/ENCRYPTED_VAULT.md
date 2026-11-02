# NDR encrypted folder format v3 - single-file AES-GCM filesystem

## November 2026 replacement

NDR no longer uses SQLCipher, SQLite, or a VeraCrypt volume for newly created .ndr containers. The SQLCipher v1 parser and legacy backward compatibility are intentionally removed at the user's request. Previous .ndr files do NOT open in this version. No user documents are automatically deleted; test assets discovered in ordinary install/work locations were empty.

Storage design is inspired by CoverFS's single-file, dynamically growing index and block layout (https://github.com/s-macke/CoverFS). NDR has its own on-disk format and relies on the maintained OpenSSL AES-256-GCM implementation, not CoverFS's legacy CBC implementation. It is not compatible with CoverFS, VeraCrypt, or Cryptomator.

## Storage and security

- Physical storage: exactly one .ndr file, growing on demand; no preallocated disk volume and no file-system mount driver.
- 128-byte fixed header: magic NDRV3FS!, 16-byte random PBKDF2 salt, KDF parameters, two generation-indexed metadata pointers protected by keyed HMAC-SHA256 (128-bit truncated MAC). Header fields and overall file length are not secret; names and data are encrypted.
- Password: >=12 characters for newly created containers, PBKDF2-HMAC-SHA256 with 600,000 iterations, 256-bit key. Longer random passwords are strongly recommended. No password or key stored in settings.
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
- Delete is a logical unlink, NOT secure physical erasure: encrypted older data blocks remain in the .ndr file until a separate compaction pass. The file does not shrink automatically; deleting data can leave dead encrypted space. This is a known limitation and requires compaction/space-reclamation engineering. Guaranteed physical erasure on SSD cannot be promised, even after compaction.
- The current architecture processes the 8 MiB chunk inside the egui repaint tick for some transfers; UI responsiveness may still degrade for large copy jobs. Future improvement: bounded background worker.

## UI / behavior

- Ctrl+K opens Create Encrypted Folder in the Folder menu immediately after New Folder. Normal new-folder Alt+K remains.
- Existing .ndr extension is displayed as an encrypted folder in NDR panes; unlocked panels have the pink border.
- Password on first unlock; same process-based or minute-based session policies; regular folder navigation, clipboard copy/paste, create, rename, export, delete.
- Explicit exports create plaintext files by user choice; ordinary editing that exports to temp can also create plaintext.
- Decrypting an old v1 SQLCipher file or disposable v2 container fails with a clear unsupported legacy format error. The user authorized discarding the earlier test format rather than migrating it.

## Known QA requirements

- Verify password failure; ciphertext and metadata corruption rejection; true GCM tag integrity; aborted stream rollback; 24 MiB transfer; cross-vault copy; recursive folders; conflict handling; deletion timing vs previous SQLCipher implementation; reopen after crash; restart NDR UI; ensure user settings unchanged.
- Always keep real external encrypted data out of automated deletion. Any benchmark file must be created inside a unique temporary test root.
