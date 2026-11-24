param(
    [Parameter(Mandatory = $true)]
    [string]$Package
)

$ErrorActionPreference = "Stop"

# Runs one consumer against a fresh local mock provider. The provider prints
# its port on the first standard output line, and the consumer receives the
# provider base URL as its only argument. Both must exit with 0: the consumer
# checks the results it received, and the provider checks the requests it
# served.
function Invoke-ConsumerWithMockProvider {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$MockProvider,
        [Parameter(Mandatory = $true)][string]$Consumer,
        [Parameter(Mandatory = $false)][string]$ConsumerArguments = ""
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new($MockProvider)
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $provider = [System.Diagnostics.Process]::Start($startInfo)
    try {
        $portLine = $provider.StandardOutput.ReadLine()
        $port = 0
        if (-not [int]::TryParse($portLine, [ref]$port) -or $port -le 0) {
            throw "$Name mock provider did not report a port: '$portLine'"
        }

        # Run consumer with optional arguments (e.g., "dotnet-guide")
        & $Consumer $ConsumerArguments "http://127.0.0.1:$port"
        if ($LASTEXITCODE -ne 0) { throw "$Name consumer execution failed: $LASTEXITCODE" }

        if (-not $provider.WaitForExit(30000)) {
            throw "$Name mock provider did not finish after the consumer exited."
        }
        if ($provider.ExitCode -ne 0) {
            throw "$Name mock provider reported unexpected requests: $($provider.ExitCode)"
        }
    } finally {
        if (-not $provider.HasExited) {
            $provider.Kill()
            [void]$provider.WaitForExit(5000)
        }
        $provider.Dispose()
    }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$fixture = Join-Path $repoRoot "native/consumer-smoke/windows"
$workDir = Join-Path ([System.IO.Path]::GetTempPath()) ("native-oidc-release-consumer-" + [guid]::NewGuid())

try {
    Expand-Archive -Path $Package -DestinationPath $workDir
    $packageRoot = Get-Item $workDir
    if (-not (Test-Path (Join-Path $packageRoot.FullName "include/native_oidc_windows.h"))) {
        $packageRoot = Get-ChildItem -Path $workDir -Directory | Select-Object -First 1
    }
    if ($null -eq $packageRoot -or -not (Test-Path (Join-Path $packageRoot.FullName "include/native_oidc_windows.h"))) {
        throw "Windows release archive has no package root with include/native_oidc_windows.h."
    }
    foreach ($file in "LICENSE", "THIRD_PARTY_LICENSES.txt", "RUST_STANDARD_LIBRARY_COPYRIGHT.html") {
        $path = Join-Path $packageRoot.FullName $file
        if (-not (Test-Path $path) -or (Get-Item $path).Length -eq 0) {
            throw "Windows release archive is missing $file."
        }
    }

    $mockBuild = Join-Path $workDir "mock-provider-build"
    dotnet build (Join-Path $fixture "mock-provider/NativeOidcMockProvider.csproj") `
        --configuration Release `
        --runtime win-x64 --self-contained true `
        "--output=$mockBuild"
    if ($LASTEXITCODE -ne 0) { throw "Mock provider build failed: $LASTEXITCODE" }
    $mockProvider = Join-Path $mockBuild "NativeOidcMockProvider.exe"

    $cppBuild = Join-Path $workDir "cpp-build"
    cmake -S (Join-Path $fixture "cpp") -B $cppBuild -A x64 "-DNATIVE_OIDC_PACKAGE=$($packageRoot.FullName)"
    if ($LASTEXITCODE -ne 0) { throw "C++ consumer configure failed: $LASTEXITCODE" }
    cmake --build $cppBuild --config Release
    if ($LASTEXITCODE -ne 0) { throw "C++ consumer build failed: $LASTEXITCODE" }
    Invoke-ConsumerWithMockProvider -Name "C++" -MockProvider $mockProvider `
        -Consumer (Join-Path $cppBuild "Release/native_oidc_release_consumer.exe")

    $dotnetBuild = Join-Path $workDir "dotnet-build"
    dotnet build (Join-Path $fixture "dotnet/NativeOidcReleaseConsumer.csproj") `
        --configuration Release `
        --runtime win-x64 --self-contained true `
        "--property:NativeOidcPackage=$($packageRoot.FullName)" `
        "--output=$dotnetBuild"
    if ($LASTEXITCODE -ne 0) { throw ".NET consumer build failed: $LASTEXITCODE" }
    Invoke-ConsumerWithMockProvider -Name ".NET" -MockProvider $mockProvider `
        -Consumer (Join-Path $dotnetBuild "NativeOidcReleaseConsumer.exe")

    # Build and run the .NET guide examples consumer
    $guideAppBuild = Join-Path $workDir "dotnet-guide-app-build"
    dotnet build (Join-Path $fixture "dotnet/NativeOidcGuideApp.csproj") `
        --configuration Release `
        --runtime win-x64 --self-contained true `
        "--property:NativeOidcPackage=$($packageRoot.FullName)" `
        "--output=$guideAppBuild"
    if ($LASTEXITCODE -ne 0) { throw ".NET guide app build failed: $LASTEXITCODE" }

    # Run the .NET guide examples consumer with dotnet-guide mode
    Invoke-ConsumerWithMockProvider -Name ".NET guide examples" -MockProvider $mockProvider `
        -Consumer (Join-Path $guideAppBuild "NativeOidcGuideApp.exe")
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $workDir
}
