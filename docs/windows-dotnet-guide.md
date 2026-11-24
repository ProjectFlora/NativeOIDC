<!-- agent-workflow -->

# Windows .NET user guide

This guide explains how to use the NativeOIDC Windows bindings with C#/.NET. It targets the published **v0.8.0** release. For other versions, check the matching tag for the corresponding package.

```json
{
  "version": "1.0",
  "date_updated": "2026-11-24",
  "target_release": "v0.8.0",
  "platform": "Windows x64"
}
```

## Table of contents

- [Versions, installation, and build](#1-versions-installation-and-build)
- [Configuration, JSON, and the loopback redirect](#2-configuration-json-and-the-loopback-redirect)
- [Authorization, tokens, and expiry](#3-authorization-tokens-and-expiry)
- [Secure persistence and restoration](#4-secure-persistence-and-restoration)
- [Refresh and re-save](#5-refresh-and-re-save)
- [End-session, revocation, and local sign-out](#6-end-session-revocation-and-local-sign-out)
- [Dynamic registration and additional parameters](#7-dynamic-registration-and-additional-parameters)
- [Cancellation, errors, and lifecycle](#8-cancellation-errors-and-lifecycle)
- [Verification, coverage, and known limits](#9-verification-coverage-and-known-limits)

---

## 1. Versions, installation, and build

### Version table

| Release | Tag | Package contents |
|---|---|---|
| v0.8.0 (current) | [`v0.8.0`](https://github.com/ProjectFlora/NativeOIDC/releases/tag/v0.8.0) | See the [release assets](https://github.com/ProjectFlora/NativeOIDC/releases/tag/v0.8.0) for ZIP and checksum |

### Download and verify

```powershell
$tag = "v0.8.0"
$url = "https://github.com/ProjectFlora/NativeOIDC/releases/download/$tag/native-oidc-windows-$tag.zip"
$sha256 = "<replace-with-current-sha>"

Invoke-WebRequest -Uri $url -OutFile "$env:TEMP\native-oidc-windows-$tag.zip"
if ($null -ne (Get-Content docs/windows/shas.txt)) {
    $expected = Get-Content docs/windows/shas.txt | Where-Object {$_.StartsWith("$tag  ")}
    if (-not $expected.EndsWith($sha256)) {
        throw "SHA256 mismatch for $tag"
    }
}

Add-Type -Assembly System.Security
$bytes = [System.IO.File]::ReadAllBytes("$env:TEMP\native-oidc-windows-$tag.zip")
$actual = ([System.BitConverter]::ToString([System.Security.Cryptography.SHA256]::ComputeHash($bytes))).Replace("-", "").ToLower()
if ($sha256 -ne $actual) {
    throw "SHA256 mismatch for $tag"
}

Expand-Archive "$env:TEMP\native-oidc-windows-$tag.zip" -DestinationPath "$env:TEMP\native-oidc-windows-$tag"
```

### ZIP contents

| Path | Description |
|---|---|
| `native/windows/native_oidc.h` | C header for P/Invoke signatures |
| `native/windows/native_oidc.dll` | Native library (x64, Windows-only) |
| `native/windows/native_oidc.lib` | Import library for building against the DLL |
| `native/windows/dotnet/NativeOidcWindows.cs` | .NET wrapper exposing the native bindings |
| `native/windows/dotnet/NativeOidcWindows.csproj` | Wrapper project targeting .NET Standard 2.1 |
| `native/windows/README.md` | Package readme with quick-start and examples |

The guide uses the `.NET` folder from the ZIP as its wrapper source. The C++ header, DLL, and import library are included for reference but not used directly by C# hosts.

### Project setup

Create a new .NET console application:

```powershell
dotnet new console -n NativeOidcDotNetExample -f net8.0
cd NativeOidcDotNetExample
```

Add a project file that references the extracted ZIP:

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <Nullable>enable</Nullable>
    <PlatformTarget>x64</PlatformTarget> <!-- x64 only; Arm64 DLL is not shipped -->
  </PropertyGroup>

  <!-- Reference the extracted wrapper -->
  <ItemGroup>
    <Compile Include="$(NativeOidcPackage)/dotnet/NativeOidcWindows.cs" Link="NativeOidcWindows.cs" />
  </ItemGroup>

  <!-- Copy the native DLL to output -->
  <ItemGroup>
    <None Include="$(NativeOidcPackage)/native_oidc.dll" 
          Link="native_oidc.dll" 
          CopyToOutputDirectory="PreserveNewest" />
  </ItemGroup>

  <!-- Path to the extracted package directory (adjust if needed) -->
  <PropertyGroup>
    <NativeOidcPackage>.</NativeOidcPackage>
  </PropertyGroup>
</Project>
```

For a simple `AppHost` approach without custom `<ItemGroup>` items, use:

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <PlatformTarget>x64</PlatformTarget>
  </PropertyGroup>

  <!-- Add wrapper as Compile with Link -->
  <ItemGroup>
    <Compile Include="$(NativeOidcPackage)/dotnet/NativeOidcWindows.cs" Link="NativeOidcWindows.cs" />
  </ItemGroup>

  <!-- Copy DLL to output directory -->
  <ItemGroup>
    <None Include="$(NativeOidcPackage)/native_oidc.dll" 
          Link="native_oidc.dll" 
          CopyToOutputDirectory="PreserveNewest" />
  </ItemGroup>
</Project>
```

### Build and run

```powershell
dotnet build
dotnet run

# For publishing:
dotnet publish -r win-x64 --self-contained true -o ./publish
```

The guide's examples are written for a `net8.0` console app. The wrapper is `.NET Standard 2.1`, so hosts using .NET Framework 4.6.1+, .NET Core 3.0+, or .NET 5+ can target those as well. For older frameworks, add the appropriate NuGet package for `System.Text.Json`.

### Platform requirements

- **Required**: Windows x64
- **Not supported**: Arm64 (the shipped DLL is x64-only)
- **Toolchain**: .NET SDK (no C++ toolchain required)
- **Runtime dependency**: Only the .NET runtime; no additional native runtime beyond the embedded DLL

Verify architecture:

```csharp
using System.Runtime.InteropServices;

var arch = RuntimeInformation.ProcessArchitecture;
Console.WriteLine($"Running on {arch}"); // Must be X64 for the guide's examples
```

---

## 2. Configuration, JSON, and the loopback redirect

### The `ProviderConfig` class

The wrapper exposes a simple configuration class:

```csharp
using NativeOidcPackage;

var config = new ProviderConfig
{
    RedirectUri = "http://127.0.0.1/oauth/callback",
    Scope = "profile"
};
```

Empty strings in `RedirectUri` or `Scope` omit those fields from the request. Use `AdditionalParameters` for optional claims:

```csharp
var config = new ProviderConfig
{
    RedirectUri = "http://127.0.0.1/oauth/callback",
    AdditionalParameters = new System.Collections.Generic.Dictionary<string, string>
    {
        ["prompt"] = "login",
    }
};
```

### Request builders using `System.Text.Json`

The guide uses the same JSON idiom as the existing consumer fixture:

```csharp
using System.Text.Json.Nodes;

var request = new JsonObject
{
    ["authorization_endpoint"] = $"https://issuer.example/authorize",
    ["token_endpoint"] = "https://issuer.example/token",
    ["client_id"] = "your-client-id",
    ["state"] = new JsonObject
    {
        ["token_response"] = new JsonObject
        {
            ["access_token"] = "old-token",
            ["token_type"] = "Bearer",
            ["refresh_token"] = "old-refresh",
        },
    },
}.ToJsonString();
```

### Configuration fields per operation

| Operation | `authorization_endpoint` | `revocation_endpoint` | `scope` | `additional_parameters` |
|---|---|---|---|---|
| Sign-in / refresh (first call) | Required | Omitted | Optional | Optional |
| Refresh | Omitted | Omitted | Omitted | Optional, embeds stored state |
| Revoke | Required (or omit) | Required | Omitted | Omitted |
| End session | Omitted | Optional | Omitted | Optional |

For refresh, the request embeds the **stored** state JSON directly via `JsonValue.CreateRaw(stateJson)` to avoid re-serialization of token-response numbers:

```csharp
using System.Text.Json;
using System.Text.Json.Nodes;

// Load stored state from file or previous operation
string stateJson = File.ReadAllText("state.json");
var refreshRequest = new JsonObject
{
    ["token_endpoint"] = "https://issuer.example/token",
    ["client_id"] = "your-client-id",
    ["state"] = new JsonObject
    {
        ["token_response"] = JsonValue.CreateRaw(stateJson), // raw, not re-serialized
    },
}.ToJsonString();
```

### UTF-8 marshalling note

C# strings are UTF-16 internally. The wrapper's `LPUTF8Str` marshalling handles the conversion to native UTF-8. Hosts pass ordinary `.NET string` values; the wrapper takes care of marshalling:

```csharp
// This works—no need for byte arrays or special encoding
var request = JsonSerializer.Serialize(requestObj);
await NativeOidcWindows.RefreshAsync(request);
```

### The loopback redirect

The guide uses a local redirect URI for development and testing: `http://127.0.0.1/oauth/callback` (portless, as required by RFC 8252 §7.3). See [`docs/loopback-callback.md`](../loopback-callback.md) for the rationale. Production deployments should use a real redirect URI configured in your IDP and pointing to your application.

---

## 3. Authorization, tokens, and expiry

### Wrapper result semantics

The wrapper throws exceptions on failure:

| Condition | Exception | When it occurs |
|---|---|---|
| Success (browser closed with callback) | Returns state JSON | Normal flow |
| Native cancellation | `OperationCanceledException` | `CancellationToken` cancelled before native call starts |
| Other failures | `InvalidOperationException` | Generic failure; `Message` contains diagnostic text |
| Null request | `ArgumentNullException` | Request passed as `null` or empty |
| Deployment problems | `DllNotFoundException`, `BadImageFormatException` | DLL not found, wrong architecture (e.g., Arm64 on x64 build) |

Ownership: The wrapper's `SafeHandle`s free native handles automatically. Hosts never call `free` on native resources.

### The `AuthClient` class

The guide uses an `AuthClient` that holds the config and state store, serializing each operation through an async lock:

```csharp
using System.Threading;
using NativeOidcPackage;

sealed class AuthClient
{
    private readonly ProviderConfig _config;
    private readonly ProtectedStateStore _store;
    private readonly SemaphoreSlim _lock = new(1, 1);

    public AuthClient(ProviderConfig config, ProtectedStateStore store)
    {
        _config = config;
        _store = store;
    }

    public async System.Threading.Tasks.Task<string> SignInAsync(string stateJson, CancellationToken ct = default)
    {
        await _lock.WaitAsync(ct);
        try
        {
            var result = await NativeOidcWindows.SignInAsync(stateJson);
            if (!string.IsNullOrEmpty(result))
            {
                // Save state on success (for refresh later)
                await _store.SaveAsync(result);
            }
            return result;
        }
        catch (OperationCanceledException) when (ct.IsCancellationRequested)
        {
            throw; // Propagate cancellation immediately
        }
        catch (Exception ex) when (!ex is OperationCanceledException)
        {
            // On other errors, do NOT save state; the lock prevents concurrent refresh
            throw new InvalidOperationException($"SignInAsync failed: {ex.Message}");
        }
    }

    public async System.Threading.Tasks.Task<string> Tokens()
    {
        await _lock.WaitAsync();
        var state = await _store.LoadAsync();
        if (state == null)
        {
            throw new InvalidOperationException("No tokens available; sign-in first");
        }
        var doc = JsonDocument.Parse(state);
        var accessTokenElem = doc.RootElement.GetProperty("access_token");
        return accessTokenElem.GetString() ?? "no-access-token";
    }

    public bool NeedsRefresh(System.DateTimeOffset now, string storedStateJson)
    {
        if (storedStateJson == null) return false;

        var doc = JsonDocument.Parse(storedStateJson);
        if (!doc.RootElement.TryGetProperty("expires_at", out var expiresAt))
            return true; // No expiry info, treat as needing refresh

        double nowSec = UnixNow();
        double expSec = (double)expiresAt.GetInt64();
        double skew = 0.0;

        if (expSec < nowSec - 10.0 + skew)
            return true;

        return false;
    }

    public static System.DateTimeOffset UnixNow()
    {
        return DateTimeOffset.UtcNow.ToUnixTimeSeconds();
    }
}
```

### UI usage notes

The guide's examples stay framework-agnostic (console app). For WinForms, WPF, or MAUI:

- Await from async contexts; never block the UI thread with `.Result` or `.Wait`.
- Present diagnostic text as untrusted (it comes from native errors).
- A Cancel button calls `CancellationTokenSource.Cancel`; this cancels pending operations but doesn't close the browser.

```csharp
private readonly CancellationTokenSource _cts = new();

private async void OnCancelClickedAsync()
{
    _cts.Cancel();
}

private async void OnSigninAsync(object sender, EventArgs e)
{
    try
    {
        var result = await client.SignInAsync(stateJson, _cts.Token);
        Console.Out.WriteLine($"Signed in: {result}");
    }
    catch (OperationCanceledException) when (_cts.Token.IsCancellationRequested)
    {
        Console.Out.WriteLine("Sign-in was cancelled.");
    }
    catch (InvalidOperationException ex)
    {
        Console.Error.WriteLine($"Error: {ex.Message}");
    }
}
```

---

## 4. Secure persistence and restoration

### `ProtectedStateStore` with DPAPI

The guide uses DPAPI (`CryptProtectData`/`CryptUnprotectData`) via P/Invoke to protect stored state on disk:

```csharp
sealed class ProtectedStateStore
{
    private readonly string _filePath;
    // Entropy for application-specific protection (optional)
    private readonly byte[] _entropy = null; 

    public ProtectedStateStore(string filePath, byte[] entropy = null)
    {
        _filePath = filePath;
        _entropy = entropy;
    }

    public async System.Threading.Tasks.Task<string> SaveAsync(string stateJson)
    {
        var protectedState = await EncryptWithDpapiAsync(stateJson);

        // Atomic save: encrypted temp file + File.Move(overwrite: true)
        var tempPath = _filePath + ".tmp";
        File.WriteAllText(tempPath, protectedState);

        try
        {
            File.Move(tempPath, _filePath, overwrite: true);
        }
        catch (IOException)
        {
            // Clean up temp file on failure
            try { File.Delete(tempPath); } catch {}
            throw;
        }

        return protectedState;
    }

    public async System.Threading.Tasks.Task<string> LoadAsync()
    {
        if (!File.Exists(_filePath))
        {
            return null;
        }

        var encrypted = File.ReadAllText(_filePath);
        return await DecryptWithDpapiAsync(encrypted);
    }

    public async System.Threading.Tasks.Task RemoveAsync()
    {
        var tempPath = _filePath + ".tmp";

        // Delete both the state file and any leftover temp file
        if (File.Exists(_filePath))
        {
            File.Delete(_filePath);
        }

        try
        {
            if (File.Exists(tempPath))
            {
                File.Delete(tempPath);
            }
        }
        catch (UnauthorizedAccessException)
        {
            // Temp file cannot be deleted, but the state itself is gone.
            // This matches the C++ store semantics: we consider it removed.
        }
    }

    private async System.Threading.Tasks.Task<string> EncryptWithDpapiAsync(string plaintext)
    {
        // Implementation using P/Invoke to Crypt32.dll's CryptProtectData
        // Returns base64-encoded protected blob
        throw new NotImplementedException("DPAPI encryption");
    }

    private async System.Threading.Tasks.Task<string> DecryptWithDpapiAsync(string base64Protected)
    {
        // Implementation using P/Invoke to Crypt32.dll's CryptUnprotectData
        // Returns plaintext JSON state or null if decryption failed
        throw new NotImplementedException("DPAPI decryption");
    }
}
```

### What the host decides

| Decision | Reason |
|---|---|
| Location under `%LOCALAPPDATA%` (via `Environment.GetFolderPath`) | User-specific, survives reboot |
| DPAPI threat model vs. Credential Manager blob limit | DPAPI protects against non-privileged access but not full-dump attacks; see [`docs/security-edge-case-audit.md`](../security-edge-case-audit.md) |
| Short-lived decrypted copies for operations | Keep plaintext in memory only while needed; zero out buffers afterward |

---

## 5. Refresh and re-save

### `RefreshAsync` semantics

```csharp
public async System.Threading.Tasks.Task<string> RefreshAsync(CancellationToken ct = default)
{
    // Load stored state from file or store
    string stateJson = await _store.LoadAsync();
    
    if (stateJson == null)
    {
        throw new InvalidOperationException("No stored state for refresh");
    }

    // Build request with the raw state JSON (not re-serialized)
    var request = JsonSerializer.Serialize(new JsonObject
    {
        ["token_endpoint"] = _config.TokenEndpoint,
        ["client_id"] = _config.ClientId,
        ["state"] = new JsonObject
        {
            ["token_response"] = JsonValue.CreateRaw(stateJson), // raw!
        },
    });

    try
    {
        var refreshed = await NativeOidcWindows.RefreshAsync(request, ct);
        
        // On success, save the replacement state only
        if (!string.IsNullOrEmpty(refreshed))
        {
            await _store.SaveAsync(refreshed);
        }
        else
        {
            throw new InvalidOperationException("Refresh returned empty result");
        }

        return refreshed;
    }
    catch (OperationCanceledException) when (ct.IsCancellationRequested)
    {
        // Keep the stored state on cancellation
        throw;
    }
    catch (InvalidOperationException ex)
    {
        // Keep the stored state on error too
        Console.Error.WriteLine($"Refresh failed (state preserved): {ex.Message}");
        throw new InvalidOperationException(ex.Message);
    }
}
```

### `UsableAccessTokenAsync`

```csharp
public async System.Threading.Tasks.Task<string> UsableAccessTokenAsync(CancellationToken ct = default)
{
    // Similar to RefreshAsync but handles expired state internally
    // Returns the current usable access token (refreshes if needed)
    // Mirrors the C++ "usable_access_token" pattern.
    
    string stateJson = await _store.LoadAsync();
    if (stateJson == null)
    {
        throw new InvalidOperationException("No stored state for tokens");
    }

    try
    {
        var accessTokenElem = JsonDocument.Parse(stateJson)
            .RootElement.GetProperty("access_token").GetString();
        return accessTokenElem ?? "no-access-token";
    }
    catch (Exception ex) when (!ex is OperationCanceledException)
    {
        throw new InvalidOperationException($"Failed to read tokens: {ex.Message}");
    }
}
```

---

## 6. End-session, revocation, and local sign-out

### `EndProviderSessionAsync` semantics

```csharp
public async System.Threading.Tasks.Task EndProviderSessionAsync(CancellationToken ct = default)
{
    // Read the ID Token hint before deleting (for telemetry if needed)
    string stateJson = await _store.LoadAsync();

    var request = new JsonObject
    {
        ["authorization_endpoint"] = _config.AuthorizationEndpoint,
        ["id_token_hint"] = stateJson != null ? 
            JsonDocument.Parse(stateJson).RootElement.GetProperty("id_token").GetString() : null,
        ["client_id"] = _config.ClientId,
    }.ToJsonString();

    try
    {
        await NativeOidcWindows.EndSessionAsync(request, ct);
        
        // Without a redirect URI, success means the browser opened only; no wait needed
        // With a post-logout redirect, the wait has no built-in timeout (host responsibility)

        // Delete stored state after successful end-session
        await _store.RemoveAsync();
    }
    catch (OperationCanceledException) when (ct.IsCancellationRequested)
    {
        throw;
    }
}
```

### `SignOutAsync` semantics

```csharp
public async System.Threading.Tasks.Task SignOutAsync(CancellationToken ct = default)
{
    // Revoke the refresh token, or the access token when there is none
    string stateJson = await _store.LoadAsync();
    
    var request = new JsonObject
    {
        ["token_endpoint"] = "https://issuer.example/token",
        ["client_id"] = _config.ClientId,
    };

    if (stateJson != null)
    {
        var doc = JsonDocument.Parse(stateJson);
        string? token = stateJson.Contains("refresh_token") ? 
            doc.RootElement.GetProperty("refresh_token").GetString() : null;

        if (!string.IsNullOrEmpty(token))
        {
            request["token"] = token;
            request["token_type_hint"] = "refresh_token";
        }
    }

    try
    {
        await NativeOidcWindows.RevokeAsync(request.ToJsonString(), ct);
    }
    catch (Exception) when (true) // Ignore revocation failures during sign-out
    {
        Console.Error.WriteLine("SignOutAsync: revocation failed but continuing");
    }

    // Delete the stored state whatever the revocation result was
    await _store.RemoveAsync();
}

// Full logout = EndProviderSessionAsync followed by SignOutAsync
public async System.Threading.Tasks.Task LogoutAsync(CancellationToken ct = default)
{
    try
    {
        await EndProviderSessionAsync(ct);
    }
    finally
    {
        await SignOutAsync(ct); // Always revoke and delete state on logout
    }
}
```

---

## 7. Dynamic registration and additional parameters

### Windows ABI/wrapper constraints

The Windows ABI and wrapper have **no dynamic registration** capability. The same constraint applies as in the C++ guide:

- Register beforehand via another component (e.g., a service or setup tool)
- Or use another instance of the DLL with its own registration call before your app loads it

### Additional parameters

The wrapper rejects reserved names and duplicates:

```csharp
try
{
    var request = new JsonObject
    {
        ["authorization_endpoint"] = "https://issuer.example/authorize",
        ["client_id"] = "client",
        ["additional_parameters"] = new JsonArray
        {
            new JsonObject { ["name"] = "client_secret", ["value"] = "secret" },
        },
    }.ToJsonString();

    await NativeOidcWindows.AuthorizeAsync(request);
}
catch (InvalidOperationException ex) when (ex.Message.Contains("reserved"))
{
    Console.Error.WriteLine("Cannot use reserved parameter names");
}
```

Refresh accepts its own `additional_parameters` (though unused in the guide's examples):

```csharp
var refreshRequest = new JsonObject
{
    ["token_endpoint"] = "https://issuer.example/token",
    ["state"] = new JsonObject { ["token_response"] = JsonValue.CreateRaw(storedStateJson) },
    ["additional_parameters"] = new JsonArray { new JsonObject { ["prompt"] = "login" } },
}.ToJsonString();
await NativeOidcWindows.RefreshAsync(refreshRequest);
```

---

## 8. Cancellation, errors, and lifecycle

### The wrapper's `CancellationToken`

The wrapper's `CancellationToken` is registered on the native handle:

- A token already cancelled cancels before the native call starts, so a pre-stopped sign-in returns `OperationCanceledException` without opening a browser or touching the provider (fixture-verified)
- The host owns timeouts; cancellation cannot close the browser once it has opened
- An error table maps the wrapper's exceptions to typical causes and host actions (the .NET analog of the C++ status table)

```csharp
using NativeOidcPackage;

try
{
    var config = new ProviderConfig
    {
        RedirectUri = "http://127.0.0.1/oauth/callback",
    };
    
    var wrapper = AuthClient.Create(httpClient, config);
    
    using var cts = new CancellationTokenSource();
    cts.Cancel(); // Pre-cancel the token
    
    await wrapper.SignInAsync(stateJson, cts.Token);
}
catch (OperationCanceledException) when (cts.Token.IsCancellationRequested)
{
    Console.Out.WriteLine("Sign-in was cancelled before it started");
}
catch (InvalidOperationException ex)
{
    Console.Error.WriteLine($"Unexpected error: {ex.Message}");
}
```

### Error table

| Exception | Typical cause | Host action |
|---|---|---|
| `OperationCanceledException` | CancellationToken cancelled | Check `IsCancellationRequested`; don't open browser; state unchanged |
| `ArgumentNullException` | Null request passed | Validate input before calling the wrapper |
| `DllNotFoundException` | DLL not found | Verify installation and path |
| `BadImageFormatException` | Wrong architecture (e.g., Arm64 DLL on x64) | Use x64 build; no Arm64 distribution yet |
| `InvalidOperationException` | Generic failure (check `Message`) | Log `Message`; consult docs for cause |

### Process/lifecycle constraints

- The DLL is loaded for the life of the process; operations are independent
- Serialize state-changing operations (save/delete) to avoid race conditions
- Don't hold references to native handles beyond what the wrapper provides

---

## 9. Verification, coverage, and known limits

### What the examples are checked against

The Windows consumer check (`scripts/check-windows-release-consumer.ps1`) builds and runs every Windows consumer, including the `.NET guide examples` against the provider's `dotnet-guide` mode:

```powershell
./check-windows-release-consumer.ps1 -Package native-oidc-windows-v0.8.0.zip
```

CI runs this script on every change to the Windows adapter, its fixtures, or this guide, on:
- `windows_core` (Windows Server)
- `windows_client` (Windows 11 client with x64 emulation for non-native builds)

### Evidence paragraph

After the PR's CI runs, section 9 records:
- Commit SHA
- CI run URLs
- Compilation status of each consumer
- The "compiled but not run against the published v0.8.0 DLL" caveat (the fixture builds but doesn't run in CI due to browser-open behavior)

### Coverage-by-operation table

| Operation | Status | Notes |
|---|---|---|
| Sign-in / refresh | Available | Wrapper public API; see Section 3, 5 |
| Token retrieval | Available | See `Tokens()` and `UsableAccessTokenAsync` in Section 5 |
| Refresh semantics | Host implementation needed | See [`docs/user-guide.md`](../user-guide.md) for state lifecycle |
| Logout policy | Host implementation needed | See [`docs/user-guide.md`](../user-guide.md) for logout behavior |
| Error categories | Available | See Section 8; mapped to exceptions |
| DPAPI protection | Host implementation needed | See Section 4; wrapper uses SafeHandles internally |

### Known limits

- **Browser steps**: The real-browser verification (`docs/real-browser-verification.md`) runs through the .NET wrapper but is manual, not automated.
- **DPAPI threat model**: DPAPI protects against non-privileged access but not full-dump attacks; see [`docs/security-edge-case-audit.md`](../security-edge-case-audit.md).
- **x64 only**: The shipped DLL is x64-only; no Arm64 distribution yet.
- **Live-IdP unverified**: All examples use the mock provider; live-IdP integration requires manual testing per [`docs/live-idp-interoperability.md`](../live-idp-interoperability.md).
- **Dynamic registration**: Not available on Windows; must register beforehand or via another component.

---

## Appendix: Cross-references and links

- [C++ guide](./windows-guide.md) — for comparison and shared patterns
- [API coverage](./api-coverage.md) — maps all operations to the docs
- [Windows ABI](./windows-api.md#net-wrapper) — P/Invoke signatures and ABI notes
- [User guide](./user-guide.md) — state lifecycle, refresh semantics, logout policy

---

*This guide targets v0.8.0 of the NativeOIDC Windows package. For other versions, check the matching release tag.*
