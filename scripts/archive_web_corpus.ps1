# Download bounded upstream libarchive fixtures and check them using the exact
# bundled 7zip+ that NDR ships. No downloaded payload is executed or installed.
# Source files are upstream BSD-licensed test cases, uuencoded for transport.
param(
    [string]$Destination = 'D:\NDR\work\NDR-rust\target\archive-web-corpus',
    [string]$LibarchiveRevision = '348e5a2e27503f5bf114e298e2bd9ddcd59ab48e',
    [string]$SharpCompressRevision = '6128ec51eec4ca9f7b7b9f44b77bdb0cd8b3bd97'
)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$seven = Join-Path $project 'Resources\Tools\7zip\Full\7z.exe'
if (-not (Test-Path -LiteralPath $seven)) { throw "Bundled 7-Zip not found: $seven" }

# Keep normal, format-specific test samples. Intentionally malformed crash,
# overflow, fuzz, and traversal inputs are not included in this routine.
$samples = @(
    'test_read_format_rar_compress_normal.rar.uu',
    'test_read_format_rar5_compressed.rar.uu',
    'test_read_format_rar5_blake2.rar.uu',
    'test_read_format_rar_unicode.rar.uu',
    'test_read_format_zip.zip.uu',
    'test_compat_zip_1.zip.uu',
    'test_read_format_zip_bzip2.zipx.uu',
    'test_read_format_zip_bzip2_multi.zipx.uu',
    'test_read_format_zip_lzma.zipx.uu',
    'test_read_format_zip_lzma_multi.zipx.uu',
    'test_read_format_zip_ppmd8.zipx.uu',
    'test_read_format_zip_zstd.zipx.uu',
    'test_read_format_zip_zstd_multi.zipx.uu',
    'test_read_format_zip_xz_multi.zipx.uu',
    'test_read_format_7zip_copy.7z.uu',
    'test_read_format_7zip_lzma2.7z.uu',
    'test_read_format_7zip_zstd.7z.uu',
    'test_read_format_7zip_solid_zstd.7z.uu',
    'test_read_format_cab_1.cab.uu',
    'test_read_format_cab_2.cab.uu',
    'test_read_format_lha_lh0.lzh.uu',
    'test_read_format_lha_lh6.lzh.uu',
    'test_read_format_lha_amiga_script.lha.uu',
    'test_compat_cpio_1.cpio.uu',
    'test_read_format_cpio_bin_le.cpio.uu',
    'test_read_format_ar.ar.uu',
    'test_read_format_iso.iso.Z.uu',
    'test_read_format_iso_rockridge.iso.Z.uu',
    'test_read_format_cpio_svr4_gzip_rpm.rpm.uu',
    'test_read_format_cpio_svr4_bzip2_rpm.rpm.uu',
    'test_compat_gtar_1.tar.uu',
    'test_compat_gzip_1.tgz.uu',
    'test_compat_bzip2_1.tbz.uu',
    'test_compat_xz_1.txz.uu',
    'test_compat_lzip_1.tlz.uu',
    'test_compat_lzip_3.lz.uu',
    'test_compat_zstd_1.tar.zst.uu',
    'test_compat_lzma_1.tlz.uu'
)

