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

if (args.Length != 1)
{
    Console.Error.WriteLine("usage: NativeOidcReleaseConsumer <mock provider base URL>");
    return 2;
}
var baseUrl = args[0];

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
