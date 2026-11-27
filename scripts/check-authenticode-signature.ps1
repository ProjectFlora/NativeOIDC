# Checks that a Windows PE file (for example native_oidc.dll) carries a
# valid, timestamped Authenticode signature embedded in the file (Issue #258).
#
# Fails unless Get-AuthenticodeSignature reports:
# - SignatureType Authenticode: the signature is embedded in the file. A
#   catalog signature (as on Windows system files) does not travel with a
#   copied DLL, so it does not count;
# - Status Valid: the file is unchanged since signing and the certificate
#   chains to a root this machine trusts;
# - a TimeStamperCertificate: the signature was timestamped, so it stays
#   valid after the signing certificate expires;
# - with -ExpectedSigner, a signer certificate subject that contains that
#   text (for example "SignPath Foundation").
#
# Uses only Get-AuthenticodeSignature (Windows PowerShell 5.1 and
# PowerShell 7 on Windows), so it needs no Windows SDK signtool.
param(
    [Parameter(Mandatory = $true)]
    [string]$Path,
    [string]$ExpectedSigner
)

$ErrorActionPreference = "Stop"

$file = (Resolve-Path -LiteralPath $Path).Path
$signature = Get-AuthenticodeSignature -LiteralPath $file
$signer = $signature.SignerCertificate
$timestamper = $signature.TimeStamperCertificate

$problems = @()
if ($signature.SignatureType -ne "Authenticode") {
    $problems += "signature type is '$($signature.SignatureType)', expected an embedded Authenticode signature"
}
if ($signature.Status -ne "Valid") {
    $problems += "status is '$($signature.Status)': $($signature.StatusMessage)"
}
if ($null -eq $timestamper) {
    $problems += "the signature has no timestamp"
}
if ($ExpectedSigner) {
    if ($null -eq $signer) {
        $problems += "there is no signer certificate, expected one for '$ExpectedSigner'"
    } elseif (-not $signer.Subject.Contains($ExpectedSigner)) {
        $problems += "signer is '$($signer.Subject)', expected a subject containing '$ExpectedSigner'"
    }
}

if ($problems.Count -gt 0) {
    throw "Authenticode check failed for ${file}: $($problems -join '; ')."
}

Write-Output "Authenticode signature of ${file}: Valid"
Write-Output "  Signer:      $($signer.Subject) (thumbprint $($signer.Thumbprint))"
Write-Output "  Timestamper: $($timestamper.Subject)"
