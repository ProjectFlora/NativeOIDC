package net.projectflora.nativeoidc

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
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
        onResult: (Result<FfiAuthenticationState>) -> Unit,
    ) {
        scope.launch {
            val result: Result<FfiAuthenticationState> =
                try {
                    Result.success(
                        AndroidAuthorizationAdapter(this@AndroidAuthorizationFlowTestActivity)
                            .authorize(configuration, options, redirects, transport),
                    )
                } catch (error: Exception) {
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

/** Behaves as if no browser is installed: every Custom Tab launch fails. */
class AndroidNoBrowserFlowTestActivity : AndroidAuthorizationFlowTestActivity() {
    override fun startActivity(
        intent: Intent,
        options: Bundle?,
    ): Unit = throw ActivityNotFoundException("No browser is installed in this test")
}
