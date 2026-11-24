// The real system-browser check, shared by the built program (Program.cs)
// and the in-process mode (run-in-process.ps1, which compiles this file with
// Add-Type), so that both send the same request and check the same result.
// See docs/real-browser-verification.md.
#nullable enable
using System;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Win32;

namespace ProjectFlora.NativeOidc.RealBrowser
{
    public static class RealBrowserCheck
    {
        private const string HttpAssociation =
            @"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http";

        // Runs one authorization against the provider at baseUrl and returns
        // the process exit code: 0 on success, 1 on failure.
        public static async Task<int> RunAsync(string baseUrl, TimeSpan timeout)
        {
            Console.WriteLine($"default http handler: {DefaultHttpHandler()}");

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
        }

        // ShellExecuteW opens http URLs with the handler the user chose.
        // Newer Windows 11 builds store that choice under UserChoiceLatest
        // and can leave an older value under UserChoice, so UserChoiceLatest
        // is read first. The result names the key it was read from.
        public static string DefaultHttpHandler()
        {
            foreach (var key in new[] { @"UserChoiceLatest\ProgId", "UserChoice" })
            {
                using var choice = Registry.CurrentUser.OpenSubKey($@"{HttpAssociation}\{key}");
                if (choice?.GetValue("ProgId") is string progId && progId.Length > 0)
                {
                    return $"{progId} ({key.Split('\\')[0]})";
                }
            }
            return "not set";
        }
    }
}
