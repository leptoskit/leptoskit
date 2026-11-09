package dev.leptoskit.plugins.blob

import android.content.Context
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer

class BlobPlugin(
    @Suppress("UNUSED_PARAMETER") hostContext: Context,
) : NativePlugin {
    override val name: String = "blob"
    override val methods: IntArray = intArrayOf(0, 1, 2, 3)
    private var closed = false
    private var nextId = 0
    private val entries = LinkedHashMap<Int, ByteArray>()
    private val maxLive = 256

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0 -> {
                BlobWire.empty(payload)
                BlobWire.capability(Capability.Available)
            }

            1 -> {
                val bytes = BlobWire.create(payload)
                if (entries.size >= maxLive) throw PluginException(PluginException.BUSY)
                if (nextId == Int.MAX_VALUE) throw PluginException(PluginException.EXHAUSTED)
                nextId += 1
                entries[nextId] = bytes
                BlobWire.encodeId(nextId)
            }

            2 -> {
                val (id, offset, length) = BlobWire.read(payload)
                val entry = entries[id] ?: throw PluginException(PluginException.INVALID_ARGUMENT)
                if (offset < 0 || offset >= entry.size) {
                    BlobWire.encodeSlice(ByteArray(0))
                } else {
                    val end = minOf(offset + length, entry.size)
                    BlobWire.encodeSlice(entry.copyOfRange(offset, end))
                }
            }

            3 -> {
                val id = BlobWire.id(payload)
                if (entries.remove(id) == null) throw PluginException(PluginException.INVALID_ARGUMENT)
                ByteArray(0)
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
