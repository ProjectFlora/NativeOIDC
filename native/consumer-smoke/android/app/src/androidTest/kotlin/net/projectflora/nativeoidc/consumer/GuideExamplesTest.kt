package net.projectflora.nativeoidc.consumer

import android.app.Activity
import android.content.ActivityNotFoundException
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.browser.customtabs.CustomTabsIntent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.future.future
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import net.projectflora.nativeoidc.AndroidAuthorizationAdapter
import net.projectflora.nativeoidc.AndroidAuthorizationAdapterException
import net.projectflora.nativeoidc.AndroidRedirectConfiguration
import net.projectflora.nativeoidc.UrlConnectionNativeHttpTransport
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.native_oidc.FfiAuthenticationState
import uniffi.native_oidc.FfiAuthorizationOptions
import uniffi.native_oidc.FfiCancellationHandle
import uniffi.native_oidc.FfiEndSessionOptions
import uniffi.native_oidc.FfiEndSessionSession
import uniffi.native_oidc.FfiException
import uniffi.native_oidc.FfiHttpRequest
import uniffi.native_oidc.FfiHttpResponse
import uniffi.native_oidc.FfiParameter
import uniffi.native_oidc.FfiRegistrationOptions
import uniffi.native_oidc.FfiServiceConfiguration
import uniffi.native_oidc.NativeHttpTransport
import uniffi.native_oidc.beginAuthorization
import uniffi.native_oidc.beginEndSession
import uniffi.native_oidc.registerPublicClient
import uniffi.native_oidc.revoke
import java.io.File
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.charset.StandardCharsets
import java.security.KeyStore
import java.util.concurrent.CompletableFuture
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlin.time.Duration.Companion.minutes

/**
 * Executes the examples in `docs/kotlin-guide.md` against the AAR this
 * fixture is built with: the published Maven Central artifact
 * (`-PnativeOidcVersion=<version>`) or a local release AAR
 * (`scripts/check-android-release-consumer.sh`).
 *
 * Provider traffic goes through a scripted in-process transport, so the
 * tests are deterministic and perform no network I/O. The browser part of
 * authorization (Custom Tab and callback activity) is covered by the
 * library module's emulator tests; here the same core calls the adapter
 * makes (`beginAuthorization`, `parseRedirect`, `exchangeCode`) run with a
 * scripted callback URI.
 */
