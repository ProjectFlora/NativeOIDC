# Swift Guide

This guide walks through the public API on iOS/iPadOS and macOS, from
installation through post-authorization operations. Examples use the shipped
Swift package: the UniFFI-generated API, the platform adapters
(`IOSAuthorizationAdapter`, `MacOSAuthorizationAdapter`), and
`URLSessionNativeHttpTransport`. Common behavior — state persistence rules,
logout semantics, error classification, and timeout policy — is documented in
the [shared user guide](user-guide.md); this guide links to it instead of
repeating it. The [API coverage table](api-coverage.md) maps each operation to
the exact public API used below.

## Supported platforms and language mode

- iOS/iPadOS 18+ and macOS 15+. Mac Catalyst, tvOS, and watchOS are not
  included.
- A Swift 6.0+ toolchain (Xcode 16+) is required to build the package.
- The `NativeOIDC` package target compiles in **Swift 5 language mode**. The
  UniFFI-generated API is not Swift 6 strict-concurrency clean, so the package
  pins its target to Swift 5 mode in `Package.swift`. Your app target keeps its
  own language mode; the two coexist in the same app.
- Both adapters are `@MainActor`: create them, call `authorize`, and call
  `cancel` on the main actor.

## Installation

### Swift Package Manager (recommended, v0.5.0+)

In Xcode, choose **File > Add Package Dependencies...**, enter
`https://github.com/ProjectFlora/NativeOIDC`, set **Dependency Rule** to
**Up to Next Minor Version** from `0.8.0`, and add the `NativeOIDC` library to
your app target. In a `Package.swift` manifest:

```swift
dependencies: [
    .package(
        url: "https://github.com/ProjectFlora/NativeOIDC",
        .upToNextMinor(from: "0.8.0")
    ),
],
targets: [
    .target(
        name: "MyApp",
        dependencies: [
            .product(name: "NativeOIDC", package: "NativeOIDC"),
        ]
    ),
],
```

