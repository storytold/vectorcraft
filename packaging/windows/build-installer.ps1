<#
.SYNOPSIS
  Package Vector W3K2 for Windows x64: an installer (Inno Setup) and a portable zip.

.DESCRIPTION
  Uses the release binaries already built (cargo build --release -p vectorcraft -p vectorcraft-cli)
  and writes, in $env:DIST (default: dist/release):
    vector-w3k2-<version>-windows-x64-setup.exe     installer (current user, or all users)
    vector-w3k2-<version>-windows-x64-portable.zip  "Vector W3K2.exe" + vectorcraft-cli.exe

  Needs Inno Setup 6 (ISCC.exe): winget install JRSoftware.InnoSetup --scope user

.EXAMPLE
  pwsh packaging/windows/build-installer.ps1
#>
param([string] $BinDir)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

$Version = $null
$inPkg = $false
foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
  if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
  if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }

if (-not $BinDir) {
  $TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
  $BinDir = Join-Path $TargetDir 'release'
}
foreach ($exe in 'vectorcraft.exe', 'vectorcraft-cli.exe') {
  if (-not (Test-Path (Join-Path $BinDir $exe))) { throw "$exe not found in ${BinDir}: build the release binaries first" }
}
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

$Iscc = @(
  (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
  (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
  (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $Iscc) { throw 'ISCC.exe not found: winget install JRSoftware.InnoSetup --scope user' }

Write-Output "==> installer (Vector W3K2 $Version)"
& $Iscc /Q "/DVersion=$Version" "/DBinDir=$BinDir" "/DRoot=$Root" "/DOutDir=$Dist" (Join-Path $PSScriptRoot 'vector-w3k2.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC failed with exit code $LASTEXITCODE" }

Write-Output '==> portable zip'
$Stage = Join-Path $Dist "vector-w3k2-$Version-windows-x64-portable"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item (Join-Path $BinDir 'vectorcraft.exe') (Join-Path $Stage 'Vector W3K2.exe')
Copy-Item (Join-Path $BinDir 'vectorcraft-cli.exe') $Stage
foreach ($f in 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE') { Copy-Item (Join-Path $Root $f) $Stage }
$Zip = "$Stage.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path $Stage -DestinationPath $Zip
Remove-Item -Recurse -Force $Stage

Get-Item (Join-Path $Dist "vector-w3k2-$Version-windows-x64-setup.exe"), $Zip | Format-Table Name, Length
