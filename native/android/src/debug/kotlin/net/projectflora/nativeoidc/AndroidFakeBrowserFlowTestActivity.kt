package net.projectflora.nativeoidc

import android.content.Intent
import android.net.Uri
import android.os.Bundle

/**
 * Behaves as if a browser were installed: every Custom Tab launch succeeds
 * and is recorded in [lastLaunchedUri] and [launchCount], so an
 * authorization reaches its callback wait without starting a browser
 * process. A closed Custom Tab never notifies the application, so the
 * launch is the only step the wait depends on.
 */
class AndroidFakeBrowserFlowTestActivity : AndroidAuthorizationFlowTestActivity() {
    @Volatile
    var lastLaunchedUri: Uri? = null
        private set

    @Volatile
    var launchCount = 0
        private set

    override fun startActivity(
        intent: Intent,
        options: Bundle?,
    ) {
        lastLaunchedUri = intent.data
        launchCount += 1
    }
}