@RunWith(AndroidJUnit4::class)
class GuideExamplesTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private lateinit var storage: EncryptedStateStore

    @Before
    fun setUp() {
        storage = EncryptedStateStore(File(context.noBackupFilesDir, "guide-test-state.bin"), KEY_ALIAS)
        storage.delete()
    }

    @After
    fun tearDown() {
        storage.delete()
        KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(KEY_ALIAS)
    }

    // Guide section 3: authorization, token access and expiry.
    @Test
    fun authorizationExchangeReturnsTokensAndExpiry() = runBlocking {
        val transport = ScriptedTransport(json(200, TOKEN_RESPONSE))
        val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN)
        val options =
            FfiAuthorizationOptions(
                clientId = CLIENT_ID,
                redirectUri = REDIRECT,
                scopes = listOf("profile"),
                additionalParameters = listOf(FfiParameter("prompt", "login")),
                additionalTokenParameters = emptyList(),
            )
        val session = beginAuthorization(configuration, options, transport)
        val authorizationUrl = Uri.parse(session.authorizationUrl())
        assertEquals("login", authorizationUrl.getQueryParameter("prompt"))
        assertEquals(CLIENT_ID, authorizationUrl.getQueryParameter("client_id"))
        val state = authorizationUrl.getQueryParameter("state")!!

        val response = session.parseRedirect("$REDIRECT?code=c&state=$state")
        val before = System.currentTimeMillis() / 1000
        val authState = session.exchangeCode(response, FfiCancellationHandle())

        val tokens = authState.tokenResponse()
        assertEquals("access-1", tokens.accessToken)
        assertEquals("Bearer", tokens.tokenType)
        assertEquals("refresh-1", tokens.refreshToken)
        val expiresAt = authState.expiresAt()
        assertNotNull(expiresAt)
        assertTrue(expiresAt!!.toLong() >= before + 3600)
        assertEquals(false, authState.isExpiredAt(before.toULong()))
        assertEquals(true, authState.isExpiredAt(expiresAt))

        val exchange = transport.requests.single()
        assertEquals("POST", exchange.method)
        assertEquals(TOKEN, exchange.url)
        assertTrue(String(exchange.body, StandardCharsets.UTF_8).contains("grant_type=authorization_code"))
    }

    // Guide section 7: additional parameters are validated before any request.
    @Test
    fun reservedAndDuplicateAdditionalParametersAreRejected() {
        val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN)
        val transport = ScriptedTransport()
        for (parameters in listOf(
            listOf(FfiParameter("state", "x")),
            listOf(FfiParameter("prompt", "login"), FfiParameter("prompt", "consent")),
        )) {
            try {
                beginAuthorization(
                    configuration,
                    FfiAuthorizationOptions(CLIENT_ID, REDIRECT, emptyList(), parameters, emptyList()),
                    transport,
                )
                fail("Expected FfiException.InvalidRequest for $parameters")
            } catch (error: FfiException.InvalidRequest) {
                // Expected.
            }
        }
        assertTrue(transport.requests.isEmpty())
    }

    // Guide section 4: Keystore-protected save, restore and local deletion.
    @Test
    fun keystoreProtectedStateRoundTrips() = runBlocking {
        val original = FfiAuthenticationState.fromJson(STATE_JSON)
        storage.save(original)

        val ciphertext = storage.file.readBytes()
        assertFalse(String(ciphertext, StandardCharsets.ISO_8859_1).contains("refresh-0"))

        val restored = storage.load()
        assertNotNull(restored)
        assertEquals(original.toJson(), restored!!.toJson())

        storage.delete()
        assertNull(storage.load())
    }

    // Guide section 5: refresh, rotation, and re-save.
    @Test
    fun refreshUpdatesStateAndKeepsUnrotatedRefreshToken() = runBlocking {
        val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN)
        val state = FfiAuthenticationState.fromJson(STATE_JSON)

        val rotating = ScriptedTransport(json(200, ROTATED_TOKEN_RESPONSE))
        refreshAndSave(state, configuration, rotating, FfiCancellationHandle())
        assertEquals("access-2", state.tokenResponse().accessToken)
        assertEquals("refresh-2", state.tokenResponse().refreshToken)
        assertTrue(String(rotating.requests.single().body, StandardCharsets.UTF_8).contains("refresh_token=refresh-0"))

        val unrotated = ScriptedTransport(json(200, """{"access_token":"access-3","token_type":"Bearer"}"""))
        refreshAndSave(state, configuration, unrotated, FfiCancellationHandle())
        val saved = storage.load()!!.tokenResponse()
        assertEquals("access-3", saved.accessToken)
        assertEquals("refresh-2", saved.refreshToken)
    }

    // Guide sections 5 and 8: failure and cancellation leave state unchanged.
    @Test
    fun refreshFailureAndCancellationLeaveStateUnchanged() = runBlocking {
        val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN)
        val state = FfiAuthenticationState.fromJson(STATE_JSON)
        val before = state.toJson()

        val rejected = ScriptedTransport(json(400, """{"error":"invalid_grant"}"""))
        try {
            state.refresh(configuration, CLIENT_ID, emptyList(), rejected, FfiCancellationHandle())
            fail("Expected FfiException.AuthorizationServer")
        } catch (error: FfiException.AuthorizationServer) {
            assertEquals("invalid_grant", error.code)
        }
        assertEquals(before, state.toJson())

        val unavailable = ScriptedTransport(json(503, ""))
        try {
            state.refresh(configuration, CLIENT_ID, emptyList(), unavailable, FfiCancellationHandle())
            fail("Expected FfiException.HttpStatus")
        } catch (error: FfiException.HttpStatus) {
            assertEquals(503.toUShort(), error.status)
        }
        assertEquals(before, state.toJson())

        val failing = ScriptedTransport(failure = IllegalStateException("offline"))
        try {
            state.refresh(configuration, CLIENT_ID, emptyList(), failing, FfiCancellationHandle())
            fail("Expected FfiException.TransportFailure")
        } catch (error: FfiException.TransportFailure) {
            // Expected.
        }
        assertEquals(before, state.toJson())

        val cancelled = FfiCancellationHandle().apply { cancel() }
        val untouched = ScriptedTransport(json(200, ROTATED_TOKEN_RESPONSE))
        try {
            state.refresh(configuration, CLIENT_ID, emptyList(), untouched, cancelled)
            fail("Expected FfiException.Cancelled")
        } catch (error: FfiException.Cancelled) {
            // Expected.
        }
        assertTrue(untouched.requests.isEmpty())
        assertEquals(before, state.toJson())

        // Cancel while the refresh request is in flight.
        val inFlight = FfiCancellationHandle()
        val blocking = BlockingTransport()
        val pending =
            async(Dispatchers.Default) {
                runCatching { state.refresh(configuration, CLIENT_ID, emptyList(), blocking, inFlight) }
            }
        val entered = withTimeoutOrNull(10_000) { blocking.started.await() } != null
        assertTrue("refresh did not reach the transport within 10 seconds", entered)
        inFlight.cancel()
        val outcome = withTimeoutOrNull(10_000) { pending.await() }
        pending.cancel()
        assertNotNull("refresh did not complete within 10 seconds after cancellation", outcome)
        assertTrue(
            "Expected FfiException.Cancelled, got ${outcome!!.exceptionOrNull()}",
            outcome.exceptionOrNull() is FfiException.Cancelled,
        )
        assertEquals(before, state.toJson())
    }

    // Guide sections 2 and 8: adapter configuration errors raised before any
    // browser opens. This app does not register the callback activity.
    @Test
    fun adapterRejectsUnregisteredCallbackAndInvalidFallback() = runBlocking {
        ActivityScenario.launch(GuideHostActivity::class.java).use { scenario ->
            var host: Activity? = null
            scenario.onActivity { host = it }
            val adapter = AndroidAuthorizationAdapter(host!!)
            val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN)
            val transport = ScriptedTransport()

            suspend fun failure(customSchemeUri: String): Throwable {
                try {
                    adapter.authorize(
                        configuration,
                        FfiAuthorizationOptions(CLIENT_ID, customSchemeUri, emptyList(), emptyList(), emptyList()),
                        AndroidRedirectConfiguration(appLinkUri = null, customSchemeUri = customSchemeUri),
                        transport,
                    )
                } catch (error: Throwable) {
                    return error
                }
                throw AssertionError("Authorization unexpectedly succeeded for $customSchemeUri")
            }

            val unregistered = failure(REDIRECT)
            assertTrue(
                "Expected CallbackActivityNotRegistered, got $unregistered",
                unregistered is AndroidAuthorizationAdapterException.CallbackActivityNotRegistered,
            )
            val invalid = failure("https://login.example.com/oauth/callback")
            assertTrue(
                "Expected InvalidRedirectConfiguration, got $invalid",
                invalid is AndroidAuthorizationAdapterException.InvalidRedirectConfiguration,
            )
            assertTrue(transport.requests.isEmpty())
        }
    }

    // Guide section 6: end-session URL and post-logout callback validation.
    @Test
    fun endSessionBuildsUrlAndValidatesCallback() {
        val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN).withEndSessionEndpoint(LOGOUT)
        val session =
            beginEndSession(
                configuration,
                FfiEndSessionOptions(CLIENT_ID, idTokenHint = "id-token", postLogoutRedirectUri = LOGOUT_REDIRECT),
            )
        val url = Uri.parse(session.endSessionUrl())
        assertEquals("id-token", url.getQueryParameter("id_token_hint"))
        assertEquals(LOGOUT_REDIRECT, url.getQueryParameter("post_logout_redirect_uri"))
        val state = url.getQueryParameter("state")!!

        assertEquals(state, session.parseRedirect("$LOGOUT_REDIRECT?state=$state").state)
        try {
            session.parseRedirect("$LOGOUT_REDIRECT?state=other")
            fail("Expected FfiException.InvalidResponse")
        } catch (error: FfiException.InvalidResponse) {
            // Expected.
        }
    }

    // Guide section 6: the host's pending-logout handling in the callback activity.
    @Test
    fun pendingLogoutAcceptsOnlyAValidCallbackForTheRetainedSession() {
        val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN).withEndSessionEndpoint(LOGOUT)
        fun begin(): String {
            val session = beginEndSession(configuration, FfiEndSessionOptions(CLIENT_ID, null, LOGOUT_REDIRECT))
            PendingLogout.begin(session)
            return Uri.parse(session.endSessionUrl()).getQueryParameter("state")!!
        }

        // No session (for example after process recreation).
        PendingLogout.abandon()
        assertFalse(PendingLogout.complete("$LOGOUT_REDIRECT?state=x"))

        // A launch without data keeps the pending session.
        val state = begin()
        assertFalse(PendingLogout.complete(null))
        assertTrue(PendingLogout.complete("$LOGOUT_REDIRECT?state=$state"))
        // The session is consumed.
        assertFalse(PendingLogout.complete("$LOGOUT_REDIRECT?state=$state"))

        // A stale callback from an earlier attempt neither completes nor
        // cancels the current attempt.
        val first = begin()
        val second = begin()
        assertFalse(PendingLogout.complete("$LOGOUT_REDIRECT?state=$first"))
        assertTrue(PendingLogout.complete("$LOGOUT_REDIRECT?state=$second"))
    }

    // Guide section 6: a browser launch failure clears the pending attempt.
    @Test
    fun logoutLaunchFailureClearsThePendingSession() {
        val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN).withEndSessionEndpoint(LOGOUT)
        var launched: Uri? = null
        val started =
            startLogout(configuration, idToken = null) { url ->
                launched = url
                throw ActivityNotFoundException("no browser")
            }
        assertFalse(started)
        val state = launched!!.getQueryParameter("state")!!
        assertFalse(PendingLogout.complete("$LOGOUT_REDIRECT?state=$state"))
    }

    // Guide section 6: revocation, then local deletion regardless of the result.
    @Test
    fun signOutRevokesAndDeletesLocalState() = runBlocking {
        val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN).withRevocationEndpoint(REVOKE)
        storage.save(FfiAuthenticationState.fromJson(STATE_JSON))

        val transport = ScriptedTransport(json(200, ""))
        signOut(configuration, transport)
        assertNull(storage.load())
        val body = String(transport.requests.single().body, StandardCharsets.UTF_8)
        assertTrue(body.contains("token=refresh-0"))
        assertTrue(body.contains("token_type_hint=refresh_token"))

        storage.save(FfiAuthenticationState.fromJson(STATE_JSON))
        signOut(configuration, ScriptedTransport(json(503, "")))
        assertNull(storage.load())

        // Corrupt ciphertext cannot be restored, but sign-out still deletes it.
        storage.file.writeBytes(byteArrayOf(12, 1, 2, 3))
        val unused = ScriptedTransport(json(200, ""))
        signOut(configuration, unused)
        assertFalse(storage.file.exists())
        assertTrue(unused.requests.isEmpty())
    }

    // Guide section 7: dynamic public-client registration.
    @Test
    fun registrationReturnsClientIdAndClassifiesErrors() = runBlocking {
        val configuration = FfiServiceConfiguration(AUTHORIZE, TOKEN).withRegistrationEndpoint(REGISTER)
        val options = FfiRegistrationOptions(listOf(REDIRECT), clientName = "Example", initialAccessToken = null)

        val registered =
            registerPublicClient(
                configuration,
                options,
                ScriptedTransport(json(201, """{"client_id":"registered","application_type":"native","token_endpoint_auth_method":"none"}""")),
                FfiCancellationHandle(),
            )
        assertEquals("registered", registered.clientId)

        try {
            registerPublicClient(
                configuration,
                options,
                ScriptedTransport(json(400, """{"error":"invalid_redirect_uri"}""")),
                FfiCancellationHandle(),
            )
            fail("Expected FfiException.ClientRegistrationServer")
        } catch (error: FfiException.ClientRegistrationServer) {
            assertEquals("invalid_redirect_uri", error.code)
        }
    }

    /** The guide's refresh-and-persist example. */
    private suspend fun refreshAndSave(
        state: FfiAuthenticationState,
        configuration: FfiServiceConfiguration,
        transport: NativeHttpTransport,
        cancellation: FfiCancellationHandle,
    ) {
        state.refresh(configuration, CLIENT_ID, emptyList(), transport, cancellation)
        storage.save(state)
    }

    /** The guide's sign-out example. */
    private suspend fun signOut(
        configuration: FfiServiceConfiguration,
        transport: NativeHttpTransport,
    ) {
        try {
            // Unreadable state (invalidated key, corrupt file) has nothing to revoke.
            val state = runCatching { storage.load() }.getOrNull()
            state?.use {
                it.tokenResponse().refreshToken?.let { token ->
                    revoke(configuration, CLIENT_ID, token, "refresh_token", transport, FfiCancellationHandle())
                }
            }
        } catch (error: FfiException) {
            // Revocation failure does not block local sign-out.
        } finally {
            storage.delete()
        }
    }

    private companion object {
        const val KEY_ALIAS = "nativeoidc-guide-test"
        const val CLIENT_ID = "client"
        const val AUTHORIZE = "https://idp.example/authorize"
        const val TOKEN = "https://idp.example/token"
        const val REVOKE = "https://idp.example/revoke"
        const val REGISTER = "https://idp.example/register"
        const val LOGOUT = "https://idp.example/logout"
        const val REDIRECT = "com.example.app:/oauth/callback"
        const val LOGOUT_REDIRECT = "com.example.app.logout:/callback"
        const val STATE_JSON =
            """{"token_response":{"access_token":"access-0","token_type":"Bearer","refresh_token":"refresh-0"}}"""
        const val TOKEN_RESPONSE =
            """{"access_token":"access-1","token_type":"Bearer","expires_in":3600,"refresh_token":"refresh-1"}"""
        const val ROTATED_TOKEN_RESPONSE =
            """{"access_token":"access-2","token_type":"Bearer","expires_in":3600,"refresh_token":"refresh-2"}"""

        fun json(
            status: Int,
            body: String,
        ) = FfiHttpResponse(status.toUShort(), emptyList(), body.toByteArray(StandardCharsets.UTF_8))
    }
}

