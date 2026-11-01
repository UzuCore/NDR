# NDR Rust 배포: 준비 → 격리 프로필 시작 확인 → zip 확인 → 배포 폴더 교체(이전 묶음은 old로) → 배치본 시작 확인.
# 사용: powershell -ExecutionPolicy Bypass -File scripts\release.ps1 -Series rust-20261027-r2 -Readme <README.md 경로>
param(
    [Parameter(Mandatory = $true)][string]$Series,
    [Parameter(Mandatory = $true)][string]$Readme,
    [string]$Release = 'D:\NDR\release'
)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $project 'target\release\ndr.exe'
if (-not (Test-Path $exe)) { throw "release build missing: $exe" }
# 소스보다 오래된 실행 파일은 배포하지 않는다(빌드와 배포를 동시에 돌리면 옛 실행 파일이 묶일 수 있다).
$newest = Get-ChildItem -LiteralPath (Join-Path $project 'src') -Recurse -File | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if ($newest.LastWriteTime -gt (Get-Item -LiteralPath $exe).LastWriteTime) { throw "release build is older than $($newest.FullName); run cargo build --release first" }
# 매니페스트에 쓸 rustc 판은 배포 폴더를 바꾸기 전에 읽는다(PATH에 없으면 아무것도 바꾸지 않고 멈춘다).
$rustcVersion = (& rustc --version)
# 배포 폴더의 NDR.exe가 실행 중이면 바꿀 수 없다. 아무것도 바꾸기 전에 멈춘다(r21: 중간에 멈춰 r20 묶음만 old로 옮겨졌던 일).
$releaseRoot = [IO.Path]::GetFullPath($Release).TrimEnd('\') + '\'
$running = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($releaseRoot, [StringComparison]::OrdinalIgnoreCase) })
if ($running.Count -gt 0) { throw "close the running release copy first: $(($running | ForEach-Object { "$($_.Id) $($_.Path)" }) -join '; ')" }
$stamp = $Series -replace '^rust-', ''
$stage = Join-Path $project "target\stage-$Series"
$work = Join-Path $project "target\smoke-$Series"
foreach ($dir in $stage, $work) { if (Test-Path $dir) { Remove-Item -LiteralPath $dir -Recurse -Force } }
New-Item -ItemType Directory -Path $stage, $work | Out-Null

# 1. 준비
Copy-Item -LiteralPath $exe -Destination (Join-Path $stage 'NDR.exe')
if (-not (Test-Path -LiteralPath (Join-Path $stage 'NDR.exe'))) { throw "staging copy of NDR.exe failed: $exe -> $stage [$((Get-ChildItem -Force -LiteralPath $stage | ForEach-Object Name) -join ',')]" }
foreach ($name in 'Languages', 'Skins', 'Tools', 'Colors') { Copy-Item -LiteralPath (Join-Path $project "Resources\$name") -Destination (Join-Path $stage $name) -Recurse }
# 원본 완료 소리(0x43d9fc: <프로그램 폴더>\complete.wav).
Copy-Item -LiteralPath (Join-Path $project 'Resources\complete.wav') -Destination (Join-Path $stage 'complete.wav')
$notices = @((Join-Path $project 'THIRD-PARTY-NOTICES.txt'), (Join-Path $Release 'THIRD-PARTY-NOTICES.txt')) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $notices) { throw 'THIRD-PARTY-NOTICES.txt not found' }
Copy-Item -LiteralPath $notices -Destination (Join-Path $stage 'THIRD-PARTY-NOTICES.txt')

