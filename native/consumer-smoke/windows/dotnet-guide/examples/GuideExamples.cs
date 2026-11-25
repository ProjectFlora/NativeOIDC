// Runs the examples of docs/windows-dotnet-guide.md against the local mock
// provider in its `dotnet-guide` mode. GuideClient.cs, generated before the
// build, contains every csharp block of the guide, in order (see
// ../GuideSources.targets), so this file only calls the guide's code and
// checks the results. The provider checks the requests and their order;
// this program checks what the host sees.
//
// The requests sent must match GuideSteps in ../../mock-provider/Program.cs.
// Everything after them fails before any network request or browser launch.

using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using MyApp;
using ProjectFlora.NativeOidc;

if (RuntimeInformation.ProcessArchitecture != Architecture.X64)
{
    Console.Error.WriteLine("The .NET guide examples must run as x64.");
    return 1;
}
if (args.Length != 1)
{
    Console.Error.WriteLine("usage: NativeOidcGuideExamples <mock provider base URL>");
    return 2;
}
var baseUrl = args[0];
var failures = 0;
var directory = Path.Combine(Path.GetTempPath(), $"native-oidc-dotnet-guide-{Environment.ProcessId}");
var file = Path.Combine(directory, "state.bin");
var store = new ProtectedStateStore(file);

try
{
    CheckStorage();
    CheckRequests();
    using (var client = new AuthClient(MockConfig(), store))
    {
        await CheckRefreshAsync(client);
        await CheckSignOutAsync(client);
    }
    await CheckLocalErrorsAsync();
}
catch (Exception error)
{
    Console.Error.WriteLine($"FAILED: unexpected exception: {error}");
    failures++;
}
finally
{
    try
    {
        Directory.Delete(directory, recursive: true);
    }
    catch (Exception error) when (error is IOException or UnauthorizedAccessException)
    {
    }
}
Console.Error.WriteLine(failures == 0 ? "All .NET guide checks passed." : $"{failures} .NET guide checks failed.");
return failures == 0 ? 0 : 1;

void Check(bool condition, string description)
{
    Console.Error.WriteLine($"{(condition ? "ok" : "FAILED")}: {description}");
    if (!condition)
    {
        failures++;
    }
}

// The exception an action throws, or null when it succeeds, so that a
// success is never mistaken for the expected failure.
static Exception? Caught(Action action)
{
    try
    {
        action();
        return null;
    }
    catch (Exception error)
    {
        return error;
    }
}

static async Task<Exception?> CaughtAsync(Func<Task> action)
{
    try
    {
        await action();
        return null;
    }
    catch (Exception error)
    {
        return error;
    }
}

ProviderConfig MockConfig() => new()
{
    AuthorizationEndpoint = $"{baseUrl}/authorize",
    TokenEndpoint = $"{baseUrl}/token",
    RevocationEndpoint = $"{baseUrl}/revoke",
    ClientId = "net-guide-client",
    RedirectUri = "http://127.0.0.1/oauth/callback",
    Scopes = new[] { "profile" },
};

static string StoredStateJson(string refreshToken, long expiresAt) =>
    $$"""{"token_response":{"access_token":"net-guide-access-1","token_type":"Bearer","refresh_token":"{{refreshToken}}"},"expires_at":{{expiresAt}}}""";

static bool ContainsText(byte[] bytes, string text) =>
    Encoding.Latin1.GetString(bytes).Contains(text, StringComparison.Ordinal)
    || Encoding.Unicode.GetString(bytes).Contains(text, StringComparison.Ordinal);

