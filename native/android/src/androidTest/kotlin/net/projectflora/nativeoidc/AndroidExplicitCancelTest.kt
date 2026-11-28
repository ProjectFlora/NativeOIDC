package net.projectflora.nativeoidc

import android.net.Uri
import android.os.SystemClock
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.native_oidc.FfiAuthenticationState
import uniffi.native_oidc.FfiAuthorizationOptions
import uniffi.native_oidc.FfiException
import uniffi.native_oidc.FfiServiceConfiguration

/**
 * An explicit cancel releases the callback registration, and the next
 * authorization can start. They run against the real adapter, the real
 * callback registry, and the real host activity, with a faked browser
 * launch: on Android a closed Custom Tab never notifies the application, so
 * a real browser process would add nothing to the cancel/wait contract under
 * test.
 */
@RunWith(AndroidJUnit4::class)
class AndroidExplicitCancelTest {
    @Test
    fun explicitCancelReleasesCallbackAndNextAuthorizationStarts() = runBlocking {
        val scenario = ActivityScenario.launch(AndroidFakeBrowserFlowTestActivity::class.java)
        try {
            val first = startAuthorization(scenario)
            cancelAndExpectCancelled(first)

            // Had the cancelled operation kept the callback route registered,
            // the next authorization would fail with AuthorizationAlreadyRunning
            // and never launch.
            val second = startAuthorization(scenario, expectedLaunch = 2)
            cancelAndExpectCancelled(second)

            // The cancelled operations released the callback route again.
            AndroidAuthorizationCallbackRegistry.unregister(
                AndroidAuthorizationCallbackRegistry.register(redirectUri),
            )
        } finally {
            scenario.close()
        }
    }

    private class AuthorizationAttempt(
        val result: CompletableDeferred<Result<FfiAuthenticationState>>,
    ) {
        @Volatile
        var adapter: AndroidAuthorizationAdapter? = null

        @Volatile
        var failure: Throwable? = null
    }

    private fun startAuthorization(
        scenario: ActivityScenario<AndroidFakeBrowserFlowTestActivity>,
        expectedLaunch: Int = 1,
    ): AuthorizationAttempt {
        val attempt = AuthorizationAttempt(CompletableDeferred())
        scenario.onActivity { activity ->
            activity.authorize(
                configuration =
                    FfiServiceConfiguration(
                        authorizationEndpoint = "https://issuer.example/authorize",
                        tokenEndpoint = "https://issuer.example/token",
                    ),
                options =
                    FfiAuthorizationOptions(
                        clientId = "client",
                        redirectUri = redirectUri,
                        scopes = emptyList(),
                        additionalParameters = emptyList(),
                        additionalTokenParameters = emptyList(),
                    ),
                redirects = AndroidRedirectConfiguration(null, redirectUri),
                transport = UrlConnectionNativeHttpTransport(),
                onAdapter = { attempt.adapter = it },
            ) { result ->
                attempt.failure = result.exceptionOrNull()
                attempt.result.complete(result)
            }
        }
        // The activity's scope is Dispatchers.Main.immediate, so the
        // authorization runs synchronously on the main thread up to the
        // callback wait by the time onActivity returns; the faked launch has
        // been recorded by then.
        val deadline = SystemClock.uptimeMillis() + 10_000
        var launches = 0
        var launchedUri: Uri? = null
        while (true) {
            scenario.onActivity { activity ->
                launches = activity.launchCount
                launchedUri = activity.lastLaunchedUri
            }
            if (launches >= expectedLaunch || attempt.result.isCompleted) break
            val detail =
                if (attempt.result.isCompleted) {
                    "failed: " + (attempt.failure ?: "completed without failure")
                } else {
                    "pending"
                }
            assertTrue(
                "authorization $expectedLaunch did not start within 10s " +
                    "(launchCount=$launches, result=$detail)",
                SystemClock.uptimeMillis() < deadline,
            )
            Thread.sleep(50)
        }
        assertEquals(
            "authorization $expectedLaunch launched the browser $launches times",
            expectedLaunch,
            launches,
        )
        val launched =
            launchedUri
                ?: throw AssertionError("authorization $expectedLaunch did not launch a browser")
        assertNotNull(
            "the launched authorization URL has no state parameter",
            launched.getQueryParameter("state"),
        )
        return attempt
    }

    private suspend fun cancelAndExpectCancelled(attempt: AuthorizationAttempt) {
        val adapter = attempt.adapter ?: throw AssertionError("the adapter was not created")
        adapter.cancel()
        // The resumption runs on the main thread; await it before reading the
        // recorded failure.
        withTimeout(5_000) { attempt.result.await() }
        val error = attempt.failure
        assertTrue(
            "the authorization did not fail with FfiException.Cancelled after " +
                "the explicit cancel (failure=$error)",
            error is FfiException.Cancelled,
        )
    }

    private companion object {
        // Registered for the callback activity in the androidTest manifest,
        // the same route the other adapter tests use.
        const val redirectUri = "com.example.app:/oauth/callback"
    }
}
