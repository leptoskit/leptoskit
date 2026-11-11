package dev.leptoskit.plugins.devclient

import android.app.Activity
import android.content.Context
import com.journeyapps.barcodescanner.ScanIntentResult
import com.journeyapps.barcodescanner.ScanOptions
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import dev.leptoskit.mobile.DevelopmentSession
import java.nio.ByteBuffer

/**
 * Follows a development guest and reads one QR code.
 *
 * `connect` only starts the follower. The activity swaps the guest in when
 * the dev server has one. `scan` needs the hosting activity's result broker.
 */
class DevClientPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "dev-client"
    override val methods = intArrayOf(STATUS, CONNECT, SCAN)

    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            STATUS -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                DevClientWire.status(true)
            }

            CONNECT -> {
                val url = DevClientWire.guestUrl(payload)
                if (!DevelopmentSession.follow(context, url)) {
                    throw PluginException(PluginException.UNAVAILABLE)
                }
                ByteArray(0)
            }

            SCAN -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                DevClientWire.encode(scan())
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    override fun close() {
        closed = true
    }

    private suspend fun scan(): String {
        val host = context as? NativeActivityResultHost ?: throw PluginException(PluginException.UNAVAILABLE)
        val options =
            ScanOptions()
                .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                .setPrompt("Scan the code from leptoskit dev")
                .setBeepEnabled(false)
                .setOrientationLocked(false)
        val result =
            try {
                host.launchPluginActivity(options.createScanIntent(context))
            } catch (error: PluginException) {
                throw error
            }
        if (result.resultCode == Activity.RESULT_CANCELED) {
            throw PluginException(PluginException.CANCELLED)
        }
        val text = ScanIntentResult.parseActivityResult(result.resultCode, result.data).contents
        if (text.isNullOrEmpty()) throw PluginException(PluginException.CANCELLED)
        return text
    }

    private companion object {
        const val STATUS = 0
        const val CONNECT = 1
        const val SCAN = 2
    }
}
