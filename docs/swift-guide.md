# Swift Guide

This guide covers the public API usage for iOS/iPadOS and macOS applications.

## Supported platforms

- iOS/iPadOS 18+
- macOS 15+
- Xcode 16+
- Swift 6.4+

The library is delivered as a static `NativeOIDC.xcframework` or via Swift Package Manager (v0.5.0+).
See [Swift XCFramework installation](swift-xcframework.md) for manual integration details.

## Objective-C interoperability

Objective-C applications have no library-provided entry point. UniFFI generates Swift bindings only,
so hosts must provide a wrapper:

- **Wrap the Swift API in an `@objc` class** with completion-handler methods that run the async work
  in a Swift task. The wrapper is where main-thread hops and lifecycle handling belong.
- Both adapters (`IOSAuthorizationAdapter` and `MacOSAuthorizationAdapter`) are `@MainActor`.
- Authorization requires platform configuration (presentation context provider for iOS, loopback URL for macOS),
  options (timeout policy, callback scheme fallback), and a native HTTP transport.
- The adapter returns `FfiAuthenticationState` after authorization; the wrapper should retain or forward this state
  to callers who need it for refresh or exchange operations.

Example pattern (iOS/iPadOS):

```swift
@objc class NativeOIDCWrapper: NSObject {
    // iOS initializer needs a presentation context provider and platform configuration
    private let adapter = IOSAuthorizationAdapter(
        configuration: .init(
            issuer: "https://your-idp.example/.well-known/openid-configuration",
            clientId: "your-client-id"
        ),
        options: .init(timeout: 30.0) // seconds
    )
    
    @objc func authorizeWithCompletion(state: String, callbackURLScheme: String?, 
                                      completion: @escaping (Error?, FfiAuthenticationState?) -> Void) {
        Task { @MainActor in
            do {
                let result = try await adapter.authorize(
                    configuration: .init(
                        issuer: "https://your-idp.example/.well-known/openid-configuration",
                        clientId: "your-client-id"
                    ),
                    options: .init(timeout: 30.0),
                    transport: NativeHttpTransport() // or your custom transport
                )
                
                // Forward state to caller if needed
                completion(nil, result)
            } catch {
                completion(error, nil)
            }
        }
    }
    
    @objc func cancel() {
        adapter.cancel()
    }
}
```

Example pattern (macOS):

```swift
@objc class NativeOIDCWrapper: NSObject {
    // macOS uses loopback redirect; configure ephemeral port or fixed URL
    private let adapter = MacOSAuthorizationAdapter(
        configuration: .init(
            issuer: "https://your-idp.example/.well-known/openid-configuration",
            clientId: "your-client-id"
        ),
        options: .init(timeout: 30.0)
    )
    
    @objc func authorizeWithCompletion(state: String, 
                                      completion: @escaping (Error?, FfiAuthenticationState?) -> Void) {
        Task { @MainActor in
            do {
                let result = try await adapter.authorize(
                    configuration: .init(
                        issuer: "https://your-idp.example/.well-known/openid-configuration",
                        clientId: "your-client-id"
                    ),
                    options: .init(timeout: 30.0),
                    transport: NativeHttpTransport() // or your custom transport
                )
                
                completion(nil, result)
            } catch {
                completion(error, nil)
            }
        }
    }
    
    @objc func cancel() {
        adapter.cancel()
    }
}
```

This pattern allows legacy Objective-C code to use the library's async API with traditional
completion handlers. The wrapper is where timeout policy, cancellations, and lifecycle
events are managed.

## Complete Swift usage guide

For iOS/iPadOS and macOS applications that call the native adapters directly (not through a wrapper),
follow this complete path from installation through refresh operations. See the full documentation below.

### Configuration

Register your public client with the provider and configure transport:

```swift
let configuration = AuthorizationConfiguration(
    issuer: "https://your-idp.example/.well-known/openid-configuration",
    clientId: "your-client-id"
)

// For mobile, use verified HTTPS callback or custom scheme fallback
let transport = URLSessionNativeHttpTransport()
```

### Authorization and state retention

Start the platform callback handler or loopback listener before opening the browser.
The adapters manage this for authorization.

```swift
try await adapter.authorize(
    configuration: configuration,
    options: .init(timeout: 30.0),
    transport: transport
)
```

