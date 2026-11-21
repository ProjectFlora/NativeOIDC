// Runs one authorization through native_oidc_windows_authorize, which opens
// the authorization URL with ShellExecuteW in the user's default browser,
// against scripts/real_browser_mock_provider.py.
//
// Usage: NativeOidcRealBrowserCheck <provider base URL> [timeout seconds]
// See docs/real-browser-verification.md.
using System;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using Microsoft.Win32;
using ProjectFlora.NativeOidc;

if (args.Length is < 1 or > 2
    || (args.Length == 2 && (!double.TryParse(args[1], out var parsed) || parsed <= 0)))
{
    Console.Error.WriteLine("usage: NativeOidcRealBrowserCheck <provider base URL> [timeout seconds]");
    return 2;
}
var baseUrl = args[0];
var timeout = TimeSpan.FromSeconds(args.Length == 2 ? double.Parse(args[1]) : 120);

// ShellExecuteW opens http URLs with the handler the user chose.
using (var choice = Registry.CurrentUser.OpenSubKey(
    @"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice"))
{
    Console.WriteLine($"default http handler: {choice?.GetValue("ProgId") ?? "not set"}");
}

var request = new JsonObject
{
    ["authorization_endpoint"] = $"{baseUrl}/authorize",
    ["token_endpoint"] = $"{baseUrl}/token",
    ["client_id"] = "real-browser-check",
    ["redirect_uri"] = "http://127.0.0.1/oauth/callback",
    ["scopes"] = new JsonArray("profile"),
}.ToJsonString();

using var cancellation = new CancellationTokenSource(timeout);
try
{
    var state = await NativeOidcWindows.AuthorizeAsync(request, cancellation.Token);
    using var document = JsonDocument.Parse(state);
    var accessToken = document.RootElement
        .GetProperty("token_response")
        .GetProperty("access_token")
        .GetString();
    if (accessToken != "real-browser-access-token")
    {
        Console.Error.WriteLine("real system-browser authorization returned an unexpected access token");
        return 1;
    }
}
catch (OperationCanceledException)
{
    Console.Error.WriteLine($"real system-browser authorization failed: no callback within {timeout.TotalSeconds} seconds");
    return 1;
}
catch (Exception error)
{
    Console.Error.WriteLine($"real system-browser authorization failed: {error.Message}");
    return 1;
}
Console.WriteLine("real system-browser authorization completed");
return 0;
