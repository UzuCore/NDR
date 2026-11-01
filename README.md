# NDR — native Rust file manager for Windows

NDR is a dual-pane Windows file manager, developed in Rust. The project builds
with the stable **x86_64-pc-windows-msvc** Rust toolchain.

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

NDR supports `.ndr` SQLCipher-based encrypted containers. Use **File → New
encrypted folder (.ndr)** to create one; double-click an `.ndr` file in NDR
to open the password-protected virtual folder browser. Configure session
locking in **Options → NDR → Encrypted Folder**. A password is required after
every session expiration. See [security limits and implementation details](docs/ENCRYPTED_VAULT.md)
before trusting the feature with sensitive information. No separate drive
mount is required; native pane operations are not all integrated yet.

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

The original menu metadata in `docs/original-menus.txt` is also tracked; UI
regression tests compare the implemented menu items against this fixture.