/**
 * The guide's host-side storage example: AES-GCM with a non-exportable key
 * in Android Keystore, written to app-private storage.
 */
class EncryptedStateStore(
    val file: File,
    private val keyAlias: String,
) {
    suspend fun save(state: FfiAuthenticationState) {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val ciphertext = cipher.doFinal(state.toJson().toByteArray(StandardCharsets.UTF_8))
        val iv = cipher.iv
        val output = ByteBuffer.allocate(1 + iv.size + ciphertext.size).put(iv.size.toByte()).put(iv).put(ciphertext)
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
        val json = String(cipher.doFinal(ciphertext), StandardCharsets.UTF_8)
        return FfiAuthenticationState.fromJson(json)
    }

    fun delete() {
        if (file.exists() && !file.delete()) throw IOException("Could not delete stored state")
    }

    private fun key(): SecretKey {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (keyStore.getKey(keyAlias, null) as SecretKey?)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec
                .Builder(keyAlias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build(),
        )
        return generator.generateKey()
    }
}

/** Signals when a request starts, then waits for cancellation, like the library coverage test. */
private class BlockingTransport : NativeHttpTransport {
    val started = CompletableDeferred<Unit>()

    override suspend fun execute(
        request: FfiHttpRequest,
        cancellation: FfiCancellationHandle,
    ): FfiHttpResponse {
        started.complete(Unit)
        cancellation.cancelled()
        throw FfiException.Cancelled()
    }
}

