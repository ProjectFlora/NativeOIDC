using System;
using System.Collections.Generic;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

// Local OAuth provider for the Windows release consumer fixtures.
//
// It listens on an ephemeral 127.0.0.1 port and prints that port as the first
// standard output line. It then serves exactly one refresh request on /token
// and one revocation request on /revoke, in either order, and checks their
// form fields. It exits with 0 only when both expected requests arrived;
// any unexpected request, form mismatch or the overall timeout exits with 1.
// Diagnostics go to standard error.
//
// With the single argument `guide`, it instead serves the fixed sequence of
// GuideSteps below, in order, for the C++ guide examples
// (native/consumer-smoke/windows/cpp/guide_examples.cpp). With `dotnet-guide`,
// it serves the same sequence with `net-`-prefixed client ID and tokens for
// the .NET guide examples
// (native/consumer-smoke/windows/dotnet-guide/examples/GuideExamples.cs). The
// default mode, used by the C++ and .NET release consumers, is unchanged.

var guideMode = args.Length == 1 && args[0] is "guide" or "dotnet-guide";
if (args.Length != 0 && !guideMode)
{
    Console.Error.WriteLine("usage: NativeOidcMockProvider [guide | dotnet-guide]");
    return 2;
}
// Keeps the two guides' values apart, so that each fixture only passes
// against its own mode.
var p = guideMode && args[0] == "dotnet-guide" ? "net-" : "";

// The guide examples' requests, in the order guide_examples.cpp and
// GuideExamples.cs send them. A stalled step reads the request, sends no
// response, and waits for the client to close the connection (the guide
// cancels the refresh).
var guideSteps = new[]
{
    // Refresh with rotation.
    new GuideStep("/token", new Expectation(
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["grant_type"] = "refresh_token",
            ["refresh_token"] = $"{p}guide-refresh-1",
            ["client_id"] = $"{p}guide-client",
        },
        "application/json",
        $$"""{"access_token":"{{p}}guide-access-2","token_type":"Bearer","expires_in":3600,"refresh_token":"{{p}}guide-refresh-2"}"""),
        200, Stall: false),
    // Refresh rejected by the provider.
    new GuideStep("/token", new Expectation(
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["grant_type"] = "refresh_token",
            ["refresh_token"] = $"{p}guide-refresh-2",
            ["client_id"] = $"{p}guide-client",
        },
        "application/json",
        """{"error":"invalid_grant","error_description":"refresh token expired"}"""),
        400, Stall: false),
    // Two refreshes that never answer: the client's timeout cancels the
    // first, and another client thread cancels the second.
    StalledRefresh($"{p}guide-refresh-2", $"{p}guide-client"),
    StalledRefresh($"{p}guide-refresh-2", $"{p}guide-client"),
    // Sign-out with successful revocation.
    new GuideStep("/revoke", new Expectation(
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["token"] = $"{p}guide-refresh-2",
            ["token_type_hint"] = "refresh_token",
            ["client_id"] = $"{p}guide-client",
        },
        "text/plain",
        ""),
        200, Stall: false),
    // Sign-out with failed revocation.
    new GuideStep("/revoke", new Expectation(
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["token"] = $"{p}guide-refresh-3",
            ["token_type_hint"] = "refresh_token",
            ["client_id"] = $"{p}guide-client",
        },
        "text/plain",
        ""),
        503, Stall: false),
};
if (p == "net-")
{
    // The .NET guide examples then hold the client's state lock with one
    // more refresh that never answers, while a logout and another refresh
    // queue behind it (GuideExamples.cs, CheckLocalErrorsAsync).
    guideSteps = [.. guideSteps, StalledRefresh("net-guide-refresh-4", "net-guide-client")];
}

var expectations = new Dictionary<string, Expectation>(StringComparer.Ordinal)
{
    ["/token"] = new Expectation(
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["grant_type"] = "refresh_token",
            ["refresh_token"] = "old-refresh",
            ["client_id"] = "consumer",
        },
        "application/json",
        """{"access_token":"new-access","token_type":"Bearer","expires_in":3600,"refresh_token":"new-refresh"}"""),
    ["/revoke"] = new Expectation(
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["token"] = "new-refresh",
            ["token_type_hint"] = "refresh_token",
            ["client_id"] = "consumer",
        },
        "text/plain",
        ""),
};