# Standard UUdecode, performed in memory without trusting the embedded filename.
function Decode-UuToFile([string]$source, [string]$destination) {
    $lines = [IO.File]::ReadAllLines($source, [Text.Encoding]::ASCII)
    if ($lines.Count -lt 4 -or $lines[0] -notmatch '^begin [0-7]{3} (.+)$') { throw "Invalid uuencoded header: $source" }
    $embedded = $Matches[1]
    if ($embedded -ne [IO.Path]::GetFileName($destination)) { throw "Unexpected embedded filename: $embedded" }
    $bytes = [Collections.Generic.List[byte]]::new()
    $terminated = $false
    foreach ($line in $lines | Select-Object -Skip 1) {
        if ($line -eq 'end') { $terminated = $true; break }
        if ($line.Length -eq 0) { continue }
        $n = (([int][char]$line[0]) - 32) -band 63
        if ($n -eq 0) { continue }
        if ($n -gt 45) { throw "UU line exceeds 45 bytes: $source" }
        $chunks = [int][Math]::Ceiling($n / 3)
        if ($line.Length -lt 1 + 4 * $chunks) { throw "Truncated UU line: $source" }
        for ($k = 0; $k -lt $chunks; $k++) {
            $idx = 1 + 4 * $k
            $a = (([int][char]$line[$idx]) - 32) -band 63
            $b = (([int][char]$line[$idx + 1]) - 32) -band 63
            $c = (([int][char]$line[$idx + 2]) - 32) -band 63
            $d = (([int][char]$line[$idx + 3]) - 32) -band 63
            $remain = $n - 3 * $k
            if ($remain -ge 1) { $bytes.Add([byte](($a -shl 2) -bor ($b -shr 4))) }
            if ($remain -ge 2) { $bytes.Add([byte]((($b -band 15) -shl 4) -bor ($c -shr 2))) }
            if ($remain -ge 3) { $bytes.Add([byte]((($c -band 3) -shl 6) -bor $d)) }
        }
        if ($bytes.Count -gt 4MB) { throw "Fixture exceeds 4MB decoded size cap: $source" }
    }
    if (-not $terminated -or $bytes.Count -lt 4) { throw "Incomplete UU source: $source" }
    [IO.File]::WriteAllBytes($destination, $bytes.ToArray())
    return $bytes.Count
}

$headers = @{ 'User-Agent' = 'NDR-archive-format-validation'; 'Accept' = 'application/vnd.github+json' }
$commit = $LibarchiveRevision
if ($commit -notmatch '^[a-f0-9]{40}$') { throw "Cannot pin libarchive revision: $commit" }
$srcBase = "https://raw.githubusercontent.com/libarchive/libarchive/$commit/libarchive/test/"
$raw = Join-Path $Destination 'encoded'
$out = Join-Path $Destination 'samples'
$logs = Join-Path $Destination 'logs'
New-Item -ItemType Directory -Path $raw, $out, $logs -Force | Out-Null
$results = [Collections.Generic.List[object]]::new()

foreach ($name in $samples) {
    $sourceUrl = $srcBase + $name
    $rawFile = Join-Path $raw $name
    $fileName = $name.Substring(0, $name.Length - 3)
    $sample = Join-Path $out $fileName
    $row = [ordered]@{
        name = $fileName; url = $sourceUrl; decoded_bytes = 0; sha256 = '';
        sevenzip_list = ''; sevenzip_test = ''; note = ''
    }
    try {
        if (-not (Test-Path -LiteralPath $rawFile)) {
            Invoke-WebRequest -Uri $sourceUrl -OutFile $rawFile -TimeoutSec 20
        }
        if ((Get-Item -LiteralPath $rawFile).Length -gt 250KB) { throw 'Download size exceeded 250KB cap' }
        $size = Decode-UuToFile $rawFile $sample
        $row.decoded_bytes = $size
        $row.sha256 = (Get-FileHash -LiteralPath $sample -Algorithm SHA256).Hash.ToLower()
        $savedErrorAction = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $listOutput = & $seven l -slt -ba -sccUTF-8 -- $sample 2>&1 | Out-String
            $row.sevenzip_list = if ($LASTEXITCODE -eq 0) { 'PASS' } else { "FAIL:$LASTEXITCODE" }
            [IO.File]::WriteAllText((Join-Path $logs "$name.list.txt"), $listOutput, [Text.Encoding]::UTF8)
            if ($row.sevenzip_list -ne 'PASS') { $row.sevenzip_test = 'NOT_RUN' }
            else {
                $numbers = [regex]::Matches($listOutput, '(?m)^Size = (\d+)\s*$') | ForEach-Object { [long]$_.Groups[1].Value }
                if (($numbers | Measure-Object -Sum).Sum -gt 64MB -or ($numbers | Measure-Object -Maximum).Maximum -gt 32MB) {
                    $row.sevenzip_test = 'SKIPPED_SIZE_CAP'
                } else {
                    $testOutput = & $seven t -y -sccUTF-8 -- $sample 2>&1 | Out-String
                    $row.sevenzip_test = if ($LASTEXITCODE -eq 0) { 'PASS' } else { "FAIL:$LASTEXITCODE" }
                    [IO.File]::WriteAllText((Join-Path $logs "$name.test.txt"), $testOutput, [Text.Encoding]::UTF8)
                    if ($row.sevenzip_test -ne 'PASS') {
                        $row.note = ([regex]::Matches($testOutput, '(?mi)ERROR: .*') | Select-Object -First 1 | ForEach-Object { $_.Value.Trim() }) -join ''
                    }
                }
            }
        } finally {
            $ErrorActionPreference = $savedErrorAction
        }
    } catch {
        if (-not $row.sevenzip_list) { $row.sevenzip_list = 'ERROR' }
        $row.note = $_.Exception.Message
    }
    $results.Add([pscustomobject]$row)
    Write-Output ("{0,-62} {1,-9} {2,-15} {3}" -f $row.name, $row.sevenzip_list, $row.sevenzip_test, $row.note)
}

