# Assemble a portable GitHub Actions release without touching the user's local
# D:\NDRworks\release folder or invoking the interactive desktop GUI smoke test.
# Tests and the optimized executable are built by the workflow before calling
# this script. All temporary files are confined to target/ci-release.
param([string]$OutputDir = 'target/ci-release')
$ErrorActionPreference = 'Stop'

$project = Split-Path -Parent $PSScriptRoot
$target = [IO.Path]::GetFullPath((Join-Path $project 'target'))
$output = [IO.Path]::GetFullPath((Join-Path $project $OutputDir))
$comparison = [StringComparison]::OrdinalIgnoreCase
if (-not $output.StartsWith($target.TrimEnd('\') + '\', $comparison)) {
    throw "CI output must be a subdirectory of target/: $output"
}
$binary = Join-Path $project 'target/release/ndr.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw "Optimized Windows executable missing: $binary"
}
$newestSource = Get-ChildItem (Join-Path $project 'src') -Recurse -File |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($newestSource.LastWriteTime -gt (Get-Item -LiteralPath $binary).LastWriteTime) {
    throw "NDR.exe predates source: $($newestSource.FullName)"
}

$required = @(
    'Resources/Colors/black.col',
    'Resources/Languages/ko.lang',
    'Resources/Languages/en.lang',
    'Resources/Skins/black/conf.ini',
    'Resources/Skins/classic/conf.ini',
    'Resources/Tools/7zip/7za.exe',
    'Resources/Tools/7zip/Full/7z.exe',
    'Resources/Tools/7zip/Full/7z.dll',
    'Resources/complete.wav',
    'THIRD-PARTY-NOTICES.txt'
)
foreach ($file in $required) {
    $path = Join-Path $project $file
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required redistributable file missing: $file"
    }
}

if (Test-Path -LiteralPath $output) {
    Remove-Item -LiteralPath $output -Recurse -Force
}
$stage = Join-Path $output '_stage'
New-Item -ItemType Directory -Path $stage -Force | Out-Null

Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'NDR.exe')
foreach ($folder in @('Colors', 'Languages', 'Skins', 'Tools')) {
    Copy-Item -LiteralPath (Join-Path $project "Resources/$folder") `
        -Destination (Join-Path $stage $folder) -Recurse
}
Copy-Item -LiteralPath (Join-Path $project 'Resources/complete.wav') `
    -Destination (Join-Path $stage 'complete.wav')
Copy-Item -LiteralPath (Join-Path $project 'THIRD-PARTY-NOTICES.txt') `
    -Destination (Join-Path $stage 'THIRD-PARTY-NOTICES.txt')

$stagedFiles = @(Get-ChildItem -LiteralPath $stage -File -Recurse)
if ($stagedFiles.Count -lt 80) {
    throw "Runtime package is incomplete: only $($stagedFiles.Count) files"
}

$zipPath = Join-Path $output 'NDR-windows-x64.zip'
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zipPath -CompressionLevel Optimal
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($zipPath)
try {
    $entries = @($zip.Entries | Where-Object { -not $_.FullName.EndsWith('/') })
    if ($entries.Count -ne $stagedFiles.Count) {
        throw "ZIP file count mismatch: zip=$($entries.Count) staged=$($stagedFiles.Count)"
    }
    foreach ($item in @('NDR.exe', 'Languages/ko.lang', 'Skins/black/conf.ini',
                       'Tools/7zip/7za.exe', 'Tools/7zip/Full/7z.exe',
                       'Tools/7zip/Full/7z.dll', 'THIRD-PARTY-NOTICES.txt')) {
        # Windows PowerShell Compress-Archive writes backslash entry names.
        # Normalize separators when verifying cross-platform release payloads.
        if (-not ($entries | Where-Object { $_.FullName.Replace('\', '/') -eq $item })) {
            throw "Missing required file in ZIP: $item"
        }
    }
    $archiveExecutable = $entries | Where-Object { $_.FullName -eq 'NDR.exe' } | Select-Object -First 1
    $stream = $archiveExecutable.Open()
    try {
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            $zipExeHash = ([BitConverter]::ToString($sha.ComputeHash($stream)) -replace '-', '').ToLowerInvariant()
        } finally { $sha.Dispose() }
    } finally { $stream.Dispose() }
} finally { $zip.Dispose() }

$exeHash = (Get-FileHash -LiteralPath (Join-Path $stage 'NDR.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
if ($zipExeHash -ne $exeHash) {
    throw 'ZIP NDR.exe SHA-256 differs from the staged EXE'
}
$zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
@("$zipHash  NDR-windows-x64.zip") |
    Set-Content -LiteralPath (Join-Path $output 'SHA256SUMS.txt') -Encoding Ascii

$version = [regex]::Match((Get-Content -LiteralPath (Join-Path $project 'Cargo.toml') -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { throw 'Could not parse Cargo.toml version' }
$commit = (& git -C $project rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or -not $commit) { throw 'Could not obtain Git commit' }
$rust = (& rustc --version).Trim()
if ($LASTEXITCODE -ne 0 -or -not $rust) { throw 'Could not obtain Rust version' }
$manifest = [ordered]@{
    version = $version
    commit = $commit
    target = 'x86_64-pc-windows-msvc'
    rustc = $rust
    utc = [DateTime]::UtcNow.ToString('o')
    filesInZip = $entries.Count
    zip = [ordered]@{ filename = 'NDR-windows-x64.zip'; bytes = (Get-Item $zipPath).Length; sha256 = $zipHash }
    exe = [ordered]@{ filename = 'NDR.exe (inside ZIP)'; bytes = (Get-Item (Join-Path $stage 'NDR.exe')).Length; sha256 = $exeHash }
}
$manifest | ConvertTo-Json -Depth 4 |
    Set-Content -LiteralPath (Join-Path $output 'build-manifest.json') -Encoding UTF8

@"
# NDR Windows x64 automatic build

This release was built automatically from commit ``$commit`` using ``$rust``.

**Installation:** download and extract the **entire NDR-windows-x64.zip** to a folder, then run **NDR.exe** from inside that folder. Keep the included ``Colors``, ``Languages``, ``Skins``, ``Tools`` and ``complete.wav`` beside the executable. The executable is not distributed separately.

Build: ``$env:GITHUB_RUN_NUMBER`` | Cargo version: ``$version`` | Target: ``x86_64-pc-windows-msvc``

- Complete portable package: **NDR-windows-x64.zip** ($($entries.Count) files)
- Verify package integrity using the SHA-256 checksum recorded by the workflow
- Original license notices and bundled 7zip+ license files are inside the ZIP.

The workflow runs the full Rust release test suite before publishing. Optional user-owned archive/ISO fixtures are not included in public CI.
"@ | Set-Content -LiteralPath (Join-Path $output 'RELEASE-NOTES.md') -Encoding UTF8

Remove-Item -LiteralPath $stage -Recurse -Force
Write-Host "CI_PACKAGE_OK files=$($entries.Count)"
Write-Host "NDR_EXE_SHA256 $exeHash"
Write-Host "NDR_ZIP_SHA256 $zipHash"
Write-Host "CI_PACKAGE_OUTPUT $output"