using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(60));
var listener = new TcpListener(IPAddress.Loopback, 0);
listener.Start();
try
{
    Console.Out.WriteLine(((IPEndPoint)listener.LocalEndpoint).Port);
    Console.Out.Flush();

    if (guideMode)
    {
        return await ServeGuideStepsAsync(listener, guideSteps, timeout.Token);
    }

    while (expectations.Count > 0)
    {
        using var client = await listener.AcceptTcpClientAsync(timeout.Token);
        using var stream = client.GetStream();
        var request = await HttpRequest.ReadAsync(stream, timeout.Token);
        if (request is null)
        {
            Console.Error.WriteLine("Mock provider: connection closed before a complete request.");
            return 1;
        }

        if (request.Method != "POST" || !expectations.Remove(request.Target, out var expectation))
        {
            Console.Error.WriteLine($"Mock provider: unexpected request {request.Method} {request.Target}.");
            await HttpRequest.WriteResponseAsync(stream, 404, "text/plain", "", timeout.Token);
            return 1;
        }

        var form = HttpRequest.ParseForm(request.Body);
        var mismatch = expectation.FindMismatch(form);
        if (mismatch is not null)
        {
            Console.Error.WriteLine($"Mock provider: {request.Target} {mismatch}.");
            await HttpRequest.WriteResponseAsync(stream, 400, "text/plain", "", timeout.Token);
            return 1;
        }

        await HttpRequest.WriteResponseAsync(
            stream, 200, expectation.ContentType, expectation.ResponseBody, timeout.Token);
        Console.Error.WriteLine($"Mock provider: served {request.Target}.");
    }

    return 0;
}
catch (OperationCanceledException) when (timeout.IsCancellationRequested)
{
    var pending = guideMode ? "the guide sequence" : string.Join(", ", expectations.Keys);
    Console.Error.WriteLine($"Mock provider: timed out waiting for {pending}.");
    return 1;
}
catch (Exception error)
{
    Console.Error.WriteLine($"Mock provider: {error.Message}");
    return 1;
}
finally
{
    listener.Stop();
}

// Serves the steps in order, one connection each. Returns 0 when every step
// was served as expected, and 1 on the first unexpected request.
static async Task<int> ServeGuideStepsAsync(
    TcpListener listener,
    IReadOnlyList<GuideStep> steps,
    CancellationToken cancellation)
{
    for (var index = 0; index < steps.Count; index++)
    {
        var step = steps[index];
        using var client = await listener.AcceptTcpClientAsync(cancellation);
        using var stream = client.GetStream();
        var request = await HttpRequest.ReadAsync(stream, cancellation);
        if (request is null)
        {
            Console.Error.WriteLine($"Mock provider: step {index + 1}: connection closed before a complete request.");
            return 1;
        }
        if (request.Method != "POST" || request.Target != step.Target)
        {
            Console.Error.WriteLine(
                $"Mock provider: step {index + 1}: unexpected request {request.Method} {request.Target}, expected POST {step.Target}.");
            await HttpRequest.WriteResponseAsync(stream, 404, "text/plain", "", cancellation);
            return 1;
        }
        var mismatch = step.Expectation.FindMismatch(HttpRequest.ParseForm(request.Body));
        if (mismatch is not null)
        {
            Console.Error.WriteLine($"Mock provider: step {index + 1}: {request.Target} {mismatch}.");
            await HttpRequest.WriteResponseAsync(stream, 400, "text/plain", "", cancellation);
            return 1;
        }

        if (step.Stall)
        {
            // Wait until the client gives up on the request and closes the
            // connection.
            var buffer = new byte[256];
            while (await stream.ReadAsync(buffer.AsMemory(), cancellation) > 0)
            {
            }
            Console.Error.WriteLine($"Mock provider: step {index + 1}: client closed the stalled {step.Target}.");
            continue;
        }

        await HttpRequest.WriteResponseAsync(
            stream, step.Status, step.Expectation.ContentType, step.Expectation.ResponseBody, cancellation);
        Console.Error.WriteLine($"Mock provider: step {index + 1}: served {step.Target} with {step.Status}.");
    }
    return 0;
}

static GuideStep StalledRefresh(string refreshToken, string clientId) =>
    new("/token", new Expectation(
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["grant_type"] = "refresh_token",
            ["refresh_token"] = refreshToken,
            ["client_id"] = clientId,
        },
        "application/json",
        ""),
        0, Stall: true);

sealed record GuideStep(string Target, Expectation Expectation, int Status, bool Stall);

