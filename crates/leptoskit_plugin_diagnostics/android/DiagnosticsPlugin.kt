package dev.leptoskit.plugins.diagnostics

import android.content.Context
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

class DiagnosticsPlugin(
    @Suppress("UNUSED_PARAMETER") hostContext: Context,
) : NativePlugin {
    override val name: String = "diagnostics"
    override val methods: IntArray = intArrayOf(0, 1, 2, 3)
    private var closed = false
    private var nextId = 0
    private val open = LinkedHashSet<Int>()
    private val maxOpen = 256

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0 -> {
                DiagnosticsWire.empty(payload)
                DiagnosticsWire.capability(Capability.Available)
            }
            1 -> {
                DiagnosticsWire.name(payload)
                if (open.size >= maxOpen) throw PluginException(PluginException.BUSY)
                if (nextId == Int.MAX_VALUE) throw PluginException(PluginException.EXHAUSTED)
                nextId += 1
                open.add(nextId)
                DiagnosticsWire.encodeId(nextId)
            }
            2 -> {
                val id = DiagnosticsWire.id(payload)
                if (!open.remove(id)) throw PluginException(PluginException.INVALID_ARGUMENT)
                ByteArray(0)
            }
            3 -> {
                DiagnosticsWire.name(payload)
                ByteArray(0)
            }
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        closed = true
    }
}
