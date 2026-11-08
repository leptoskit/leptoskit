package dev.leptoskit.plugins.sms

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Looper
import dev.leptoskit.bridge.NativeActivityResultHost
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * Presents the system SMS composer through ACTION_SENDTO. An empty reply is the
 * OS activity result only, never delivery confirmation. Outbound sms: URLs stay
 * in the linking plugin.
 */
class SmsPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "sms"
    override val methods = intArrayOf(AVAILABILITY, PRESENT)
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

            else -> {
                present(SmsCompose.decode(payload))
            }
        }
    }

    private fun availability(): Int =
        when {
            busy -> BUSY
            !canCompose() -> ABSENT
            host == null || activeOwner() == null -> INACTIVE
            else -> READY
        }

    private suspend fun present(compose: SmsCompose): ByteArray {
        if (busy) throw PluginException(PluginException.BUSY)

        val host = host ?: throw PluginException(PluginException.UNAVAILABLE)

        if (!canCompose()) throw PluginException(PluginException.UNAVAILABLE)

        if (activeOwner() == null) throw PluginException(PluginException.UNAVAILABLE)

        currentCoroutineContext().ensureActive()
        busy = true

        try {
            val intent =
                Intent(Intent.ACTION_SENDTO).apply {
                    data = Uri.parse(smsUri(compose.recipients))
                    if (compose.body.isNotEmpty()) putExtra("sms_body", compose.body)
                }
            val result = host.launchPluginActivity(intent)

            currentCoroutineContext().ensureActive()

            if (closed) throw PluginException(PluginException.CANCELLED)

            // RESULT_CANCELED and RESULT_OK both mean the composer finished.
            // Neither is a delivery receipt.
            if (result.resultCode != Activity.RESULT_OK && result.resultCode != Activity.RESULT_CANCELED) {
                throw PluginException(PluginException.NATIVE)
            }

            return byteArrayOf()
        } finally {
            busy = false
        }
    }

    private fun canCompose(): Boolean {
        val intent = Intent(Intent.ACTION_SENDTO, Uri.parse("smsto:"))

        return context.packageManager.resolveActivity(intent, PackageManager.MATCH_DEFAULT_ONLY) != null
    }

    private fun activeOwner(): Activity? =
        (context as? Activity)?.takeIf {
            !it.isFinishing && !it.isDestroyed && it.window?.decorView?.isAttachedToWindow == true
        }

    private fun smsUri(recipients: List<String>): String = "smsto:" + recipients.joinToString(";") { Uri.encode(it) }

    private fun encode(status: Int): ByteArray =
        ByteBuffer
            .allocate(4)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(status)
            .array()

    private fun checkMain() {
        check(Looper.myLooper() == Looper.getMainLooper())
    }

    private companion object {
        const val AVAILABILITY = 0
        const val PRESENT = 1
        const val ABSENT = 1
        const val READY = 2
        const val INACTIVE = 3
        const val BUSY = 4
    }
}
