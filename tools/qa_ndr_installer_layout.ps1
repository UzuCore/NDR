# Render the production installer controls without evaluating its installation
# body. No archive extraction, process termination, copying or restart.
param(
    [Parameter(Mandatory=$true)][string]$CatalogDir,
    [Parameter(Mandatory=$true)][string]$OutputDir
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()
$source = Join-Path (Split-Path -Parent $PSScriptRoot) 'src\apply-update.ps1'
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0) { throw ($errors | Out-String) }
# Evaluate only the presentation functions and default message table.
$functionNames = @('Import-InstallerText','New-InstallProgress','Set-InstallProgress')
foreach ($name in $functionNames) {
    $definition = $ast.EndBlock.Statements | Where-Object {
        $_ -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $_.Name -eq $name
    }
    if (@($definition).Count -ne 1) { throw "Missing function: $name" }
    . ([scriptblock]::Create($definition.Extent.Text))
}
$defaultTable = $ast.EndBlock.Statements | Where-Object {
    $_ -is [System.Management.Automation.Language.AssignmentStatementAst] -and $_.Left.Extent.Text -eq '$script:installerText'
}
if (@($defaultTable).Count -ne 1) { throw 'Missing default installer message table' }
$catalogs = @(Get-ChildItem -LiteralPath $CatalogDir -Filter '*.json' -File)
if ($catalogs.Count -ne 40) { throw "Expected 40 catalogs, found $($catalogs.Count)" }
New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
$stages = [ordered]@{checking=2;extracting=25;closing=45;waiting=45;installing=75;finished=100;failed=0}
$results = [Collections.Generic.List[object]]::new()
foreach ($catalog in $catalogs) {
    . ([scriptblock]::Create($defaultTable.Extent.Text))
    $expected = Get-Content -LiteralPath $catalog.FullName -Encoding UTF8 -Raw | ConvertFrom-Json
    Import-InstallerText $catalog.FullName
    foreach ($key in $script:installerText.Keys) {
        if ($script:installerText[$key] -cne $expected.PSObject.Properties[$key].Value) {
            throw "$($catalog.BaseName): message $key did not round-trip"
        }
    }
    foreach ($scale in @(1.0,1.5)) {
        New-InstallProgress
        $form = $script:installerForm
        try {
            $form.ShowInTaskbar = $false
            $form.TopMost = $false
            $form.StartPosition = 'Manual'
            $form.Location = [Drawing.Point]::new(-10000,-10000)
            $script:installerHeading.Font = [Drawing.Font]::new('Segoe UI', [single](12*$scale), [Drawing.FontStyle]::Bold)
            $script:installerLabel.Font = [Drawing.Font]::new('Segoe UI', [single](9*$scale))
            $form.Show()
            foreach ($key in $stages.Keys) {
                $message = $script:installerText[$key]
                Set-InstallProgress $message $stages[$key]
                $form.PerformLayout()
                $h = $script:installerHeading
                $l = $script:installerLabel
                $b = $script:installerBar
                $client = $form.ClientSize
                $hp = $h.GetPreferredSize([Drawing.Size]::new($h.MaximumSize.Width,0))
                $lp = $l.GetPreferredSize([Drawing.Size]::new($l.MaximumSize.Width,0))
                $ok = ($form.Text -ceq $expected.title -and
                    $h.Text -ceq $expected.heading -and
                    $l.Text -ceq ('{0}  ({1}%)' -f $expected.PSObject.Properties[$key].Value,$stages[$key]) -and
                    $h.Height -ge $hp.Height -and $l.Height -ge $lp.Height -and
                    $h.Right -le $client.Width -and $l.Right -le $client.Width -and
                    $h.Bottom -lt $l.Top -and $l.Bottom -lt $b.Top -and
                    $b.Right -le $client.Width -and $b.Bottom -le $client.Height -and
                    $b.Value -eq $stages[$key])
                $results.Add([pscustomobject]@{
                    locale=$catalog.BaseName;scale=$scale;stage=$key;ok=$ok
                    heading=$h.Text;message=$l.Text;client=@($client.Width,$client.Height)
                    headingBounds=@($h.Left,$h.Top,$h.Width,$h.Height)
                    messageBounds=@($l.Left,$l.Top,$l.Width,$l.Height)
                    barBounds=@($b.Left,$b.Top,$b.Width,$b.Height)
                })
                if ($scale -eq 1.5 -and $key -in @('installing','failed') -and $catalog.BaseName -in @('de','el','ja','ko','ru','zh-TW')) {
                    $bitmap = [Drawing.Bitmap]::new($form.Width,$form.Height)
                    try {
                        $form.DrawToBitmap($bitmap,[Drawing.Rectangle]::new(0,0,$bitmap.Width,$bitmap.Height))
                        $bitmap.Save((Join-Path $OutputDir "$($catalog.BaseName)-$key.png"),[Drawing.Imaging.ImageFormat]::Png)
                    } finally { $bitmap.Dispose() }
                }
            }
        } finally {
            $form.Close()
            $form.Dispose()
            $script:installerForm = $null
        }
    }
    Write-Output "$($catalog.BaseName): 14 installer layouts"
}
$failed = @($results | Where-Object { -not $_.ok })
$report = [pscustomobject]@{total=$results.Count;passed=$results.Count-$failed.Count;failed=$failed.Count;cases=$results}
$report | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $OutputDir 'manifest.json') -Encoding UTF8
Write-Output ("Installer layouts: {0}/{1} PASS" -f $report.passed,$report.total)
if ($failed.Count) { $failed | Select-Object locale,scale,stage | Format-Table; exit 1 }