void CheckStorage()
{
    store.Remove();
    Check(store.Load() is null, "load returns nothing before the first save");
    var state = StoredStateJson("net-guide-refresh-1", 1);
    store.Save(state);
    Check(store.Load() == state, "DPAPI save and load round-trip the state");
    var bytes = File.ReadAllBytes(file);
    Check(bytes.Length > 0 && !ContainsText(bytes, "net-guide-access-1") && !ContainsText(bytes, "net-guide-refresh-1"),
        "the stored file does not contain the tokens in plain text");
    Check(Caught(() => ProtectedData.Unprotect(bytes, null, DataProtectionScope.CurrentUser)) is CryptographicException,
        "the stored file is a DPAPI blob that needs the application's entropy");
    var temporary = file + ".tmp";
    Check(!File.Exists(temporary), "no temporary file is left after the atomic replace");

    // A temporary file left by an interrupted save is deleted with the state.
    File.WriteAllText(temporary, "orphan");
    store.Remove();
    Check(!File.Exists(temporary) && !File.Exists(file),
        "remove also deletes a temporary file left by an interrupted save");

    // A temporary path that cannot be deleted (a directory) must not keep
    // the state itself from being deleted.
    store.Save(state);
    Directory.CreateDirectory(Path.Combine(temporary, "locked"));
    Check(Caught(store.Remove) is IOException, "remove reports a temporary file it cannot delete");
    Check(!File.Exists(file), "remove deletes the state even when the temporary file cannot be deleted");
    Directory.Delete(temporary, recursive: true);

    // A file cannot replace a directory, so this save fails after writing
    // the temporary file.
    Directory.CreateDirectory(file);
    var saveError = Caught(() => store.Save(state));
    Check(saveError is IOException or UnauthorizedAccessException, "a failed replace throws");
    Check(!File.Exists(temporary), "a failed replace leaves no temporary file");
    Directory.Delete(file);

    store.Save(state);  // the state the refresh checks start from
}

void CheckRequests()
{
    const string preserved = "123456789012345678901234567890";
    var state = $$"""{"token_response":{"access_token":"a","token_type":"Bearer","preserved":{{preserved}}},"expires_at":1}""";
    Check(Requests.Refresh(MockConfig(), state).Contains($"\"preserved\":{preserved}", StringComparison.Ordinal),
        "the refresh request passes the stored state's numbers through unchanged");
    var tokens = Tokens.Read(StoredStateJson("net-guide-refresh-1", 1));
    Check(!tokens.ToString().Contains("net-guide-", StringComparison.Ordinal), "Tokens.ToString does not print the tokens");
}

async Task CheckRefreshAsync(AuthClient client)
{
    // Step 1: the stored state expired at 1970-01-01T00:00:01Z.
    Check(await client.UsableAccessTokenAsync() == "net-guide-access-2",
        "UsableAccessTokenAsync refreshes an expired state");
    var refreshed = store.Load();
    Check(refreshed is not null, "the refreshed state is stored");
    if (refreshed is null)
    {
        return;
    }
    var tokens = Tokens.Read(refreshed);
    Check(tokens.RefreshToken == "net-guide-refresh-2", "the rotated refresh token is saved");
    Check(tokens.ExpiresAt > Tokens.UnixNow(), "the new absolute expiry is saved");
    Check(!tokens.NeedsRefresh(Tokens.UnixNow()), "the refreshed token does not need a refresh");
    Check(await client.UsableAccessTokenAsync() == "net-guide-access-2",
        "UsableAccessTokenAsync uses a valid token without a refresh");

    // Step 2: the provider rejects the refresh token.
    var rejected = await CaughtAsync(() => client.RefreshAsync(TimeSpan.FromSeconds(30)));
    Check(rejected is InvalidOperationException, "a rejected refresh throws InvalidOperationException");
    Check(store.Load() == refreshed, "a rejected refresh keeps the stored state");

    // Step 3: the provider never answers; the host timeout cancels.
    var timedOut = await CaughtAsync(() => client.RefreshAsync(TimeSpan.FromSeconds(3)));
    Check(timedOut is OperationCanceledException, "the host timeout cancels a refresh");
    Check(store.Load() == refreshed, "a timed-out refresh keeps the stored state");

    // Step 4: the provider never answers; another thread cancels the token.
    using var cancellation = new CancellationTokenSource();
    var running = Task.Run(() => client.RefreshAsync(TimeSpan.FromSeconds(60), cancellation.Token));
    await Task.Delay(TimeSpan.FromSeconds(3));
    Check(!running.IsCompleted, "the refresh waits for the provider until it is cancelled");
    cancellation.Cancel();
    var cancelled = await CaughtAsync(() => running);
    Check(cancelled is OperationCanceledException, "a token cancelled from another thread cancels a running refresh");
    Check(store.Load() == refreshed, "a cancelled refresh keeps the stored state");
}

