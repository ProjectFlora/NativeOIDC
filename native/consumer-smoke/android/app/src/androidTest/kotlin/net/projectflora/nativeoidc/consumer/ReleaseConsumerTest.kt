package net.projectflora.nativeoidc.consumer

import android.net.Uri
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.native_oidc.FfiAuthenticationState
import uniffi.native_oidc.FfiCancellationHandle
import uniffi.native_oidc.FfiEndSessionOptions
import uniffi.native_oidc.FfiException
import uniffi.native_oidc.FfiHttpHeader
import uniffi.native_oidc.FfiHttpRequest
import uniffi.native_oidc.FfiHttpResponse
import uniffi.native_oidc.FfiServiceConfiguration
import uniffi.native_oidc.NativeHttpTransport
import uniffi.native_oidc.beginEndSession
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class ReleaseConsumerTest {
    @Test
    fun constructsAndCallsThePublishedKotlinApi() {
        val configuration = FfiServiceConfiguration(
            "https://issuer.example/authorize",
            "https://issuer.example/token",
        )

        assertEquals("https://issuer.example/authorize", configuration.authorizationEndpoint())
        assertEquals("https://issuer.example/token", configuration.tokenEndpoint())
    }

    // Representative runtime branches for the guide examples (#85), mirroring
    // the Apple release consumer's guide checks: state update, in-flight
    // cancellation, and error classification. The release consumer script
    // runs this test on an emulator against the installed AAR (the published
    // release AAR in the release workflow, the branch-built AAR in PR CI),
    // without a network connection.

    private companion object {
        const val STATE_JSON =
            """{"token_response":{"access_token":"old","token_type":"Bearer","refresh_token":"refresh"}}"""
    }

    /** A transport that returns a fixed JSON response. */
    private class GuideScriptedTransport(
        private val status: UShort,
        private val json: String,
    ) : NativeHttpTransport {
        @Volatile
        var requestCount = 0
            private set

        override suspend fun execute(
            request: FfiHttpRequest,
            cancellation: FfiCancellationHandle,
        ): FfiHttpResponse {
            requestCount += 1
            return FfiHttpResponse(
                status = status,
                headers = listOf(
                    FfiHttpHeader(
                        name = "content-type",
                        value = "application/json".toByteArray(),
                    ),
                ),
                body = json.toByteArray(),
            )
        }
    }

    /** A transport that suspends until the cancellation handle fires, then
     * reports `.Cancelled` like the Rust core does for a cancelled request. */
    private class GuideBlockingTransport(
        private val entered: CountDownLatch,
    ) : NativeHttpTransport {
        override suspend fun execute(
            request: FfiHttpRequest,
            cancellation: FfiCancellationHandle,
        ): FfiHttpResponse {
            entered.countDown()
            cancellation.cancelled()
            throw FfiException.Cancelled()
        }
    }

    @Test
    fun refreshUpdatesStateAndRoundTripsThroughJson() {
        runBlocking {
            val state = FfiAuthenticationState.fromJson(serialized = STATE_JSON)
            val configuration = FfiServiceConfiguration(
                authorizationEndpoint = "https://idp.example/authorize",
                tokenEndpoint = "https://idp.example/token",
            )
            val transport = GuideScriptedTransport(
                status = 200.toUShort(),
                json = """{"access_token":"new","token_type":"Bearer","expires_in":3600,"refresh_token":"rotated","scope":"profile"}""",
            )

            state.refresh(
                configuration = configuration,
                clientId = "native-public-client",
                additionalParameters = emptyList(),
                transport = transport,
                cancellation = FfiCancellationHandle(),
            )

            val tokens = state.tokenResponse()
            assertEquals("refresh must update the state in place", "new", tokens.accessToken)
            assertEquals("refresh must rotate the stored refresh token", "rotated", tokens.refreshToken)
            assertEquals("refresh must make exactly one token request", 1, transport.requestCount)

            // The updated complete state round-trips through JSON.
            val restored = FfiAuthenticationState.fromJson(serialized = state.toJson())
            assertEquals(
                "the re-serialized state must keep the rotated refresh token",
                "rotated",
                restored.tokenResponse().refreshToken,
            )
        }
    }

    @Test
    fun inFlightCancellationSurfacesCancelled() {
        val entered = CountDownLatch(1)
        val transport = GuideBlockingTransport(entered)
        val cancellation = FfiCancellationHandle()
        val outcome = arrayOfNulls<Any>(1)
        val scopeJob = SupervisorJob()
        val scope = CoroutineScope(Dispatchers.Default + scopeJob)
        val job = scope.launch {
            try {
                FfiServiceConfiguration.discover(
                    issuer = "https://idp.example/",
                    transport = transport,
                    cancellation = cancellation,
                )
                outcome[0] = "completed"
            } catch (error: Throwable) {
                outcome[0] = error
            }
        }

        assertTrue(
            "the discovery request must enter the transport",
            entered.await(10, TimeUnit.SECONDS),
        )
        cancellation.cancel()
        runBlocking {
            withTimeout(10_000) { job.join() }
        }
        scopeJob.cancel()

        // A cancel that lands while discovery is in flight must end the
        // operation with .Cancelled, so the host never starts the browser
        // after a cancel.
        val error = outcome[0]
        assertTrue(
            "expected FfiException.Cancelled, got $error",
            error is FfiException.Cancelled,
        )
    }

    @Test
    fun errorBranchesClassifyFailures() {
        runBlocking {
            val state = FfiAuthenticationState.fromJson(serialized = STATE_JSON)
            val configuration = FfiServiceConfiguration(
                authorizationEndpoint = "https://idp.example/authorize",
                tokenEndpoint = "https://idp.example/token",
            ).withEndSessionEndpoint(endpoint = "https://idp.example/logout")

            // Provider rejection surfaces as AuthorizationServer with the code.
            val transport = GuideScriptedTransport(
                status = 400.toUShort(),
                json = """{"error":"invalid_grant","error_description":"stale"}""",
            )
            try {
                state.refresh(
                    configuration = configuration,
                    clientId = "native-public-client",
                    additionalParameters = emptyList(),
                    transport = transport,
                    cancellation = FfiCancellationHandle(),
                )
                fail("refresh unexpectedly succeeded")
            } catch (error: FfiException) {
                assertTrue(
                    "expected FfiException.AuthorizationServer(invalid_grant), got $error",
                    error is FfiException.AuthorizationServer,
                )
                assertEquals(
                    "invalid_grant",
                    (error as FfiException.AuthorizationServer).code,
                )
            }

            // Malformed stored JSON surfaces as InvalidRequest.
            try {
                FfiAuthenticationState.fromJson(serialized = "{")
                fail("malformed state JSON unexpectedly parsed")
            } catch (error: FfiException) {
                assertTrue(
                    "expected FfiException.InvalidRequest, got $error",
                    error is FfiException.InvalidRequest,
                )
            }

            // A registered post-logout redirect validates the returned state.
            val endSession = beginEndSession(
                configuration = configuration,
                options = FfiEndSessionOptions(
                    clientId = "native-public-client",
                    idTokenHint = null,
                    postLogoutRedirectUri = "https://idp.example/post-logout",
                ),
            )
            val url = Uri.parse(endSession.endSessionUrl())
            val stateParameter = url.getQueryParameter("state")
            assertNotNull("the end-session URL must carry a state parameter", stateParameter)

            val okResponse = endSession.parseRedirect(
                "https://idp.example/post-logout?state=$stateParameter"
            )
            assertEquals(
                "the validated state must be echoed back",
                stateParameter,
                okResponse.state,
            )

            // A wrong state fails validation with InvalidResponse.
            try {
                endSession.parseRedirect("https://idp.example/post-logout?state=wrong_state")
                fail("mismatched state unexpectedly validated")
            } catch (error: FfiException) {
                assertTrue(
                    "expected FfiException.InvalidResponse, got $error",
                    error is FfiException.InvalidResponse,
                )
            }
        }
    }

    // An in-memory secure store for the post-authorization lifecycle checks.
    private class InMemoryGuideStore : GuideSecureStore {
        private val values = mutableMapOf<String, String>()

        override fun getString(key: String): String? = values[key]
        override fun putString(key: String, value: String) {
            values[key] = value
        }
        override fun remove(key: String) {
            values.remove(key)
        }
    }

    private fun guideConfiguration(): FfiServiceConfiguration =
        FfiServiceConfiguration(
            authorizationEndpoint = "https://idp.example/authorize",
            tokenEndpoint = "https://idp.example/token",
        ).withEndSessionEndpoint(endpoint = "https://idp.example/logout")

    // A missing stored state must restore to null, not crash: the first
    // launch, and the launch after sign-out, have nothing stored.
    @Test
    fun restoreStateHandlesMissingState() {
        val store = InMemoryGuideStore()
        val session = GuideSession(
            store = store,
            configuration = guideConfiguration(),
            scope = CoroutineScope(Dispatchers.Default + SupervisorJob()),
        )

        // First launch: nothing is stored yet.
        assertNull(
            "first launch has no stored state and must not crash",
            session.restoreState(),
        )

        // After storing state and signing out, storage is empty again.
        store.putString("native-oidc/state", STATE_JSON)
        assertNotNull(
            "a stored state must restore",
            session.restoreState(),
        )
        session.signOut()
        assertNull(
            "after sign-out the stored state is gone and must not crash",
            session.restoreState(),
        )
    }

    // The end-session helper must retain the prepared session with its URL:
    // validation needs the session whose random state is embedded in the URL,
    // and a scripted callback derived from that URL validates against it.
    @Test
    fun endSessionLifecycleRetainsPreparedSession() {
        val session = GuideSession(
            store = InMemoryGuideStore(),
            configuration = guideConfiguration(),
            scope = CoroutineScope(Dispatchers.Default + SupervisorJob()),
        )

        val prepared = session.prepareEndSession()
        val stateParameter = prepared.url.getQueryParameter("state")
        assertNotNull("the end-session URL must carry a state parameter", stateParameter)

        // A scripted callback derived from the generated URL validates against
        // the retained session.
        assertTrue(
            "a callback derived from the generated URL must validate",
            session.validateEndSessionRedirect(
                prepared.session,
                Uri.parse("https://your-app.example/oidc/post-logout?state=$stateParameter"),
            ),
        )

        // A mismatched state must not validate.
        assertFalse(
            "a mismatched state must not validate",
            session.validateEndSessionRedirect(
                prepared.session,
                Uri.parse("https://your-app.example/oidc/post-logout?state=wrong_state"),
            ),
        )
    }
}
