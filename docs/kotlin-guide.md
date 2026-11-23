# Kotlin/Android user guide

Return to the [documentation index](README.md). This guide covers the
Android-specific setup and Kotlin calls. Behavior shared by every language
(state lifecycle, refresh semantics, logout policy, error categories) is
described once in the [shared user guide](user-guide.md) and is not repeated
here. The [API coverage matrix](api-coverage.md) lists every operation.

The guide targets the published **v0.8.0** release. Check the matching tag
when you install another version: requirements on current `main` can postdate
a release, and the [support matrix](support-matrix.md) records them.

Java applications have no library-provided entry point. Put a small Kotlin
wrapper in the app (see [Java hosts](#java-hosts)).

## 1. Versions, installation, and imports

| Item | v0.8.0 |
| --- | --- |
| Language | Kotlin (coroutines); Java through an app-side Kotlin wrapper |
| Minimum SDK | API 28 (Android 9). Gradle rejects an app with a lower `minSdk` when it merges the library manifest |
| ABIs | `arm64-v8a`, `x86_64` |
| Runtime dependencies | `net.java.dev.jna:jna:5.12.0@aar`, `org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2`, `androidx.browser:browser:1.8.0` |

### Maven Central (v0.7.0 and later)

With `mavenCentral()` in the repositories, one dependency brings in the AAR
and its runtime dependencies:

```kotlin
dependencies {
    implementation("net.project-flora.nativeoidc:native-oidc-android:0.8.0")
}
```

### GitHub Release AAR

Use this channel for releases before v0.7.0, which are not on Maven Central,
or when Maven Central is unavailable. Download the AAR and
`SHA256SUMS` from the [v0.8.0 release](https://github.com/ProjectFlora/NativeOIDC/releases/tag/v0.8.0)
and verify the checksum from the download directory:

```sh
grep -F '  native-oidc-android-0.8.0.aar' SHA256SUMS | shasum -a 256 -c -
```

Copy the AAR into the app's `libs` directory. A raw AAR has no dependency
metadata, so declare the runtime dependencies yourself:

```kotlin
dependencies {
    implementation(files("libs/native-oidc-android-0.8.0.aar"))
    implementation("net.java.dev.jna:jna:5.12.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("androidx.browser:browser:1.8.0")
}
```

### Imports

The Android adapter lives in `net.projectflora.nativeoidc`; the generated
UniFFI API lives in `uniffi.native_oidc`:

```kotlin
import net.projectflora.nativeoidc.AndroidAuthorizationAdapter
import net.projectflora.nativeoidc.AndroidAuthorizationAdapterException
import net.projectflora.nativeoidc.AndroidRedirectConfiguration
import net.projectflora.nativeoidc.UrlConnectionNativeHttpTransport
import uniffi.native_oidc.FfiAuthenticationState
import uniffi.native_oidc.FfiAuthorizationOptions
import uniffi.native_oidc.FfiCancellationHandle
import uniffi.native_oidc.FfiEndSessionOptions
import uniffi.native_oidc.FfiException
import uniffi.native_oidc.FfiParameter
import uniffi.native_oidc.FfiRegistrationOptions
import uniffi.native_oidc.FfiServiceConfiguration
import uniffi.native_oidc.beginEndSession
import uniffi.native_oidc.registerPublicClient
import uniffi.native_oidc.revoke
```

Generated objects such as `FfiServiceConfiguration` and
`FfiAuthenticationState` hold native resources. They implement
`AutoCloseable`; call `close()` (or `use { }`) when you no longer need one.

## 2. Configuration, transport, and callback registration

### Provider configuration

Discover the configuration from an HTTPS issuer:

```kotlin
val transport = UrlConnectionNativeHttpTransport()
val configuration = FfiServiceConfiguration.discover(
    "https://login.example.com/",
    transport,
    FfiCancellationHandle(),
)
```

Or build it from explicit endpoints. Add the OIDC validation endpoints when
you request `openid`, and add optional endpoints only when the provider
supports them:

```kotlin
val configuration = FfiServiceConfiguration(
    "https://login.example.com/authorize",
    "https://login.example.com/token",
)
    .withOidcValidationEndpoints("https://login.example.com/", "https://login.example.com/jwks")
    .withEndSessionEndpoint("https://login.example.com/logout")
    .withRevocationEndpoint("https://login.example.com/revoke")
    .withRegistrationEndpoint("https://login.example.com/register")
```

Each `with…` call returns a new configuration and throws
`FfiException.InvalidRequest` for an invalid URL.

### Transport

`UrlConnectionNativeHttpTransport` is the supplied `NativeHttpTransport`. It
does not follow HTTP redirects, disables HTTP caching, and disconnects the
active request when its cancellation handle fires. A custom transport must do
the same (see the [shared guide](user-guide.md#configure-a-native-public-client)).

### Callback activity, App Links, and custom-scheme fallback

Register both redirect URIs with the provider, then declare the library's
callback activity in the **host app's** manifest. The adapter checks that the
chosen URI resolves to this activity before it opens the browser; without the
declaration, authorization fails with
`AndroidAuthorizationAdapterException.CallbackActivityNotRegistered` or
`InvalidRedirectConfiguration`.

```xml
<activity
    android:name="net.projectflora.nativeoidc.AndroidAuthorizationCallbackActivity"
    android:exported="true"
    android:noHistory="true">
    <intent-filter android:autoVerify="true">
        <action android:name="android.intent.action.VIEW" />
        <category android:name="android.intent.category.DEFAULT" />
        <category android:name="android.intent.category.BROWSABLE" />
        <data
            android:scheme="https"
            android:host="login.example.com"
            android:path="/oauth/callback" />
    </intent-filter>
    <intent-filter>
        <action android:name="android.intent.action.VIEW" />
        <category android:name="android.intent.category.DEFAULT" />
        <category android:name="android.intent.category.BROWSABLE" />
        <data
            android:scheme="com.example.app"
            android:path="/oauth/callback" />
    </intent-filter>
</activity>
```

- **App Link (HTTPS)**: host `/.well-known/assetlinks.json` on the callback
  domain with your application ID and release signing-certificate SHA-256.
  The adapter uses the HTTPS URI only when Android reports the domain as
  verified for the app and the URI resolves to the callback activity. The
  [HTTPS callback domain](https-callback-domain.md) procedure shows the file
  and how to check it.
- **Custom scheme (fallback)**: a hierarchical, app-specific scheme such as
  `com.example.app:/oauth/callback`. `http`, `https`, and opaque URIs are
  rejected as `InvalidRedirectConfiguration`.

The adapter ignores callbacks whose scheme, host, port, or path do not match
the active redirect; the Rust core validates the complete callback and state.

## 3. Authorization, tokens, and expiry

`AndroidAuthorizationAdapter.authorize` selects the redirect, opens a Custom
Tab, waits for the callback, validates it, and exchanges the code. It is a
`suspend` function and returns `FfiAuthenticationState`:

```kotlin
val adapter = AndroidAuthorizationAdapter(activity)
val state: FfiAuthenticationState = adapter.authorize(
    configuration,
    FfiAuthorizationOptions(
        clientId = "client",
        redirectUri = "com.example.app:/oauth/callback", // replaced by the selected redirect
        scopes = listOf("openid", "profile"),
        additionalParameters = emptyList(),
        additionalTokenParameters = emptyList(),
    ),
    AndroidRedirectConfiguration(
        appLinkUri = "https://login.example.com/oauth/callback",
        customSchemeUri = "com.example.app:/oauth/callback",
    ),
    UrlConnectionNativeHttpTransport(),
)
```

The adapter overwrites `options.redirectUri` with the URI it selects from
`AndroidRedirectConfiguration`. Only one authorization may run on an adapter
at a time; a second call fails with `AuthorizationAlreadyRunning`.

Read tokens and expiry from the state. The getters are `suspend` functions:

```kotlin
val tokens = state.tokenResponse()
val accessToken = tokens.accessToken
val refreshToken: String? = tokens.refreshToken
val idToken: String? = tokens.idToken
val expiresAt: ULong? = state.expiresAt() // Unix seconds, null when unknown
val expired: Boolean? = state.isExpiredAt((System.currentTimeMillis() / 1000).toULong())
```

A `null` expiry means "unknown", not "valid forever". The library does not
refresh tokens automatically.

## 4. Secure persistence and restoration

`toJson()` serializes the complete state (tokens, absolute expiry, and OIDC
nonce); `FfiAuthenticationState.fromJson()` restores it. The library does not
store anything: encryption and storage are the host's job. The following
example encrypts the JSON with AES-GCM under a non-exportable Android Keystore
key and writes it to app-private, no-backup storage:

```kotlin
class EncryptedStateStore(private val file: File, private val keyAlias: String) {
    suspend fun save(state: FfiAuthenticationState) {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val ciphertext = cipher.doFinal(state.toJson().toByteArray(Charsets.UTF_8))
        val iv = cipher.iv
        val output = ByteBuffer.allocate(1 + iv.size + ciphertext.size)
            .put(iv.size.toByte()).put(iv).put(ciphertext)
        val temporary = File(file.parentFile, "${file.name}.tmp")
        temporary.writeBytes(output.array())
        if (!temporary.renameTo(file)) error("Could not replace stored state")
    }

    fun load(): FfiAuthenticationState? {
        if (!file.exists()) return null
        val input = ByteBuffer.wrap(file.readBytes())
        val iv = ByteArray(input.get().toInt()).also(input::get)
        val ciphertext = ByteArray(input.remaining()).also(input::get)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, iv))
        return FfiAuthenticationState.fromJson(String(cipher.doFinal(ciphertext), Charsets.UTF_8))
    }

    fun delete() {
        file.delete()
    }

    private fun key(): SecretKey {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (keyStore.getKey(keyAlias, null) as SecretKey?)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(keyAlias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build(),
        )
        return generator.generateKey()
    }
}

val store = EncryptedStateStore(File(context.noBackupFilesDir, "oidc-state.bin"), "oidc-state-key")
store.save(state)
val restored: FfiAuthenticationState? = store.load()
```

What the application still has to decide:

- Associate the stored state with its issuer, client ID, and configuration;
  the JSON does not contain them.
- Handle a decryption failure (for example, a key invalidated after a lock
  screen change, or a restored backup) by deleting the file and signing in
  again.
- Add key requirements that match your threat model, such as
  `setUserAuthenticationRequired`, and serialize concurrent writes.
- Keep tokens and state JSON out of logs.

Restoration does not contact the provider; it does not prove the tokens are
still accepted.

## 5. Refresh and re-save

Refresh is explicit. `refresh` updates the state in place only after a
successful response, keeps the old refresh token when the response omits one,
and leaves the state unchanged on failure or cancellation. Persist the state
after every success, including refresh-token rotation:

```kotlin
suspend fun refreshAndSave(state: FfiAuthenticationState) {
    state.refresh(
        configuration,
        "client",
        emptyList(), // additional token-request parameters
        UrlConnectionNativeHttpTransport(),
        FfiCancellationHandle(),
    )
    store.save(state)
}
```

An `FfiException.AuthorizationServer` with `code == "invalid_grant"` usually
means the refresh token is no longer accepted; the retry or re-authorization
policy is the host's. Serialize refresh and sign-out so a late refresh cannot
write state back after local deletion (see the
[shared guide](user-guide.md#save-restore-and-refresh)).

## 6. End-session, revocation, and local sign-out

The authorization adapter has **no logout method**. Sign-out combines three
separate actions; see the [shared logout policy](user-guide.md#logout-revocation-and-local-deletion).

### Provider end-session (RP-Initiated Logout)

`beginEndSession` builds the logout URL. Open it in a browser you manage, and
validate the post-logout callback with the same session:

```kotlin
val endSession = beginEndSession(
    configuration, // requires withEndSessionEndpoint or a discovered endpoint
    FfiEndSessionOptions(
        clientId = "client",
        idTokenHint = state.tokenResponse().idToken,
        postLogoutRedirectUri = "com.example.app:/oauth/logout",
    ),
)
CustomTabsIntent.Builder().build().launchUrl(activity, Uri.parse(endSession.endSessionUrl()))

// Later, in the activity that receives com.example.app:/oauth/logout:
val response = endSession.parseRedirect(intent.dataString!!) // throws InvalidResponse on a state mismatch
```

The library's callback activity serves authorization only. Register your own
activity for the post-logout redirect, keep the `FfiEndSessionSession` until
the callback arrives (or the user abandons it), and close it afterwards.
Without a post-logout redirect, opening the URL does not confirm that the
provider completed logout.

### Token revocation and local deletion

`revoke` asks the provider to invalidate one token. It is a separate network
call, not a browser action. Delete local state whether or not it succeeds:

```kotlin
suspend fun signOut() {
    val state = store.load()
    try {
        state?.tokenResponse()?.refreshToken?.let { token ->
            revoke(configuration, "client", token, "refresh_token", UrlConnectionNativeHttpTransport(), FfiCancellationHandle())
        }
    } catch (error: FfiException) {
        // Revocation failure does not block local sign-out.
    } finally {
        store.delete()
        state?.close()
    }
}
```

The type hint is `"access_token"`, `"refresh_token"`, or `null`; any other
value is `InvalidRequest`. Revocation requires a revocation endpoint.

## 7. Dynamic registration and additional parameters

Register a native public client when the provider exposes a registration
endpoint:

```kotlin
val registration = registerPublicClient(
    configuration, // requires withRegistrationEndpoint or a discovered endpoint
    FfiRegistrationOptions(
        redirectUris = listOf("https://login.example.com/oauth/callback", "com.example.app:/oauth/callback"),
        clientName = "Example",
        initialAccessToken = null,
    ),
    UrlConnectionNativeHttpTransport(),
    FfiCancellationHandle(),
)
val clientId = registration.clientId
```

A registration rejection is `FfiException.ClientRegistrationServer`, distinct
from `AuthorizationServer`.

Pass extra authorization-request and token-request parameters in
`FfiAuthorizationOptions`, and extra refresh parameters in `refresh`:

```kotlin
FfiAuthorizationOptions(
    clientId = "client",
    redirectUri = "com.example.app:/oauth/callback",
    scopes = listOf("openid", "profile", "email"),
    additionalParameters = listOf(FfiParameter("prompt", "select_account")),
    additionalTokenParameters = listOf(FfiParameter("resource", "https://api.example.com")),
)
```

Parameters that the library sets itself (for example `state`, `nonce`,
`client_id`, `redirect_uri`, `scope`) and duplicate names are rejected with
`FfiException.InvalidRequest` before any request is sent.

## 8. Cancellation, errors, and lifecycle

### Cancellation and timeouts

- `AndroidAuthorizationAdapter.authorize` follows coroutine cancellation.
  Launch it in a scope tied to the UI (for example `lifecycleScope`) and use
  `withTimeout` for a timeout policy; the library sets none. `adapter.cancel()`
  stops the active callback wait and core request from outside the coroutine.
  An adapter-level cancellation surfaces as `FfiException.Cancelled`.
- Direct UniFFI calls (`discover`, `refresh`, `revoke`, `registerPublicClient`)
  take an explicit `FfiCancellationHandle`. Call `cancel()` on it to stop the
  operation; the result is `FfiException.Cancelled`. A cancelled handle stays
  cancelled, so create a new one per operation.
- Cancellation does not close the Custom Tab and cannot undo a request the
  provider already processed.

### Errors

| Type | Meaning |
| --- | --- |
| `FfiException.InvalidRequest` | Invalid caller input or a missing endpoint; nothing was sent |
| `FfiException.InvalidResponse` | Malformed or invalid provider data, ID Token, or callback state |
| `FfiException.AuthorizationServer` | OAuth error from the provider (`code`, optional `description`) |
| `FfiException.ClientRegistrationServer` | Registration error from the provider |
| `FfiException.HttpStatus` | Unexpected HTTP status (`status`) |
| `FfiException.TransportFailure` / `Transport` | Network or transport failure |
| `FfiException.Cancelled` | Cancelled by the handle, the adapter, or the coroutine |
| `FfiException.Clock`, `Randomness`, `Internal` | Platform or internal failure |
| `AndroidAuthorizationAdapterException.CallbackActivityNotRegistered` | The selected URI does not resolve to the callback activity |
| `AndroidAuthorizationAdapterException.InvalidRedirectConfiguration` | Invalid redirect URIs, or an unverified App Link without a usable fallback |
| `AndroidAuthorizationAdapterException.BrowserUnavailable` | No browser could be opened, or the host activity is finishing |
| `AndroidAuthorizationAdapterException.AuthorizationAlreadyRunning` | Another authorization is active |

Treat provider descriptions as untrusted diagnostic text. The cross-language
categories are in the [shared error guide](user-guide.md#cancellation-timeouts-and-errors).

### Activity and process lifecycle

- The adapter holds the `Activity` it was created with. Create it for the
  current activity and do not keep it across configuration changes or
  activity recreation; a finishing or destroyed activity fails the launch
  with `BrowserUnavailable`.
- The callback wait lives in the app process. If the process dies while the
  Custom Tab is open, the pending authorization is lost; start a new one.
- Callbacks that arrive when no authorization is waiting are ignored.

### Java hosts

Expose the `suspend` call from a Kotlin wrapper, for example as a
`CompletableFuture` (`kotlinx-coroutines-core` provides `future`):

```kotlin
class AuthFacade(private val scope: CoroutineScope, private val activity: Activity) {
    fun signIn(configuration: FfiServiceConfiguration): CompletableFuture<FfiAuthenticationState> =
        scope.future { AndroidAuthorizationAdapter(activity).authorize(/* … */) }
}
```

The wrapper is also where main-thread hops, lifecycle handling, and timeout
policy belong; see [supported application languages](user-guide.md#supported-application-languages).

## 9. Verification, coverage, and known limits

### What the examples are checked against

The consumer fixture test
[`GuideExamplesTest`](../native/consumer-smoke/android/app/src/androidTest/kotlin/net/projectflora/nativeoidc/consumer/GuideExamplesTest.kt)
compiles this guide's examples and executes them against a packaged AAR,
using a scripted in-process transport (no network):

- Authorization request with additional parameters, callback validation, code
  exchange, token and expiry access (the core calls the adapter makes, with a
  scripted callback URI).
- Reserved and duplicate additional parameters rejected before any request.
- The `EncryptedStateStore` Keystore round trip and local deletion.
- Refresh with rotation and re-save; a response without a refresh token keeps
  the old one.
- Refresh failure (`AuthorizationServer`, `HttpStatus`, `TransportFailure`)
  and pre-cancelled refresh (`Cancelled`) leave the state unchanged.
- End-session URL construction and callback validation, including a state
  mismatch (`InvalidResponse`).
- Revocation followed by local deletion, also when revocation fails.
- Registration success and `ClientRegistrationServer`.
- The adapter call, the error-classification `when`, and the Java
  `CompletableFuture` wrapper are compiled but not executed there: they need
  a host activity, a registered callback activity, and a browser.

The Custom Tab flow, callback activity, adapter cancellation, and the
adapter's redirect-selection errors run in the library module's emulator
tests against a local mock provider. Every UniFFI operation and error variant
is covered by the [Kotlin UniFFI coverage tests](api-coverage.md#kotlin-uniffi-operation-and-error-coverage).

Commands:

```sh
# Against the published Maven Central artifact
./native/android/gradlew -p native/consumer-smoke/android -PnativeOidcVersion=0.8.0 --refresh-dependencies connectedDebugAndroidTest

# Against a downloaded or locally built release AAR
scripts/check-android-release-consumer.sh native-oidc-android-0.8.0.aar

# Library module tests (unit, then emulator)
./native/android/gradlew -p native/android testDebugUnitTest
./native/android/gradlew -p native/android connectedDebugAndroidTest
```

Evidence for this revision:

- Pull request CI (`Android adapters validation`) builds the repository AAR
  and runs the consumer fixture, including `GuideExamplesTest`, on x86_64
  API 28 and API 35 emulators. See the checks of
  [PR #220](https://github.com/ProjectFlora/NativeOIDC/pull/220).
- `GuideExamplesTest` and `ReleaseConsumerTest` (9 tests) passed locally
  against the published `native-oidc-android-0.8.0.aar` GitHub Release asset,
  checksum verified, on an arm64-v8a API 28 emulator (2026-11-23).

### Coverage by operation

| Operation group | Kotlin status |
| --- | --- |
| Discovery, explicit and optional endpoints | Available (`FfiServiceConfiguration`) |
| Browser authorization and callback | Available (`AndroidAuthorizationAdapter`); manifest setup and `assetlinks.json` are host work |
| Callback validation and code exchange | Available (inside the adapter; also `beginAuthorization` for custom browsers) |
| Tokens, expiry, serialize and restore | Available (`FfiAuthenticationState`) |
| Secure persistence | Host (example above) |
| Refresh with additional parameters | Available (`refresh`); re-save is host work |
| End-session URL and callback validation | Available (`beginEndSession`, `FfiEndSessionSession`) |
| Logout browser and callback lifecycle | Host; the adapter has no logout method |
| Token revocation | Available (`revoke`) |
| Public-client registration | Available (`registerPublicClient`) |
| Additional request parameters | Available (`FfiAuthorizationOptions`, `refresh`) |
| Cancellation | Available (coroutines and `cancel()` for the adapter; `FfiCancellationHandle` for direct calls) |
| Token-based state construction | Not exposed; only `fromJson` and the exchange result |
| Local sign-out | Host |

### Known limits

- Live identity-provider interoperability and physical-device execution are
  not verified; see [interoperability status](live-idp-interoperability.md)
  and the [support matrix](support-matrix.md). arm64 execution is unverified.
- The fixture's scripted transport checks the library's behavior, not a
  provider's.
- The storage example is a starting point, not a complete storage layer; the
  items listed in section 4 remain the application's responsibility.