Then `import NativeOIDC` in your Swift code. Pin to a released version, not
`main`: each release tag's `Package.swift` pins the release archive's URL and
checksum, but the release workflow never updates `main`'s manifest, so
depending on `main` mixes an older binary with newer Swift sources. See
[Apple Swift Package Manager](../README.md#apple-swift-package-manager-spm)
for the manifest rules and the pre-1.0 versioning policy.

### XCFramework archive

To integrate without SPM, download and verify the release archive from the
[GitHub Release](https://github.com/ProjectFlora/NativeOIDC/releases/tag/v0.8.0)
and add its `NativeOIDC.xcframework` and `Swift/` sources to the app target,
both from the same release. See
[Swift XCFramework installation](swift-xcframework.md).

The examples below work with either installation method, except the
`import NativeOIDC` lines: the module exists only in the Swift Package
Manager installation. If you integrated the archive's Swift sources directly
into the app target, omit that import; every other line of the examples is
unchanged.

## Configuration, transport, and callbacks

The operations in this guide start from two host-supplied objects:

```swift
import NativeOIDC

// URLSession-based transport: returns HTTP redirects without following them
// and disables HTTP caching so the Rust core controls JWKS freshness.
let transport = URLSessionNativeHttpTransport()

// Shared cancellation signal; create one per operation.
let cancellation = FfiCancellationHandle()
```

`NativeHttpTransport` is a callback interface, not a concrete type — instantiate
the shipped `URLSessionNativeHttpTransport` (shown above) or implement the
protocol for custom networking. A custom transport must return HTTP redirects
without following them and propagate cancellation.

### Service configuration

Discover the provider's metadata over HTTPS:

```swift
// Fetches and validates the OIDC discovery document.
let configuration = try await FfiServiceConfiguration.discover(
    issuer: "https://your-idp.example",
    transport: transport,
    cancellation: cancellation
)
```

Or construct it from explicit endpoints when the provider is not discovery
capable. Requests with the `openid` scope require OIDC validation
configuration:

```swift
let configuration = try FfiServiceConfiguration(
    authorizationEndpoint: "https://your-idp.example/authorize",
    tokenEndpoint: "https://your-idp.example/token"
)
.withOidcValidationEndpoints(
    issuer: "https://your-idp.example",
    jwksUri: "https://your-idp.example/jwks"
)
```

Optional endpoints are added the same way; each must be supported by the
provider before the operation that uses it:

```swift
let configuration = try configuration
    .withEndSessionEndpoint(endpoint: "https://your-idp.example/session/end")
    .withRevocationEndpoint(endpoint: "https://your-idp.example/revoke")
    .withRegistrationEndpoint(endpoint: "https://your-idp.example/register")
```

### Authorization options

```swift
let options = FfiAuthorizationOptions(
    clientId: "native-public-client",
    // Must exactly match the redirect registered with the provider.
    redirectUri: "https://your-app.example/oidc/callback",
    scopes: ["openid", "profile"],
    // Optional pass-through for the authorization request and token exchange.
    additionalParameters: [FfiParameter(name: "prompt", value: "consent")],
    additionalTokenParameters: []
)
```

Provider registration — a native public client, the exact redirect URIs, and
the optional endpoints above — is documented in
[Configure a native public client](user-guide.md#configure-a-native-public-client).

### Platform callback setup

- **iOS/iPadOS**: the adapter runs the authorization in
  `ASWebAuthenticationSession` and requires a presentation context provider.
  For an HTTPS redirect, configure the `webcredentials:<host>` associated
  domain and the matching server association; for a custom-scheme fallback,
  register the scheme in the app target and with the provider. See
  [Configure authorization callbacks](swift-xcframework.md#configure-authorization-callbacks).
- **macOS**: the adapter binds an ephemeral port on IPv4 loopback and opens the
  authorization URL in the external browser. Use a loopback redirect template
  such as `http://127.0.0.1/oauth/callback` in both the provider registration
  and `redirectUri`. See [loopback callback](loopback-callback.md).

## Authorization

### iOS/iPadOS

```swift
import AuthenticationServices
import NativeOIDC

/// Supplies the presentation anchor for the authentication session.
final class PresentationProvider: NSObject,
    ASWebAuthenticationPresentationContextProviding {
    func presentationAnchor(
        for session: ASWebAuthenticationSession
    ) -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first(where: \.isKeyWindow) ?? ASPresentationAnchor()
    }
}

@MainActor
final class AuthService {
    private let adapter = IOSAuthorizationAdapter(
        // Set true to avoid sharing cookies with the user's regular browsing session.
        presentationContextProvider: PresentationProvider(),
        prefersEphemeralWebBrowserSession: true
    )

    /// Opens the authentication session and completes code exchange.
    func signIn(
        configuration: FfiServiceConfiguration,
        options: FfiAuthorizationOptions,
        transport: URLSessionNativeHttpTransport
    ) async throws -> FfiAuthenticationState {
        try await adapter.authorize(
            configuration: configuration,
            options: options,
            transport: transport
        )
    }

    /// Cancels the browser session and any in-flight core request.
    func cancel() {
        adapter.cancel()
    }
}
```

The host cancels by calling `cancel()` or by cancelling the surrounding Swift
task; both dismiss the session and stop the in-flight core work.

### macOS

```swift
import AppKit
import NativeOIDC

@MainActor
final class AuthService {
    // The default closure opens the URL in the default browser via NSWorkspace;
    // hosts that manage browser presentation themselves can supply their own.
    private let adapter = MacOSAuthorizationAdapter()

    /// Binds the loopback listener, opens the browser, and exchanges the callback code.
    func signIn(
        configuration: FfiServiceConfiguration,
        options: FfiAuthorizationOptions,
        transport: URLSessionNativeHttpTransport
    ) async throws -> FfiAuthenticationState {
        try await adapter.authorize(
            configuration: configuration,
            options: options,
            transport: transport
        )
    }

    /// Stops the listener and any in-flight core request. It cannot dismiss
    /// the external browser.
    func cancel() {
        adapter.cancel()
    }
}
```

### Using the result

Call `signIn` from any `@MainActor` context; the adapter returns
`FfiAuthenticationState`, which holds the tokens, the OIDC nonce, and the
expiry. The object is released by ARC, which frees its Rust handle.

```swift
@MainActor
func performSignIn(
    configuration: FfiServiceConfiguration,
    options: FfiAuthorizationOptions,
    transport: URLSessionNativeHttpTransport
) async throws -> FfiAuthenticationState {
    let authService = AuthService() // the platform adapter of choice
    let state = try await authService.signIn(
        configuration: configuration,
        options: options,
        transport: transport
    )
    // Return the state to the host: it holds the tokens, the OIDC nonce, and
    // the expiry that the persistence, refresh, and end-session operations
    // below require, and dropping it would leave the host with nothing to
    // act on.
    return state
}
```

Do not reconstruct state from only an access token; expiry, refresh token, and
the OIDC nonce are needed for later operations. See
[Authorize and retain state](user-guide.md#authorize-and-retain-state).

## Token access and expiry

```swift
let tokens = await state.tokenResponse()
// tokens.accessToken, tokens.tokenType ("Bearer"), tokens.expiresIn,
// tokens.refreshToken, tokens.scope, and the verified tokens.idToken

let expiresAt = await state.expiresAt() // Unix seconds, if known
let expired = await state.isExpiredAt(
    unixTimeSeconds: UInt64(Date.now.timeIntervalSince1970.rounded())
)
```

An unknown expiry is not proof that a token is valid indefinitely, and the
library does not refresh tokens automatically. Use the access token for
resource-server calls; ID Tokens are identity assertions. See
[Authorize and retain state](user-guide.md#authorize-and-retain-state).

## Secure state persistence

Serialize the complete state and persist it in host-managed secure storage:

```swift
import Foundation
import Security

enum StateStorageError: Error {
    case add(OSStatus)
    case missing
}

func saveState(_ json: String, account: String) throws {
    let delete: [CFString: Any] = [
        kSecClass: kSecClassGenericPassword,
        kSecAttrService: "com.example.app",
        kSecAttrAccount: account,
    ]
    SecItemDelete(delete as CFDictionary)
    var add = delete
    add[kSecValueData] = Data(json.utf8)
    let status = SecItemAdd(add as CFDictionary, nil)
    guard status == errSecSuccess else { throw StateStorageError.add(status) }
}

func loadState(account: String) throws -> String {
    let query: [CFString: Any] = [
        kSecClass: kSecClassGenericPassword,
        kSecAttrService: "com.example.app",
        kSecAttrAccount: account,
        kSecReturnData: true,
        kSecMatchLimit: kSecMatchLimitOne,
    ]
    var item: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &item)
    guard status == errSecSuccess, let data = item as? Data else {
        throw StateStorageError.missing
    }
    return String(decoding: data, as: UTF8.self)
}
```

```swift
// After authorization: serialize the complete state, then store it.
// Keep tokens, nonce, and state JSON out of logs.
let stateJson = try await state.toJson()
try saveState(stateJson, account: "native-oidc/state")

// On the next launch: restore it before any operation that needs the state.
// Restoration does not contact the provider or prove that previously issued
// tokens are still accepted.
let state = try FfiAuthenticationState.fromJson(
    serialized: try loadState(account: "native-oidc/state")
)
```

The Keychain item above is a sketch: the application still chooses the
accessibility class, scopes the item per user and issuer, and makes the write
atomic. Retain the original absolute expiry instead of recalculating it from
`expires_in` on each launch. See
[Save, restore, and refresh](user-guide.md#save-restore-and-refresh).

## Refresh

Refresh requires a non-empty refresh token and updates `state` in place only
after a successful response. Local sign-out sets the `signedOut` flag under
the same lock, so a refresh that outlives sign-out cannot write the deleted
state back:

```swift
// Shared with the local sign-out in "End-session and revocation".
let sessionLock = NSLock()
var signedOut = false

try await state.refresh(
    configuration: configuration,
    clientId: "native-public-client",
    additionalParameters: [],
    transport: transport,
    cancellation: FfiCancellationHandle()
)

// Refresh tokens may rotate; persist the updated complete state. Serialize
// the state outside the lock — `toJson` is a core request, and the lock
// must not be held across it — then check the flag under the lock.
let stateJson = try await state.toJson()
sessionLock.lock()
if !signedOut {
    try saveState(stateJson, account: "native-oidc/state")
}
sessionLock.unlock()
```

On failure or cancellation the local previous state is unchanged, which does
not guarantee that the server still accepts its tokens. See
[Save, restore, and refresh](user-guide.md#save-restore-and-refresh) for
`invalid_grant` handling and the refresh/logout serialization rule.

## End-session and revocation

The authorization adapters have **no** logout method. Provider logout uses the
UniFFI end-session API, and the host manages the browser and any post-logout
callback:

```swift
// Register the app's post-logout redirect with the provider; a random state
// is then generated and validated on return.
let endSession = try beginEndSession(
    configuration: configuration, // requires an end-session endpoint
    options: FfiEndSessionOptions(
        clientId: "native-public-client",
        idTokenHint: await state.tokenResponse().idToken,
        postLogoutRedirectUri: "https://your-app.example/oidc/post-logout"
    )
)

let logoutURL = URL(string: endSession.endSessionUrl())!
// Open `logoutURL` in the external browser: ASWebAuthenticationSession on
// iOS/iPadOS, NSWorkspace on macOS. Retain the prepared `endSession` until
// the callback returns.
// When the provider returns the app to the registered redirect, validate the
// callback it carried:
do {
    let response = try endSession.parseRedirect(redirectUri: callbackURL.absoluteString)
    // `response.state` matched the generated state; the provider completed
    // the logout.
} catch {
    // The callback did not validate (missing or mismatched state): report
    // the provider logout as unverified.
}
```

With `postLogoutRedirectUri: nil` there is no callback to validate: opening
the browser is not confirmation that the provider completed logout.

Token revocation requests invalidation of the supplied token. HTTP 200
completes the operation even for an already-invalid token; it does not delete
local state, and failure does not confirm remote invalidation:

```swift
try await revoke(
    configuration: configuration, // requires a revocation endpoint
    clientId: "native-public-client",
    token: tokens.refreshToken ?? tokens.accessToken,
    tokenTypeHint: tokens.refreshToken != nil ? "refresh_token" : "access_token",
    transport: transport,
    cancellation: FfiCancellationHandle()
)
```

Local sign-out is independent of both. Set the sign-out flag under the same
lock the refresh write takes, then clear the stored state (for example
`SecItemDelete` for the item above) and drop in-memory references, and run it
even if provider cleanup fails or is cancelled:

```swift
sessionLock.lock()
signedOut = true
sessionLock.unlock()
```

See
[Logout, revocation, and local deletion](user-guide.md#logout-revocation-and-local-deletion).

## Dynamic registration

```swift
let registered = try await registerPublicClient(
    configuration: configuration, // requires a registration endpoint
    options: FfiRegistrationOptions(
        redirectUris: ["https://your-app.example/oidc/callback"],
        clientName: "Example App",
        initialAccessToken: nil // only for a protected registration endpoint
    ),
    transport: transport,
    cancellation: FfiCancellationHandle()
)
let clientId = registered.clientId
```

## Cancellation and errors

`cancel()` on the adapter or cancellation of the surrounding Swift task stops
active work where supported; it cannot undo a request the provider already
processed. Handles are shared signals and the host owns timeout policy. See
[Cancellation, timeouts, and errors](user-guide.md#cancellation-timeouts-and-errors).

Direct UniFFI operations throw the `FfiError` cases:

| Failure | `FfiError` case |
| --- | --- |
| Caller configuration | `InvalidRequest` |
| Invalid protocol response or ID Token | `InvalidResponse` |
| Provider rejection | `AuthorizationServer(code, description)` |
| Registration rejected | `ClientRegistrationServer(code, description)` |
| Unexpected HTTP status | `HttpStatus(status)` |
| Transport failure | `Transport`, `TransportFailure` |
| Cancellation | `Cancelled` |
| Clock / randomness | `Clock`, `Randomness` |
| Internal | `Internal` |

The adapters additionally throw their platform errors:
`IOSAuthorizationAdapterError` (session already running, session could not
start, invalid authorization URL, invalid callback URL, missing callback) and
`MacOSAuthorizationAdapterError` (session already running, browser could not
be opened, invalid authorization URL, invalid loopback redirect, malformed
loopback request). Present provider descriptions as untrusted diagnostic text,
and apply host retry policy to transient failures rather than validation
errors.

## Objective-C interoperability

Objective-C applications have no library-provided entry point: UniFFI generates
Swift bindings only, so hosts wrap the Swift API in a small `@objc` class that
runs the async work in a Swift task. The wrapper is where main-thread hops and
lifecycle handling belong:

- The adapter initializers take **platform configuration only**:
  `IOSAuthorizationAdapter` takes a presentation context provider (plus the
  optional ephemeral-session flag), and `MacOSAuthorizationAdapter` takes an
  optional browser-opening closure. Service configuration, options, and the
  transport are supplied to `authorize(configuration:options:transport:)`.
- `NativeHttpTransport` is a callback interface, not a concrete type — supply
  the shipped `URLSessionNativeHttpTransport`.
- `FfiAuthenticationState` is a Swift class invisible to Objective-C. Pass the
  state across the boundary as the JSON from `toJson()`, and restore it in
  Swift with `FfiAuthenticationState.fromJson(serialized:)` when a later
  operation needs it.
- Both adapters are `@MainActor`. A `@objc` entry point on a `@MainActor`
  class traps when called off the main thread, so call the wrapper from the
  main thread.
- Discovery is a core request, not an adapter operation: `cancel()` on the
  adapter alone would not stop a suspended discovery, and the task would
  still open the browser when it resumes. The wrapper therefore retains the
  discovery cancellation handle and the task, signals both from `cancel()`,
  and checks for cancellation before starting the adapter.
- The wrapper claims a single in-flight slot before discovery starts. A
  second `authorize` call while an operation is in flight fails with
  `AuthorizationInProgress` instead of replacing the first operation's task
  and handle, which would leave its discovery un-cancellable and let it open
  the browser when it resumes. The slot is released when the operation ends.

### iOS/iPadOS wrapper

```swift
import AuthenticationServices
import NativeOIDC

/// Supplies the presentation anchor for the authentication session.
final class PresentationProvider: NSObject,
    ASWebAuthenticationPresentationContextProviding {
    func presentationAnchor(
        for session: ASWebAuthenticationSession
    ) -> ASPresentationAnchor {
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first(where: \.isKeyWindow) ?? ASPresentationAnchor()
    }
}

/// An `authorize` call was rejected because an operation is already in
/// flight.
struct AuthorizationInProgress: Error {}

@MainActor
@objc public final class NativeOIDCService: NSObject {
    private let adapter: IOSAuthorizationAdapter
    private var activeCancellation: FfiCancellationHandle?
    private var activeTask: Task<Void, Never>?
    // Bumped for every claimed operation, so a superseded task cannot
    // release a slot that a newer operation owns.
    private var inFlightGeneration = 0

    @objc public override init() {
        self.adapter = IOSAuthorizationAdapter(
            presentationContextProvider: PresentationProvider(),
            prefersEphemeralWebBrowserSession: false
        )
        super.init()
    }

    /// Runs authorization in the system browser session. The completion
    /// handler reports the serialized state JSON (or `nil`) and any error.
    /// A call that arrives while an operation is in flight reports
    /// `AuthorizationInProgress`.
    @objc public func authorize(
        issuer: String,
        clientId: String,
        redirectUri: String,
        scopes: [String],
        completion: @escaping @Sendable (String?, Error?) -> Void
    ) {
        // Claim the single in-flight slot before discovery starts: a second
        // call must be rejected here, not replace the first operation's task
        // and handle, which would leave its discovery un-cancellable.
        guard activeTask == nil else {
            completion(nil, AuthorizationInProgress())
            return
        }
        inFlightGeneration += 1
        let generation = inFlightGeneration
        let transport = URLSessionNativeHttpTransport()
        let cancellation = FfiCancellationHandle()
        let task = Task { @MainActor in
            do {
                let configuration = try await FfiServiceConfiguration.discover(
                    issuer: issuer,
                    transport: transport,
                    cancellation: cancellation
                )
                // A cancel that lands between discovery and the adapter must
                // not start the session either.
                guard !cancellation.isCancelled() else {
                    throw FfiError.Cancelled
                }
                let options = FfiAuthorizationOptions(
                    clientId: clientId,
                    redirectUri: redirectUri,
                    scopes: scopes,
                    additionalParameters: [],
                    additionalTokenParameters: []
                )
                let state = try await adapter.authorize(
                    configuration: configuration,
                    options: options,
                    transport: transport
                )
                completion(try await state.toJson(), nil)
            } catch {
                completion(nil, error)
            }
            // Release the slot once this operation has fully ended —
            // success, failure, or cancellation — only if it is still the
            // claimed one.
            if inFlightGeneration == generation {
                activeTask = nil
                activeCancellation = nil
            }
        }
        activeCancellation = cancellation
        activeTask = task
    }

    /// Cancels the active browser session, any in-flight core request, and a
    /// suspended discovery request alike.
    @objc public func cancel() {
        adapter.cancel()
        activeCancellation?.cancel()
        activeTask?.cancel()
        activeCancellation = nil
        activeTask = nil
    }
}
```

### macOS wrapper

```swift
import AppKit
import NativeOIDC

/// An `authorize` call was rejected because an operation is already in
/// flight.
struct AuthorizationInProgress: Error {}

@MainActor
@objc public final class NativeOIDCService: NSObject {
    private let adapter: MacOSAuthorizationAdapter
    private var activeCancellation: FfiCancellationHandle?
    private var activeTask: Task<Void, Never>?
    // Bumped for every claimed operation, so a superseded task cannot
    // release a slot that a newer operation owns.
    private var inFlightGeneration = 0

    @objc public override init() {
        // The default closure opens the URL in the default browser.
        self.adapter = MacOSAuthorizationAdapter()
        super.init()
    }

    /// Binds the loopback listener and opens the external browser. The
    /// completion handler reports the serialized state JSON (or `nil`) and
    /// any error. A call that arrives while an operation is in flight
    /// reports `AuthorizationInProgress`.
    @objc public func authorize(
        issuer: String,
        clientId: String,
        redirectUri: String,
        scopes: [String],
        completion: @escaping @Sendable (String?, Error?) -> Void
    ) {
        // Claim the single in-flight slot before discovery starts: a second
        // call must be rejected here, not replace the first operation's task
        // and handle, which would leave its discovery un-cancellable.
        guard activeTask == nil else {
            completion(nil, AuthorizationInProgress())
            return
        }
        inFlightGeneration += 1
        let generation = inFlightGeneration
        let transport = URLSessionNativeHttpTransport()
        let cancellation = FfiCancellationHandle()
        let task = Task { @MainActor in
            do {
                let configuration = try await FfiServiceConfiguration.discover(
                    issuer: issuer,
                    transport: transport,
                    cancellation: cancellation
                )
                // A cancel that lands between discovery and the adapter must
                // not start the listener either.
                guard !cancellation.isCancelled() else {
                    throw FfiError.Cancelled
                }
                let options = FfiAuthorizationOptions(
                    clientId: clientId,
                    redirectUri: redirectUri,
                    scopes: scopes,
                    additionalParameters: [],
                    additionalTokenParameters: []
                )
                let state = try await adapter.authorize(
                    configuration: configuration,
                    options: options,
                    transport: transport
                )
                completion(try await state.toJson(), nil)
            } catch {
                completion(nil, error)
            }
            // Release the slot once this operation has fully ended —
            // success, failure, or cancellation — only if it is still the
            // claimed one.
            if inFlightGeneration == generation {
                activeTask = nil
                activeCancellation = nil
            }
        }
        activeCancellation = cancellation
        activeTask = task
    }

    /// Cancels the active browser session, any in-flight core request, and a
    /// suspended discovery request alike.
    @objc public func cancel() {
        adapter.cancel()
        activeCancellation?.cancel()
        activeTask?.cancel()
        activeCancellation = nil
        activeTask = nil
    }
}
```

## Example verification

The Swift examples in this guide are exercised at the following scope:

- **Build**: the examples are mirrored in the Apple release consumer app
  (`native/consumer-smoke/apple`, with the per-platform halves in
  `GuideExamplesMacOS.swift` and `GuideExamplesIOS.swift`).
  `scripts/check-apple-release-consumer.sh` compiles the mirror against the
  release archive's generated bindings, adapters, and XCFramework — the
  distributed artifacts, not the module sources — for a macOS host (which it
  then runs) and for the iOS simulator (linked). The published `v0.8.0`
  simulator slice is arm64-only, so the iOS build excludes `x86_64`.
- **Runtime**: the macOS host runs the representative state-update,
  cancellation, and error branches the examples rely on (#85): a scripted
  refresh that updates the state in place and round-trips it through JSON, an
  in-flight discovery ended by cancellation, a provider-rejected refresh,
  malformed stored state, the post-logout callback with a registered redirect
  (a matching state validates, a mismatched one fails; both through the
  prepared-session lifecycle entry points), and two overlapping
  authorizations (the second is rejected, the first ends cancelled while its
  discovery is in flight, and the single-flight slot is released after). The
  authorization, callback-exchange, and cancellation paths are additionally
  exercised by the repository's Swift test suite
  (`SwiftTests/NativeOIDCTests`, a macOS loopback flow against a local mock
  provider; `swift test` at `v0.8.0` ran 44 tests with 0
  failures) and by the consumer app (see [Release consumer
  validation](../README.md#release-consumer-validation)). The refresh,
  end-session, revocation, and registration examples are not executed against
  a real IdP or on real devices, consistent with the repository's
  [live IdP interoperability](live-idp-interoperability.md) policy.

### Example verification commands

From a checkout that includes the consumer app, against a downloaded and
checksum-verified release archive:

```sh
scripts/check-apple-release-consumer.sh dist/NativeOIDC-Apple-0.8.0.zip
```

From a checkout at tag `v0.8.0`:

```sh
# Package build for macOS, including the generated API and both adapters.
swift build -c release

# macOS loopback authorization flow against a local mock provider.
swift test

# iOS simulator build of the package (arm64-only published slice).
xcodebuild -scheme NativeOIDC -destination 'generic/platform=iOS Simulator' \
    EXCLUDED_ARCHS=x86_64 build
```

## Apple adapter notes

See the [Apple adapter notes](../README.md#apple-adapters) for callback
behavior, cancellation, and native HTTP transport requirements.
