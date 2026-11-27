# Shared user guide

Return to the [documentation index](README.md). Language-specific entry points
and the [API coverage matrix](api-coverage.md) identify the concrete APIs.

## Supported application languages

The public API targets Swift on Apple platforms and Kotlin on Android.
Windows desktop hosts use the C ABI, callable from C++ and from .NET through
P/Invoke, and Rust callers use the core crate directly.

- Objective-C and Java applications have no library-provided entry point.
  See the [Swift guide](swift-guide.md#objective-c-interoperability) for how to
  wrap the Swift API in an `@objc` class with completion-handler methods.

Java hosts wrap the Kotlin API using callbacks or `CompletableFuture`. The pattern
is similar to Objective-C: place a small binding class in the app that manages
lifecycle, cancellation, and error reporting. The adapter is bound to the
`Activity` it was created with, so construct it with a live activity. Example:

```kotlin
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import net.projectflora.nativeoidc.AndroidAuthorizationAdapter
import net.projectflora.nativeoidc.AndroidAuthorizationAdapterException
import net.projectflora.nativeoidc.AndroidRedirectConfiguration
import net.projectflora.nativeoidc.UrlConnectionNativeHttpTransport
import uniffi.native_oidc.FfiAuthorizationOptions
import uniffi.native_oidc.FfiException
import uniffi.native_oidc.FfiServiceConfiguration

/** Java-callable result callback (a SAM interface). */
fun interface AuthorizationCallback {
    fun onResult(stateJson: String?, error: Throwable?)
}

class NativeOIDCWrapper(
    private val adapter: AndroidAuthorizationAdapter,
    // Host-owned scope tied to the screen's lifecycle; cancel it when the
    // screen goes away.
    private val scope: CoroutineScope,
) {
    fun authorize(
        configuration: FfiServiceConfiguration,
        options: FfiAuthorizationOptions,
        redirectConfiguration: AndroidRedirectConfiguration,
        callback: AuthorizationCallback,
    ) {
        scope.launch(Dispatchers.Main) {
            try {
                val state = adapter.authorize(
                    configuration = configuration,
                    options = options,
                    redirectConfiguration = redirectConfiguration,
                    transport = UrlConnectionNativeHttpTransport(),
                )
                // FfiAuthenticationState is a Kotlin class; hand the Java caller
                // the serialized state for persistence and later refresh.
                callback.onResult(state.toJson(), null)
            } catch (error: CancellationException) {
                // The host cancelled; report it as cancellation, not a provider
                // failure. Do not swallow it in a broader catch.
                callback.onResult(null, error)
            } catch (error: AndroidAuthorizationAdapterException) {
                // Browser and callback setup failures.
                callback.onResult(null, error)
            } catch (error: FfiException) {
                // Configuration, provider, and transport failures.
                callback.onResult(null, error)
            }
        }
    }

    fun cancel() {
        adapter.cancel()
    }
}
```

See [Android installation and adapter](../README.md#android-adapter) for setup details.

## Kotlin usage

For Android developers using the Kotlin API directly. The host must register
`AndroidAuthorizationCallbackActivity` and its redirect URI intent filters in
the manifest, and keep the `Activity` passed to the adapter alive for the
duration of the authorization. See
[Android installation and adapter](../README.md#android-adapter) for the
complete setup.

```kotlin
import android.app.Activity
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import net.projectflora.nativeoidc.AndroidAuthorizationAdapter
import net.projectflora.nativeoidc.AndroidRedirectConfiguration
import net.projectflora.nativeoidc.UrlConnectionNativeHttpTransport
import uniffi.native_oidc.FfiAuthorizationOptions
import uniffi.native_oidc.FfiCancellationHandle
import uniffi.native_oidc.FfiServiceConfiguration

// A host-owned scope ties the operation to the screen's lifecycle.
val scope = CoroutineScope(Dispatchers.Main + SupervisorJob())

// The adapter is bound to the live activity that opens the Custom Tab.
val adapter = AndroidAuthorizationAdapter(activity = activity)

scope.launch {
    val transport = UrlConnectionNativeHttpTransport()
    val cancellation = FfiCancellationHandle()

    // Discovers and validates the provider's OIDC metadata. For an
    // explicit-configuration provider, construct instead:
    // FfiServiceConfiguration(authorizationEndpoint, tokenEndpoint)
    val configuration = FfiServiceConfiguration.discover(
        issuer = "https://your-idp.example",
        transport = transport,
        cancellation = cancellation,
    )

    val options = FfiAuthorizationOptions(
        clientId = "native-public-client",
        // Must exactly match the redirect registered with the provider.
        redirectUri = "https://your-app.example/oidc/callback",
        scopes = listOf("openid", "profile"),
        additionalParameters = emptyList(),
        additionalTokenParameters = emptyList(),
    )

    // authorize takes the four shipped arguments: service configuration,
    // options, redirect routing, and transport.
    val state = adapter.authorize(
        configuration = configuration,
        options = options,
        redirectConfiguration = AndroidRedirectConfiguration(
            appLinkUri = "https://your-app.example/oidc/callback",
            customSchemeUri = "exampleapp://oidc/callback",
        ),
        transport = transport,
    )

    // Persist the complete state in host-managed secure storage, then use
    // state.tokenResponse() to call the resource server.
}
```

Cancelling the surrounding coroutine or calling `adapter.cancel()` stops the
callback wait and any in-flight core request. `CancellationException`
propagates out of the coroutine: report it as a cancellation rather than
swallowing it in a broad `catch (Exception)`.

### Post-authorization operations

`secureStore` below is a host-supplied abstraction (for example
`EncryptedSharedPreferences`); the application chooses the backend and its
security properties.

```kotlin
import android.net.Uri
import kotlinx.coroutines.Job
import uniffi.native_oidc.FfiAuthenticationState
import uniffi.native_oidc.FfiEndSessionOptions
import uniffi.native_oidc.FfiException
import uniffi.native_oidc.beginEndSession

// On the next launch: restore the state before any operation that needs it.
// The first launch, and the launch after sign-out, have nothing stored, so
// restore only when a value exists — the absence of state means signed-out,
// not an error.
val state = secureStore.getString("native-oidc/state")?.let {
    FfiAuthenticationState.fromJson(serialized = it)
}

// Serialize the refresh write against local sign-out so a late refresh
// cannot restore state after deletion.
val sessionLock = Any()
var signedOut = false

scope.launch {
    // Nothing to refresh when there is no restored state (first launch or
    // signed out).
    val state = state ?: return@launch
    // Refresh requires a non-empty refresh token and updates `state` in place
    // after a successful response.
    state.refresh(
        configuration = configuration,
        clientId = "native-public-client",
        additionalParameters = emptyList(),
        transport = UrlConnectionNativeHttpTransport(),
        cancellation = FfiCancellationHandle(),
    )
    // Refresh tokens may rotate; persist the updated complete state, guarded
    // by the sign-out flag.
    val stateJson = state.toJson()
    synchronized(sessionLock) {
        if (signedOut) return@launch
        secureStore.putString("native-oidc/state", stateJson)
    }
}

// Provider logout: the authorization adapter has no logout method. Register
// the app's post-logout redirect with the provider; a random state is then
// generated and validated on return.
val endSession = beginEndSession(
    configuration = configuration, // requires an end-session endpoint
    options = FfiEndSessionOptions(
        clientId = "native-public-client",
        // Optional hint: the current ID Token, read inside a coroutine with
        // state.tokenResponse().idToken.
        idTokenHint = null,
        // Must exactly match the registered post-logout redirect.
        postLogoutRedirectUri = "https://your-app.example/oidc/post-logout",
    ),
)
val logoutUrl = Uri.parse(endSession.endSessionUrl())
// Open the Custom Tab or browser with `logoutUrl`. Retain the prepared
// `endSession` until the callback returns, then validate it:
try {
    endSession.parseRedirect(callbackUri.toString())
    // The callback's state matched; the provider completed the logout.
} catch (error: FfiException) {
    // The callback did not validate (missing or mismatched state): report
    // the provider logout as unverified.
}

// Local sign-out is independent of provider cleanup: flag sign-out, cancel
// in-flight work, then clear the stored state and any in-memory copies.
synchronized(sessionLock) {
    signedOut = true
    scope.coroutineContext[Job]?.cancel()
    secureStore.remove("native-oidc/state")
}
```

With `postLogoutRedirectUri = null` there is no callback to validate:
opening the browser is not confirmation that the provider completed logout.

Dynamic registration and token revocation use the same pattern as the Swift
guide: `registerPublicClient` with `FfiRegistrationOptions`, and `revoke`
with the chosen token, type hint, transport, and a fresh
`FfiCancellationHandle`. Failures surface as `FfiException` subclasses
(`AuthorizationServer`, `ClientRegistrationServer`, `HttpStatus`,
`Transport`, `TransportFailure`, `Cancelled`, `InvalidRequest`,
`InvalidResponse`, `Clock`, `Randomness`); the adapter adds its own
`AndroidAuthorizationAdapterException` types for browser and callback setup
failures.

The Kotlin examples above are mirrored in the release consumer app
(`native/consumer-smoke/android`): the main sources compile the wrapper, the
"Kotlin usage" flow, and the post-authorization operations, and the
instrumented tests run the representative state-update, cancellation, and
error branches (#85) with scripted transports, without a network
connection, plus the state-restoration branches (a missing stored state
restores to `null` rather than crashing) and the end-session lifecycle entry
points (the prepared session retained with its URL, and a scripted callback
derived from the generated URL, matching and mismatched). `scripts/check-android-release-consumer.sh` builds the consumer
app against the release AAR and runs the instrumented tests on an emulator,
so the examples are verified against the distributed artifact — against the
AAR built from the branch in pull request CI, and against the published
release AAR in the release consumer workflow — rather than against the
module sources. The generated bindings were additionally verified from the
`v0.8.0` tag, where the module's unit tests pass
(`./native/android/gradlew -p native/android testDebugUnitTest`). The
examples are build-verified but not exercised against a real IdP, consistent
with the repository's [live IdP interoperability](live-idp-interoperability.md)
policy.

## Configure a native public client

Register a native public client and the exact redirect URIs with your provider.
Do not embed a client secret. Discover configuration from an HTTPS issuer, or
provide explicit HTTPS endpoints. For explicit OIDC configuration, include
both issuer and JWKS URI. Requests with the `openid` scope require OIDC
validation configuration; returned ID Tokens are verified before exposure.
Optional end-session, revocation, and registration endpoints must be supported
by the provider and discovered or explicitly configured before use.

Explicitly configured authorization, token, and revocation endpoints may also
use loopback HTTP (`127.0.0.0/8`, `::1`, `localhost`). The issuer, JWKS,
end-session, and registration endpoints, and every discovered endpoint,
require HTTPS. OAuth requires TLS for these endpoints (RFC 6749 §3.1 and
§3.2), and RFC 7009 §2 says the client MUST verify that the revocation URL is
HTTPS. NativeOIDC accepts loopback anyway because loopback traffic never
leaves the device, and local development and the test mock providers need it.
Use loopback HTTP endpoints only for local development and testing; do not
ship a production configuration that points at one. The decision is recorded
in the [roadmap](../ROADMAP.md#confirmed-decisions)
([#269](https://github.com/ProjectFlora/NativeOIDC/issues/269)).

Rust callers supply an async transport and, for `ReqwestTransport`, a Tokio
runtime. Swift and Kotlin callers supply `NativeHttpTransport`; the supplied
URLSession and URLConnection implementations can be used. A custom transport
must return HTTP redirects without following them and propagate cancellation.
The supplied native transports disable platform HTTP caching; the core owns
JWKS freshness. Windows operations use their internal runtime and transport.

## Authorize and retain state

Start the platform callback handler or loopback listener before opening the
external browser. The adapters manage this for authorization. Use verified
HTTPS callbacks on mobile where configured, with registered custom schemes
as fallback; desktop adapters use a dynamically allocated IPv4 loopback port.
The core validates the complete callback and state before code exchange.

Rust callers use `AuthorizationSession::authentication_state` after exchange
to retain the authorization nonce. UniFFI exchange and the platform adapters
already return authentication state; Windows returns its JSON serialization.
Do not reconstruct state from only an access token: expiry, refresh token,
and OIDC nonce are needed for later operations.

Read token values from state to call the resource server. Access-token expiry
is an optional Unix timestamp in seconds. An unknown expiry is not proof that
a token is valid indefinitely. The library does not automatically refresh
tokens or retry resource-server calls. ID Tokens are identity assertions;
use the access token for resource-server authorization.

## Save, restore, and refresh

Serialize the complete state using Serde in Rust or `to_json` in UniFFI;
Windows authorization and refresh already return JSON. Persist it using
host-managed secure storage and encryption. Keep tokens, nonce, and state
JSON out of logs. Restore the full JSON using Serde, UniFFI `from_json`, or
the Windows refresh request's `state` object. Restoration does not contact
the provider or prove that previously issued tokens are still accepted.

The host associates stored state with the correct issuer, client ID, and
configuration; state JSON does not contain that complete configuration.
Retain the original absolute expiry instead of recalculating it from
`expires_in` on each application launch.

Refresh requires a non-empty refresh token. Rust and UniFFI update state only
after a successful response and any returned ID Token validation. Windows
returns a replacement state JSON. Persist the updated complete state after
every success, including refresh-token rotation. A missing refresh token,
scope, or ID Token in a successful refresh response retains the old value.

State keeps the identity claims of its ID Token (`iss`, `sub`, `aud`, `azp`,
and `auth_time`) under `id_token_claims`. A refreshed ID Token must have the
same `iss`, `sub`, and `aud` as the original one (OpenID Connect Core 1.0
§12.2). If it contains `auth_time`, that value must equal the original one;
a refreshed ID Token without `auth_time` is accepted and the original value
is kept. Otherwise refresh fails as for any invalid ID Token, and the state
is unchanged. State saved by v0.9.0 or earlier has no `id_token_claims`; it
takes them from its stored ID Token when it is restored, so no
reauthentication is needed. If that state has no ID Token, the first ID Token
validated on a later refresh becomes the one that later refreshes are
compared with.

On failure or cancellation, the local previous state is unchanged. This does
not guarantee that the server still accepts its tokens: a refresh may reach
the server before the response is lost. The host decides retry or
reauthentication policy, including how to handle `invalid_grant`. Serialize
refresh and logout at the application level so a late refresh cannot restore
state after local logout. Protect the write to persistent storage as well as
the in-memory update.

## Logout, revocation, and local deletion

| Action | Effect | Host responsibility |
| --- | --- | --- |
| OIDC end-session | Opens the provider's RP-Initiated Logout URL; optionally validates a post-logout callback | Manage browser/callback lifecycle where no logout adapter exists; delete local state separately |
| OAuth token revocation | Requests invalidation of the supplied access or refresh token | Choose the token and optional type hint; own retries; delete local state even on failure or cancellation |
| Local state deletion | Removes the application's stored credentials | Clear stored and in-memory state; prevent concurrent operations from writing it back |

End-session requires an HTTPS end-session endpoint. An ID Token hint and
post-logout redirect are optional. With a redirect, retain the prepared logout
session and validate the callback/state. Without one, opening the browser is
not confirmation that the provider completed logout. Swift/Kotlin expose URL
construction and callback validation, but their existing authorization
adapters do not provide a logout method. Rust similarly leaves browser work
to the host. Windows handles the browser and optional loopback callback.

Revocation requires an HTTPS revocation endpoint; an explicitly configured
one may also use loopback HTTP, as the token endpoint can (see
[Configure a native public client](#configure-a-native-public-client)). HTTP 200
completes the operation even for an already-invalid token; it does not
delete local state.
Failure does not confirm remote invalidation. The library does not retry
automatically or expose Retry-After. If logout policy attempts network cleanup,
retain only the values needed for that attempt, and ensure local deletion
runs even if cleanup fails or is cancelled. Do not make local sign-out depend
on provider availability.

## Cancellation, timeouts, and errors

Cancellation handles are shared signals. Calling `cancel` is idempotent; a
cancelled handle stays cancelled, so use a new handle for a new operation.
The host owns authorization timeout policy. Cancellation stops active work
where supported, but cannot undo a request already processed by the provider.
macOS and Windows cannot dismiss the external browser; Android does not
promise to close the Custom Tab. iOS cancels its authentication session.
Swift Task and Kotlin coroutine cancellation are connected by the supplied
authorization adapters; direct core/UniFFI operations use their explicit
handle. Follow each language guide for lifecycle integration.

| Failure category | Rust / UniFFI | Windows / .NET |
| --- | --- | --- |
| Caller configuration | `InvalidConfiguration` / `InvalidRequest` | Error status / `InvalidOperationException` |
| Invalid protocol response or ID Token | Specific validation errors / `InvalidResponse` | Error status / `InvalidOperationException` |
| Provider rejection | `AuthorizationServer`, `ClientRegistrationServer` with code and optional description | Error message; no structured OAuth fields |
| HTTP or network | `HttpStatus`, `Transport`, `TransportFailure` | Error message; no structured HTTP fields |
| Cancellation | `Cancelled` | Cancelled status / `OperationCanceledException` |
| Platform/browser/listener | Adapter-specific errors | Error status / `InvalidOperationException` |

Rust also exposes clock/randomness failures; UniFFI exposes clock/randomness
and internal failures. Rust's error enum is non-exhaustive. Preserve cancellation
as a distinct result, avoid parsing Windows error text into a protocol contract,
and present provider descriptions as untrusted diagnostic text. Apply host retry
policy to transient failures; validation errors need investigation rather than
blind retries. See the [Windows contract](windows-api.md) for exact statuses.

## Structure for the language guides

The follow-up guides use this common sequence, with concrete imports, request
construction, and compilable examples for their language:

1. Supported versions, download/checksum, dependencies, installation and build.
2. Provider configuration, transport, callback registration and platform setup.
3. Authorization, token access and expiry.
4. Secure state persistence and restoration.
5. Explicit refresh and persistence of updated state.
6. End-session, revocation and local sign-out.
7. Dynamic registration and additional parameters where exposed.
8. Cancellation, timeout policy, errors and platform lifecycle constraints.
9. Example verification commands, evidence and known limits.

Common behavior is documented here. The language guides carry the
platform-specific setup, callbacks, and extended examples: the
[Swift guide](swift-guide.md), the [Kotlin/Android guide](kotlin-guide.md),
the [Windows C++ guide](windows-cpp-guide.md), and the
[Windows .NET guide](windows-dotnet-guide.md). The [Kotlin usage](#kotlin-usage)
section above is a short summary of the Android lifecycle. Sample storage
abstractions must say which work the application still needs to implement.
