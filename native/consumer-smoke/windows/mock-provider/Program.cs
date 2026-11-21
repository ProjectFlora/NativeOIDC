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
    Console.Error.WriteLine(
        $"Mock provider: timed out waiting for {string.Join(", ", expectations.Keys)}.");
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
