package net.projectflora.nativeoidc

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import uniffi.native_oidc.FfiAuthenticationState
import uniffi.native_oidc.FfiAuthorizationOptions
import uniffi.native_oidc.FfiServiceConfiguration
import uniffi.native_oidc.NativeHttpTransport

open class AndroidAuthorizationFlowTestActivity : Activity() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    fun authorize(
        configuration: FfiServiceConfiguration,
        options: FfiAuthorizationOptions,
        redirects: AndroidRedirectConfiguration,
        transport: NativeHttpTransport,
        // Seams for tests that reach the adapter, for example to call cancel()
        // while the operation waits for a callback.
        onAdapter: (AndroidAuthorizationAdapter) -> Unit = {},
        onResult: (Result<FfiAuthenticationState>) -> Unit,
    ) {
        scope.launch {
            val adapter = AndroidAuthorizationAdapter(this@AndroidAuthorizationFlowTestActivity)
            onAdapter(adapter)
            val result: Result<FfiAuthenticationState> =
                try {
                    Result.success(
                        adapter.authorize(configuration, options, redirects, transport),
                    )
                } catch (error: Throwable) {
                    // Errors such as NoClassDefFoundError reach the test as a
                    // failure instead of crashing the instrumentation process.
                    Result.failure(error)
                }
            onResult(result)
        }
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }
}

/**
 * Behaves as if no browser is installed: every Custom Tab launch fails. The
 * URL of the last attempted launch is kept in [lastLaunchedUri].
 */
class AndroidNoBrowserFlowTestActivity : AndroidAuthorizationFlowTestActivity() {
    @Volatile
    var lastLaunchedUri: Uri? = null
        private set

    override fun startActivity(
        intent: Intent,
        options: Bundle?,
    ) {
        lastLaunchedUri = intent.data
        throw ActivityNotFoundException("No browser is installed in this test")
    }
}
