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
import org.junit.Assert.assertNotNull
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
}
