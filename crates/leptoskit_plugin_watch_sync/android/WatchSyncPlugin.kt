package dev.leptoskit.plugins.watchsync

import android.content.Context
import com.google.android.gms.wearable.DataClient
import com.google.android.gms.wearable.DataEvent
import com.google.android.gms.wearable.DataEventBuffer
import com.google.android.gms.wearable.DataMapItem
import com.google.android.gms.wearable.MessageClient
import com.google.android.gms.wearable.MessageEvent
import com.google.android.gms.wearable.PutDataMapRequest
import com.google.android.gms.wearable.Wearable
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.tasks.await
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.ArrayDeque

/**
 * Bidirectional phone <-> Wear OS (including Samsung Galaxy Watch) companion
 * messaging and durable transfers backed by Google Play Services Wearable.
 */
class WatchSyncPlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "watch-sync"
    override val methods = intArrayOf(0, 1, 2, 3, 4, 5)

    private val lock = Any()
    private val incoming = ArrayDeque<ByteArray>()
    private var incomingWaiter: CompletableDeferred<ByteArray>? = null
    private var closed = false

    private val messageListener =
        MessageClient.OnMessageReceivedListener { event: MessageEvent ->
            enqueue(encodeEnvelope("", event.path, System.currentTimeMillis(), event.data))
        }

    private val dataListener =
        DataClient.OnDataChangedListener { buffer: DataEventBuffer ->
            for (event in buffer) {
                if (event.type == DataEvent.TYPE_CHANGED) {
                    val map = DataMapItem.fromDataItem(event.dataItem).dataMap
                    val id = map.getString("id") ?: ""
                    val topic = map.getString("topic") ?: event.dataItem.uri.path ?: "/transfer"
                    val payload = map.getByteArray("payload") ?: ByteArray(0)
                    enqueue(encodeEnvelope(id, topic, System.currentTimeMillis(), payload))
                }
            }
        }

    init {
        runCatching {
            Wearable.getMessageClient(context).addListener(messageListener)
            Wearable.getDataClient(context).addListener(dataListener)
        }
    }

    override suspend fun invoke(method: Int, payload: ByteBuffer): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0 -> status()
            1 -> sendMessage(payload)
            2 -> queueTransfer(payload)
            3 -> nextIncoming()
            4 -> ByteArray(0)
            5 -> status()
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        closed = true
        runCatching {
            Wearable.getMessageClient(context).removeListener(messageListener)
            Wearable.getDataClient(context).removeListener(dataListener)
        }
        synchronized(lock) {
            incomingWaiter?.cancel()
            incomingWaiter = null
        }
    }

    private suspend fun status(): ByteArray {
        val nodes =
            runCatching { Wearable.getNodeClient(context).connectedNodes.await() }
                .getOrElse { emptyList() }
        val peer = nodes.firstOrNull()
        var flags = 1 // supported
        if (peer != null) {
            flags = flags or 2 or 8 // paired + installed
            if (peer.isNearby) flags = flags or 4 // reachable
        }
        return encodeStatus(2, flags, peer?.displayName)
    }

    private suspend fun sendMessage(payload: ByteBuffer): ByteArray {
        val reader = WireReader(payload)
        val topic = reader.string()
        val data = reader.bytes()
        reader.finished()
        val nodes = Wearable.getNodeClient(context).connectedNodes.await()
        if (nodes.isEmpty()) throw PluginException(PluginException.UNAVAILABLE)
        val client = Wearable.getMessageClient(context)
        for (node in nodes) {
            client.sendMessage(node.id, topic, data).await()
        }
        return ByteArray(0)
    }

    private suspend fun queueTransfer(payload: ByteBuffer): ByteArray {
        val reader = WireReader(payload)
        val id = reader.string()
        val topic = reader.string()
        val data = reader.bytes()
        reader.finished()
        val request =
            PutDataMapRequest.create("/leptoskit/transfer/$id").apply {
                dataMap.putString("id", id)
                dataMap.putString("topic", topic)
                dataMap.putByteArray("payload", data)
                dataMap.putLong("ts", System.currentTimeMillis())
            }
        Wearable.getDataClient(context).putDataItem(request.asPutDataRequest().setUrgent()).await()
        return ByteArray(0)
    }

    private suspend fun nextIncoming(): ByteArray {
        synchronized(lock) {
            val existing = incoming.pollFirst()
            if (existing != null) return existing
            val deferred = CompletableDeferred<ByteArray>()
            incomingWaiter = deferred
            return deferred.await()
        }
    }

    private fun enqueue(envelope: ByteArray) {
        synchronized(lock) {
            val waiter = incomingWaiter
            if (waiter != null) {
                incomingWaiter = null
                waiter.complete(envelope)
            } else {
                if (incoming.size >= 64) incoming.removeFirst()
                incoming.addLast(envelope)
            }
        }
    }
}

private const val ABSENT = -1

private class WireReader(payload: ByteBuffer) {
    private val buf = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

    fun u32(): Int {
        if (buf.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        return buf.int
    }

    fun bytes(): ByteArray {
        val len = u32()
        if (len < 0 || len > buf.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val out = ByteArray(len)
        buf.get(out)
        return out
    }

    fun string(): String = String(bytes(), Charsets.UTF_8)

    fun finished() {
        if (buf.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}

private fun encodeStatus(transport: Int, flags: Int, peerName: String?): ByteArray {
    val out = ByteArrayOutputStream()
    writeU32(out, transport)
    writeU32(out, flags)
    if (peerName == null) {
        writeU32(out, ABSENT)
    } else {
        val bytes = peerName.toByteArray(Charsets.UTF_8)
        writeU32(out, bytes.size)
        out.write(bytes)
    }
    return out.toByteArray()
}

private fun encodeEnvelope(id: String, topic: String, receivedMs: Long, payload: ByteArray): ByteArray {
    val out = ByteArrayOutputStream()
    writeBytes(out, id.toByteArray(Charsets.UTF_8))
    writeBytes(out, topic.toByteArray(Charsets.UTF_8))
    writeI64(out, receivedMs)
    writeBytes(out, payload)
    return out.toByteArray()
}

private fun writeBytes(out: ByteArrayOutputStream, bytes: ByteArray) {
    writeU32(out, bytes.size)
    out.write(bytes)
}

private fun writeU32(out: ByteArrayOutputStream, value: Int) {
    out.write(value and 0xFF)
    out.write((value shr 8) and 0xFF)
    out.write((value shr 16) and 0xFF)
    out.write((value shr 24) and 0xFF)
}

private fun writeI64(out: ByteArrayOutputStream, value: Long) {
    for (shift in 0 until 64 step 8) {
        out.write(((value shr shift) and 0xFF).toInt())
    }
}
