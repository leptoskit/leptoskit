package dev.leptoskit.field

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import dev.leptoskit.bridge.NativeActivityResult
import dev.leptoskit.bridge.NativeActivityResultBroker
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePermissionBroker
import dev.leptoskit.bridge.NativePermissionHost
import dev.leptoskit.mobile.GeneratedPlugins
import dev.leptoskit.mobile.LeptosKitApplicationView

class MainActivity :
    Activity(),
    NativePermissionHost,
    NativeActivityResultHost {
    private var content: LeptosKitApplicationView? = null
    private val requests = NativeRequests(this)

    override suspend fun requestNativePermissions(permissions: Array<String>): IntArray = requests.permissions.request(permissions)

    override suspend fun launchPluginActivity(intent: Intent): NativeActivityResult = requests.activityResults.launch(intent)

    @Deprecated("Android platform callback; delegated to the owned activity-result broker")
    override fun onActivityResult(
        requestCode: Int,
        resultCode: Int,
        data: Intent?,
    ) {
        super.onActivityResult(requestCode, resultCode, data)
        requests.activityResults.onResult(requestCode, resultCode, data)
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        requests.permissions.onResult(requestCode, permissions, grantResults)
    }

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        requests.restoreState(state)
        content = applicationView(this)
        setContentView(content)
        noteDeepLink(intent)
    }

    @Deprecated("Android platform callback; routed through the file router when possible")
    override fun onBackPressed() {
        if (offerSystemBack(content)) {
            return
        }
        @Suppress("DEPRECATION")
        super.onBackPressed()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        noteDeepLink(intent)
    }

    override fun onSaveInstanceState(state: Bundle) {
        requests.saveState(state)
        super.onSaveInstanceState(state)
    }

    override fun onResume() {
        super.onResume()
        requests.setActive(true)
    }

    override fun onPause() {
        requests.setActive(false)
        super.onPause()
    }

    override fun onDestroy() {
        requests.close()
        content?.close()
        content = null
        super.onDestroy()
    }
}

private class NativeRequests(
    private val activity: Activity,
) {
    val permissions by lazy { NativePermissionBroker(activity) }
    val activityResults by lazy { NativeActivityResultBroker(activity) }

    fun restoreState(state: Bundle?) {
        permissions.restoreState(state)
        activityResults.restoreState(state)
    }

    fun saveState(state: Bundle) {
        permissions.saveState(state)
        activityResults.saveState(state)
    }

    fun setActive(active: Boolean) {
        permissions.setActive(active)
        activityResults.setActive(active)
    }

    fun close() {
        permissions.close()
        activityResults.close()
    }
}

private fun applicationView(activity: Activity): LeptosKitApplicationView {
    val embedded = activity.assets.open("app.lkb").use { it.readBytes() }
    val configuration = activity.assets.open("codepush.json").use { it.readBytes() }

    return LeptosKitApplicationView(activity, embedded, configuration) { bridge, dispatcher ->
        GeneratedPlugins.register(activity, bridge, dispatcher)
    }
}

/** Asks the guest's `leptoskit_system_back` export to pop the file router. */
private fun offerSystemBack(content: LeptosKitApplicationView?): Boolean {
    if (content == null) {
        return false
    }
    return try {
        val method = content.javaClass.getMethod("offerSystemBack")
        method.invoke(content) as? Boolean == true
    } catch (_: ReflectiveOperationException) {
        false
    }
}

/** Pushes an inbound VIEW intent into the deep-link plugin when registered. */
private fun noteDeepLink(intent: Intent?) {
    if (intent == null) {
        return
    }
    try {
        val plugin = Class.forName("dev.leptoskit.plugins.deeplink.DeepLinkPlugin")
        plugin.getMethod("noteIntentIfPresent", Intent::class.java).invoke(null, intent)
    } catch (_: ClassNotFoundException) {
    } catch (_: ReflectiveOperationException) {
    }
}
