package net.projectflora.nativeoidc

import android.content.ActivityNotFoundException
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.native_oidc.FfiAuthenticationState
import uniffi.native_oidc.FfiAuthorizationOptions
import uniffi.native_oidc.FfiServiceConfiguration

/**
 * Adapter errors raised before a browser opens. They run against the real
 * package manager and host activity, so no browser or provider is needed.
 */
@RunWith(AndroidJUnit4::class)
class AndroidAuthorizationAdapterErrorTest {
    @Test
    fun unregisteredCustomSchemeFailsWithCallbackActivityNotRegistered() {
        // The androidTest manifest registers only com.example.app and the
        // login.example.test App Link for the callback activity.
        val error =
            authorizeAndAwaitFailure(redirectUri = "com.example.unregistered:/oauth/callback")

        assertTrue(
            "Expected CallbackActivityNotRegistered, got $error",
            error is AndroidAuthorizationAdapterException.CallbackActivityNotRegistered,
        )
    }

    @Test
    fun missingBrowserFailsWithBrowserUnavailableAndReleasesCallback() {
        val redirectUri = "com.example.app:/oauth/callback"
        val error =
            authorizeAndAwaitFailure(
                redirectUri,
                activityClass = AndroidNoBrowserFlowTestActivity::class.java,
            )

        assertTrue(
            "Expected BrowserUnavailable, got $error",
            error is AndroidAuthorizationAdapterException.BrowserUnavailable,
        )
        assertTrue(
            "Expected the ActivityNotFoundException as the cause, got ${error.cause}",
            error.cause is ActivityNotFoundException,
        )
        // The failed operation must not keep the callback route registered.
        AndroidAuthorizationCallbackRegistry.unregister(
            AndroidAuthorizationCallbackRegistry.register(redirectUri),
        )
    }

    @Test
    fun finishingHostActivityFailsWithBrowserUnavailableAndReleasesCallback() {
        val redirectUri = "com.example.app:/oauth/callback"
        val error = authorizeAndAwaitFailure(redirectUri, finishActivityFirst = true)

        assertTrue(
            "Expected BrowserUnavailable, got $error",
            error is AndroidAuthorizationAdapterException.BrowserUnavailable,
        )
        // The failed operation must not keep the callback route registered.
        AndroidAuthorizationCallbackRegistry.unregister(
            AndroidAuthorizationCallbackRegistry.register(redirectUri),
        )
    }

    private fun authorizeAndAwaitFailure(
        redirectUri: String,
        finishActivityFirst: Boolean = false,
        activityClass: Class<out AndroidAuthorizationFlowTestActivity> =
            AndroidAuthorizationFlowTestActivity::class.java,
    ): Throwable =
        runBlocking {
            val scenario = ActivityScenario.launch(activityClass)
            try {
                val result = CompletableDeferred<Result<FfiAuthenticationState>>()
                scenario.onActivity { activity ->
                    if (finishActivityFirst) {
                        activity.finish()
                    }
                    activity.authorize(
                        configuration =
                            FfiServiceConfiguration(
                                "https://issuer.example/authorize",
                                "https://issuer.example/token",
                            ),
                        options =
                            FfiAuthorizationOptions(
                                "client",
                                redirectUri,
                                emptyList(),
                                emptyList(),
                                emptyList(),
                            ),
                        redirects = AndroidRedirectConfiguration(null, redirectUri),
                        transport = UrlConnectionNativeHttpTransport(),
                    ) { result.complete(it) }
                }
                withTimeout(5_000) { result.await() }.exceptionOrNull()
                    ?: throw AssertionError("Authorization unexpectedly succeeded")
            } finally {
                scenario.close()
            }
        }
}
