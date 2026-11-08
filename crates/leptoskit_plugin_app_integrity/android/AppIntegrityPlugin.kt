package dev.leptoskit.plugins.appintegrity

import android.content.Context
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

/**
 * App attestation. Play Integrity remains later scope; this MVP validates the
 * wire and returns Unavailable without inventing a token.
 */
class AppIntegrityPlugin(
    @Suppress("UNUSED_PARAMETER") hostContext: Context,
) : NativePlugin {
    override val name: String = "app-integrity"
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
                AppIntegrityWire.capability(Capability.Unavailable)
            }

            1 -> {
                AppIntegrityWire.challenge(payload)
                AppIntegrityWire.attestationUnavailable()
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }
    }

    override fun close() {
        closed = true
    }
}