async Task CheckSignOutAsync(AuthClient client)
{
    // Step 5: revocation succeeds.
    var revocationError = await client.SignOutAsync(TimeSpan.FromSeconds(30));
    Check(revocationError is null, "SignOutAsync revokes the refresh token");
    Check(store.Load() is null, "SignOutAsync deletes the stored state");

    // Step 6: revocation fails with HTTP 503.
    store.Save(StoredStateJson("net-guide-refresh-3", Tokens.UnixNow() + 3600));
    var failed = await client.SignOutAsync(TimeSpan.FromSeconds(30));
    Check(failed is InvalidOperationException, "a failed revocation is reported");
    Check(store.Load() is null, "SignOutAsync deletes the stored state when revocation fails");
}

// None of these reach the provider or open a browser.
async Task CheckLocalErrorsAsync()
{
    using var signedOut = new AuthClient(MockConfig(), store);
    Check(await CaughtAsync(() => signedOut.RefreshAsync(TimeSpan.FromSeconds(10))) is InvalidOperationException,
        "refresh without a stored state throws InvalidOperationException");
    Check(await signedOut.UsableAccessTokenAsync() is null, "UsableAccessTokenAsync returns null when signed out");
    Check(await signedOut.StoredTokensAsync() is null, "StoredTokensAsync returns null when signed out");

    using var single = new AuthClient(MockConfig() with { TokenEndpoint = null }, store);
    Check(await CaughtAsync(() => single.SignInAsync(TimeSpan.FromSeconds(10))) is InvalidOperationException,
        "SignInAsync rejects a single explicit endpoint");

    using var localhost = new AuthClient(MockConfig() with { RedirectUri = "http://localhost/oauth/callback" }, store);
    Check(await CaughtAsync(() => localhost.SignInAsync(TimeSpan.FromSeconds(10))) is InvalidOperationException,
        "SignInAsync rejects a non-127.0.0.1 redirect");

    var reservedConfig = MockConfig() with { AdditionalParameters = new[] { new Parameter("state", "chosen-by-host") } };
    using var reserved = new AuthClient(reservedConfig, store);
    Check(await CaughtAsync(() => reserved.SignInAsync(TimeSpan.FromSeconds(10))) is InvalidOperationException,
        "SignInAsync rejects a reserved additional parameter");

    Check(await CaughtAsync(() => signedOut.EndProviderSessionAsync(TimeSpan.FromSeconds(10))) is InvalidOperationException,
        "end-session with explicit endpoints requires EndSessionEndpoint");

    Check(await CaughtAsync(() => NativeOidcWindows.RefreshAsync(null!)) is ArgumentNullException,
        "a null request throws ArgumentNullException");

    // A valid configuration: only the cancelled token prevents the browser
    // launch.
    using var cancelled = new CancellationTokenSource();
    cancelled.Cancel();
    using var valid = new AuthClient(MockConfig(), store);
    Check(await CaughtAsync(() => valid.SignInAsync(TimeSpan.FromSeconds(10), cancelled.Token)) is OperationCanceledException,
        "a sign-in whose token was cancelled before it started throws OperationCanceledException");
    Check(await CaughtAsync(() => NativeOidcWindows.AuthorizeAsync(Requests.Authorization(MockConfig()), cancelled.Token))
            is OperationCanceledException,
        "AuthorizeAsync with a token cancelled before the call throws OperationCanceledException");

    var command = new SignInCommand(single);
    Check(await command.RunAsync() == "Sign-in failed. Try again.", "SignInCommand reports a failed sign-in");
    Check(!command.IsRunning, "SignInCommand is idle after the sign-in finished");

    Check(store.Load() is null, "failed operations store no state");
}