Read token values from state to call the resource server. Access-token expiry is an optional Unix timestamp in seconds. An unknown expiry is not proof that a token is valid indefinitely. The library does not automatically refresh tokens or retry resource-server calls. ID Tokens are identity assertions; use the access token for resource-server authorization.

### Save, restore, and refresh

Serialize the complete state using `to_json` in UniFFI; persist it using host-managed secure storage and encryption. Keep tokens, nonce, and state JSON out of logs. Restore the full JSON using `from_json`. Restoration does not contact the provider or prove that previously issued tokens are still accepted.

The host associates stored state with the correct issuer, client ID, and configuration; state JSON does not contain that complete configuration. Retain the original absolute expiry instead of recalculating it from `expires_in` on each application launch.

Refresh requires a non-empty refresh token. UniFFI updates state only after a successful response and any returned ID Token validation. Persist the updated complete state after every success, including refresh-token rotation. A missing refresh token, scope, or ID Token in a successful refresh response retains the old value.

On failure or cancellation, the local previous state is unchanged. Serialize refresh and logout at the application level so a late refresh cannot restore state after local logout. Protect the write to persistent storage as well as the in-memory update.

### Logout, revocation, and local deletion

- **End-session**: Opens the provider's RP-Initiated Logout URL; optionally validates a post-logout callback. Manage browser/callback lifecycle where no logout adapter exists; delete local state separately.
- **Token revocation**: Requests invalidation of the supplied access or refresh token. Choose the token and optional type hint; own retries; delete local state even on failure or cancellation.
- **Local state deletion**: Removes the application's stored credentials. Clear stored and in-memory state; prevent concurrent operations from writing it back.

End-session requires an HTTPS end-session endpoint. An ID Token hint and post-logout redirect are optional. With a redirect, retain the prepared logout session and validate the callback/state. Without one, opening the browser is not confirmation that the provider completed logout. Swift exposes URL construction and callback validation, but existing authorization adapters do not provide a logout method. Rust similarly leaves browser work to the host. Windows handles the browser and optional loopback callback.

Revocation requires an HTTPS revocation endpoint; an explicitly configured one may also use loopback HTTP, as the token endpoint can. HTTP 200 completes the operation even for an already-invalid token; it does not delete local state. Failure does not confirm remote invalidation. The library does not retry automatically or expose Retry-After. If logout policy attempts network cleanup, retain only the values needed for that attempt, and ensure local deletion runs even if cleanup fails or is cancelled. Do not make local sign-out depend on provider availability.

### Cancellation, timeouts, and errors

Cancellation handles are shared signals. Calling `cancel` is idempotent; a cancelled handle stays cancelled, so use a new handle for a new operation. The host owns authorization timeout policy. Cancellation stops active work where supported, but cannot undo a request already processed by the provider. macOS and Windows cannot dismiss the external browser; Android does not promise to close the Custom Tab. iOS cancels its authentication session. Swift Task and Kotlin coroutine cancellation are connected by the supplied authorization adapters; direct core/UniFFI operations use their explicit handle.

| Failure category | UniFFI |
| --- | --- |
| Caller configuration | `InvalidConfiguration` |
| Invalid protocol response or ID Token | Specific validation errors |
| Provider rejection | `AuthorizationServer`, `ClientRegistrationServer` with code and optional description |
| HTTP or network | `HttpStatus`, `Transport`, `TransportFailure` |
| Cancellation | `Cancelled` |
| Platform/browser/listener | Adapter-specific errors |

Rust also exposes clock/randomness failures; UniFFI exposes clock/randomness and internal failures. Rust's error enum is non-exhaustive. Preserve cancellation as a distinct result, avoid parsing Windows error text into a protocol contract, and present provider descriptions as untrusted diagnostic text. Apply host retry policy to transient failures; validation errors need investigation rather than blind retries.

### Example verification commands

```bash
# Build the XCFramework release artifact
cargo build --release --target x86_64-apple-darwin \
  -p native_oidc \
  --features apple \
  -C profile=release

# Verify SPM package resolves and builds on macOS
swift build -c release \
  --package-path /path/to/NativeOIDC-Apple/ \
  --enable-dependency-cache
```

## Apple adapter notes

See the [Apple adapter notes](https://github.com/ProjectFlora/NativeOIDC#apple-adapters) for callback behavior, cancellation, and native HTTP transport requirements.
