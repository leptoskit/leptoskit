package dev.leptoskit.plugins.diagnostics
import android.content.Context
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
class DiagnosticsPlugin(@Suppress("UNUSED_PARAMETER") hostContext: Context) : NativePlugin {
    override val name: String = "diagnostics"
    override val methods: IntArray = intArrayOf(0, 1, 2)
    private var closed = false
    override suspend fun invoke(method: Int, payload: ByteBuffer): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        when (method) {
            0, 2 -> { DiagnosticsWire.name(payload); throw PluginException(PluginException.UNAVAILABLE) }
            1 -> { DiagnosticsWire.id(payload); throw PluginException(PluginException.UNAVAILABLE) }
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }
    override fun close() { closed = true }
}
