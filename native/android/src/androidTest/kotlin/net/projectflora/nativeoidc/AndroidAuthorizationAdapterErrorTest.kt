package net.projectflora.nativeoidc

import android.content.ActivityNotFoundException
import android.net.Uri
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
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

    @Test
    fun unverifiedAppLinkFallsBackToRegisteredCustomScheme() {
        // The emulator never verifies login.example.test, and API 28 to 30
        // have no DomainVerificationManager, so the custom scheme is selected.
        val customSchemeUri = "com.example.app:/oauth/callback"
        var launchedUri: Uri? = null
        val error =
            authorizeAndAwaitFailure(
                customSchemeUri,
                appLinkUri = APP_LINK_URI,
                activityClass = AndroidNoBrowserFlowTestActivity::class.java,
            ) { activity ->
                launchedUri = (activity as AndroidNoBrowserFlowTestActivity).lastLaunchedUri
            }

        // BrowserUnavailable comes from the launch, after redirect selection.
        assertTrue(
            "Expected BrowserUnavailable, got $error",
            error is AndroidAuthorizationAdapterException.BrowserUnavailable,
        )
        assertEquals(customSchemeUri, launchedUri?.getQueryParameter("redirect_uri"))
        // The failed operation must not keep the callback route registered.
        AndroidAuthorizationCallbackRegistry.unregister(
            AndroidAuthorizationCallbackRegistry.register(customSchemeUri),
        )
    }

    @Test
    fun unverifiedAppLinkWithoutFallbackFailsWithInvalidRedirectConfiguration() {
        val error =
            authorizeAndAwaitFailure(
                APP_LINK_URI,
                appLinkUri = APP_LINK_URI,
                customSchemeUri = null,
            )

        assertTrue(
            "Expected InvalidRedirectConfiguration, got $error",
            error is AndroidAuthorizationAdapterException.InvalidRedirectConfiguration,
        )
    }

    private fun authorizeAndAwaitFailure(
        redirectUri: String,
        appLinkUri: String? = null,
        customSchemeUri: String? = redirectUri,
        finishActivityFirst: Boolean = false,
        activityClass: Class<out AndroidAuthorizationFlowTestActivity> =
            AndroidAuthorizationFlowTestActivity::class.java,
        // Runs after the failure; pass it only when the activity stays alive.
        inspectActivity: ((AndroidAuthorizationFlowTestActivity) -> Unit)? = null,
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
                        redirects = AndroidRedirectConfiguration(appLinkUri, customSchemeUri),
                        transport = UrlConnectionNativeHttpTransport(),
                    ) { result.complete(it) }
                }
                val error =
                    withTimeout(5_000) { result.await() }.exceptionOrNull()
                        ?: throw AssertionError("Authorization unexpectedly succeeded")
                if (inspectActivity != null) {
                    scenario.onActivity { inspectActivity(it) }
                }
                error
            } finally {
                scenario.close()
            }
        }

    private companion object {
        // Registered for the callback activity in the androidTest manifest,
        // without assetlinks.json, so Android never verifies it.
        const val APP_LINK_URI = "https://login.example.test/oauth/callback"
    }
}
