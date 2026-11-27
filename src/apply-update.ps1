# Runs as a separate PowerShell process after NDR closes. The package is
# already downloaded and SHA-256 verified against GitHub Releases by NDR.
param(
    [Parameter(Mandatory=$true)][string]$Archive,
    [Parameter(Mandatory=$true)][string]$Destination,
    [Parameter(Mandatory=$true)][int]$ParentPid,
    [Parameter(Mandatory=$true)][string]$LogPath,
    [switch]$SkipRestart,
    [switch]$ShowProgress,
    [switch]$ManagedCache,
    [string]$TextPath
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$script:installerForm = $null
$script:installerHeading = $null
$script:installerLabel = $null
$script:installerBar = $null
# Standalone/headless test invocation remains possible without a text file.
$script:installerText = @{
    title = 'NDR Update'
    heading = 'Installing NDR update'
    checking = 'Checking the installation package...'
    extracting = 'Preparing the installation files...'
    closing = 'Checking whether NDR has closed...'
    waiting = 'Waiting for NDR to close...'
    installing = 'Installing the NDR files...'
    finished = 'Installation complete. Restarting NDR...'
    failed = 'The update failed. See update.log for details.'
    failureTitle = 'NDR update error'
}
function Import-InstallerText([string]$path) {
if ($path -and (Test-Path -LiteralPath $path -PathType Leaf)) {
    try {
        $translated = Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json
        foreach ($key in @($script:installerText.Keys)) {
            $property = $translated.PSObject.Properties[[string]$key]
            if ($property -and $property.Value -is [string] -and -not [string]::IsNullOrWhiteSpace($property.Value)) {
                $script:installerText[$key] = $property.Value
            }
        }
    } catch {
        # Invalid overlay data must not prevent an otherwise verified update.
    }
}
}
Import-InstallerText $TextPath

# When the NDR window closes, this separate STA PowerShell process keeps a
# compact, real installation-progress window alive. The QA mode stays headless.
function Set-InstallProgress([string]$message, [int]$percent) {
    if (-not $script:installerForm) { return }
    $script:installerLabel.Text = ('{0}  ({1}%)' -f $message, $percent)
    $script:installerBar.Value = [Math]::Max(0, [Math]::Min(100, $percent))
    # Reflow after the actual translated text and font sizes are known, even
    # after the form's first layout or a DPI/text-size change.
    $script:installerForm.PerformLayout()
    $script:installerLabel.Top = [Math]::Max(52, $script:installerHeading.Bottom + 8)
    $script:installerForm.PerformLayout()
    $script:installerBar.Top = [Math]::Max(88, $script:installerLabel.Bottom + 12)
    $client = $script:installerForm.ClientSize
    $minimumHeight = $script:installerBar.Bottom + 18
    if ($client.Height -lt $minimumHeight) {
        $script:installerForm.ClientSize = [System.Drawing.Size]::new($client.Width, $minimumHeight)
    }
    [System.Windows.Forms.Application]::DoEvents()
}
function New-InstallProgress {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    [System.Windows.Forms.Application]::EnableVisualStyles()
    $script:installerForm = [System.Windows.Forms.Form]::new()
    $script:installerForm.Text = $script:installerText.title
    $script:installerForm.ClientSize = [System.Drawing.Size]::new(450, 132)
    $script:installerForm.StartPosition = 'CenterScreen'
    $script:installerForm.FormBorderStyle = 'FixedDialog'
    $script:installerForm.MaximizeBox = $false
    $script:installerForm.MinimizeBox = $false
    $script:installerForm.ControlBox = $false
    $script:installerForm.TopMost = $true
    $script:installerForm.BackColor = [System.Drawing.Color]::FromArgb(245, 247, 251)
    $heading = [System.Windows.Forms.Label]::new()
    $script:installerHeading = $heading
    $heading.Text = $script:installerText.heading
    $heading.Font = [System.Drawing.Font]::new('Segoe UI', 12, [System.Drawing.FontStyle]::Bold)
    $heading.Location = [System.Drawing.Point]::new(22, 17)
    $heading.MaximumSize = [System.Drawing.Size]::new(408, 0)
    $heading.AutoSize = $true
    $script:installerForm.Controls.Add($heading)
    $script:installerLabel = [System.Windows.Forms.Label]::new()
    $script:installerLabel.Location = [System.Drawing.Point]::new(23, [Math]::Max(52, $heading.Bottom + 8))
    $script:installerLabel.MaximumSize = [System.Drawing.Size]::new(408, 0)
    $script:installerLabel.AutoSize = $true
    $script:installerLabel.Font = [System.Drawing.Font]::new('Segoe UI', 9)
    $script:installerForm.Controls.Add($script:installerLabel)
    $script:installerBar = [System.Windows.Forms.ProgressBar]::new()
    $script:installerBar.Location = [System.Drawing.Point]::new(23, 88)
    $script:installerBar.Size = [System.Drawing.Size]::new(408, 16)
    $script:installerBar.Style = 'Continuous'
    $script:installerForm.Controls.Add($script:installerBar)

    Set-InstallProgress $script:installerText.checking 2
}
# An updater can outlive its originating NDR PID. A separate, exclusive
# session handle protects its working data from cache maintenance and a
# second installer invocation. Never truncate an existing lock file.
$work = Split-Path -Parent ([IO.Path]::GetFullPath($Archive))
$lockPath = Join-Path $work '.ndr-install.lock'
if (Test-Path -LiteralPath $lockPath) {
    $lockItem = Get-Item -LiteralPath $lockPath -Force
    if ($lockItem.PSIsContainer -or ($lockItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Invalid update session lock'
    }
}
$installerLock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
# A successful session is immutable and must not be started again while
# another instance is pruning its redundant ZIP/extracted payload.
if ((Test-Path -LiteralPath $LogPath -PathType Leaf) -and
    ((Get-Content -LiteralPath $LogPath -Raw -Encoding UTF8).Trim() -match '^Update installed: [0-9]+ files$')) {
    $installerLock.Dispose()
    throw 'Update session is already complete'
}
$staging = Join-Path $work 'extracted'
$backup = Join-Path $work 'backup'
# Only the app's own dedicated cache session is eligible for immediate
# payload removal. Standalone invocations keep their caller-owned ZIP.
function Test-PlainTree([string]$path) {
    try {
        $todo = [Collections.Generic.Stack[string]]::new()
        $todo.Push($path)
        while ($todo.Count -gt 0) {
            $item = Get-Item -LiteralPath ($todo.Pop()) -Force -ErrorAction Stop
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { return $false }
            if ($item.PSIsContainer) {
                foreach ($child in @(Get-ChildItem -LiteralPath $item.FullName -Force -ErrorAction Stop)) {
                    $todo.Push($child.FullName)
                }
            }
        }
        return $true
    } catch { return $false }
}
function Clear-VerifiedPayload {
    if (-not $ManagedCache) { return }
    try {
        $updates = Split-Path -Parent $work
        $cache = Split-Path -Parent $updates
        if ((Split-Path -Leaf $work) -notmatch '^ndr-update-build-[1-9][0-9]*-[1-9][0-9]*-[0-9]+$' -or
            (Split-Path -Leaf $updates) -ne 'updates' -or (Split-Path -Leaf $cache) -ne 'cache' -or
            [IO.Path]::GetFullPath($Archive) -ne (Join-Path $work 'NDR-windows-x64.zip') -or
            [IO.Path]::GetFullPath($LogPath) -ne (Join-Path $work 'update.log') -or
            -not (Test-Path -LiteralPath (Join-Path $backup 'NDR.exe') -PathType Leaf)) { return }
        # Never follow a junction in the cache ancestry or payload tree.
        $parent = $work
        while ($parent) {
            $item = Get-Item -LiteralPath $parent -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { return }
            $next = Split-Path -Parent $parent
            if ($next -eq $parent) { break }
            $parent = $next
        }
        if ((Test-Path -LiteralPath $staging -PathType Container) -and (Test-PlainTree $staging)) {
            Remove-Item -LiteralPath $staging -Recurse -Force -ErrorAction Stop
        }
        if ((Test-Path -LiteralPath $Archive -PathType Leaf) -and (Test-PlainTree $Archive)) {
            Remove-Item -LiteralPath $Archive -Force -ErrorAction Stop
        }
    } catch {
        # Cleanup is optional; do not turn a completed installation into
        # a failure or discard its rollback merely because a file is busy.
    }
}
try {
    if ($ShowProgress -and -not $SkipRestart) {
        New-InstallProgress
        $script:installerForm.Show()
    }
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
            Set-InstallProgress $script:installerText.extracting (5 + [int](35 * $entryIndex / $entryCount))
        }
    } finally { $zip.Dispose() }
    foreach ($required in @('NDR.exe','Skins/black/conf.ini','Skins/classic/conf.ini','Languages/ko.lang','Tools/7zip/7za.exe')) {
        if (-not (Test-Path -LiteralPath (Join-Path $staging $required) -PathType Leaf)) { throw "Package missing $required" }
    }

    # The running executable is locked. Wait for only its originating process;
    # never kill any other NDR instance.
    $deadline = [DateTime]::UtcNow.AddMinutes(2)
    Set-InstallProgress $script:installerText.closing 45
    while (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'NDR did not exit before update deadline' }
        Start-Sleep -Milliseconds 500
        Set-InstallProgress $script:installerText.waiting 45
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
            Set-InstallProgress $script:installerText.installing (50 + [int](45 * $copied.Count / [Math]::Max(1, $installFiles.Count)))
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
    Set-InstallProgress $script:installerText.finished 100
    if (-not $SkipRestart) {
        Start-Process -FilePath (Join-Path $root 'NDR.exe') -WorkingDirectory $root
    }
    Clear-VerifiedPayload
    if ($script:installerForm) { $script:installerForm.Close(); $script:installerForm.Dispose() }
    exit 0
} catch {
    "Update failed: $_" | Set-Content -LiteralPath $LogPath -Encoding UTF8
    if ($script:installerForm) {
        $script:installerForm.ControlBox = $true
        Set-InstallProgress $script:installerText.failed 0
        $script:installerForm.TopMost = $false
        $failureText = $script:installerText.failed + [Environment]::NewLine + [Environment]::NewLine + $LogPath
        [void][System.Windows.Forms.MessageBox]::Show($script:installerForm, $failureText, $script:installerText.failureTitle)
        $script:installerForm.Close()
        $script:installerForm.Dispose()
    }
    exit 1
} finally {
    $installerLock.Dispose()
}