# Raw ISO9660 samples shipped as compress(.Z) streams: decode the extra .Z
# wrapper into actual .iso files in another isolated target directory.
foreach ($encodedIso in $results | Where-Object { $_.name -match '\.iso\.Z$' -and $_.sevenzip_test -eq 'PASS' }) {
    $wrapped = Join-Path $out $encodedIso.name
    $isoName = $encodedIso.name.Substring(0, $encodedIso.name.Length - 2)
    $isoOutput = Join-Path $out $isoName
    $extractDir = Join-Path $Destination ('iso-staging-' + [IO.Path]::GetFileNameWithoutExtension($isoName))
    New-Item -ItemType Directory -Path $extractDir -Force | Out-Null
    $null = & $seven x -y -sccUTF-8 ("-o$extractDir") -- $wrapped 2>&1
    if ($LASTEXITCODE -ne 0) { Write-Output "ISO_WRAPPER_DECODE_FAILED $($encodedIso.name)"; continue }
    $decoded = Get-ChildItem -LiteralPath $extractDir -Recurse -File | Select-Object -First 1
    if (-not $decoded -or $decoded.Length -gt 16MB) { Write-Output "ISO_WRAPPER_UNSAFE $($encodedIso.name)"; continue }
    Copy-Item -LiteralPath $decoded.FullName -Destination $isoOutput -Force
    $check = & $seven t -y -sccUTF-8 -- $isoOutput 2>&1 | Out-String
    $verified = if ($LASTEXITCODE -eq 0) { 'PASS' } else { "FAIL:$LASTEXITCODE" }
    $results.Add([pscustomobject][ordered]@{
        name = $isoName; url = $encodedIso.url; decoded_bytes = $decoded.Length;
        sha256 = (Get-FileHash -LiteralPath $isoOutput -Algorithm SHA256).Hash.ToLower();
        sevenzip_list = $verified; sevenzip_test = $verified;
        note = 'ISO decoded from original libarchive .iso.Z fixture'
    })
    Write-Output "ISO_DECODED $isoName $verified"
    Remove-Item -LiteralPath $extractDir -Force -Recurse
}