/** Records requests and returns one scripted response or failure. */
private class ScriptedTransport(
    private val response: FfiHttpResponse? = null,
    private val failure: Exception? = null,
) : NativeHttpTransport {
    private val recorded = mutableListOf<FfiHttpRequest>()

    val requests: List<FfiHttpRequest>
        get() = synchronized(recorded) { recorded.toList() }

    override suspend fun execute(
        request: FfiHttpRequest,
        cancellation: FfiCancellationHandle,
    ): FfiHttpResponse {
        synchronized(recorded) { recorded.add(request) }
        failure?.let { throw it }
        return response ?: error("No scripted response")
    }
}

/*
 * Compile-only checks for the guide's adapter and Java-wrapper examples.
 * A successful authorization needs a registered callback activity, a
 * browser and a provider, so these are type-checked here; the library
 * module's emulator tests run the Custom Tab flow.
 */

@Suppress("unused")
private suspend fun guideSignIn(
    adapter: AndroidAuthorizationAdapter,
    configuration: FfiServiceConfiguration,
): FfiAuthenticationState =
    adapter.authorize(
        configuration,
        FfiAuthorizationOptions(
            clientId = "client",
            redirectUri = "com.example.app:/oauth/callback",
            scopes = listOf("openid", "profile"),
            additionalParameters = emptyList(),
            additionalTokenParameters = emptyList(),
        ),
        AndroidRedirectConfiguration(
            // v0.9.0 falls back on API 28–30 by itself; the guard also covers v0.8.0.
            appLinkUri =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    "https://login.example.com/oauth/callback"
                } else {
                    null
                },
            customSchemeUri = "com.example.app:/oauth/callback",
        ),
        UrlConnectionNativeHttpTransport(),
    )

