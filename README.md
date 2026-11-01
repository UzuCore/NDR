# NDR — native Rust file manager for Windows

NDR is a dual-pane Windows file manager, developed in Rust. The project builds
with the stable **x86_64-pc-windows-msvc** Rust toolchain.

## Download a release

The latest automatic build is published under **[GitHub Releases](https://github.com/UzuCore/NDR/releases/latest)**.
Download `NDR-windows-x64.zip`, extract it, and run `NDR.exe`. Keep `Languages`,
`Skins`, `Colors`, `Tools`, and `complete.wav` next to the executable.
The separately offered `NDR.exe` can replace an executable in an existing
installation; it does **not** include the runtime resources by itself.

## GitHub Actions

The [Windows build and release workflow](.github/workflows/windows-release.yml)
automatically runs on every push to `main` and can also be started manually
using **Actions → Windows build and release → Run workflow**. Pull requests
are compiled and tested, but do not publish a release.

Each successful build runs `cargo test --release --locked` and
`cargo build --release --locked` on GitHub-hosted Windows, produces the complete
portable ZIP and a standalone EXE, checks their SHA-256 hashes, uploads
workflow artifacts, and publishes a GitHub Release named `build-<run number>`.
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
