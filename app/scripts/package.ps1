# Builds the release exe and a portable zip into dist-release\ at the repository root:
#   DiskScanner-<version>.exe           - keeps its data in %APPDATA%
#   DiskScanner-<version>-portable.zip  - exe plus portable.txt: data stays beside it
# Run from app\:  npm run release   (or from the repository root: npm --prefix app run release)
#
# The exe is signed when a certificate is configured; see sign.ps1 for the
# variables. With -RequireSignature an unsigned build is an error.
param([switch] $RequireSignature)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot)

# The exe embeds the UI; WebView2 ships with Windows 10 and 11, so no installer is needed.
npx tauri build --no-bundle
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$version = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
$exe = Resolve-Path "../target/release/disk-scanner.exe"

# Signed before copying, so the exe and the one in the zip carry the signature.
$signed = & (Join-Path $PSScriptRoot "sign.ps1") -File $exe
if (-not $signed) {
    if ($RequireSignature) { throw "no signing certificate configured (see app\scripts\sign.ps1)" }
    Write-Warning "Not signed: no certificate configured (see app\scripts\sign.ps1). Windows SmartScreen will warn about an unknown publisher."
}
$out = Join-Path (Resolve-Path "..") "dist-release"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item $exe (Join-Path $out "DiskScanner-$version.exe") -Force

$stage = Join-Path $out "portable-stage"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory $stage | Out-Null
Copy-Item $exe (Join-Path $stage "DiskScanner.exe")
@"
Disk Scanner, portable mode.

While this file lies next to DiskScanner.exe, settings and snapshots are kept
in the "data" folder beside it, not in %APPDATA%. Delete this file to switch
back to the normal mode.
"@ | Set-Content (Join-Path $stage "portable.txt") -Encoding utf8
$zip = Join-Path $out "DiskScanner-$version-portable.zip"
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip -Force
Remove-Item -Recurse -Force $stage

Get-ChildItem $out -File | Format-Table Name, @{ n = "MB"; e = { [math]::Round($_.Length / 1MB, 1) } }