function Invoke-Smoke([string]$ndr, [string]$label) {
    $root = Join-Path $work $label
    $left = Join-Path $root 'left'; $right = Join-Path $root 'right'
    New-Item -ItemType Directory -Path $left, $right -Force | Out-Null
    'smoke' | Set-Content -LiteralPath (Join-Path $left 'smoke.txt')
    $report = Join-Path $root 'report.json'
    $process = Start-Process -FilePath $ndr -ArgumentList @('--profile', (Join-Path $root 'profile'), '--left', $left, '--right', $right, '--language', 'ko', '--smoke-seconds', '4', '--report', $report) -PassThru -Wait
    if ($process.ExitCode -ne 0) { throw "$label smoke exit $($process.ExitCode)" }
    $data = Get-Content -LiteralPath $report -Raw | ConvertFrom-Json
    # 왼쪽 목록 = ".."(r106부터, 원본 GetList) + smoke.txt.
    if ($data.frames -lt 1 -or $data.leftCount -ne 2 -or $data.leftError -or $data.rightError -or $data.leftLoading) { throw "$label smoke report: $($data | ConvertTo-Json -Compress)" }
    # 시작 확인 창은 앞 창이 되면 안 된다(사용자 화면 맨 앞을 가리지 않기).
    if ($data.foreground -ne $false) { throw "$label smoke window became the foreground window: $($data | ConvertTo-Json -Compress)" }
    "$label smoke ok: frames=$($data.frames) leftCount=$($data.leftCount) foreground=$($data.foreground)"
}

# 2. 준비본 시작 확인
Invoke-Smoke (Join-Path $stage 'NDR.exe') 'staged'

