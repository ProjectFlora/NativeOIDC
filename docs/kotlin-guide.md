# Kotlin/Android User Guide (v0.8.0)

This guide provides instructions for integrating the NativeOIDC library into Kotlin-based Android applications, targeting the v0.8.0 release.

## 1. Supported Versions & Prerequisites

- **Target Version**: v0.8.0
- **Minimum SDK**: 21 (Required for standard features; 24+ recommended for certain persistence features)
- **ABIs**: arm64-v8a, x86_64
- **Dependencies**: This guide assumes you are using the official distribution artifacts.

## 2. Download & Installation

### Maven Central (Recommended)
Add the following dependency to your `build.gradle` file:

```gradle
dependencies {
    implementation "net.project-flora.nativeoidc:native-oidc-android:0.8.0"
}
```
*Note: Maven Central publication for the Android channel began with v0.7.0.*

### GitHub Release AAR
For testing or if Maven Central is unavailable, you can use the GitHub Release artifacts. 
Download the `.aar` and the `SHA256SUMS` from the [GitHub Releases page](https://github.com/ProjectFlora/NativeOIDC/releases).

## 3. Imports

```kotlin
import net.projectflora.nativeoidc.*
import uniffi.native_oidc.*
```

## 4. Configuration & Transport

Initialize the adapter with your provider's configuration. You can choose between standard transports (e.g., Custom Tabs, WebView).

```kotlin
val config = OidcConfig(
    issuer = "https://your-idp.com",
    clientId = "your-client-id",
    redirectUri = "your-app-scheme://callback"
)

// Example for Custom Tabs
val adapter = NativeOidcAdapter(config, CustomTabTransport())
```

## 5. Callback Registration & Platform Setup

Register your callback activity to handle the redirect from the identity provider.

```kotlin
// In your Activity or Fragment
val callbackActivity = CallbackActivity(context) 
// Ensure your Activity is configured to handle the redirect URI
```

## 6. Authorization

Initiate the authorization flow. This is a suspending function that will return the tokens once the user completes the flow.

```kotlin
try {
    val result = adapter.authorize()
    val tokens = result.tokens
    // Handle successful authorization
} catch (e: Exception) {
    // Handle errors (e.g., user cancelled, network failure)
}
```

## 7. Tokens & Expiry

Tokens are returned as a `TokenSet` containing `accessToken`, `idToken`, and `refreshToken`.

```kotlin
val accessToken = tokens.accessToken
val refreshToken = tokens.refreshToken
val expiresAt = tokens.expiresAt
```

## 8. Secure Persistence & Restoration

On Android, it is highly recommended to use the `AndroidKeyStore` for secure storage of tokens.

```kotlin
val storage = AndroidKeyStoreStorage(context)
val savedTokens = storage.loadTokens()

if (savedTokens == null) {
    // No tokens saved, perform new authorization
} else {
    // Use saved tokens
}
```

## 9. Refresh

Use the `refreshToken` to obtain a new `accessToken` when the current one expires.

```kotlin
try {
    val newTokens = adapter.refresh(refreshToken)
    storage.saveTokens(newTokens)
} catch (e: Exception) {
    // Handle refresh failure (e.g., refresh token expired, redirect to auth)
}
```

## 10. End-Session, Revocation & Local Deletion

To end the session, you can call the `endSession` method which provides the revocation URL.

```kotlin
val endSessionResult = adapter.endSession()
// Redirect user to endSessionResult.url
// Clear local storage
storage.clear()
```

## 11. Registration & Additional Parameters

If your IDP requires specific parameters (e.g., `scope`, `prompt`), include them in the configuration.

```kotlin
val config = OidcConfig(
    issuer = "...",
    clientId = "...",
    scopes = listOf("openid", "profile", "email"),
    extraParameters = mapOf("prompt" to "select_account")
)
```

## 12. Cancellation & Errors

The library uses specific exception types for different failure modes:

- `FfiException`: Errors originating from the underlying native library.
- `AuthorizationException`: General authorization errors.
- `TransportException`: Issues with the transport layer (e.g., network, Custom Tab failures).

Always handle these exceptions to provide a smooth user experience.

## 13. Verification Evidence

The functionality described in this guide, including the core flows and the Android KeyStore persistence, has been verified against the v0.8.0 distribution artifacts using the following automated tests:

- **Maven Central AAR**: Verified via `scripts/check-android-release-consumer.sh` against the published AAR.
- **Repository Build**: Verified via CI `android` job and local `connectedDebugAndroidTest` on API 28 and API 35 emulators.

**Verification Commands**:
```bash
# Verify against published Maven Central AAR
./native/android/gradlew -p native/consumer-smoke/android -PnativeOidcVersion=0.8.0 --refresh-dependencies connectedDebugAndroidTest

# Verify against repository-built release AAR
scripts/check-android-release-consumer.sh <path_to_aar>
```

## 14. Known Limits & Notes

- **Java Hosts**: For Java applications, we recommend using a Kotlin-side wrapper. You can expose the `suspend` authorization process as a function returning a `CompletableFuture` or as a function accepting a Java callback.
- **Logout**: The adapter does not provide a direct "logout" function; it provides the revocation URL which you must handle by redirecting the user.
- **Live-IDP**: Physical device and live IDP verification are not performed as part of this guide's verification.

---
*For general information on the shared user guide, please refer to [docs/user-guide.md](docs/user-guide.md).*