@Suppress("unused")
private suspend fun guideSignInWithTimeout(
    adapter: AndroidAuthorizationAdapter,
    configuration: FfiServiceConfiguration,
): FfiAuthenticationState? =
    try {
        withTimeoutOrNull(5.minutes) { guideSignIn(adapter, configuration) }
    } catch (error: FfiException.Cancelled) {
        null
    }

@Suppress("unused")
private fun guideDescribe(error: Throwable): String =
    when (error) {
        is FfiException.Cancelled -> "cancelled"
        is FfiException.AuthorizationServer -> "provider rejected: ${error.code}"
        is FfiException.HttpStatus -> "HTTP ${error.status}"
        is FfiException.TransportFailure -> "network failure"
        is FfiException -> "protocol or configuration error"
        is AndroidAuthorizationAdapterException.CallbackActivityNotRegistered -> "callback activity missing"
        is AndroidAuthorizationAdapterException.InvalidRedirectConfiguration -> "invalid redirect configuration"
        is AndroidAuthorizationAdapterException.BrowserUnavailable -> "no browser"
        is AndroidAuthorizationAdapterException.AuthorizationAlreadyRunning -> "already running"
        else -> "unexpected"
    }

/** The guide's pending-logout holder used by the logout callback activity. */
object PendingLogout {
    private var session: FfiEndSessionSession? = null

