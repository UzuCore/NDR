# NDR — native Rust file manager for Windows

NDR is a dual-pane Windows file manager, developed in Rust. The project builds
with the stable **x86_64-pc-windows-msvc** Rust toolchain.

## Local development release to both Windows installations

From the repository root, run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-and-deploy-local.ps1
```

This runs the release regression suite, builds the optimized executable, and
checks startup before deploying **the same verified `NDR.exe`** to
`D:\NDRworks\release\NDR.exe` and `D:\NDR\NDR.exe`. Both deployed copies
receive a startup check. Existing `settings.json` files and runtime resources
are preserved byte for byte, and previous executables are backed up under
`D:\NDRworks\release\old`. If either copy is running, deployment stops
without terminating it. After tests have *already been run on the exact same
sources*, the `-SkipTests` flag permits redeployment without rerunning them.

This is the **local developer workflow**; official GitHub release ZIPs continue
to use the complete packaged resources described below.

## Download a release

The latest automatic build is published under **[GitHub Releases](https://github.com/UzuCore/NDR/releases/latest)**.
Download `NDR-windows-x64.zip`, extract the **entire archive**, and run `NDR.exe`
from the extracted folder. `Languages`, `Skins`, `Colors`, `Tools`, and
`complete.wav` must remain beside the executable. A standalone `NDR.exe` is
**not distributed** because it cannot run properly without these resources.

## GitHub Actions

The [Windows build and release workflow](.github/workflows/windows-release.yml)
runs **only when manually started** using
**Actions → Windows build and release → Run workflow**. Git pushes and pull
requests do not start builds. Every successful manual build publishes a release.

## Updating NDR

NDR checks the latest **UzuCore/NDR GitHub Release** automatically at startup
when **Check for updates on startup** is enabled (the default). A new build
is indicated in the menu bar; **Help → Check for updates** also checks manually.
NDR offers to download the *complete* portable ZIP, verifies its size and
GitHub-published SHA-256 digest, and asks permission to restart and install it.
**Only one approval is required**: after accepting the update, NDR displays
actual download bytes and progress, verifies the ZIP, and launches the
installer automatically. A separate compact installer progress window remains
visible while NDR exits and the files are replaced; NDR then restarts without
another confirmation. Installation errors are recorded in `update.log`.
The updater waits until NDR has exited, replaces the packaged application
files, keeps `settings.json` and other personal files intact, then relaunches
NDR. The previous files are backed up in the update staging directory under
the user's temporary folder; `update.log` records installation failures.

Only a release with a higher `build-N` number is installed. Official Actions
builds embed their build number; local development builds have build number 0.
Existing custom HTTPS update manifest settings continue to work as before.

Each successful build runs `cargo test --release --locked` and
`cargo build --release --locked` on GitHub-hosted Windows, produces the complete
portable ZIP, verifies the ZIP and embedded EXE SHA-256 hashes, uploads
workflow artifacts, and publishes **only the ZIP** to a GitHub Release named `build-<run number>`.
Re-running a workflow updates the existing release assets instead of creating
a duplicate. The workflow uses GitHub's built-in `GITHUB_TOKEN` with
`contents: write`; no personal access token needs to be configured.

## Encrypted folders (experimental)

NDR stores encrypted folders in single-file AES-GCM v3 containers. Use
**Folder → Create Encrypted Folder** (Ctrl+K) to create one. Double-click a
`.ndr` file and enter its password to unlock it. Configure session locking in
**Options → NDR → Encrypted Folder**. The default Windows build uses WinFsp
to expose the unlocked folder to Windows applications, and offers the bundled
driver installer when needed. See the [encrypted folder documentation](docs/ENCRYPTED_VAULT.md)
for session and storage behavior.

When both panes display different unlocked `.ndr` containers, use **F3** to
copy and **F4** to move complete folder trees directly between the encrypted
containers, without a plaintext temporary folder. A cross-container move first
commits the destination and only then deletes the source. Failed source
deletion retains both encrypted copies. Encrypted transfers display the same
progress window and duplicate-name dialog as regular file operations, with
overwrite, skip, rename, cancel and apply-to-all choices. Each selected root
is committed in its own transaction; cancellation retains completed items.
Password and creation dialogs use the same centered,
movable chrome as other NDR dialogs. Core vault text is available in the
40 selectable locale catalogs; missing entries fall back to English.

Large encrypted files are copied through 8 MiB authenticated chunks
instead of loading entire files into RAM. The former 64 MiB file / 512 MiB
transfer limits no longer apply to the file-management transfer paths.
Individual file transfers show byte progress and allow cancellation between
chunks; recursive directory transfers still complete one folder tree at a
time. This version opens v3 containers. Legacy v1 and v2 containers require
the version that created them.

## Build locally

Install Visual Studio C++ Build Tools, stable Rust MSVC, and Strawberry Perl
(required by the bundled OpenSSL build). Then run in PowerShell:

```powershell
$env:NDR_RESOURCE_DIR = (Resolve-Path Resources).Path
cargo test --release --locked
cargo build --release --locked
./scripts/package-ci.ps1 -OutputDir target/ci-release
```

The package and manifest will be written under `target/ci-release`.
The `scripts/release.ps1` script is for **local deployment only** and must not
be run by GitHub Actions, because it manages local release directories.

Runtime assets and third-party license notices are retained under `Resources/`
and `THIRD-PARTY-NOTICES.txt`. The bundled 7zip+ upstream source reference is
in `Resources/Tools/7zip/SOURCE.txt`.

## Languages

Use the language menu or settings list to select one of 40 locale catalogs.
Portuguese (Brazil and Portugal) and Chinese (Simplified and Traditional) are
separate choices. Regional forms such as `en-US` and `ko_KR` are recognized
without forcing a change to a saved profile. Translation coverage varies:
main commands, copy preparation and update titles are localized in every
catalog; unfilled application messages use English. System errors and
third-party diagnostic text can retain their original wording.

Each locale is a UTF-8 file in `Resources/Languages`. Prefer named catalog keys
for UI captions and preserve format arguments when translating. See
`tools/ndr_locale_catalog.py` for checking or regenerating the remaining
legacy bilingual literal entries and merging new catalog keys.

## Bundled resources and compatibility

NDR's bundled themes, color presets, completion cue, and UI strings are
maintained as NDR resources. Its built-in toolbar and key bindings are defined
in Rust; they do not require a separate toolbar file. Install or update the
complete package so the bundled resources change with the executable.

NDR's keyboard and two-pane behavior was developed with reference to NexusFile
by Xiles. The compatibility layer still reads existing custom toolbar XML and
legacy shortcut or settings files. The NexusFile application is not bundled.
