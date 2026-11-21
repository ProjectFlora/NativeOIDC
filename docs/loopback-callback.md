# Loopback callback listener specification

Desktop authorization receives its callback on a loopback HTTP listener
(`http://127.0.0.1:<port>/...`). Two independent implementations exist:

- Windows: Rust, [`src/windows/loopback.rs`](../src/windows/loopback.rs)
  (`parse_redirect_template`, `wait_for_callback`,
  `handle_callback_connection`, `parse_callback_request`), driven by
  `authorize_with_browser` and `end_session_with_browser` in
  [`src/windows/ops.rs`](../src/windows/ops.rs).
- macOS: Swift, `MacOSAuthorizationAdapter` and `LoopbackHTTPListener` in
  [`Sources/NativeOIDC/MacOSAuthorizationAdapter.swift`](../Sources/NativeOIDC/MacOSAuthorizationAdapter.swift).

This document is the shared specification for both. A change to either
listener must keep it true, or update it together with the other
implementation. The listener only routes the HTTP request: the callback URL
it returns is validated by the Rust core (`parse_redirect` on the
authorization or end-session session: redirect URI match, fragment, state,
query encoding, duplicate parameters). That validation is shared and is not
repeated here.

Where the two implementations currently disagree, the specification below
states the common behavior, and the difference is listed under
[Known differences](#known-differences).

## Specification

### Address, port, and redirect URI template

- The caller supplies a redirect URI template of the form
  `http://127.0.0.1/<path>[?<query>]`. The template must use the `http`
  scheme and the literal host `127.0.0.1`, and must not contain a port, user
  information, or a fragment. `localhost`, `[::1]`, other addresses, and
  `https` are rejected before the listener is created. An empty path becomes
  `/`.
- The listener binds TCP on `127.0.0.1` (IPv4 only) with a port chosen by the
  operating system, so each operation gets its own unused port.
- The bound port is substituted into the template, and that URI is sent as
  `redirect_uri` (or `post_logout_redirect_uri`). The listener is ready before
  the browser is opened, so the callback cannot arrive before it is listening.
- Each operation has its own listener. Two concurrent operations never share
  a port, a callback, or a cancellation.

### Routing a request

- The listener reads the request line and headers, up to the first
  `\r\n\r\n`, and at most about 16 KiB. The request body is never read.
- The request target's path (the part before `?`) is compared with the
  template's path. The comparison is exact and case-sensitive on the
  percent-encoded form; the query is not part of routing.
- A request whose path does not match is **unrelated**: the listener answers
  it with an error (`404 Not Found`, or `400 Bad Request` when the request
  could not be read as a request at all) and keeps waiting. Browsers commonly
  send such requests, such as `/favicon.ico`.

### Malformed requests

A request is handled according to whether its path matches.

| Request | Response | Effect on the operation |
| --- | --- | --- |
| No `\r\n\r\n` within the size limit, or the peer closes or fails before sending complete headers | `400 Bad Request` (if the peer is still connected) | None; keep waiting |
| Unreadable request line (see differences for what each implementation rejects here) | `400 Bad Request` | None; keep waiting |
| Path does not match | `404 Not Found` | None; keep waiting |
| Path matches, but the method is not `GET` or the version is not `HTTP/1.0` / `HTTP/1.1` | `400 Bad Request` | **Terminal**: the operation fails with a malformed-callback error |
| Path matches, but there is not exactly one `Host` header equal to `127.0.0.1:<port>` | `400 Bad Request` | **Terminal** |
| Path matches, but the callback URL cannot be built from the target | `400 Bad Request` | **Terminal** |
| Path matches and the request is well formed | `200 OK`, body `Authorization complete. You may close this window.` | The callback URL is returned for core validation |

A matching request is terminal even when it is malformed, so that a broken
callback fails the operation instead of leaving it waiting until it is
cancelled. The decision depends only on parsing; whether the browser reads
the error page does not change it. Every response is `text/plain;
charset=utf-8` with `Connection: close`, and the connection is closed after
the response.

### Slow and duplicate connections

- Connections are handled concurrently. A connection that sends nothing, or
  only part of its headers, does not block a later matching callback.
- Exactly one matching request completes the wait. Other connections that
  are still open at that point are closed, and no later connection is
  accepted. Which request wins when matching requests overlap differs (D8):
  - Windows: each connection writes its response before it reports its
    result, and `wait_for_callback` takes the first result it collects from
    its `JoinSet`. Overlapping matching requests can each have received a
    response by then, and the winner is the connection whose handling
    finished first, not necessarily the one parsed first.
  - macOS: the listener actor handles parsed requests one at a time. The
    first matching request parsed wins: the listener stops, the other
    connections are closed without a response, and only the winner gets a
    response.
- The port is released once the winning request has been handled, before
  the callback is validated and before the code exchange. A second
  identical callback, for example from a browser retry, cannot connect.

### Cancellation

- Cancelling the operation while it is waiting for a matching request stops
  the wait immediately with the cancelled error, closes every open
  connection, and releases the port.
- On macOS, once a matching request has been parsed, the listener has
  already stopped and is sending that request's response; cancelling at
  that point does not interrupt the send, and the result is delivered when
  the send completes. The adapter then checks the operation's cancellation
  and returns the cancelled error (see the next items). On Windows,
  cancellation is observed while a response is being written, because the
  wait selects on it first.
- Cancellation before the wait starts ends the operation with the cancelled
  error without accepting a callback.
- For authorization, cancellation that arrives after the callback was
  received still results in the cancelled error: the code exchange observes
  it, and macOS also checks it right after the callback. The Windows
  end-session operation has no step after the callback, so cancellation at
  that point does not change its result.
- Cancelling one operation does not affect another operation's listener.

## Implementation and test map

Rust tests are in the `tests` modules of `src/windows/loopback.rs` and
`src/windows/ops.rs`, run by `cargo test --lib` on the
`x86_64-pc-windows-msvc` CI job. Swift checks are in
[`native/apple/MacOSAuthorizationFlowSmoke.swift`](../native/apple/MacOSAuthorizationFlowSmoke.swift),
run by [`scripts/check-apple-adapters.sh`](../scripts/check-apple-adapters.sh)
on the macOS CI job.

| Item | Rust implementation | Rust test | Swift implementation | Swift check |
| --- | --- | --- | --- | --- |
| Template validation | `parse_redirect_template` | `redirect_template_accepts_ipv4_loopback_and_rejects_other_hosts_or_ports` | `MacOSAuthorizationAdapter.loopbackRedirectComponents` | None |
| Bind `127.0.0.1` with an OS-chosen port | `authorize_with_browser`, `end_session_with_browser` (`TcpListener::bind`) | `independent_loopback_operations_keep_callbacks_and_cancellation_separate` | `LoopbackHTTPListener.init` (`requiredLocalEndpoint`, `acceptLocalOnly`), `start` | `checkLoopbackListener` (two listeners get different ports) |
| Port substituted into the redirect URI, listener ready before the browser opens | `authorize_with_browser`, `end_session_with_browser` | `authorization_opens_external_url_and_exchanges_validated_callback`, `end_session_with_redirect_validates_callback` | `MacOSAuthorizationAdapter.authorize` | `main` (full flow against `MacOSMockOAuthProvider`) |
| Header read and size limit | `read_request_headers` (`MAX_REQUEST_SIZE`) | None | `LoopbackHTTPListener.receive`, `received` (`maximumRequestSize`) | None |
| Unrelated path: `404`, keep waiting | `parse_callback_request` (`Unrelated`), `handle_callback_connection` | `ignores_unrelated_paths_then_accepts_matching_callback` | `LoopbackHTTPListener.received` (`respond(status: 404, ...)`) | `checkLoopbackListener` (`/favicon.ico`) |
| Matching but malformed: terminal | `parse_callback_request` (`Malformed { matches_path: true }`), `handle_callback_connection` | `matching_callback_with_invalid_host_is_terminal` | `LoopbackHTTPListener.received` (`completeCallback(.failure(...))`) | None |
| Matching callback: `200`, URL returned | `parse_callback_request` (`Matching`) | `ignores_unrelated_paths_then_accepts_matching_callback` | `LoopbackHTTPListener.received`, `completeCallback`, `resolveCallback` | `checkLoopbackListener` |
| Core validation of the returned URL | `authorize_with_browser` (`parse_redirect`) | `authorization_opens_external_url_and_exchanges_validated_callback` | `MacOSAuthorizationAdapter.authorize` (`parseRedirect`) | `checkInvalidStateAndReuse` (state mismatch) |
| Slow connection does not block | `wait_for_callback` (`JoinSet`, `CALLBACK_REQUEST_TIMEOUT`) | `stalled_connection_does_not_block_matching_callback` | Independent `NWConnection` per request | `checkLoopbackListener` (stalled socket) |
| Duplicate connections: one callback, port released | `wait_for_callback` (`abort_all`, listener dropped on return) | `duplicate_connections_return_one_callback_and_release_the_port`, `stalled_connection_does_not_block_matching_callback` | `LoopbackHTTPListener.completeCallback` | `checkLoopbackListener` (second request after the callback fails to connect) |
| Cancellation | `wait_for_callback`, `read_request_headers` (`cancellation.cancelled()`) | `cancellation_stops_the_loopback_callback_wait`, `independent_loopback_operations_keep_callbacks_and_cancellation_separate` | `LoopbackHTTPListener.cancel`, `MacOSAuthorizationAdapter.cancel`, `authorize` (`onCancel`, `isCancelled()` check) | `checkLoopbackListener` (cancel before start, cancel during wait) |
| Cancellation isolated per operation | Separate `CancellationHandle` per call | `independent_loopback_operations_keep_callbacks_and_cancellation_separate` | One listener per `authorize` call; one active call per adapter | `checkLoopbackListener` (independent listener cancelled) |

These checks drive the listener with a simulated browser (a closure or a
direct HTTP client). They do not open the real system browser.

## Known differences

The two implementations currently differ in the following cases. None of
them changes the result of a normal browser callback; they matter for
unusual or hostile local clients and for future changes. Each is a candidate
for a follow-up issue that aligns the implementations.

| # | Case | Windows (Rust) | macOS (Swift) |
| --- | --- | --- | --- |
| D1 | Connection that stays open without completing its headers | Closed without a response after 5 seconds (`CALLBACK_REQUEST_TIMEOUT`) | No timeout; stays open until the callback arrives or the operation is cancelled |
| D2 | Number of simultaneous connections | At most 32 being handled (`MAX_CALLBACK_CONNECTIONS`); further connections wait in the accept queue | No limit |
| D3 | Matching path with a header line that has no `:` | Terminal (`400`) | The line is ignored; the request can succeed |
| D4 | Request line with more than three tokens, such as `GET /callback HTTP/1.1 extra` | Rejected before routing: `400`, keep waiting | Routed by path: terminal `400` when the path matches |
| D5 | Request-line separators | Any whitespace (`split_whitespace`) | Space only |
| D6 | Target that does not start with `/` (for example the absolute form `http://127.0.0.1:<port>/callback`), or that contains `#` | `400`, keep waiting | Not starting with `/`: treated as an unrelated path (`404`). Matching path with `#`: returned as the callback, which the core rejects because of the fragment, so the operation fails |
| D7 | Header encoding | Must be UTF-8, otherwise `400`, keep waiting | Decoded as ASCII, otherwise `400`, keep waiting |
| D8 | Winner among overlapping matching requests, and when the listener stops accepting | The connection whose handling (including writing its response) finished first wins; overlapping matching requests can all have received a response. The listener stops after the winner's response is written | The first matching request parsed wins and only it gets a response. The listener stops as soon as that request is parsed, before its response is written; cancellation during that send waits for it to complete |
| D9 | Path comparison against the template | The template path as normalized by the `url` crate (dot segments removed, characters percent-encoded) | `URLComponents.percentEncodedPath` as written in the template, without normalization. A template path with `.` or `..` segments can therefore never match on macOS, because browsers remove dot segments before sending the request |
| D10 | Loopback post-logout redirect | Supported: `end_session_with_browser` uses the same listener | Not provided by `MacOSAuthorizationAdapter` |

Until they are aligned, use redirect URI templates whose path has no `.` or
`..` segments and no characters that need percent-encoding.

Test coverage also differs: the Swift checks do not cover template
validation, terminal handling of malformed matching requests (such as a
`Host` mismatch), or the request size limit, and neither implementation has
a test for the size limit.

## Mobile redirect matching

iOS/iPadOS and Android do not use a loopback listener. The operating system
delivers the callback to the application, and the adapter only decides
whether a delivered URL belongs to the pending operation. As on desktop,
the delivered URL is then validated by the Rust core's `parse_redirect`.

### iOS/iPadOS

`IOSAuthorizationAdapter.callback(for:)` in
[`Sources/NativeOIDC/IOSAuthorizationAdapter.swift`](../Sources/NativeOIDC/IOSAuthorizationAdapter.swift)
turns the redirect URI into an `ASWebAuthenticationSession.Callback`:

- `https`: requires a host, port 443 or none, no user information, and no
  fragment, and matches on host and path (an empty path becomes `/`). The
  application must have the associated domain; see the
  [HTTPS callback domain](https-callback-domain.md).
- Any other scheme except `http`: matches on the custom scheme. `http` is
  rejected.

`ASWebAuthenticationSession` routes the callback to the session, so the
adapter receives at most one URL per operation.

### Android

[`AndroidRedirectSelector.kt`](../native/android/src/main/kotlin/net/projectflora/nativeoidc/AndroidRedirectSelector.kt)
selects the redirect URI, and `PendingRedirect` / `RedirectTarget` in
[`AndroidAuthorizationAdapter.kt`](../native/android/src/main/kotlin/net/projectflora/nativeoidc/AndroidAuthorizationAdapter.kt)
match delivered callbacks:

- Selection: a verified App Link (`https`, a host, port 443 or none) whose
  callback activity is registered is preferred; otherwise a registered
  hierarchical custom-scheme URI (not `http` or `https`) is used. Both must
  be absolute and have no user information or fragment.
- Matching: a delivered URL belongs to the operation when its scheme and host
  (IDN-normalized, case-insensitive), port (default 443 for `https`, 80 for
  `http`), and path (an empty path becomes `/`) equal the selected URI. The
  query and fragment are ignored for matching, so a malformed response on the
  matching route reaches the core and fails the operation, while URLs for
  other routes are ignored.
- Only the first matching delivery completes the operation; later deliveries
  are ignored.

Tests: [`AndroidRedirectSelectorTest.kt`](../native/android/src/test/kotlin/net/projectflora/nativeoidc/AndroidRedirectSelectorTest.kt)
(`redirectTargetIgnoresResponseQueryButRequiresExactRoute`,
`callbackRegistryIgnoresOtherRoutesAndDeliversTheMatchingCallback`,
`registryRejectsConcurrentRegistrationAndIgnoresDuplicateDelivery`,
`matchingMalformedCallbacksReachCoreValidation`,
`concurrentDeliveriesCompleteOnlyOnce`, and the selection tests).
