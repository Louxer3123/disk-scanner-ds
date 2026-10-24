# Makes a self-signed code signing certificate as a .pfx file, to try signing
# locally. It is written to a file only: nothing is added to the Windows
# certificate stores, and Windows does not trust it, so SmartScreen still warns.
# Real releases need a certificate from a certificate authority (see README).
#
#   powershell -File scripts\new-test-certificate.ps1 -Out test-signing.pfx -Password secret
param(
    [string] $Out = "test-signing.pfx",
    [Parameter(Mandatory)] [string] $Password,
    [string] $Subject = "CN=Disk Scanner test signing"
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Security

$rsa = [System.Security.Cryptography.RSA]::Create(3072)
$request = [System.Security.Cryptography.X509Certificates.CertificateRequest]::new(
    $Subject, $rsa,
    [System.Security.Cryptography.HashAlgorithmName]::SHA256,
    [System.Security.Cryptography.RSASignaturePadding]::Pkcs1)
# Code signing only.
$usage = [System.Security.Cryptography.OidCollection]::new()
[void]$usage.Add([System.Security.Cryptography.Oid]::new("1.3.6.1.5.5.7.3.3"))
$request.CertificateExtensions.Add(
    [System.Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($usage, $true))
$request.CertificateExtensions.Add(
    [System.Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new(
        [System.Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature, $true))

$now = [DateTimeOffset]::Now
$cert = $request.CreateSelfSigned($now.AddMinutes(-5), $now.AddYears(1))
$bytes = $cert.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Pfx, $Password)
$path = [System.IO.Path]::GetFullPath($Out)
[System.IO.File]::WriteAllBytes($path, $bytes)
Write-Host "Test certificate: $path"
Write-Host "Thumbprint: $($cert.Thumbprint)"
Write-Host "Use it:  `$env:DISKSCANNER_SIGN_PFX = '$path'; `$env:DISKSCANNER_SIGN_PFX_PASSWORD = '<password>'"