# A second independent open-source corpus supplies genuine ACE, ARJ, and
# WinZip-made ZIPX archives, not mere extension-renamed ZIP files.
$sharpRev = $SharpCompressRevision
if ($sharpRev -notmatch '^[a-f0-9]{40}$') { throw "Cannot pin SharpCompress revision: $sharpRev" }
$sharpBase = "https://raw.githubusercontent.com/adamhathcock/sharpcompress/$sharpRev/tests/TestArchives/Archives/"
$sharpFiles = @(
    'Ace.store.ace', 'Ace.method1.ace',
    'Arj.store.arj', 'Arj.method1.arj', 'Arj.method2.arj', 'Arj.method4.arj',
    'WinZip26.nocomp.multi.zipx', 'WinZip26_BZip2.zipx', 'WinZip26_LZMA.zipx',
    'WinZip27_XZ.zipx', 'WinZip27_ZSTD.zipx',
    '7Zip.ZSTD.7z', 'Tar.tar.zst', 'Tar.tar.lz', 'Tar.tar.Z',
    'Tar.tar.xz', 'Tar.tar.gz', 'Tar.tar.bz2'
)
$sharpDir = Join-Path $out 'sharpcompress'
New-Item -ItemType Directory -Path $sharpDir -Force | Out-Null
foreach ($name in $sharpFiles) {
    $target = Join-Path $sharpDir $name
    $url = $sharpBase + $name
    $row = [ordered]@{
        name = "sharpcompress/$name"; url = $url; decoded_bytes = 0; sha256 = '';
        sevenzip_list = ''; sevenzip_test = ''; note = ''
    }
    try {
        if (-not (Test-Path -LiteralPath $target)) {
            Invoke-WebRequest -Uri $url -OutFile $target -TimeoutSec 20
        }
        $size = (Get-Item -LiteralPath $target).Length
        if ($size -lt 16 -or $size -gt 600KB) { throw "Fixture outside bounded 16-byte..600-KB range: $size" }
        $row.decoded_bytes = $size
        $row.sha256 = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLower()
        $savedErrorAction = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $listed = & $seven l -slt -ba -sccUTF-8 -- $target 2>&1 | Out-String
            $listCode = $LASTEXITCODE
            if ($listCode -eq 0) {
                $tested = & $seven t -y -sccUTF-8 -- $target 2>&1 | Out-String
                $testCode = $LASTEXITCODE
            }
        } finally { $ErrorActionPreference = $savedErrorAction }
        $row.sevenzip_list = if ($listCode -eq 0) { 'PASS' } else { "FAIL:$listCode" }
        $row.sevenzip_test = if ($listCode -ne 0) { 'NOT_RUN' } elseif ($testCode -eq 0) { 'PASS' } else { "FAIL:$testCode" }
        [IO.File]::WriteAllText((Join-Path $logs ("sharpcompress-$name.list.txt")), $listed, [Text.Encoding]::UTF8)
        if ($testCode -ne 0) {
            [IO.File]::WriteAllText((Join-Path $logs ("sharpcompress-$name.test.txt")), $tested, [Text.Encoding]::UTF8)
            $row.note = ([regex]::Matches($tested, '(?mi)^(?:Error:|ERROR:|.*Unsupported Method.*)$') | Select-Object -First 2 | ForEach-Object {$_.Value.Trim()}) -join ' | '
        }
    } catch {
        if (-not $row.sevenzip_list) { $row.sevenzip_list = 'ERROR' }
        $row.note = $_.Exception.Message
    }
    $results.Add([pscustomobject]$row)
    Write-Output ("{0,-62} {1,-9} {2,-15} {3}" -f $row.name, $row.sevenzip_list, $row.sevenzip_test, $row.note)
}

$csv = Join-Path $Destination 'inventory.csv'
$json = Join-Path $Destination 'inventory.json'
$results | Export-Csv -LiteralPath $csv -NoTypeInformation -Encoding UTF8
[pscustomobject]@{ upstream = 'libarchive/libarchive; adamhathcock/sharpcompress'; libarchive_commit = $commit; sharpcompress_commit = $sharpRev; collected_utc = (Get-Date).ToUniversalTime().ToString('o'); fixtures = $results } |
    ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $json -Encoding UTF8
$pass = @($results | Where-Object { $_.sevenzip_list -eq 'PASS' -and $_.sevenzip_test -eq 'PASS' }).Count
$fail = @($results | Where-Object { $_.sevenzip_list -like 'FAIL*' -or $_.sevenzip_test -like 'FAIL*' -or $_.sevenzip_list -eq 'ERROR' }).Count
Write-Output "CORPUS_DONE fixtures=$($results.Count) engine_pass=$pass issues=$fail inventory=$csv libarchive=$commit sharpcompress=$sharpRev"
