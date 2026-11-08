package dev.leptoskit.plugins.livephoto

import android.content.Context
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

/**
 * Android has no first-party Live Photo surface in this MVP. Wire validation
 * still runs so guests get honest `Unavailable` rather than invented motion.
 */
class LivePhotoPlugin(
    @Suppress("UNUSED_PARAMETER") hostContext: Context,
) : NativePlugin {
    override val name: String = "live-photo"
    override val methods: IntArray = intArrayOf(0, 1, 2, 3, 4)

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        when (method) {
            0 -> LivePhotoWire.create(payload)
            1 -> LivePhotoWire.startPlayback(payload)
            2, 3, 4 -> LivePhotoWire.session(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
        throw PluginException(PluginException.UNAVAILABLE)
    }

    override fun close() {}
}
