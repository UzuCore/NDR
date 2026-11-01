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

Each successful build runs `cargo test --release --locked` and
`cargo build --release --locked` on GitHub-hosted Windows, produces the complete
portable ZIP, verifies the ZIP and embedded EXE SHA-256 hashes, uploads
workflow artifacts, and publishes **only the ZIP** to a GitHub Release named `build-<run number>`.
Re-running a workflow updates the existing release assets instead of creating
a duplicate. The workflow uses GitHub's built-in `GITHUB_TOKEN` with
`contents: write`; no personal access token needs to be configured.

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
