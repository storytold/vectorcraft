<# Experimental, unsupported Windows 7 SP1 x64 build (docs/windows7.md).
   Build on Windows 10/11 with the VS 2022 x64 C++ tools and Windows SDK.
   From the repository root: powershell -File packaging/windows/windows7.ps1 [-Test]
   The resulting portable ZIP targets Windows 7 SP1 x64 with an OpenGL 3.3 driver.
   The compiler runs on the build host, not on Windows 7. -Test also runs the app and CLI tests
   on the build host. #>
param([string] $Toolchain = 'nightly-2026-10-01', [switch] $Test)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Target = 'x86_64-win7-windows-msvc'
function Invoke-Checked([scriptblock] $Command) {
  & $Command
  if ($LASTEXITCODE -ne 0) { throw "Command failed with exit code $LASTEXITCODE" }
}
# Cargo.lock without windows-link's registry source and checksum lines (and with LF line endings),
# so the lock files with and without the patch below compare equal.
function Get-LockWithoutWindowsLinkSource([string] $Text) {
  ($Text -replace "`r`n", "`n") -replace '(?m)^(name = "windows-link"\nversion = "[^"]*"\n)source = "[^"]*"\nchecksum = "[^"]*"\n', '$1'
}
Push-Location $Root
$FlagName = 'CARGO_TARGET_X86_64_WIN7_WINDOWS_MSVC_RUSTFLAGS'
$OldFlags = [Environment]::GetEnvironmentVariable($FlagName)
$OldWinres = $env:VECTORCRAFT_REQUIRE_WINRES
$Lock = Join-Path $Root 'Cargo.lock'
$LockBytes = [IO.File]::ReadAllBytes($Lock)
try {
  Invoke-Checked { rustup toolchain install $Toolchain --profile minimal --component rust-src }
  # windows-sys imports CoTaskMemFree from combase.dll, which Windows 7 lacks. Only this build uses
  # the patched copy in vendor/windows-link (see its VECTORCRAFT-PATCH.md); every other build keeps
  # the crates.io crate. Cargo records the patch in Cargo.lock, so resolve once, check that nothing
  # but windows-link's source moved, then build --locked. The finally block restores Cargo.lock.
  $Patch = @('--config', "patch.crates-io.windows-link.path='vendor/windows-link'")
  Invoke-Checked { cargo "+$Toolchain" metadata --format-version 1 @Patch | Out-Null }
  $Before = Get-LockWithoutWindowsLinkSource ([Text.Encoding]::UTF8.GetString($LockBytes))
  $After = Get-LockWithoutWindowsLinkSource ([IO.File]::ReadAllText($Lock))
  if ($Before -cne $After) { throw 'Cargo.lock is out of date: applying the windows-link patch changed other entries' }
  $CargoArgs = @('--locked') + $Patch + @('--target', $Target, '--no-default-features', '--features', 'vectorcraft/windows7')
  $Tree = cargo "+$Toolchain" tree @CargoArgs -p vectorcraft -e normal --prefix none
  if ($LASTEXITCODE -ne 0) { throw 'dependency inspection failed' }
  if ($Tree -match '^(wgpu|wgpu-core|wgpu-hal|accesskit_windows|craft-print) v') { throw 'unsupported Windows backend in compatibility build' }
  if (-not ($Tree -match '^windows-link v[^ ]+ \(.*vendor[\\/]windows-link\)')) { throw 'the patched windows-link is not in use' }
  # The ordinary pc-windows-msvc standard library requires Windows 10. Rebuild std for win7.
  [Environment]::SetEnvironmentVariable($FlagName, '-C target-feature=+crt-static')
  $env:VECTORCRAFT_REQUIRE_WINRES = '1'
  $CargoArgs += @('-p', 'vectorcraft', '-p', 'vectorcraft-cli')
  Invoke-Checked { cargo "+$Toolchain" build -Z build-std=std,panic_unwind @CargoArgs --release }
  $TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
  $Bin = Join-Path $TargetDir "$Target\release"
  foreach ($Name in 'vectorcraft.exe', 'vectorcraft-cli.exe') {
    $Exe = Join-Path $Bin $Name
    $Bytes = [IO.File]::ReadAllBytes($Exe)
    $Pe = [BitConverter]::ToInt32($Bytes, 0x3c)
    if ([BitConverter]::ToUInt16($Bytes, $Pe + 4) -ne 0x8664) { throw "$Name is not x64" }
    $Major = [BitConverter]::ToUInt16($Bytes, $Pe + 0x48)
    $Minor = [BitConverter]::ToUInt16($Bytes, $Pe + 0x4a)
    if ($Major -gt 6 -or ($Major -eq 6 -and $Minor -gt 1)) { throw "$Name requires subsystem $Major.$Minor" }
    $Imports = & dumpbin /nologo /imports $Exe
    if ($LASTEXITCODE -ne 0) { throw "Could not inspect $Name imports" }
    # Regression gate for known loader failures; actual Win7 runtime QA remains necessary.
    if ($Imports -match '(?i)\b(GetSystemTimePreciseAsFileTime|ProcessPrng|WaitOnAddress|WakeByAddressSingle|WakeByAddressAll)\b|combase\.dll|bcryptprimitives\.dll|d3d12\.dll|api-ms-win-core-path-') {
      throw "$Name directly imports a known post-Windows-7 API"
    }
  }
  Invoke-Checked { & (Join-Path $Bin 'vectorcraft-cli.exe') --version }
  $Stage = Join-Path $TargetDir ('windows7-portable-' + [Guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Force $Stage | Out-Null
  Copy-Item (Join-Path $Bin 'vectorcraft.exe'), (Join-Path $Bin 'vectorcraft-cli.exe') $Stage
  Copy-Item LICENSE-MIT, LICENSE-APACHE, NOTICE, ASSETS.md $Stage
  Copy-Item -Recurse licenses $Stage
  Copy-Item vendor/windows-link/license-mit (Join-Path $Stage 'LICENSE-windows-link-MIT')
  Copy-Item vendor/windows-link/license-apache-2.0 (Join-Path $Stage 'LICENSE-windows-link-Apache-2.0')
  Copy-Item docs/windows7.md (Join-Path $Stage 'WINDOWS7.md')
  # portable.txt switches on portable mode, as in the regular portable zip (#1366).
  Copy-Item (Join-Path $PSScriptRoot 'portable.txt') $Stage
  if ($env:CRAFT_FONTS_DIR) {
    foreach ($ofl in Get-ChildItem (Join-Path $env:CRAFT_FONTS_DIR 'fonts\*\OFL.txt')) {
      Copy-Item $ofl.FullName (Join-Path $Stage "OFL-$($ofl.Directory.Name).txt")
    }
  }
  New-Item -ItemType Directory -Force dist/release | Out-Null
  $Zip = Join-Path $Root 'dist/release/vectorcraft-windows7-x64-portable.zip'
  Compress-Archive -Path (Join-Path $Stage '*') -DestinationPath $Zip -Force
  Remove-Item -Recurse -Force $Stage
  Get-Item $Zip
  if ($Test) {
    # Exercises the Windows code paths on the build host only; it doesn't prove Windows 7 runtime support.
    Invoke-Checked { cargo "+$Toolchain" test -Z build-std=std,panic_unwind,test @CargoArgs }
  }
} finally {
  [IO.File]::WriteAllBytes($Lock, $LockBytes)
  [Environment]::SetEnvironmentVariable($FlagName, $OldFlags)
  $env:VECTORCRAFT_REQUIRE_WINRES = $OldWinres
  Pop-Location
}
