// Runs one authorization through native_oidc_windows_authorize, which opens
// the authorization URL with ShellExecuteW in the user's default browser,
// against scripts/real_browser_mock_provider.py. The check itself is in
// RealBrowserCheck.cs, which the in-process mode (run-in-process.ps1) also
// uses.
//
// Usage: NativeOidcRealBrowserCheck <provider base URL> [timeout seconds]
// See docs/real-browser-verification.md.
using System;
using ProjectFlora.NativeOidc.RealBrowser;

if (args.Length is < 1 or > 2
    || (args.Length == 2 && (!double.TryParse(args[1], out var parsed) || parsed <= 0)))
{
    Console.Error.WriteLine("usage: NativeOidcRealBrowserCheck <provider base URL> [timeout seconds]");
    return 2;
}
var timeout = TimeSpan.FromSeconds(args.Length == 2 ? double.Parse(args[1]) : 120);
return await RealBrowserCheck.RunAsync(args[0], timeout);
