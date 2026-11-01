# Runs as a separate PowerShell process after NDR closes. The package is
# already downloaded and SHA-256 verified against GitHub Releases by NDR.
param(
    [Parameter(Mandatory=$true)][string]$Archive,
    [Parameter(Mandatory=$true)][string]$Destination,
    [Parameter(Mandatory=$true)][int]$ParentPid,
    [Parameter(Mandatory=$true)][string]$LogPath,
    [switch]$SkipRestart,
    [switch]$ShowProgress
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$script:installerForm = $null
$script:installerLabel = $null
$script:installerBar = $null

# When the NDR window closes, this separate STA PowerShell process keeps a
# compact, real installation-progress window alive. The QA mode stays headless.
function Set-InstallProgress([string]$message, [int]$percent) {
    if (-not $script:installerForm) { return }
    $script:installerLabel.Text = ('{0}  ({1}%)' -f $message, $percent)
    $script:installerBar.Value = [Math]::Max(0, [Math]::Min(100, $percent))
    [System.Windows.Forms.Application]::DoEvents()
}
if ($ShowProgress -and -not $SkipRestart) {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    [System.Windows.Forms.Application]::EnableVisualStyles()
    $script:installerForm = [System.Windows.Forms.Form]::new()
    $script:installerForm.Text = 'NDR 업데이트'
    $script:installerForm.Size = [System.Drawing.Size]::new(460, 172)
    $script:installerForm.StartPosition = 'CenterScreen'
    $script:installerForm.FormBorderStyle = 'FixedDialog'
    $script:installerForm.MaximizeBox = $false
    $script:installerForm.MinimizeBox = $false
    $script:installerForm.ControlBox = $false
    $script:installerForm.TopMost = $true
    $script:installerForm.BackColor = [System.Drawing.Color]::FromArgb(245, 247, 251)
    $heading = [System.Windows.Forms.Label]::new()
    $heading.Text = 'NDR  |  업데이트 설치'
    $heading.Font = [System.Drawing.Font]::new('Segoe UI', 12, [System.Drawing.FontStyle]::Bold)
    $heading.Location = [System.Drawing.Point]::new(22, 17)
    $heading.Size = [System.Drawing.Size]::new(400, 28)
    $script:installerForm.Controls.Add($heading)
    $script:installerLabel = [System.Windows.Forms.Label]::new()
    $script:installerLabel.Location = [System.Drawing.Point]::new(23, 52)
    $script:installerLabel.Size = [System.Drawing.Size]::new(410, 24)
    $script:installerLabel.Font = [System.Drawing.Font]::new('Segoe UI', 9)
    $script:installerForm.Controls.Add($script:installerLabel)
    $script:installerBar = [System.Windows.Forms.ProgressBar]::new()
    $script:installerBar.Location = [System.Drawing.Point]::new(23, 88)
    $script:installerBar.Size = [System.Drawing.Size]::new(408, 16)
    $script:installerBar.Style = 'Continuous'
    $script:installerForm.Controls.Add($script:installerBar)
    $script:installerForm.Show()
    Set-InstallProgress '설치 패키지를 확인하고 있습니다...' 2
}
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
        $entryIndex = 0
        $entryCount = [Math]::Max(1, @($zip.Entries | Where-Object { -not $_.FullName.EndsWith('/') }).Count)
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
            $entryIndex++
            Set-InstallProgress '안전하게 설치 파일을 준비하고 있습니다...' (5 + [int](35 * $entryIndex / $entryCount))
        }
    } finally { $zip.Dispose() }
    foreach ($required in @('NDR.exe','Skins/black/conf.ini','Skins/classic/conf.ini','Languages/ko.lang','Tools/7zip/7za.exe')) {
        if (-not (Test-Path -LiteralPath (Join-Path $staging $required) -PathType Leaf)) { throw "Package missing $required" }
    }

    # The running executable is locked. Wait for only its originating process;
    # never kill any other NDR instance.
    $deadline = [DateTime]::UtcNow.AddMinutes(2)
    Set-InstallProgress 'NDR 종료를 확인하고 있습니다...' 45
    while (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'NDR did not exit before update deadline' }
        Start-Sleep -Milliseconds 500
        Set-InstallProgress 'NDR 종료를 기다리고 있습니다...' 45
    }
    New-Item -ItemType Directory -Path $backup -Force | Out-Null
    $copied = [Collections.Generic.List[string]]::new()
    $installFiles = @(Get-ChildItem $staging -Recurse -File)
    try {
        foreach ($source in $installFiles) {
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
            Set-InstallProgress 'NDR 파일을 설치하고 있습니다...' (50 + [int](45 * $copied.Count / [Math]::Max(1, $installFiles.Count)))
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
    Set-InstallProgress '설치 완료. NDR을 다시 실행합니다...' 100
    if (-not $SkipRestart) {
        Start-Process -FilePath (Join-Path $root 'NDR.exe') -WorkingDirectory $root
    }
    if ($script:installerForm) { $script:installerForm.Close(); $script:installerForm.Dispose() }
    exit 0
} catch {
    "Update failed: $_" | Set-Content -LiteralPath $LogPath -Encoding UTF8
    if ($script:installerForm) {
        $script:installerForm.ControlBox = $true
        Set-InstallProgress ('설치 오류: ' + $_.Exception.Message) 0
        $script:installerForm.TopMost = $false
        [void][System.Windows.Forms.MessageBox]::Show('업데이트 실패. 자세한 내용은 update.log를 확인하세요.', 'NDR 업데이트 오류')
        $script:installerForm.Close()
        $script:installerForm.Dispose()
    }
    exit 1
}
