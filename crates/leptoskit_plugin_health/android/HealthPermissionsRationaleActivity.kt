package dev.leptoskit.plugins.health

import android.app.Activity
import android.os.Bundle

/**
 * The permission-rationale entry Health Connect requires on Android 13 and
 * earlier: it answers `androidx.health.ACTION_SHOW_PERMISSIONS_RATIONALE`.
 *
 * Health Connect only shows its permission screen to apps that declare this
 * path. A generated app has no privacy policy of its own to present, so this
 * activity finishes immediately; an app that wants to explain its health-data
 * use declares its own activity with the same intent filter in
 * `hosts/android/app/src/main/AndroidManifest.xml`, which host generation then
 * leaves untouched, and this generated entry stays unused.
 */
class HealthPermissionsRationaleActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // The NoDisplay theme requires finishing before the window shows.
        finish()
    }
}
