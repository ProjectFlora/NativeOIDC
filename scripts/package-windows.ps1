# Builds the Windows release library and packages it as <Package>.zip, with
# the header, the .NET wrapper, LICENSE and the license notices at the zip
# root. Run scripts/generate-third-party-licenses.sh first (from bash, with
# the same Rust toolchain); the package takes its notices from
# target/licenses/. Fails when the DLL's version resource does not name
# NativeOIDC at the crate version.
param(
    [Parameter(Mandatory = $true)]
    [string]$Package
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $repoRoot

try {
    $notices = "target/licenses/THIRD_PARTY_LICENSES.txt", "target/licenses/RUST_STANDARD_LIBRARY_COPYRIGHT.html"
    foreach ($notice in $notices) {
        if (-not (Test-Path $notice)) {
            throw "$notice is missing; run scripts/generate-third-party-licenses.sh first."
        }
    }

    cargo build --locked --release --lib --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "Windows release build failed: $LASTEXITCODE" }
    $target = "target/x86_64-pc-windows-msvc/release"

    # The version resource that build.rs links into the DLL (Issue #258):
    # SignPath enforces the product name and version of signed binaries, so
    # a DLL without them, or with another crate version, is not packaged.
    $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed: $LASTEXITCODE" }
    $crateVersion = ($metadata.packages | Where-Object { $_.name -eq "native-oidc" }).version
    if (-not $crateVersion) { throw "cargo metadata has no native-oidc package." }
    $numericVersion = ($crateVersion -split "[-+]")[0] + ".0"
    $dll = (Resolve-Path "$target/native_oidc.dll").Path
    $versionInfo = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($dll)
    $expected = [ordered]@{
        ProductName      = "NativeOIDC"
        ProductVersion   = $crateVersion
        FileVersion      = $crateVersion
        CompanyName      = "ProjectFlora"
        OriginalFilename = "native_oidc.dll"
        InternalName     = "native_oidc"
    }
    foreach ($field in $expected.Keys) {
        if ($versionInfo.$field -ne $expected[$field]) {
            throw "native_oidc.dll version resource has $field '$($versionInfo.$field)', expected '$($expected[$field])'."
        }
    }
    $numericFile = "$($versionInfo.FileMajorPart).$($versionInfo.FileMinorPart).$($versionInfo.FileBuildPart).$($versionInfo.FilePrivatePart)"
    $numericProduct = "$($versionInfo.ProductMajorPart).$($versionInfo.ProductMinorPart).$($versionInfo.ProductBuildPart).$($versionInfo.ProductPrivatePart)"
    if ($numericFile -ne $numericVersion -or $numericProduct -ne $numericVersion) {
        throw "native_oidc.dll numeric version is $numericFile (product $numericProduct), expected $numericVersion."
    }
    Write-Output "native_oidc.dll version resource: $($versionInfo.ProductName) $($versionInfo.ProductVersion) ($numericFile)"

    New-Item -ItemType Directory -Force "$Package/include" | Out-Null
    New-Item -ItemType Directory -Force "$Package/dotnet" | Out-Null
    Copy-Item "$target/native_oidc.dll" $Package
    Copy-Item "$target/native_oidc.dll.lib" $Package
    Copy-Item "native/windows/include/native_oidc_windows.h" "$Package/include"
    Copy-Item "native/windows/dotnet/NativeOidcWindows.cs" "$Package/dotnet"
    Copy-Item "native/windows/dotnet/NativeOidcWindows.csproj" "$Package/dotnet"
    Copy-Item "native/windows/README.md" $Package
    Copy-Item "LICENSE" $Package
    Copy-Item $notices $Package
    Compress-Archive -Path "$Package/*" -DestinationPath "$Package.zip" -Force
} finally {
    Pop-Location
}
