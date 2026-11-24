# Runs one authorization through the real Windows system browser:
# native_oidc_windows_authorize opens the authorization URL with
# ShellExecuteW, scripts/real_browser_mock_provider.py redirects the browser
# to the loopback callback, and the library exchanges the code. Run it in an
# interactive desktop session. See docs/real-browser-verification.md.
#
# -Package is a Windows package ZIP: a release asset
# (native-oidc-windows-<version>.zip) or the output of
# scripts/package-windows.ps1.
#
# -InProcess runs the check inside a pwsh process (compiled with Add-Type)
# instead of building and running a program, for machines where an
# Application Control policy blocks locally built files. The provider,
# result files, and pass/fail decision are the same in both modes.
param(
    [Parameter(Mandatory = $true)]
    [string]$Package,
    [string]$ResultDirectory = "target/real-browser/windows",
    [int]$TimeoutSeconds = 120,
    [string]$Python = "python",
    [switch]$InProcess
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
New-Item -ItemType Directory -Force $ResultDirectory | Out-Null
$ResultDirectory = (Resolve-Path $ResultDirectory).Path
# Clear every file of the previous run first, so that a run that stops
# early (even on an invalid -Package path) cannot leave older evidence.
foreach ($name in "provider-result.json", "provider-result.json.tmp", "check-output.txt", "environment.txt", "provider-log.txt") {
    Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $ResultDirectory $name)
}
$Package = (Resolve-Path $Package).Path
$workDir = Join-Path ([System.IO.Path]::GetTempPath()) ("native-oidc-real-browser-" + [guid]::NewGuid())

try {
    Expand-Archive -Path $Package -DestinationPath $workDir
    $packageRoot = Get-Item $workDir
    if (-not (Test-Path (Join-Path $packageRoot.FullName "dotnet/NativeOidcWindows.cs"))) {
        $packageRoot = Get-ChildItem -Path $workDir -Directory | Select-Object -First 1
    }
    if ($null -eq $packageRoot -or -not (Test-Path (Join-Path $packageRoot.FullName "dotnet/NativeOidcWindows.cs"))) {
        throw "Windows package has no package root with dotnet/NativeOidcWindows.cs."
    }

    $checkSource = Join-Path $repoRoot "native/consumer-smoke/windows/real-browser"
    if ($InProcess) {
        $mode = "in-process"
        # A separate pwsh process, so that the loaded native_oidc.dll does
        # not keep the package directory locked, and the check's output and
        # exit code arrive as they do from the built program.
        $checkCommand = (Get-Process -Id $PID).Path
        $checkArguments = @(
            "-NoProfile", "-NonInteractive", "-File", (Join-Path $checkSource "run-in-process.ps1"),
            "-PackageRoot", $packageRoot.FullName)
    } else {
        $mode = "program"
        $build = Join-Path $workDir "real-browser-build"
        dotnet build (Join-Path $checkSource "NativeOidcRealBrowserCheck.csproj") `
            --configuration Release `
            --runtime win-x64 --self-contained true `
            "-p:NativeOidcPackage=$($packageRoot.FullName)" `
            "--output=$build"
        if ($LASTEXITCODE -ne 0) { throw "Real browser check build failed: $LASTEXITCODE" }
        $checkCommand = Join-Path $build "NativeOidcRealBrowserCheck.exe"
        $checkArguments = @()
    }

    $os = Get-CimInstance Win32_OperatingSystem
    @(
        "date: $([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ'))"
        "commit: $(git -C $repoRoot rev-parse HEAD 2>$null)"
        "package: $(Split-Path -Leaf $Package)"
        "os: $($os.Caption) $($os.Version) build $($os.BuildNumber)"
        "architecture: $env:PROCESSOR_ARCHITECTURE"
        "mode: $mode"
    ) | Set-Content (Join-Path $ResultDirectory "environment.txt")
    Get-Content (Join-Path $ResultDirectory "environment.txt")

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new($Python)
    foreach ($argument in @(
            (Join-Path $repoRoot "scripts/real_browser_mock_provider.py"),
            "--result-file", (Join-Path $ResultDirectory "provider-result.json"),
            "--timeout", "$($TimeoutSeconds + 30)")) {
        $startInfo.ArgumentList.Add($argument)
    }
    $startInfo.UseShellExecute = $false
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $provider = [System.Diagnostics.Process]::Start($startInfo)
    $providerLog = $provider.StandardError.ReadToEndAsync()
    try {
        $baseUrl = $provider.StandardOutput.ReadLine()
        if ($baseUrl -notmatch '^http://127\.0\.0\.1:\d+$') {
            throw "The mock provider did not report its address: '$baseUrl'"
        }

        # Keep the check's error output in check-output.txt as well. Merged
        # stderr lines arrive as error records, so they must not stop the
        # script, and they are written as plain text.
        $ErrorActionPreference = "Continue"
        & $checkCommand @checkArguments $baseUrl $TimeoutSeconds 2>&1 |
            ForEach-Object { "$_" } |
            Tee-Object -FilePath (Join-Path $ResultDirectory "check-output.txt")
        $checkStatus = $LASTEXITCODE
        $ErrorActionPreference = "Stop"
        if ($checkStatus -ne 0) {
            # The provider would otherwise wait for its own timeout.
            $provider.Kill()
        }
        [void]$provider.WaitForExit(($TimeoutSeconds + 60) * 1000)
        $providerStatus = if ($provider.HasExited) { $provider.ExitCode } else { "running" }
    } finally {
        if (-not $provider.HasExited) {
            $provider.Kill()
            [void]$provider.WaitForExit(5000)
        }
        $providerLog.Result | Set-Content (Join-Path $ResultDirectory "provider-log.txt")
        $provider.Dispose()
    }

    $resultFile = Join-Path $ResultDirectory "provider-result.json"
    if (Test-Path $resultFile) { Get-Content $resultFile } else { Get-Content (Join-Path $ResultDirectory "provider-log.txt") }
    if ($checkStatus -ne 0 -or $providerStatus -ne 0) {
        throw "Real system-browser check failed (library: $checkStatus, provider: $providerStatus)"
    }
    Write-Output "Real system-browser check passed; results in $ResultDirectory"
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $workDir
}