sealed record Expectation(
    IReadOnlyDictionary<string, string> Form,
    string ContentType,
    string ResponseBody)
{
    public string? FindMismatch(IReadOnlyDictionary<string, string> actual)
    {
        if (actual.ContainsKey("client_secret"))
        {
            return "form contains client_secret";
        }
        foreach (var (name, value) in Form)
        {
            if (!actual.TryGetValue(name, out var actualValue) || actualValue != value)
            {
                return $"form field {name} is '{actualValue}', expected '{value}'";
            }
        }
        return null;
    }
}

sealed record HttpRequest(string Method, string Target, byte[] Body)
{
    private const int MaxHeaderBytes = 16 * 1024;
    private const int MaxBodyBytes = 64 * 1024;

    // Reads the request line and headers up to the blank line, then exactly
    // Content-Length body bytes. It never waits for the client to close the
    // connection, because the client keeps it open for the response.
    public static async Task<HttpRequest?> ReadAsync(NetworkStream stream, CancellationToken cancellation)
    {
        var buffer = new List<byte>();
        var chunk = new byte[4096];
        var headerEnd = -1;
        while (headerEnd < 0)
        {
            var read = await stream.ReadAsync(chunk.AsMemory(), cancellation);
            if (read == 0)
            {
                return null;
            }
            buffer.AddRange(chunk.AsSpan(0, read).ToArray());
            headerEnd = FindHeaderEnd(buffer);
            if (headerEnd < 0 && buffer.Count > MaxHeaderBytes)
            {
                throw new InvalidOperationException("request headers are too large");
            }
        }

        var head = Encoding.ASCII.GetString(buffer.GetRange(0, headerEnd).ToArray());
        var lines = head.Split("\r\n");
        var requestLine = lines[0].Split(' ');
        if (requestLine.Length != 3)
        {
            throw new InvalidOperationException($"malformed request line '{lines[0]}'");
        }

        var contentLength = 0;
        foreach (var line in lines[1..])
        {
            var separator = line.IndexOf(':');
            if (separator > 0
                && line[..separator].Trim().Equals("Content-Length", StringComparison.OrdinalIgnoreCase))
            {
                contentLength = int.Parse(line[(separator + 1)..].Trim());
            }
        }
        if (contentLength < 0 || contentLength > MaxBodyBytes)
        {
            throw new InvalidOperationException($"unsupported Content-Length {contentLength}");
        }

        var body = new byte[contentLength];
        var buffered = Math.Min(buffer.Count - (headerEnd + 4), contentLength);
        buffer.CopyTo(headerEnd + 4, body, 0, buffered);
        var filled = buffered;
        while (filled < contentLength)
        {
            var read = await stream.ReadAsync(body.AsMemory(filled), cancellation);
            if (read == 0)
            {
                return null;
            }
            filled += read;
        }

        return new HttpRequest(requestLine[0], requestLine[1], body);
    }

    public static Dictionary<string, string> ParseForm(byte[] body)
    {
        var form = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var pair in Encoding.ASCII.GetString(body).Split('&', StringSplitOptions.RemoveEmptyEntries))
        {
            var separator = pair.IndexOf('=');
            var name = WebUtility.UrlDecode(separator < 0 ? pair : pair[..separator]);
            var value = separator < 0 ? "" : WebUtility.UrlDecode(pair[(separator + 1)..]);
            if (!form.TryAdd(name, value))
            {
                throw new InvalidOperationException($"duplicate form field {name}");
            }
        }
        return form;
    }

    public static async Task WriteResponseAsync(
        NetworkStream stream,
        int status,
        string contentType,
        string body,
        CancellationToken cancellation)
    {
        var reason = status switch
        {
            200 => "OK",
            400 => "Bad Request",
            503 => "Service Unavailable",
            _ => "Not Found",
        };
        var bodyBytes = Encoding.UTF8.GetBytes(body);
        var head = $"HTTP/1.1 {status} {reason}\r\n"
            + $"Content-Type: {contentType}\r\n"
            + $"Content-Length: {bodyBytes.Length}\r\n"
            + "Connection: close\r\n\r\n";
        await stream.WriteAsync(Encoding.ASCII.GetBytes(head).AsMemory(), cancellation);
        await stream.WriteAsync(bodyBytes.AsMemory(), cancellation);
        await stream.FlushAsync(cancellation);
    }

    private static int FindHeaderEnd(List<byte> buffer)
    {
        for (var index = 0; index + 3 < buffer.Count; index++)
        {
            if (buffer[index] == '\r' && buffer[index + 1] == '\n'
                && buffer[index + 2] == '\r' && buffer[index + 3] == '\n')
            {
                return index;
            }
        }
        return -1;
    }
}
