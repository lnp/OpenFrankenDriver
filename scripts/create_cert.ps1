param(
    [string]$CertName = "OpenFrankenDriver",
    [string]$Password = "1234",
    [string]$OutputDir = "$PSScriptRoot\..\certs"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $OutputDir)) {
    New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
}

$pfxPath = Join-Path $OutputDir "$CertName.pfx"
$cerPath = Join-Path $OutputDir "$CertName.cer"

Write-Host "Creating self-signed code signing certificate: $CertName..."
$cert = New-SelfSignedCertificate -Subject "CN=$CertName" `
    -Type CodeSigningCert `
    -CertStoreLocation Cert:\CurrentUser\My `
    -NotAfter (Get-Date).AddYears(15)

$secPwd = ConvertTo-SecureString -String $Password -Force -AsPlainText

Write-Host "Exporting PFX to $pfxPath..."
Export-PfxCertificate -Cert $cert -FilePath $pfxPath -Password $secPwd | Out-Null

Write-Host "Exporting CER to $cerPath..."
Export-Certificate -Cert $cert -FilePath $cerPath | Out-Null

Write-Host "Registering certificate in local trusted root and trusted publisher stores..."
certutil -addstore -f Root $cerPath | Out-Null
certutil -addstore -f TrustedPublisher $cerPath | Out-Null

Write-Host "Certificate creation and local trust registration complete!"
