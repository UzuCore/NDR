# Verified local release: build + tests + startup smoke + release and D:\NDR.
# Run: powershell -ExecutionPolicy Bypass -File scripts\build-and-deploy-local.ps1
# The running NDR process is NEVER terminated; close it normally before deploying.
param(
    [string]$Release = 'D:\NDRworks\release',
    [string]$Installed = 'D:\NDR',
    [switch]$SkipTests
)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$source = Join-Path $project 'target\release\ndr.exe'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$backupDir = Join-Path $Release 'old'
$reportDir = Join-Path (Split-Path -Parent $project) 'analysis'
New-Item -ItemType Directory -Path $backupDir, $reportDir -Force | Out-Null
Push-Location $project
try {
    $env:NDR_RESOURCE_DIR = $Release
    $strawberryPerl = 'D:\NDRworks\work\tools\strawberry-perl-5.42.3.1\perl\bin'
    if (Test-Path -LiteralPath $strawberryPerl) {
        $env:PATH = "$strawberryPerl;$env:PATH"
    }
    if (-not $SkipTests) {
        & cargo test --release --locked --quiet
        if ($LASTEXITCODE -ne 0) { throw 'Release regression tests failed; deployment cancelled' }
    }
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed; deployment cancelled' }
    if (-not (Test-Path -LiteralPath $source)) { throw "Missing release executable: $source" }
    $hash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash

    function Assert-NoRunningNdr {
        $running = @(Get-Process -Name NDR -ErrorAction SilentlyContinue | Where-Object {
            $_.Path -and ($_.Path -ieq (Join-Path $Release 'NDR.exe') -or $_.Path -ieq (Join-Path $Installed 'NDR.exe'))
        })
        if ($running.Count -gt 0) {
            throw "NDR is running. Close it normally and retry; processes: $(($running | ForEach-Object { $_.Id }) -join ', ')"
        }
    }
    function Assert-SameHash([string]$path, [string]$expected) {
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $expected) {
            throw "SHA256 mismatch: $path"
        }
    }
    function Invoke-NdrSmoke([string]$exe, [string]$label) {
        $report = Join-Path $reportDir "ndr-local-$label-$stamp.json"
        $smokeProfile = Join-Path $reportDir "ndr-local-smoke-profile-$stamp"
        $env:NDR_RESOURCE_DIR = $Release
        & $exe --profile $smokeProfile --smoke-seconds 1 --report $report
        if ($LASTEXITCODE -ne 0) { throw "Failed to start $label executable" }
        # NDR is a Windows GUI subsystem executable: PowerShell returns as
        # soon as the process starts, before its smoke report has been flushed.
        # Wait for the file, rather than treating normal asynchronous GUI
        # startup as a deployment failure.
        for ($attempt = 0; $attempt -lt 100 -and -not (Test-Path -LiteralPath $report); $attempt++) {
            Start-Sleep -Milliseconds 200
        }
        if (-not (Test-Path -LiteralPath $report)) { throw "Missing startup report: $report" }
        $data = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
        if ($data.frames -lt 1 -or $data.leftError -or $data.rightError -or $data.slowFrames -gt 0) {
            throw "Startup check failed: $label : $($data | ConvertTo-Json -Compress)"
        }
        Write-Output "$label startup PASS ($($data.frames) frames, $($data.slowFrames) slow)"
    }

    # Preflight BOTH destinations before the first replacement.
    Assert-NoRunningNdr
    foreach ($root in @($Release, $Installed)) {
        if (-not (Test-Path -LiteralPath (Join-Path $root 'NDR.exe'))) { throw "Missing NDR installation: $root" }
        $item = Get-Item -LiteralPath (Join-Path $root 'NDR.exe') -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Unexpected symlink: $($item.FullName)" }
        if (-not (Test-Path -LiteralPath (Join-Path $root 'settings.json'))) { throw "Settings missing: $root" }
    }
    Invoke-NdrSmoke $source 'built'
    Assert-NoRunningNdr

    foreach ($target in @(@{ Root = $Release; Name = 'release' }, @{ Root = $Installed; Name = 'installed' })) {
        $dest = Join-Path $target.Root 'NDR.exe'
        $backup = Join-Path $backupDir "NDR-$($target.Name)-before-$stamp.exe"
        $staged = Join-Path $target.Root "NDR-$stamp.new"
        if (Test-Path -LiteralPath $backup) { throw "Existing backup, refusing overwrite: $backup" }
        $oldHash = (Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash
        $settingsHash = (Get-FileHash -LiteralPath (Join-Path $target.Root 'settings.json') -Algorithm SHA256).Hash
        Copy-Item -LiteralPath $source -Destination $staged -ErrorAction Stop
        Assert-SameHash $staged $hash
        Assert-NoRunningNdr
        [IO.File]::Replace($staged, $dest, $backup)
        Assert-SameHash $backup $oldHash
        Assert-SameHash $dest $hash
        Assert-SameHash (Join-Path $target.Root 'settings.json') $settingsHash
        Invoke-NdrSmoke $dest $target.Name
        Write-Output "DEPLOYED: $dest (SHA256: $hash); old EXE: $backup"
    }
    Write-Output "LOCAL RELEASE COMPLETE: release and D:\NDR are identical ($hash)"
} finally {
    Pop-Location
}