# 3. zip과 확인
$zipName = "NDR-rust-win-x64-$stamp.zip"
$zip = Join-Path $work $zipName
# 시작 확인 직후 NDR.exe가 잠깐 잠겨 있을 수 있다(백신 검사 등). 몇 번 다시 해 본다.
for ($attempt = 1; ; $attempt++) {
    try {
        if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
        Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -CompressionLevel Optimal -ErrorAction Stop
        break
    } catch {
        if ($attempt -ge 6) { throw }
        Start-Sleep -Seconds 2
    }
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::OpenRead($zip)
try {
    $entries = $archive.Entries.Count
    $exeEntry = $archive.Entries | Where-Object { $_.FullName -eq 'NDR.exe' }
    if (-not $exeEntry) { throw 'NDR.exe missing from zip' }
    $stream = $exeEntry.Open()
    try { $sha = [Security.Cryptography.SHA256]::Create(); $zipExeHash = ([BitConverter]::ToString($sha.ComputeHash($stream)) -replace '-', '').ToLower() } finally { $stream.Dispose() }
} finally { $archive.Dispose() }
$exeHash = (Get-FileHash -LiteralPath (Join-Path $stage 'NDR.exe') -Algorithm SHA256).Hash.ToLower()
if ($zipExeHash -ne $exeHash) { throw 'zip NDR.exe hash mismatch' }
$zipHash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLower()
"zip ok: $entries entries"

# 4. 배포 폴더 교체: 이전 rust-* 묶음은 old로 옮기고, 루트의 실행 파일과 자원 폴더만 바꾼다.
$old = Join-Path $Release 'old'
New-Item -ItemType Directory -Path $old -Force | Out-Null
# Windows Explorer, antivirus, or an open README can hold a handle to an old
# version directory. A failed rename must not prevent an already verified new
# release from being placed. Preserve an identical checked copy under old/,
# then remove the original only when Windows permits it. Never terminate the
# unrelated process that holds the handle.
Get-ChildItem -LiteralPath $Release -Directory | Where-Object { $_.Name -like 'rust-*' -and $_.Name -ne $Series } | ForEach-Object {
    $previous = $_.FullName
    $saved = Join-Path $old $_.Name
    if (-not (Test-Path -LiteralPath $saved)) {
        $moved = $false
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            try {
                Move-Item -LiteralPath $previous -Destination $saved -ErrorAction Stop
                $moved = $true
                break
            } catch {
                if ($attempt -lt 3) { Start-Sleep -Milliseconds 500 }
            }
        }
        if (-not $moved) {
            Copy-Item -LiteralPath $previous -Destination $saved -Recurse -ErrorAction Stop
        }
    }
    if (Test-Path -LiteralPath $previous) {
        # Either a lock forced a copy or a previous run already preserved one.
        # Verify the archive byte-for-byte before even attempting to delete source.
        $sourceFiles = @(Get-ChildItem -LiteralPath $previous -Recurse -File -Force)
        $archiveFiles = @(Get-ChildItem -LiteralPath $saved -Recurse -File -Force)
        # Windows may remove some files and then refuse to remove the locked
        # directory itself. Remaining source files must be a matching subset
        # of the complete archive already saved under old/.
        if ($sourceFiles.Count -gt $archiveFiles.Count) { throw "previous bundle copy incomplete: $previous -> $saved" }
        $savedManifestPath = Join-Path $saved ("source-manifest-$($_.Name).json")
        if (Test-Path -LiteralPath $savedManifestPath) {
            $savedManifest = Get-Content -LiteralPath $savedManifestPath -Raw | ConvertFrom-Json
            $savedZip = Join-Path $saved ("NDR-rust-win-x64-" + ($_.Name -replace '^rust-', '') + '.zip')
            if ($savedManifest.series -ne $_.Name -or -not (Test-Path -LiteralPath $savedZip) -or
                (Get-FileHash -LiteralPath $savedZip -Algorithm SHA256).Hash.ToLower() -ne $savedManifest.zip.sha256) {
                throw "previous bundle ZIP/manifest verification failed: $saved"
            }
        }
        foreach ($file in $sourceFiles) {
            $relative = $file.FullName.Substring($previous.Length).TrimStart('\')
            $copy = Join-Path $saved $relative
            if (-not (Test-Path -LiteralPath $copy) -or
                (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne
                (Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash) {
                throw "previous bundle copy did not verify: $relative"
            }
        }
        try {
            Remove-Item -LiteralPath $previous -Recurse -Force -ErrorAction Stop
        } catch {
            Write-Warning "Previous release directory is locked; verified backup saved under old/, original retained until unlocked: $previous"
        }
    }
}
foreach ($name in 'NDR.exe', 'NDIR.exe', 'Languages', 'Skins', 'Tools', 'Colors', 'complete.wav', 'THIRD-PARTY-NOTICES.txt', $Series) {
    $target = Join-Path $Release $name
    if (Test-Path -LiteralPath $target) {
        $item = Get-Item -LiteralPath $target -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "unexpected link in release: $target" }
        Remove-Item -LiteralPath $target -Recurse -Force
    }
}
Get-ChildItem -LiteralPath $stage | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $Release $_.Name) -Recurse }
$bundle = Join-Path $Release $Series
New-Item -ItemType Directory -Path $bundle | Out-Null
Copy-Item -LiteralPath $zip -Destination (Join-Path $bundle $zipName)
Copy-Item -LiteralPath $Readme -Destination (Join-Path $bundle 'README.md')
"$zipHash  $zipName`n$exeHash  NDR.exe" | Set-Content -LiteralPath (Join-Path $bundle 'SHA256SUMS.txt') -Encoding ascii
$sources = [ordered]@{}
Get-ChildItem -LiteralPath $project -Recurse -File | Where-Object { $_.FullName -notmatch '\\(target|Resources)\\' } | Sort-Object FullName | ForEach-Object {
    $relative = $_.FullName.Substring($project.Length + 1).Replace('\', '/')
    $sources[$relative] = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLower()
}
$manifest = [ordered]@{
    series = $Series
    version = ((Select-String -LiteralPath (Join-Path $project 'Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value)
    rustc = $rustcVersion
    exeSha256 = $exeHash
    zip = [ordered]@{ entries = $entries; sha256 = $zipHash; bytes = (Get-Item -LiteralPath $zip).Length }
    sourceFiles = $sources.Count
    sources = $sources
}
$manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $bundle "source-manifest-$Series.json") -Encoding utf8

# 5. 배치본 시작 확인
Invoke-Smoke (Join-Path $Release 'NDR.exe') 'placed'
"exe $exeHash"
"zip $zipHash"
Remove-Item -LiteralPath $stage, $work -Recurse -Force
