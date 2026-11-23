package net.projectflora.nativeoidc.consumer

import android.app.Activity
import android.net.Uri
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import net.projectflora.nativeoidc.AndroidAuthorizationAdapter
import net.projectflora.nativeoidc.AndroidAuthorizationAdapterException
import net.projectflora.nativeoidc.AndroidRedirectConfiguration
import net.projectflora.nativeoidc.UrlConnectionNativeHttpTransport
import uniffi.native_oidc.FfiAuthorizationOptions
import uniffi.native_oidc.FfiAuthenticationState
import uniffi.native_oidc.FfiCancellationHandle
import uniffi.native_oidc.FfiEndSessionOptions
import uniffi.native_oidc.FfiEndSessionSession
import uniffi.native_oidc.FfiException
import uniffi.native_oidc.FfiServiceConfiguration
import uniffi.native_oidc.beginEndSession

// Compile-time mirror of the Kotlin examples in docs/user-guide.md (the Java
// host wrapper, the "Kotlin usage" flow, and the post-authorization
// operations). The release consumer app compiles this file against the
// published AAR (the generated bindings plus the shipped adapter), so the
// guide examples stay verified against the distributed artifact. The
// host-supplied abstractions the guide sketches (`activity`, `secureStore`)
// are the interfaces below.

/** Host-supplied secure storage (the guide's `secureStore` abstraction). */
interface GuideSecureStore {
    fun getString(key: String): String?
    fun putString(key: String, value: String)
    fun remove(key: String)
}

/** Java-callable result callback (a SAM interface). */
fun interface AuthorizationCallback {
    fun onResult(stateJson: String?, error: Throwable?)
}

/** The guide's wrapper for Java hosts. */
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
                // FfiAuthenticationState is a Kotlin class; hand the Java
                // caller the serialized state for persistence and later
                // refresh.
                callback.onResult(state.toJson(), null)
            } catch (error: CancellationException) {
                // The host cancelled; report it as cancellation, not a
                // provider failure. Do not swallow it in a broader catch.
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

/** The guide's "Kotlin usage" example, parameterized by the host activity
 * and secure store. */
class GuideKotlinUsage(
    private val activity: Activity,
    private val store: GuideSecureStore,
) {
    // A host-owned scope ties the operation to the screen's lifecycle.
    val scope = CoroutineScope(Dispatchers.Main + SupervisorJob())

    // The adapter is bound to the live activity that opens the Custom Tab.
    val adapter = AndroidAuthorizationAdapter(activity = activity)

    fun signIn(issuer: String, callback: AuthorizationCallback) {
        scope.launch {
            val transport = UrlConnectionNativeHttpTransport()
            val cancellation = FfiCancellationHandle()

            // Discovers and validates the provider's OIDC metadata.
            val configuration = FfiServiceConfiguration.discover(
                issuer = issuer,
                transport = transport,
                cancellation = cancellation,
            )

            val options = FfiAuthorizationOptions(
                clientId = "native-public-client",
                // Must exactly match the redirect registered with the
                // provider.
                redirectUri = "https://your-app.example/oidc/callback",
                scopes = listOf("openid", "profile"),
                additionalParameters = emptyList(),
                additionalTokenParameters = emptyList(),
            )

            // authorize takes the four shipped arguments: service
            // configuration, options, redirect routing, and transport.
            val state = adapter.authorize(
                configuration = configuration,
                options = options,
                redirectConfiguration = AndroidRedirectConfiguration(
                    appLinkUri = "https://your-app.example/oidc/callback",
                    customSchemeUri = "exampleapp://oidc/callback",
                ),
                transport = transport,
            )

            // Persist the complete state in host-managed secure storage.
            store.putString("native-oidc/state", state.toJson())
            callback.onResult(state.toJson(), null)
        }
    }

    fun cancel() {
        adapter.cancel()
    }
}

/** The guide's post-authorization operations: state restoration, refresh
 * with re-persistence, provider logout with a registered post-logout
 * redirect, and local sign-out. The refresh write is serialized against
 * sign-out so a late refresh cannot restore state after local deletion. */
class GuideSession(
    private val store: GuideSecureStore,
    private val configuration: FfiServiceConfiguration,
    private val scope: CoroutineScope,
) {
    private val sessionLock = Any()
    private var signedOut = false

    // On the next launch: restore the state before any operation that needs
    // it. The first launch, and the launch after sign-out, have nothing
    // stored, so restore only when a value exists — the absence of state
    // means signed-out, not an error.
    fun restoreState(): FfiAuthenticationState? =
        store.getString("native-oidc/state")?.let {
            FfiAuthenticationState.fromJson(serialized = it)
        }

    fun refresh(state: FfiAuthenticationState) {
        scope.launch {
            try {
                // Refresh requires a non-empty refresh token and updates
                // `state` in place after a successful response.
                state.refresh(
                    configuration = configuration,
                    clientId = "native-public-client",
                    additionalParameters = emptyList(),
                    transport = UrlConnectionNativeHttpTransport(),
                    cancellation = FfiCancellationHandle(),
                )
            } catch (error: CancellationException) {
                throw error
            } catch (error: FfiException) {
                // On failure or cancellation the previous state is
                // unchanged; skip the re-persistence.
                return@launch
            }
            // Refresh tokens may rotate; persist the updated complete state,
            // guarded so a refresh that outlives sign-out cannot write the
            // state back.
            val stateJson = state.toJson()
            synchronized(sessionLock) {
                if (signedOut) return@launch
                store.putString("native-oidc/state", stateJson)
            }
        }
    }

    fun signOut() {
        // Local sign-out is independent of provider cleanup: cancel
        // in-flight work, then clear the stored state and any in-memory
        // copies.
        synchronized(sessionLock) {
            signedOut = true
            // Cancels in-flight work on the host scope where supported.
            scope.coroutineContext[Job]?.cancel()
            store.remove("native-oidc/state")
        }
    }

    /** The prepared end-session session together with its URL. The session
     *  must be retained until the post-logout callback returns: validating
     *  the callback requires the original session whose random state was
     *  embedded in the URL, and recreating it would generate a different
     *  state that never matches. */
    data class PreparedEndSession(
        val url: Uri,
        val session: FfiEndSessionSession,
    )

    // Provider logout: the authorization adapter has no logout method.
    // Prepare the end-session URL with the UniFFI API; the host manages the
    // browser and any post-logout callback. Returns the prepared session
    // together with the URL so the callback can be validated against the same
    // session that generated the URL's state.
    fun prepareEndSession(): PreparedEndSession {
        val endSession = beginEndSession(
            configuration = configuration, // requires an end-session endpoint
            options = FfiEndSessionOptions(
                clientId = "native-public-client",
                idTokenHint = null,
                // Must exactly match the registered post-logout redirect;
                // the returned state is validated on return.
                postLogoutRedirectUri = "https://your-app.example/oidc/post-logout",
            ),
        )
        return PreparedEndSession(
            url = Uri.parse(endSession.endSessionUrl()),
            session = endSession,
        )
    }

    // With a post-logout redirect configured, validate the callback when it
    // returns, using the same session that prepared the URL. Returns whether
    // the callback's state matched (i.e., whether the provider logout is
    // verified).
    fun validateEndSessionRedirect(
        session: FfiEndSessionSession,
        callbackUri: Uri,
    ): Boolean {
        return try {
            session.parseRedirect(callbackUri.toString())
            true
        } catch (error: FfiException) {
            // Callback or state mismatch: report the provider logout as
            // unverified.
            false
        }
    }
}
