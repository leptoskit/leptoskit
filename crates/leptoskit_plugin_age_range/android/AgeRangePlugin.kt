package dev.leptoskit.plugins.agerange

import android.content.Context
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

/**
 * Declared age-range prompts. Android Play Age Signals remain later scope; this
 * MVP validates the wire and returns Unavailable without inventing a band.
 */
class AgeRangePlugin(
    @Suppress("UNUSED_PARAMETER") hostContext: Context,
) : NativePlugin {
    override val name: String = "age-range"
    override val methods: IntArray = intArrayOf(0, 1)
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0 -> {
                if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
                AgeRangeWire.capability(Capability.Unavailable)
            }
            1 -> {
                AgeRangeWire.threshold(payload)
                AgeRangeWire.responseUnavailable()
            }
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        closed = true
    }
}
