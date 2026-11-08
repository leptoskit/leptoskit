package dev.leptoskit.plugins.webbrowser

import android.app.Activity
import android.content.Context
import android.net.Uri
import android.os.Looper
import androidx.browser.customtabs.CustomTabsIntent
import androidx.browser.customtabs.CustomTabsClient
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Opens HTTP(S) in Chrome Custom Tabs and answers when the tab returns.
 * Distinct from web-auth (no OAuth return scheme) and linking.open (which does
 * not wait for dismiss).
 */
class WebBrowserPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "web-browser"
    override val methods = intArrayOf(AVAILABILITY, OPEN)
    private val host = context as? NativeActivityResultHost
    private var busy = false
    private var closed = false

    override fun close() {
        closed = true
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        checkMain()

        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        return when (method) {
            AVAILABILITY -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)

                encode(availability())
            }

            else -> open(WebBrowserOpen.decode(payload))
        }
    }

    private fun availability(): Int =
        when {
            busy -> BUSY
            CustomTabsClient.getPackageName(context, null) == null -> UNSUPPORTED
            host == null || activeOwner() == null -> INACTIVE
            else -> READY
        }

    private suspend fun open(request: WebBrowserOpen): ByteArray {
        if (busy) throw PluginException(PluginException.BUSY)

        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)

        if (CustomTabsClient.getPackageName(context, null) == null) {
            throw PluginException(PluginException.UNAVAILABLE)
        }

        if (activeOwner() == null) throw PluginException(PluginException.UNAVAILABLE)

        currentCoroutineContext().ensureActive()
        busy = true

        try {
            val intent = CustomTabsIntent.Builder().build().intent.apply {
                data = Uri.parse(request.url)
            }
            val result = host.launchPluginActivity(intent)

            currentCoroutineContext().ensureActive()

            if (closed) throw PluginException(PluginException.CANCELLED)

            if (result.resultCode != Activity.RESULT_OK && result.resultCode != Activity.RESULT_CANCELED) {
                throw PluginException(PluginException.NATIVE)
            }

            return byteArrayOf()
        } finally {
            busy = false
        }
    }

    private fun activeOwner(): Activity? =
        (context as? Activity)?.takeIf {
            !it.isFinishing && !it.isDestroyed && it.window?.decorView?.isAttachedToWindow == true
        }

    private fun encode(status: Int): ByteArray =
        ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(status).array()

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }

    private companion object {
        const val AVAILABILITY = 0
        const val OPEN = 1
        const val UNSUPPORTED = 0
        const val READY = 1
        const val INACTIVE = 2
        const val BUSY = 3
    }
}
