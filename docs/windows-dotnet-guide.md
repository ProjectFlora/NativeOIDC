# Windows .NET user guide

Return to the [documentation index](README.md). This guide covers the
Windows-specific setup and the C# calls through the .NET wrapper that ships
in the Windows package. Behavior shared by every language (state lifecycle,
refresh semantics, logout policy, error categories) is described once in the
[shared user guide](user-guide.md) and is not repeated here. The wrapper
contract is in the [Windows ABI reference](windows-api.md#net-wrapper), the
request fields and their validation are in
[request and response fields](windows-api.md#request-and-response-fields),
and the JSON request examples for each operation are in the
[Windows adapter README](../native/windows/README.md). The
[Windows C++ user guide](windows-cpp-guide.md) covers the same operations
through the C ABI. The [API coverage matrix](api-coverage.md) lists every
operation.

The guide targets the published **v0.8.0** release. The Windows header, C
ABI, .NET wrapper, and request/response JSON on current `main` are unchanged
from v0.8.0. Check the matching tag when you install another version.

Every `csharp` block in this guide, together with the `Program.cs` of the
[minimal application](#minimal-application), is compiled by the Windows .NET
guide fixture, which also runs the representative paths against a local mock
provider. See [section 9](#9-verification-coverage-and-known-limits).

## 1. Versions, installation, and build

| Item | v0.8.0 |
| --- | --- |
| Wrapper | C# source, `ProjectFlora.NativeOidc.NativeOidcWindows`, targeting .NET Standard 2.1 |
| Hosts | Any .NET implementation of .NET Standard 2.1 on Windows: .NET Core 3.0 or later and .NET 5 or later. .NET Framework does not implement .NET Standard 2.1 |
| Examples | C# 12 on .NET 8 (`net8.0-windows`); they use `System.Text.Json`, which is part of .NET Core 3.0 and later |
| Architecture | x64 (`x86_64-pc-windows-msvc`). The process must be x64: Windows on Arm runs an x64 process under emulation; there is no Arm64 DLL |
| Toolchain | The .NET SDK. No C++ toolchain is needed |
| Windows versions | Microsoft-supported Windows client versions, reviewed each release (see the [support matrix](support-matrix.md)) |
| Runtime dependencies | The .NET runtime (or a self-contained publish). The DLL contains its HTTP client and async runtime |

### Download and verify

Download `native-oidc-windows-0.8.0.zip` and `SHA256SUMS` from the
[v0.8.0 release](https://github.com/ProjectFlora/NativeOIDC/releases/tag/v0.8.0),
then verify the checksum from the download directory and extract the ZIP
next to the project file:

```powershell
$asset = 'native-oidc-windows-0.8.0.zip'
$expected = ((Select-String SHA256SUMS -Pattern "  $([regex]::Escape($asset))$").Line -split '\s+')[0]
$actual = (Get-FileHash $asset -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw 'SHA-256 mismatch' }
Expand-Archive $asset -DestinationPath third_party/native-oidc-windows-0.8.0
```

The ZIP has no top-level directory, so the extracted
`third_party/native-oidc-windows-0.8.0` directory is the package root. It
contains:

| File | Use |
| --- | --- |
| `dotnet/NativeOidcWindows.cs` | The .NET wrapper; compile it into the application |
| `dotnet/NativeOidcWindows.csproj` | A .NET Standard 2.1 project for the wrapper alone, for hosts that prefer a project reference; this guide compiles the source file instead |
| `native_oidc.dll` | Runtime library; ship it next to the application |
| `include/native_oidc_windows.h`, `native_oidc.dll.lib` | C header and import library (not used from .NET) |
| `README.md` | The [Windows adapter README](../native/windows/README.md) of the release |
| `LICENSE`, `THIRD_PARTY_LICENSES.txt`, `RUST_STANDARD_LIBRARY_COPYRIGHT.html` | Ship these notices with the application (see [third-party licenses](../README.md#third-party-licenses)) |

### Project file

Compile the wrapper source into the application and copy the DLL to the
output directory. The examples also use DPAPI through the
`System.Security.Cryptography.ProtectedData` package:

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0-windows</TargetFramework>
    <Nullable>enable</Nullable>
    <!-- native_oidc.dll is x64 only. -->
    <PlatformTarget>x64</PlatformTarget>
    <RuntimeIdentifier>win-x64</RuntimeIdentifier>
    <NativeOidcPackage>$(MSBuildProjectDirectory)\third_party\native-oidc-windows-0.8.0</NativeOidcPackage>
  </PropertyGroup>
  <ItemGroup>
    <!-- Keep the package's own files out of the default **/*.cs glob. -->
    <Compile Remove="third_party\**" />
    <Compile Include="$(NativeOidcPackage)\dotnet\NativeOidcWindows.cs" Link="NativeOidc\NativeOidcWindows.cs" />
    <None Include="$(NativeOidcPackage)\native_oidc.dll" Link="native_oidc.dll" CopyToOutputDirectory="PreserveNewest" />
    <PackageReference Include="System.Security.Cryptography.ProtectedData" Version="8.0.0" />
  </ItemGroup>
</Project>
```

The project has two source files next to the project file: `AuthClient.cs`,
which holds every `csharp` block of this guide in order, and the
`Program.cs` of the [minimal application](#minimal-application). Build and
run:

```powershell
dotnet build --configuration Release
dotnet run --configuration Release
# A self-contained x64 application:
dotnet publish --configuration Release --self-contained true
```

Without `PlatformTarget` and `RuntimeIdentifier`, a project can run as an
Arm64 or x86 process, and loading `native_oidc.dll` then fails with
`BadImageFormatException`. A missing DLL fails with `DllNotFoundException`
on the first call. Both come from P/Invoke, not from NativeOIDC.

### Wrapper and usings

The wrapper is one static class, `NativeOidcWindows`, with one method per
operation (`AuthorizeAsync`, `RefreshAsync`, `EndSessionAsync`,
`RevokeAsync`). It converts strings to and from UTF-8, runs the blocking
native call on a thread-pool thread, connects the `CancellationToken` to the
native cancellation handle, and frees every native handle. The host never
calls the C functions or frees anything. The examples in this guide start
with these directives:

```csharp
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading;
using System.Threading.Tasks;
using ProjectFlora.NativeOidc;

namespace MyApp;
```

## 2. Configuration, JSON, and the loopback redirect

### Provider configuration

Register a native public client with the provider (no client secret) and
the loopback redirect URI, as described in the
[shared guide](user-guide.md#configure-a-native-public-client). Each
operation takes its configuration in its own JSON request, so the host keeps
the settings and builds a request per call. This record holds them; `null`
means "omit the field":

```csharp
public sealed record Parameter(string Name, string Value);

// Provider and client settings. Store them with the state: the state JSON
// does not record which provider or client it belongs to.
public sealed record ProviderConfig
{
    // Discovery: set Issuer and leave both endpoints null.
    // Explicit configuration: set both endpoints. For OIDC ID Token
    // validation also set Issuer and JwksUri together; with explicit
    // endpoints, an issuer without JwksUri is rejected.
    public string? Issuer { get; init; }
    public string? AuthorizationEndpoint { get; init; }
    public string? TokenEndpoint { get; init; }
    public string? JwksUri { get; init; }
    // Optional operation endpoints. With explicit configuration they are
    // required for end-session and revocation; with discovery they override
    // the discovered values.
    public string? EndSessionEndpoint { get; init; }
    public string? RevocationEndpoint { get; init; }

    public required string ClientId { get; init; }
    // Portless http://127.0.0.1 templates; the adapter picks the port.
    public required string RedirectUri { get; init; }
    public string? PostLogoutRedirectUri { get; init; }  // null: no post-logout callback
    public IReadOnlyList<string> Scopes { get; init; } = Array.Empty<string>();
    // Authorization request.
    public IReadOnlyList<Parameter> AdditionalParameters { get; init; } = Array.Empty<Parameter>();
    // Code exchange.
    public IReadOnlyList<Parameter> AdditionalTokenParameters { get; init; } = Array.Empty<Parameter>();
}
```

For example, discovery from an issuer, or explicit endpoints with OIDC
validation for a provider without a discovery document:

```csharp
public static class Configurations
{
    public static ProviderConfig Discovered() => new()
    {
        Issuer = "https://issuer.example",
        ClientId = "native-client",
        RedirectUri = "http://127.0.0.1/oauth/callback",
        PostLogoutRedirectUri = "http://127.0.0.1/oauth/logout",
        Scopes = new[] { "openid", "profile", "offline_access" },
    };

    public static ProviderConfig Explicit() => new()
    {
        Issuer = "https://issuer.example",
        AuthorizationEndpoint = "https://issuer.example/authorize",
        TokenEndpoint = "https://issuer.example/token",
        JwksUri = "https://issuer.example/jwks",
        EndSessionEndpoint = "https://issuer.example/endsession",
        RevocationEndpoint = "https://issuer.example/revoke",
        ClientId = "native-client",
        RedirectUri = "http://127.0.0.1/oauth/callback",
        Scopes = new[] { "openid", "profile", "offline_access" },
    };
}
```

Endpoint URL rules (HTTPS, and where loopback HTTP is allowed) are listed in
the [ABI reference](windows-api.md#request-and-response-fields). Discovery
runs inside each operation that uses `Issuer`; there is no standalone
discovery call and no way to inspect the resolved endpoints.

### Requests for each operation

Every request is a JSON object in a .NET `string`. The wrapper marshals it
as UTF-8, so the host passes ordinary strings and never converts encodings.
Build requests with `System.Text.Json.Nodes` instead of concatenating
strings, so that values are escaped correctly:

```csharp
public static class Requests
{
    public static string Authorization(ProviderConfig config)
    {
        var request = ConfigurationFields(config);
        SetOptional(request, "jwks_uri", config.JwksUri);
        request["redirect_uri"] = config.RedirectUri;
        var scopes = new JsonArray();
        foreach (var scope in config.Scopes)
        {
            scopes.Add(JsonValue.Create(scope));
        }
        request["scopes"] = scopes;
        request["additional_parameters"] = ParametersJson(config.AdditionalParameters);
        request["additional_token_parameters"] = ParametersJson(config.AdditionalTokenParameters);
        return request.ToJsonString();
    }

    // The stored state goes back as stored. JsonNode.Parse keeps the source
    // text of numbers, so integers in preserved token-response fields are
    // not rounded on the way back.
    public static string Refresh(ProviderConfig config, string stateJson)
    {
        var request = ConfigurationFields(config);
        SetOptional(request, "jwks_uri", config.JwksUri);
        request["state"] = JsonNode.Parse(stateJson);
        return request.ToJsonString();
    }

    public static string EndSession(ProviderConfig config, string? idTokenHint)
    {
        var request = ConfigurationFields(config);
        SetOptional(request, "end_session_endpoint", config.EndSessionEndpoint);
        SetOptional(request, "id_token_hint", idTokenHint);
        SetOptional(request, "post_logout_redirect_uri", config.PostLogoutRedirectUri);
        return request.ToJsonString();
    }

    public static string Revocation(ProviderConfig config, string token, string tokenTypeHint)
    {
        var request = ConfigurationFields(config);
        SetOptional(request, "revocation_endpoint", config.RevocationEndpoint);
        request["token"] = token;
        request["token_type_hint"] = tokenTypeHint;
        return request.ToJsonString();
    }

    // Fields that select the configuration, shared by every operation.
    private static JsonObject ConfigurationFields(ProviderConfig config)
    {
        var request = new JsonObject();
        SetOptional(request, "issuer", config.Issuer);
        SetOptional(request, "authorization_endpoint", config.AuthorizationEndpoint);
        SetOptional(request, "token_endpoint", config.TokenEndpoint);
        request["client_id"] = config.ClientId;
        return request;
    }

    private static void SetOptional(JsonObject request, string name, string? value)
    {
        if (value is not null)
        {
            request[name] = value;
        }
    }

    private static JsonArray ParametersJson(IEnumerable<Parameter> parameters)
    {
        var array = new JsonArray();
        foreach (var parameter in parameters)
        {
            array.Add(new JsonObject { ["name"] = parameter.Name, ["value"] = parameter.Value });
        }
        return array;
    }
}
```

The refresh request also accepts an `additional_parameters` array for the
refresh token request; the examples do not use it.

### Loopback redirect

`RedirectUri` and `PostLogoutRedirectUri` must be portless
`http://127.0.0.1/...` URIs. For each operation the adapter binds an
available port on `127.0.0.1`, puts that port in the URI it sends, and waits
for the browser to return to it. Register the redirect with the provider in
the form it accepts for loopback clients, usually the portless URI, as
[RFC 8252 section 7.3](https://www.rfc-editor.org/rfc/rfc8252#section-7.3)
describes. `localhost`, `[::1]`, a fixed port, HTTPS, and custom schemes are
rejected. The listener's behavior is specified in
[loopback callback](loopback-callback.md).

## 3. Authorization, tokens, and expiry

### Results and exceptions

Each wrapper method returns a task that completes when the native call has
returned:

- Success: `AuthorizeAsync` and `RefreshAsync` return the authentication
  state JSON, `EndSessionAsync` returns the logout response JSON or an empty
  string, and `RevokeAsync` returns no value.
- Cancellation: `OperationCanceledException`.
- Any other failure: `InvalidOperationException`, whose `Message` is
  diagnostic text, not a stable format.
- A `null` request throws `ArgumentNullException` before any work starts.

Each call creates and frees its own native cancellation handle and result,
so calls are independent and need no cleanup.

### The client: state, operations, and serialization

`AuthClient` holds the configuration and the protected store
([section 4](#4-secure-persistence-and-restoration)). It serializes every
operation that reads or writes the state through an async lock, as the
[shared guide](user-guide.md#save-restore-and-refresh) requires, so a late
refresh cannot write tokens back after sign-out. Because the wrapper's
tasks complete only after the native call has returned, the lock is never
released while a native call still uses the state. Each operation takes a
host timeout and a `CancellationToken`; cancelling the token cancels the
operation whether it is running or still waiting for the lock. The class is
`partial` so that its operations can follow the sections of this guide:

```csharp
public sealed partial class AuthClient : IDisposable
{
    private readonly ProviderConfig config;
    private readonly ProtectedStateStore store;
    private readonly SemaphoreSlim stateLock = new(1, 1);  // held for each whole operation

    public AuthClient(ProviderConfig config, ProtectedStateStore store)
    {
        this.config = config;
        this.store = store;
    }

    public void Dispose() => stateLock.Dispose();

    // The tokens of the stored state, or null when signed out.
    public Task<Tokens?> StoredTokensAsync(CancellationToken cancellationToken = default) =>
        RunLockedAsync<Tokens?>(Timeout.InfiniteTimeSpan, cancellationToken, _ =>
        {
            var state = store.Load();
            return Task.FromResult(state is null ? null : Tokens.Read(state));
        });

    // Runs one operation under the state lock. The timeout starts once the
    // lock is held.
    private async Task<T> RunLockedAsync<T>(
        TimeSpan timeout,
        CancellationToken cancellationToken,
        Func<CancellationToken, Task<T>> operation)
    {
        await stateLock.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            return await WithTimeoutAsync(timeout, cancellationToken, operation).ConfigureAwait(false);
        }
        finally
        {
            stateLock.Release();
        }
    }

    // Passes the operation a token that is cancelled by cancellationToken or
    // when the timeout elapses. An operation already cancelled does not start.
    private static async Task<T> WithTimeoutAsync<T>(
        TimeSpan timeout,
        CancellationToken cancellationToken,
        Func<CancellationToken, Task<T>> operation)
    {
        using var limit = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        limit.CancelAfter(timeout);
        limit.Token.ThrowIfCancellationRequested();
        return await operation(limit.Token).ConfigureAwait(false);
    }
}
```

### Authorization

`NativeOidcWindows.AuthorizeAsync` resolves the configuration, binds the
loopback listener, opens the system browser with the authorization URL
(PKCE, state and, for `openid`, nonce are generated inside), validates the
callback, exchanges the code, validates any ID Token, and returns the
authentication-state JSON. The authorization URL and nonce are not exposed.

The call waits for the browser callback **without a timeout of its own**: if
the user closes the browser, it waits until it is cancelled. Always pass a
host timeout (or cancel from a Cancel button). Cancellation stops the wait
but cannot close the browser window.

```csharp
public sealed partial class AuthClient
{
    // Opens the system browser, waits for the callback, and saves the new
    // state. A failed or cancelled sign-in keeps any stored state.
    public Task SignInAsync(TimeSpan timeout, CancellationToken cancellationToken = default) =>
        RunLockedAsync(timeout, cancellationToken, async token =>
        {
            var state = await NativeOidcWindows.AuthorizeAsync(Requests.Authorization(config), token)
                .ConfigureAwait(false);
            store.Save(state);
            return true;
        });
}
```

### Using the client from a UI

Await the client from the UI thread in WinForms, WPF, WinUI, or MAUI: the
wrapper runs the native call on a thread-pool thread, and the `await`
returns to the UI thread. Never block the UI thread with `.Result` or
`.Wait()`. A Cancel button cancels the operation's
`CancellationTokenSource`:

```csharp
// Sign-in for a UI. Call RunAsync from an async Sign in button handler and
// Cancel from a Cancel button. Both run on the UI thread, so the field
// needs no lock.
public sealed class SignInCommand
{
    private readonly AuthClient client;
    private CancellationTokenSource? running;

    public SignInCommand(AuthClient client) => this.client = client;

    public bool IsRunning => running is not null;

    // Returns a message for the user. Diagnostic text goes to the log only.
    public async Task<string> RunAsync()
    {
        if (running is not null)
        {
            return "Sign-in is already running.";
        }
        using var cancellation = new CancellationTokenSource();
        running = cancellation;
        try
        {
            await client.SignInAsync(TimeSpan.FromMinutes(5), cancellation.Token);
            return "Signed in.";
        }
        catch (OperationCanceledException)
        {
            return "Sign-in was cancelled or timed out.";
        }
        catch (InvalidOperationException error)
        {
            Trace.TraceWarning("Sign-in failed: {0}", error.Message);
            return "Sign-in failed. Try again.";
        }
        finally
        {
            running = null;
        }
    }

    // Stops waiting for the browser. It cannot close the browser window.
    public void Cancel() => running?.Cancel();
}
```

Show the user your own messages. The diagnostic text can contain provider
descriptions, which are untrusted.

### Tokens and expiry

The successful value is the complete authentication state. Its format is
described in the
[ABI reference](windows-api.md#request-and-response-fields): `token_response`
holds the tokens, `expires_at` is the absolute access-token expiry in Unix
seconds (or `null`), and `oidc_nonce` is kept for later ID Token validation.
The wrapper has no token or expiry accessors, so the host reads the JSON:

```csharp
public sealed record Tokens(
    string AccessToken,
    string TokenType,
    string? RefreshToken,
    string? IdToken,
    long? ExpiresAt)  // Unix seconds
{
    // Throws JsonException, KeyNotFoundException, or InvalidOperationException
    // when the JSON is not a valid state.
    public static Tokens Read(string stateJson)
    {
        using var document = JsonDocument.Parse(stateJson);
        var state = document.RootElement;
        var response = state.GetProperty("token_response");
        return new Tokens(
            response.GetProperty("access_token").GetString() ?? throw new JsonException("access_token is null"),
            response.GetProperty("token_type").GetString() ?? throw new JsonException("token_type is null"),
            OptionalString(response, "refresh_token"),
            OptionalString(response, "id_token"),
            state.TryGetProperty("expires_at", out var expiresAt) && expiresAt.ValueKind == JsonValueKind.Number
                ? expiresAt.GetInt64()
                : null);
    }

    public static long UnixNow() => DateTimeOffset.UtcNow.ToUnixTimeSeconds();

    // True when the access token expires within skewSeconds. An unknown
    // expiry returns false: use the token and refresh when the resource
    // server rejects it.
    public bool NeedsRefresh(long now, long skewSeconds = 60) =>
        ExpiresAt is long expiry && expiry <= now + skewSeconds;

    // Records print every property by default; never print the tokens.
    public override string ToString() => $"Tokens {{ TokenType = {TokenType}, ExpiresAt = {ExpiresAt} }}";

    private static string? OptionalString(JsonElement response, string name) =>
        response.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;  // absent or null
}
```

Use the access token with its `TokenType` for resource-server calls. The ID
Token is an identity assertion, not an API credential. Do not log tokens or
the state JSON.

## 4. Secure persistence and restoration

The state contains the access, refresh, and ID Tokens, so the host must
protect it. This example stores it in a file encrypted with DPAPI for the
current Windows user. The file is replaced atomically, so a crash does not
leave a half-written state:

```csharp
public sealed class ProtectedStateStore
{
    // Application-specific DPAPI entropy. It ties the data to this
    // application but is not a secret: any process running as the same user
    // can read it.
    private static readonly byte[] Entropy = Encoding.UTF8.GetBytes("com.example.my-app/native-oidc-state/v1");

    private readonly string file;

    public ProtectedStateStore(string file) => this.file = Path.GetFullPath(file);

    private string TemporaryFile => file + ".tmp";

    public void Save(string stateJson)
    {
        var plaintext = Encoding.UTF8.GetBytes(stateJson);
        byte[] encrypted;
        try
        {
            encrypted = ProtectedData.Protect(plaintext, Entropy, DataProtectionScope.CurrentUser);
        }
        finally
        {
            CryptographicOperations.ZeroMemory(plaintext);
        }

        Directory.CreateDirectory(Path.GetDirectoryName(file)!);
        try
        {
            using (var stream = new FileStream(
                TemporaryFile, FileMode.Create, FileAccess.Write, FileShare.None, 4096, FileOptions.WriteThrough))
            {
                stream.Write(encrypted);
            }
            File.Move(TemporaryFile, file, overwrite: true);
        }
        catch
        {
            // Do not leave a copy of the tokens behind when the save fails.
            TryDelete(TemporaryFile);
            throw;
        }
    }

    // Returns null when no state is stored. Throws CryptographicException
    // when the file cannot be decrypted, for example after it was copied
    // from another user or machine.
    public string? Load()
    {
        byte[] encrypted;
        try
        {
            encrypted = File.ReadAllBytes(file);
        }
        catch (Exception error) when (error is FileNotFoundException or DirectoryNotFoundException)
        {
            return null;
        }
        var plaintext = ProtectedData.Unprotect(encrypted, Entropy, DataProtectionScope.CurrentUser);
        try
        {
            return Encoding.UTF8.GetString(plaintext);
        }
        finally
        {
            CryptographicOperations.ZeroMemory(plaintext);
        }
    }

    // Also removes a temporary file left by an interrupted save, which can
    // hold a newer refresh token than the stored state. Attempts both
    // deletions, then reports the first failure.
    public void Remove()
    {
        Exception? firstError = null;
        foreach (var path in new[] { file, TemporaryFile })
        {
            try
            {
                File.Delete(path);  // a missing file is not an error
            }
            catch (DirectoryNotFoundException)
            {
                // Nothing was ever saved.
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            {
                firstError ??= error;
            }
        }
        if (firstError is not null)
        {
            throw new IOException("Could not delete the protected state.", firstError);
        }
    }

    private static void TryDelete(string path)
    {
        try
        {
            File.Delete(path);
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException)
        {
        }
    }
}
```

What the host still decides:

- **Location:** a per-user directory such as one under `%LOCALAPPDATA%`
  (`Environment.SpecialFolder.LocalApplicationData`), one file per provider
  and client ID, because the state does not record its configuration.
- **Threat model:** DPAPI current-user protection stops other users and
  offline copies of the file, not malware running as the same user. Windows
  Credential Manager is an alternative, but its blob limit
  (`CRED_MAX_CREDENTIAL_BLOB_SIZE`, 2560 bytes) can be smaller than a state
  with an ID Token.
- **Plaintext copies:** the example clears its byte buffers, but .NET
  strings cannot be cleared, and the wrapper takes and returns strings. Keep
  the decrypted state short-lived.

Restoring is reading the file: there is no separate restore call. Pass the
loaded JSON to refresh unchanged. Restoration does not contact the provider,
so a restored token may already have been revoked.

## 5. Refresh and re-save

Refresh sends the stored refresh token and returns a replacement state.
Save it after every success, because the provider may rotate the refresh
token. On error or cancellation, keep the stored state: it is still the
latest one the host has. See the
[shared guide](user-guide.md#save-restore-and-refresh) for what that does and
does not guarantee.

The library never refreshes on its own. A typical host refreshes shortly
before expiry, then uses the new access token:

```csharp
public sealed partial class AuthClient
{
    // Saves the replacement state. On any exception the stored state is
    // unchanged.
    public Task<Tokens> RefreshAsync(TimeSpan timeout, CancellationToken cancellationToken = default) =>
        RunLockedAsync(timeout, cancellationToken, async token =>
        {
            var state = store.Load() ?? throw new InvalidOperationException("Not signed in.");
            var refreshed = await NativeOidcWindows.RefreshAsync(Requests.Refresh(config, state), token)
                .ConfigureAwait(false);
            store.Save(refreshed);
            return Tokens.Read(refreshed);
        });

    // Returns an access token for the resource server, or null when the user
    // needs to sign in (again) or the refresh did not finish in time.
    // Cancelling cancellationToken throws OperationCanceledException.
    public async Task<string?> UsableAccessTokenAsync(CancellationToken cancellationToken = default)
    {
        var tokens = await StoredTokensAsync(cancellationToken).ConfigureAwait(false);
        if (tokens is null)
        {
            return null;  // not signed in
        }
        if (!tokens.NeedsRefresh(Tokens.UnixNow()))
        {
            return tokens.AccessToken;
        }
        try
        {
            var refreshed = await RefreshAsync(TimeSpan.FromSeconds(30), cancellationToken).ConfigureAwait(false);
            return refreshed.AccessToken;
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            return null;  // the timeout; the stored state is unchanged
        }
        catch (InvalidOperationException error)
        {
            // Rejected (for example invalid_grant), network, or configuration.
            // The stored state is unchanged; retry later or sign in again.
            // Provider descriptions are untrusted text. Never log tokens.
            Trace.TraceWarning("Refresh failed: {0}", error.Message);
            return null;
        }
    }
}
```

A refresh response without a new refresh token, scope, or ID Token keeps the
previous value in the returned state, so always save the returned state as a
whole.

## 6. End-session, revocation, and local sign-out

These are three separate actions, as the
[shared guide](user-guide.md#logout-revocation-and-local-deletion) explains.

### Provider end-session (RP-Initiated Logout)

`NativeOidcWindows.EndSessionAsync` opens the provider's logout page in the
system browser. Read the ID Token for `id_token_hint` before deleting the
state:

```csharp
public sealed partial class AuthClient
{
    // Does not touch the stored state.
    public Task<string> EndProviderSessionAsync(TimeSpan timeout, CancellationToken cancellationToken = default) =>
        RunLockedAsync(timeout, cancellationToken, EndSessionLockedAsync);

    // Called with the state lock held.
    private Task<string> EndSessionLockedAsync(CancellationToken token)
    {
        var state = store.Load();
        var idTokenHint = state is null ? null : Tokens.Read(state).IdToken;
        return NativeOidcWindows.EndSessionAsync(Requests.EndSession(config, idTokenHint), token);
    }
}
```

- Without `PostLogoutRedirectUri`, success means only that the browser was
  opened, with an empty string. It does not confirm the provider's logout.
- With a `PostLogoutRedirectUri` registered at the provider, the call waits
  on the loopback listener and validates the returned `state`. The success
  value is JSON with `state` and `additional_parameters`. Like
  authorization, the wait has no timeout of its own.
- End-session does not touch the stored state.

### Token revocation and local deletion

`NativeOidcWindows.RevokeAsync` asks the provider to invalidate one token.
Sign-out revokes the refresh token (or the access token when there is none),
then deletes the stored state **whatever the revocation result**: local
sign-out must not depend on the provider:

```csharp
public sealed partial class AuthClient
{
    // Returns the revocation failure, or null. The user is signed out
    // locally either way.
    public Task<Exception?> SignOutAsync(TimeSpan timeout, CancellationToken cancellationToken = default) =>
        RunLockedAsync(timeout, cancellationToken, SignOutLockedAsync);

    // Called with the state lock held.
    private async Task<Exception?> SignOutLockedAsync(CancellationToken token)
    {
        Exception? revocationError = null;
        try
        {
            if (store.Load() is { } state)
            {
                var tokens = Tokens.Read(state);
                var request = tokens.RefreshToken is { } refreshToken
                    ? Requests.Revocation(config, refreshToken, "refresh_token")
                    : Requests.Revocation(config, tokens.AccessToken, "access_token");
                await NativeOidcWindows.RevokeAsync(request, token).ConfigureAwait(false);
            }
        }
        catch (Exception error)
        {
            // Provider, network, timeout, or an unreadable state.
            revocationError = error;
        }
        store.Remove();
        return revocationError;
    }
}
```

A successful revocation has no value. HTTP 200 counts as success even for an
already-invalid token. With explicit configuration, revocation needs
`RevocationEndpoint`; with discovery, the provider must advertise one.
Without it, the call throws and sign-out still deletes the state. A host
that also wants the access token revoked makes a second call.

For a full logout, end the provider session first (it needs the ID Token),
then sign out locally. End-session can fail: the configuration may have no
end-session endpoint, the browser may not open, or the user may close it
until the timeout or a Cancel button stops the wait. Local sign-out must
still run, with its own timeout and without the cancelled token. Both steps
also run under one hold of the state lock. Otherwise a sign-in started
while the logout page is open could save a new session, and sign-out would
then revoke and delete that session instead of the one whose ID Token went
to the provider:

```csharp
// The failures of a full logout; null when that step succeeded.
public sealed record LogoutResult(Exception? EndSessionError, Exception? RevocationError);

public sealed partial class AuthClient
{
    // Ends the provider session, then signs out locally whatever the
    // end-session result. cancellationToken stops only the end-session; the
    // logout itself always runs, so it waits for the lock without a token.
    public async Task<LogoutResult> LogOutAsync(
        TimeSpan endSessionTimeout, CancellationToken cancellationToken = default)
    {
        await stateLock.WaitAsync().ConfigureAwait(false);
        try
        {
            Exception? endSessionError = null;
            try
            {
                await WithTimeoutAsync(endSessionTimeout, cancellationToken, EndSessionLockedAsync)
                    .ConfigureAwait(false);
            }
            catch (Exception error)
            {
                endSessionError = error;
            }
            var revocationError = await WithTimeoutAsync(
                TimeSpan.FromSeconds(30), CancellationToken.None, SignOutLockedAsync).ConfigureAwait(false);
            return new LogoutResult(endSessionError, revocationError);
        }
        finally
        {
            stateLock.Release();
        }
    }
}
```

Because `AuthClient` holds its state lock for each whole operation, a
refresh that started earlier finishes before sign-out deletes the state, and
cannot write it back afterwards. An operation queued behind the logout runs
after the state is deleted.

## 7. Dynamic registration and additional parameters

The Windows ABI and the .NET wrapper have **no dynamic client
registration**. Register the client with the provider beforehand, or
register it with another component and put the resulting client ID in
`ProviderConfig`. The shared core's registration API is not exposed through
the C ABI.

Additional parameters are exposed:

- `ProviderConfig.AdditionalParameters` goes into the authorization request
  (for example `prompt` or `login_hint`). Reserved names (`client_id`,
  `redirect_uri`, `response_type`, `scope`, `state`, `nonce`,
  `code_challenge`, `code_challenge_method`) are rejected.
- `ProviderConfig.AdditionalTokenParameters` goes into the code exchange.
  Reserved token-request names, such as `client_secret`, `code`,
  `code_verifier`, and `grant_type`, are rejected.
- Refresh accepts its own `additional_parameters` array (not used by these
  examples).

Names must be unique in each array. A rejected parameter throws
`InvalidOperationException` before the browser opens.

## 8. Cancellation, errors, and lifecycle

### Cancellation and timeouts

- Every wrapper method registers its `CancellationToken` on a native
  cancellation handle of its own before the native call starts. A token
  that is already cancelled therefore cancels the call before it does any
  work: a sign-in throws `OperationCanceledException` without opening the
  browser.
- Cancellation makes the native call return as soon as the current step
  notices it. It cannot undo a request the provider already processed, and
  it cannot close the browser.
- The host owns every timeout. Authorization and end-session with a
  post-logout redirect wait for the browser indefinitely. Choose a timeout
  that leaves time to sign in (the examples use five minutes). Use a linked
  `CancellationTokenSource` with `CancelAfter`, as `AuthClient` does, so
  that the timeout and the user's cancellation stop the same call.
- The task completes only after the native call has returned, so awaiting
  it is enough before exiting. Do not abandon a running task at shutdown;
  cancel it and await it.

### Errors

The exception type is the only structured error information. The `Message`
of an `InvalidOperationException` from the wrapper is diagnostic text for
logs and support; do not parse it or show provider descriptions as trusted
text. The [shared guide](user-guide.md#cancellation-timeouts-and-errors)
maps the failure categories.

| Exception | Typical causes | Host action |
| --- | --- | --- |
| None | — | Save the returned state (authorize, refresh); continue |
| `OperationCanceledException` | Host timeout, Cancel button, shutdown | Keep the stored state; offer to try again |
| `InvalidOperationException` | Invalid request or configuration, invalid redirect, rejected parameter, browser could not be opened, listener failure | A bug or a deployment problem: fix it rather than retrying |
| `InvalidOperationException` | Provider rejection (`invalid_grant` on refresh), HTTP or network failure, invalid response or ID Token | Keep the stored state; retry later with backoff, or sign in again |
| `ArgumentNullException` | A `null` request | A bug in the host |
| `DllNotFoundException`, `BadImageFormatException`, `EntryPointNotFoundException` | `native_oidc.dll` missing, a non-x64 process, or a DLL from another version | A deployment problem |

The wrapper does not tell the two kinds of `InvalidOperationException`
apart. A configuration error usually appears on the first call after a
deployment, while the second kind can appear at any time. Retry with a
limit, and ask the user to sign in again when refresh keeps failing. The
examples' own code adds more exceptions: `CryptographicException` from
DPAPI, `IOException` and `UnauthorizedAccessException` from file
operations, and `JsonException`, `KeyNotFoundException`, or
`InvalidOperationException` from reading an invalid state.

### Process and lifecycle constraints

- The DLL is loaded on the first call and stays loaded for the life of the
  process.
- Operations are independent: each binds its own loopback port and runtime.
  Still serialize the operations that change one state, as `AuthClient`
  does.
- The adapter cannot detect that the user closed the browser; only the host
  timeout or cancellation ends the wait.

### Minimal application

This `Program.cs` completes the project from [section 1](#project-file).
`AuthClient.cs` holds the `csharp` blocks above, in order. A console
application may await on its main thread; a GUI uses `SignInCommand` from
its UI thread:

```csharp Program.cs
using System;
using System.IO;
using MyApp;

// Usage: MyApp [--sign-in]
// --sign-in starts a new sign-in even when a state is stored, for example
// after the provider rejected its refresh token.
var forceSignIn = args.Length > 0 && args[0] == "--sign-in";
// The per-user state file, under %LOCALAPPDATA%.
var stateFile = Path.Combine(
    Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
    "MyApp",
    "native-oidc-state.bin");

using var client = new AuthClient(Configurations.Discovered(), new ProtectedStateStore(stateFile));
try
{
    if (forceSignIn || await client.StoredTokensAsync() is null)
    {
        Console.WriteLine("Opening the browser to sign in...");
        await client.SignInAsync(TimeSpan.FromMinutes(5));
    }
    if (await client.UsableAccessTokenAsync() is null)
    {
        // The stored state is kept, so a transient failure can succeed on
        // the next launch; a rejected refresh token needs a new sign-in.
        Console.Error.WriteLine("No usable access token. Try again later, or run with --sign-in to sign in again.");
        return 1;
    }
    Console.WriteLine("Signed in. Call the resource server with the access token.");
    return 0;
}
catch (OperationCanceledException)
{
    // A failed sign-in keeps any previously stored state.
    Console.Error.WriteLine("Sign-in did not complete in time.");
}
catch (InvalidOperationException error)
{
    Console.Error.WriteLine($"Sign-in did not complete: {error.Message}");
}
return 1;
```

## 9. Verification, coverage, and known limits

### What the examples are checked against

The .NET guide fixture in
[`native/consumer-smoke/windows/dotnet-guide`](../native/consumer-smoke/windows/dotnet-guide)
extracts every `csharp` block of this guide, in order, into `GuideClient.cs`
before each build, and compiles it with
[`GuideExamples.cs`](../native/consumer-smoke/windows/dotnet-guide/examples/GuideExamples.cs)
and the package's wrapper source, using the package items from
[section 1](#project-file). It also builds the
[minimal application](#minimal-application) from the `csharp Program.cs`
block and the same code; that program is built but not run, because it
opens a browser. Both projects build with warnings treated as errors. The
examples run against a fresh local mock provider in its `dotnet-guide` mode
([`mock-provider`](../native/consumer-smoke/windows/mock-provider/Program.cs)),
which checks every request it receives and their order, and fails when one
is missing:

- DPAPI save, load, and removal. The stored file contains neither token in
  plain text and cannot be decrypted without the application's entropy. A
  failed replace leaves no temporary file. Removal also deletes a temporary
  file left by an interrupted save, and still deletes the state when that
  temporary file cannot be deleted.
- `UsableAccessTokenAsync` refreshes an expired state, the provider rotates
  the refresh token, and the replacement state is saved.
- A refresh rejected with `invalid_grant`, a refresh cancelled by the host
  timeout, and a refresh cancelled through a `CancellationToken` from
  another thread each throw the expected exception and leave the stored
  state unchanged.
- `SignOutAsync` revokes the refresh token and deletes the state, and still
  deletes it when revocation fails with HTTP 503.
- `LogOutAsync` still deletes the state when end-session fails (no
  end-session endpoint) or its token was cancelled, and reports both
  failures. A refresh queued while the logout runs starts only after the
  state is deleted, so it sends no request.
- Errors that need no browser: a single explicit endpoint, a non-loopback
  redirect, a reserved additional parameter, end-session without its
  endpoint, refresh without a stored state, and a `null` request. Sign-in
  through `SignInCommand` also takes this path.
- A sign-in, and a direct `AuthorizeAsync` call with a valid request, whose
  token was cancelled before the call throw `OperationCanceledException`
  without opening the browser.
- The refresh request passes the stored state's numbers through unchanged,
  and `Tokens.ToString` does not print the tokens.

Run it with the other Windows consumers on a Windows host with Visual
Studio (C++ and CMake) and the .NET SDK:

```powershell
# A downloaded release ZIP (verify its checksum first, see section 1)
scripts/check-windows-release-consumer.ps1 -Package native-oidc-windows-0.8.0.zip
```

CI runs the same script on every change to the Windows adapter, its
fixtures, or this guide (`windows_core` on Windows Server, `windows_client`
on a Windows 11 client runner with x64 emulation), against a package built
from the branch. The `Published release consumer validation` workflow
checks out the release tag's own scripts, so for v0.8.0 it runs only the
consumers that existed at that tag, not these guide examples.

Evidence for this guide (commit `6feb065`):

- Executed in CI: CI built a package from the branch and compiled all 13
  `csharp` blocks. It ran the 49 checks against that package, and all of
  them passed, with the provider serving all six steps. It also built the
  minimal application. This held both on Windows Server
  ([`windows_core`](https://github.com/ProjectFlora/NativeOIDC/actions/runs/45530623503/job/138924767865))
  and on a Windows 11 client with x64 emulation
  ([`windows_client`](https://github.com/ProjectFlora/NativeOIDC/actions/runs/45530623503/job/138926034431)).
  The Windows header, ABI, wrapper, and request/response JSON are unchanged
  between v0.8.0 and that commit. The Rust sources behind the DLL differ
  only by the refactoring of shared validation in #157.
- Executed locally against the published v0.8.0 package: the ZIP's SHA-256
  matched `SHA256SUMS`. Both projects were built with .NET SDK 11.0.100
  against the package's `NativeOidcWindows.cs`, with warnings as errors.
  The 49 checks passed against the package's `native_oidc.dll` and the
  provider's `dotnet-guide` mode. The examples were built for .NET 8, but
  this run used the .NET 11 runtime, because that machine has no .NET 8
  runtime (`DOTNET_ROLL_FORWARD=Major`).

### Coverage by operation

| Operation | Section | Availability | Checked by |
| --- | --- | --- | --- |
| Install, build, run (x64) | [1](#1-versions-installation-and-build) | Available | Fixture build and run |
| Discovery / explicit / OIDC configuration | [2](#provider-configuration) | Available in operation JSON; no standalone discovery or inspection | Explicit: fixture; discovery: compile only |
| Authorization (browser, loopback, exchange) | [3](#authorization) | Available | Compile, early errors, and cancellation only; browser flow by the manual [real-browser verification](real-browser-verification.md), which runs through this wrapper |
| Tokens and expiry | [3](#tokens-and-expiry) | Host reads state JSON | Fixture |
| Secure persistence and restoration | [4](#4-secure-persistence-and-restoration) | Host (DPAPI example) | Fixture |
| Refresh and re-save | [5](#5-refresh-and-re-save) | Available | Fixture: success, rejection, cancellation |
| End-session | [6](#provider-end-session-rp-initiated-logout) | Available | Compile, early error, and local sign-out after its failure; browser flow by Windows unit tests with a simulated browser |
| Revocation and local deletion | [6](#token-revocation-and-local-deletion) | Revocation available; deletion is host | Fixture: success and failure |
| Dynamic registration | [7](#7-dynamic-registration-and-additional-parameters) | Not exposed | — |
| Additional parameters | [7](#7-dynamic-registration-and-additional-parameters) | Available | Fixture: rejection |
| Cancellation and timeouts | [8](#cancellation-and-timeouts) | Available; timeouts are host | Fixture |
| Structured errors | [8](#errors) | Not exposed (exception type and text only) | Fixture: exception branches |

### Known limits

- The browser steps of authorization and end-session are not run by the
  fixture; CI has no interactive browser. Real-provider interoperability is
  outside these checks; see
  [interoperability status](live-idp-interoperability.md).
- CI builds and runs the examples on .NET 8 only. The fixture does not
  build the wrapper for other .NET Standard 2.1 hosts.
- The DPAPI example protects against other users and offline copies only.
- Only x64 is distributed.