    @Synchronized
    fun begin(session: FfiEndSessionSession) {
        this.session?.close()
        this.session = session
    }

    @Synchronized
    fun abandon() {
        session?.close()
        session = null
    }

    @Synchronized
    fun complete(callbackUri: String?): Boolean {
        val pending = session ?: return false
        if (callbackUri == null) return false
        return try {
            pending.parseRedirect(callbackUri)
            session = null
            pending.close()
            true
        } catch (error: FfiException) {
            false
        }
    }
}

/** The guide's logout start; returns false when the browser could not be opened. */
fun startLogout(
    configuration: FfiServiceConfiguration,
    idToken: String?,
    launch: (Uri) -> Unit,
): Boolean {
    val endSession =
        beginEndSession(
            configuration,
            FfiEndSessionOptions(
                clientId = "client",
                idTokenHint = idToken,
                postLogoutRedirectUri = "com.example.app.logout:/callback",
            ),
        )
    PendingLogout.begin(endSession)
    return try {
        launch(Uri.parse(endSession.endSessionUrl()))
        true
    } catch (error: ActivityNotFoundException) {
        PendingLogout.abandon()
        false
    }
}

@Suppress("unused")
private fun guideStartLogoutFromActivity(
    activity: Activity,
    configuration: FfiServiceConfiguration,
    idToken: String?,
): Boolean =
    startLogout(configuration, idToken) { url ->
        CustomTabsIntent.Builder().build().launchUrl(activity, url)
    }

/** Compile-only copy of the guide's logout callback activity (not in the manifest). */
@Suppress("unused")
class GuideLogoutCallbackActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        PendingLogout.complete(intent?.dataString)
        finish()
    }
}

@Suppress("unused")
private class GuideJavaFacade(
    private val scope: CoroutineScope,
    activity: Activity,
) {
    private val adapter = AndroidAuthorizationAdapter(activity)

    fun signIn(configuration: FfiServiceConfiguration): CompletableFuture<FfiAuthenticationState> =
        scope.future { guideSignIn(adapter, configuration) }

    fun cancel() = adapter.cancel()
}

@Suppress("unused")
private suspend fun guideDiscover(): FfiServiceConfiguration =
    FfiServiceConfiguration.discover(
        "https://login.example.com/",
        UrlConnectionNativeHttpTransport(),
        FfiCancellationHandle(),
    )
