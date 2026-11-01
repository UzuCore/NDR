# Runs as a separate PowerShell process after NDR closes. The package is
# already downloaded and SHA-256 verified against GitHub Releases by NDR.
param(
    [Parameter(Mandatory=$true)][string]$Archive,
    [Parameter(Mandatory=$true)][string]$Destination,
    [Parameter(Mandatory=$true)][int]$ParentPid,
    [Parameter(Mandatory=$true)][string]$LogPath,
    [switch]$SkipRestart
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$work = Split-Path -Parent $Archive
$staging = Join-Path $work 'extracted'
$backup = Join-Path $work 'backup'
try {
    if (-not (Test-Path -LiteralPath $Archive -PathType Leaf)) { throw 'Verified update ZIP missing' }
    $root = [IO.Path]::GetFullPath($Destination).TrimEnd('\') + '\'
    if (-not (Test-Path -LiteralPath (Join-Path $root 'NDR.exe') -PathType Leaf)) { throw 'NDR installation missing' }
    if (Test-Path $staging) { throw 'Update staging folder already exists' }
    New-Item -ItemType Directory -Path $staging -Force | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        $total = [long]0
        $files = 0
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName.Replace('\', '/')
            if ($name.StartsWith('/') -or $name.Contains(':') -or
                $name -match '(^|/)\.\.(/|$)' -or $name -match '(^|/)\.(/|$)' -or
                $name.Contains('//') -or $name.Contains([char]0)) { throw "Invalid archive path: $name" }
            $parts = $name.TrimEnd('/').Split('/')
            if ($parts[0] -notin @('NDR.exe', 'Colors', 'Languages', 'Skins', 'Tools', 'complete.wav', 'THIRD-PARTY-NOTICES.txt')) {
                throw "Unexpected package item: $name"
            }
            if ($parts[0] -in @('NDR.exe','complete.wav','THIRD-PARTY-NOTICES.txt') -and $parts.Count -ne 1) {
                throw "Invalid package path: $name"
            }
            if ((($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000) {
                throw "Symbolic links are not accepted: $name"
            }
            if ($name.EndsWith('/')) { continue }
            if (-not $seen.Add($name)) { throw "Duplicate archive file: $name" }
            $files++
            $total += $entry.Length
            if ($files -gt 1500 -or $total -gt 512MB) { throw 'Update archive exceeds extraction safety limits' }
            $target = [IO.Path]::GetFullPath((Join-Path $staging $name))
            $safeRoot = [IO.Path]::GetFullPath($staging).TrimEnd('\') + '\'
            if (-not $target.StartsWith($safeRoot, [StringComparison]::OrdinalIgnoreCase)) { throw "Archive path escapes staging: $name" }
            New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $false)
        }
    } finally { $zip.Dispose() }
    foreach ($required in @('NDR.exe','Skins/black/conf.ini','Skins/classic/conf.ini','Languages/ko.lang','Tools/7zip/7za.exe')) {
        if (-not (Test-Path -LiteralPath (Join-Path $staging $required) -PathType Leaf)) { throw "Package missing $required" }
    }

    # The running executable is locked. Wait for only its originating process;
    # never kill any other NDR instance.
    $deadline = [DateTime]::UtcNow.AddMinutes(2)
    while (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'NDR did not exit before update deadline' }
        Start-Sleep -Milliseconds 500
    }
    New-Item -ItemType Directory -Path $backup -Force | Out-Null
    $copied = [Collections.Generic.List[string]]::new()
    try {
        foreach ($source in (Get-ChildItem $staging -Recurse -File)) {
            $relative = $source.FullName.Substring($staging.Length).TrimStart('\')
            $target = Join-Path $root $relative
            if (Test-Path -LiteralPath $target -PathType Leaf) {
                $old = Join-Path $backup $relative
                New-Item -ItemType Directory -Path (Split-Path -Parent $old) -Force | Out-Null
                Copy-Item -LiteralPath $target -Destination $old -Force
            }
            New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
            $copied.Add($relative)
            Copy-Item -LiteralPath $source.FullName -Destination $target -Force
        }
    } catch {
        foreach ($relative in $copied) {
            $target = Join-Path $root $relative
            $old = Join-Path $backup $relative
            if (Test-Path -LiteralPath $old -PathType Leaf) { Copy-Item -LiteralPath $old -Destination $target -Force }
            else { Remove-Item -LiteralPath $target -Force -ErrorAction SilentlyContinue }
        }
        throw
    }
    # User settings, toolbar customization, favorites and other non-package
    # files are deliberately left in place.
    "Update installed: $files files" | Set-Content -LiteralPath $LogPath -Encoding UTF8
    if (-not $SkipRestart) {
        Start-Process -FilePath (Join-Path $root 'NDR.exe') -WorkingDirectory $root
    }
    exit 0
} catch {
    "Update failed: $_" | Set-Content -LiteralPath $LogPath -Encoding UTF8
    exit 1
}
