# Signs an exe with Authenticode, using whichever certificate is configured:
#
#   DISKSCANNER_SIGN_THUMBPRINT   SHA-1 thumbprint of a code signing certificate in
#                                 the Windows certificate store (CurrentUser or
#                                 LocalMachine \ My), e.g. on a USB token
#   DISKSCANNER_SIGN_PFX          path to a .pfx file, with
#   DISKSCANNER_SIGN_PFX_PASSWORD its password
#   DISKSCANNER_SIGN_COMMAND      any other signing command, e.g. for Azure Trusted
#                                 Signing; "{file}" in it is replaced by the exe path
#   DISKSCANNER_SIGN_TIMESTAMP    RFC 3161 timestamp server
#                                 (default http://timestamp.digicert.com)
#
# Returns $true if the file was signed, $false if no certificate is configured.
# A failed signing is an error.
param(
    [Parameter(Mandatory)] [string] $File
)
$ErrorActionPreference = "Stop"

function Find-SignTool {
    $inPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($inPath) { return $inPath.Source }
    # The Windows SDK puts one per version; take the newest x64 build.
    $kits = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    $found = Get-ChildItem $kits -Filter signtool.exe -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.Directory.Name -eq "x64" } |
        Sort-Object { [version]($_.Directory.Parent.Name -replace '[^\d.]', '') } -Descending |
        Select-Object -First 1
    if (-not $found) { throw "signtool.exe not found: install the Windows SDK (Signing Tools for Desktop Apps)" }
    return $found.FullName
}

$timestamp = if ($env:DISKSCANNER_SIGN_TIMESTAMP) { $env:DISKSCANNER_SIGN_TIMESTAMP } else { "http://timestamp.digicert.com" }

if ($env:DISKSCANNER_SIGN_COMMAND) {
    $command = $env:DISKSCANNER_SIGN_COMMAND.Replace("{file}", "`"$File`"")
    Write-Host "Signing with DISKSCANNER_SIGN_COMMAND"
    cmd /c $command | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "signing command failed ($LASTEXITCODE)" }
    return $true
}

$identity = $null
if ($env:DISKSCANNER_SIGN_THUMBPRINT) {
    $identity = @("/sha1", ($env:DISKSCANNER_SIGN_THUMBPRINT -replace '\s', ''))
} elseif ($env:DISKSCANNER_SIGN_PFX) {
    if (-not (Test-Path $env:DISKSCANNER_SIGN_PFX)) { throw "DISKSCANNER_SIGN_PFX not found: $env:DISKSCANNER_SIGN_PFX" }
    $identity = @("/f", $env:DISKSCANNER_SIGN_PFX)
    if ($env:DISKSCANNER_SIGN_PFX_PASSWORD) { $identity += @("/p", $env:DISKSCANNER_SIGN_PFX_PASSWORD) }
} else {
    return $false
}

$signtool = Find-SignTool
# SHA-256 digests and an RFC 3161 timestamp: the signature stays valid after the
# certificate expires.
& $signtool sign /fd SHA256 /tr $timestamp /td SHA256 /d "Disk Scanner" @identity $File | Out-Host
if ($LASTEXITCODE -ne 0) { throw "signtool sign failed ($LASTEXITCODE)" }

# Checks the signature against the trusted roots; a test certificate is not
# trusted, which is reported but is not an error.
& $signtool verify /pa /q $File | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Host "Signed and verified: $File"
} else {
    Write-Warning "Signed, but Windows does not trust the certificate (a test or self-signed one?): $File"
    # Not an error: keep the verify's exit code from failing the build.
    $global:LASTEXITCODE = 0
}
return $true
