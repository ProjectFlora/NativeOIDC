# Runs the real system-browser check (RealBrowserCheck.cs) inside this
# PowerShell process instead of as a built program, for machines where an
# Application Control policy blocks locally built files. It compiles the
# package's dotnet/NativeOidcWindows.cs and RealBrowserCheck.cs with
# Add-Type and loads the package's native_oidc.dll, so it runs the same
# check as Program.cs. scripts/check-windows-real-browser.ps1 -InProcess
# starts it in a separate pwsh process, which writes and exits like the
# built program. See docs/real-browser-verification.md.
param(
    [Parameter(Mandatory = $true)]
    [string]$PackageRoot,
    [Parameter(Mandatory = $true)]
    [string]$BaseUrl,
    [Parameter(Mandatory = $true)]
    [int]$TimeoutSeconds
)

$ErrorActionPreference = "Stop"
Add-Type -Path @(
    (Join-Path $PackageRoot "dotnet/NativeOidcWindows.cs"),
    (Join-Path $PSScriptRoot "RealBrowserCheck.cs")
)
# The DllImport("native_oidc") declarations bind to the module that is
# already loaded under that name.
[void][System.Runtime.InteropServices.NativeLibrary]::Load((Join-Path $PackageRoot "native_oidc.dll"))
$status = [ProjectFlora.NativeOidc.RealBrowser.RealBrowserCheck]::RunAsync(
    $BaseUrl, [TimeSpan]::FromSeconds($TimeoutSeconds)).GetAwaiter().GetResult()
exit $status
