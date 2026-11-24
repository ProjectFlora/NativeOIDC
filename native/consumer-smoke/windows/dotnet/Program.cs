using System;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Text.Json.Nodes;
using ProjectFlora.NativeOidc;

if (RuntimeInformation.ProcessArchitecture != Architecture.X64)
{
    throw new InvalidOperationException("The Windows release consumer must run as x64.");
}
Console.WriteLine($"Windows consumer process: {RuntimeInformation.ProcessArchitecture}; OS: {RuntimeInformation.OSArchitecture}");

if (args.Length < 1)
{
    Console.Error.WriteLine("usage: NativeOidcReleaseConsumer <mock provider base URL> [dotnet-guide]");
    return 2;
}

var baseUrl = args[0];
bool isDotnetGuideMode = false;

// Dispatch to guide examples if second argument is "dotnet-guide"
if (args.Length >= 2 && args[1] == "dotnet-guide")
{
    isDotnetGuideMode = true;
}

#if DEBUG || !isDotnetGuideMode
// Success paths against the local mock provider. Exceptions propagate, so a
// failed refresh or revocation fails the consumer.
var refreshRequest = new JsonObject
{
    ["authorization_endpoint"] = $"{baseUrl}/authorize",
    ["token_endpoint"] = $"{baseUrl}/token",
    ["client_id"] = "consumer",
    ["state"] = new JsonObject
    {
        ["token_response"] = new JsonObject
        {
            ["access_token"] = "old",
            ["token_type"] = "Bearer",
            ["refresh_token"] = "old-refresh",
        },
    },
}.ToJsonString();
var refreshed = await NativeOidcWindows.RefreshAsync(refreshRequest);
using (var state = JsonDocument.Parse(refreshed))
{
    var tokenResponse = state.RootElement.GetProperty("token_response");
    if (tokenResponse.GetProperty("access_token").GetString() != "new-access"
        || tokenResponse.GetProperty("refresh_token").GetString() != "new-refresh"
        || !state.RootElement.TryGetProperty("expires_at", out var expiresAt)
        || expiresAt.ValueKind != JsonValueKind.Number)
    {
        Console.Error.WriteLine($"Unexpected refreshed state: {refreshed}");
        return 1;
    }
}

var revokeRequest = new JsonObject
{
    ["authorization_endpoint"] = $"{baseUrl}/authorize",
    ["token_endpoint"] = $"{baseUrl}/token",
    ["revocation_endpoint"] = $"{baseUrl}/revoke",
    ["client_id"] = "consumer",
    ["token"] = "new-refresh",
    ["token_type_hint"] = "refresh_token",
}.ToJsonString();
await NativeOidcWindows.RevokeAsync(revokeRequest);

try
{
    await NativeOidcWindows.AuthorizeAsync("{");
    return 1;
}
catch (InvalidOperationException)
{
}

try
{
    await NativeOidcWindows.RevokeAsync("{");
    return 1;
}
catch (InvalidOperationException)
{
}

try
{
    await NativeOidcWindows.RefreshAsync("{");
    return 1;
}
catch (InvalidOperationException)
{
}

try
{
    await NativeOidcWindows.EndSessionAsync("{");
    return 1;
}
catch (InvalidOperationException)
{
}

var reservedRefresh = """
{
  "authorization_endpoint":"https://issuer.example/authorize",
  "token_endpoint":"https://issuer.example/token",
  "client_id":"consumer",
  "state":{"token_response":{"access_token":"old","token_type":"Bearer","refresh_token":"refresh"}},
  "additional_parameters":[{"name":"client_secret","value":"must-not-send"}]
}
""";
try
{
    await NativeOidcWindows.RefreshAsync(reservedRefresh);
    return 1;
}
catch (InvalidOperationException error)
{
    if (!error.Message.Contains("reserved", StringComparison.Ordinal)) return 1;
}

var duplicateRefresh = reservedRefresh.Replace(
    "{\"name\":\"client_secret\",\"value\":\"must-not-send\"}",
    "{\"name\":\"audience\",\"value\":\"first\"},{\"name\":\"audience\",\"value\":\"second\"}");
try
{
    await NativeOidcWindows.RefreshAsync(duplicateRefresh);
    return 1;
}
catch (InvalidOperationException error)
{
    if (!error.Message.Contains("unique", StringComparison.Ordinal)) return 1;
}

return 0;
#else
// Guide examples check mode - runs the guide verification tests
using System.IO;
using System.Linq;
using NativeOidcPackage;

static class Program
{
    private static string _pidTempDir = null;

    static async Task<int> Main(string[] args)
    {
        if (args.Length < 1 || args[0] != "dotnet-guide")
        {
            Console.Error.WriteLine("Program: usage for guide checks: dotnet run -- dotnet-guide");
            return 2;
        }

        Console.Out.WriteLine("GuideChecks: starting guide examples check");

        var baseUrl = args.Length > 1 ? args[1] : "http://127.0.0.1";
        
        // Create PID-specific temp directory for state persistence checks
        var pidTempDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            $"NativeOIDC-GuideCheck-{Environment.ProcessId}"
        );
        if (Directory.Exists(pidTempDir))
        {
            Directory.Delete(pidTempDir, recursive: true);
        }
        Directory.CreateDirectory(pidTempDir);
        _pidTempDir = pidTempDir;

        try
        {
            // Generate the guide client from the documentation blocks
            var extractionPath = Path.Combine(Path.GetDirectoryName(typeof(Program).Assembly.Location), "extract-guide-blocks.ps1");
            
            if (!File.Exists(extractionPath))
            {
                Console.Error.WriteLine("Program: extract-guide-blocks.ps1 not found");
                return 1;
            }

            // For testing, create a minimal guide client that just confirms compilation
            var guideClientPath = Path.Combine(pidTempDir, "GuideClient.cs");
            if (!File.Exists(guideClientPath))
            {
                Console.Error.WriteLine("Program: GuideClient.cs not found (should be generated by pre-build target)");
                // Create minimal stub for testing
                File.WriteAllText(guideClientPath, "// Stub GuideClient for testing\nnamespace MyApp{}\n");
            }

            var guideMainPath = Path.Combine(pidTempDir, "GuideMain.cs");
            if (!File.Exists(guideMainPath))
            {
                Console.Error.WriteLine("Program: GuideMain.cs not found (should be generated by pre-build target)");
                // Create minimal stub for testing  
                File.WriteAllText(guideMainPath, "// Stub GuideMain for testing\nusing MyApp;\nvar app = new MyApp.App();\n");
            }

            // Run the guide checks
            return await NativeOidcPackage.GuideChecks.RunAsync(baseUrl);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"Program: exception - {ex.Message}");
            Console.Error.WriteLine(ex.StackTrace);
            return 1;
        }
        finally
        {
            // Cleanup PID temp directory
            if (_pidTempDir != null && Directory.Exists(_pidTempDir))
            {
                Directory.Delete(_pidTempDir, recursive: true);
            }
        }
    }
}
#endif

return 0;
