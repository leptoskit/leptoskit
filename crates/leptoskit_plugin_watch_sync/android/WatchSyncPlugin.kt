package dev.leptoskit.plugins.watchsync

import android.content.Context
import android.net.Uri
import com.google.android.gms.tasks.Task
import com.google.android.gms.wearable.Asset
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
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.ArrayDeque
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

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
    private val pendingUris = ConcurrentHashMap<String, Uri>()
    private val assetExecutor = Executors.newSingleThreadExecutor()
    private var incomingWaiter: CompletableDeferred<ByteArray>? = null

    @Volatile
    private var localNodeId: String? = null
    private var closed = false

    private val messageListener =
        MessageClient.OnMessageReceivedListener { event: MessageEvent ->
            enqueue(encodeEnvelope("", event.path, System.currentTimeMillis(), event.data))
        }

    private val dataListener =
        DataClient.OnDataChangedListener { buffer: DataEventBuffer ->
            for (event in buffer) {
                if (event.type == DataEvent.TYPE_CHANGED) {
                    val uri = event.dataItem.uri
                    val selfId = localNodeId
                    if (selfId != null && uri.host == selfId) continue
                    val map = DataMapItem.fromDataItem(event.dataItem).dataMap
                    val id = map.getString("id") ?: ""
                    val topic = map.getString("topic") ?: uri.path ?: "/transfer"
                    val inline = map.getByteArray("payload")
                    val asset = if (inline == null) map.getAsset("payload_asset") else null
                    if (id.isNotEmpty()) pendingUris[id] = uri
                    if (inline != null) {
                        enqueue(encodeEnvelope(id, topic, System.currentTimeMillis(), inline))
                    } else if (asset != null) {
                        enqueueAsset(id, topic, asset)
                    } else {
                        enqueue(encodeEnvelope(id, topic, System.currentTimeMillis(), ByteArray(0)))
                    }
                }
            }
        }

    init {
        runCatching {
            Wearable.getNodeClient(context).localNode.addOnSuccessListener { node ->
                localNodeId = node.id
            }
            Wearable.getMessageClient(context).addListener(messageListener)
            Wearable.getDataClient(context).addListener(dataListener)
        }
    }

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0 -> status()
            1 -> sendMessage(payload)
            2 -> queueTransfer(payload)
            3 -> nextIncoming()
            4 -> ackTransfer(payload)
            5 -> watchStatus(payload)
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        closed = true
        runCatching {
            Wearable.getMessageClient(context).removeListener(messageListener)
            Wearable.getDataClient(context).removeListener(dataListener)
        }
        assetExecutor.shutdownNow()
        synchronized(lock) {
            incomingWaiter?.completeExceptionally(PluginException(PluginException.CANCELLED))
            incomingWaiter = null
        }
    }

    private suspend fun ensureLocalNodeId(): String? {
        localNodeId?.let { return it }
        return runCatching {
            Wearable
                .getNodeClient(context)
                .localNode
                .awaitResult()
                .id
        }.getOrNull()
            ?.also { localNodeId = it }
    }

    private fun enqueueAsset(
        id: String,
        topic: String,
        asset: Asset,
    ) {
        Wearable
            .getDataClient(context)
            .getFdForAsset(asset)
            .addOnSuccessListener(assetExecutor) { result ->
                if (closed) return@addOnSuccessListener
                val bytes =
                    runCatching { result.inputStream.use { stream -> stream.readBytes() } }
                        .getOrNull()
                        ?: return@addOnSuccessListener
                enqueue(encodeEnvelope(id, topic, System.currentTimeMillis(), bytes))
            }
    }

    private suspend fun status(): ByteArray {
        val nodes =
            runCatching { Wearable.getNodeClient(context).connectedNodes.awaitResult() }
                .getOrElse {
                    return encodeStatus(0, 0, null)
                }
        val peer = nodes.firstOrNull()
        var flags = 1 // supported
        if (peer != null) {
            flags = flags or 2 or 8 // paired + installed
            if (peer.isNearby) flags = flags or 4 // reachable
        }
        return encodeStatus(2, flags, peer?.displayName)
    }

    private suspend fun watchStatus(payload: ByteBuffer): ByteArray {
        val last = ByteArray(payload.remaining())
        payload.duplicate().get(last)
        while (!closed) {
            val current = status()
            if (!current.contentEquals(last)) return current
            delay(1_500L)
        }
        throw PluginException(PluginException.CANCELLED)
    }

    private suspend fun sendMessage(payload: ByteBuffer): ByteArray {
        val reader = WireReader(payload)
        val topic = reader.string()
        val data = reader.bytes()
        reader.finished()
        val nodes = Wearable.getNodeClient(context).connectedNodes.awaitResult()
        if (nodes.isEmpty()) throw PluginException(PluginException.UNAVAILABLE)
        val client = Wearable.getMessageClient(context)
        for (node in nodes) {
            client.sendMessage(node.id, topic, data).awaitResult()
        }
        return ByteArray(0)
    }

    private suspend fun queueTransfer(payload: ByteBuffer): ByteArray {
        ensureLocalNodeId()
        val reader = WireReader(payload)
        val id = reader.string()
        val topic = reader.string()
        val data = reader.bytes()
        reader.finished()
        val safeId = Uri.encode(id)
        val request =
            PutDataMapRequest.create("/leptoskit/transfer/$safeId").apply {
                dataMap.putString("id", id)
                dataMap.putString("topic", topic)
                if (data.size > INLINE_DATA_LIMIT) {
                    dataMap.putAsset("payload_asset", Asset.createFromBytes(data))
                } else {
                    dataMap.putByteArray("payload", data)
                }
                dataMap.putLong("ts", System.currentTimeMillis())
            }
        Wearable
            .getDataClient(context)
            .putDataItem(request.asPutDataRequest().setUrgent())
            .awaitResult()
        return ByteArray(0)
    }

    private suspend fun ackTransfer(payload: ByteBuffer): ByteArray {
        val reader = WireReader(payload)
        val id = reader.string()
        reader.finished()
        val uri = pendingUris.remove(id)
        if (uri != null) {
            runCatching { Wearable.getDataClient(context).deleteDataItems(uri).awaitResult() }
        }
        return ByteArray(0)
    }

    private suspend fun nextIncoming(): ByteArray {
        val deferred =
            synchronized(lock) {
                val existing = incoming.pollFirst()
                if (existing != null) return existing
                CompletableDeferred<ByteArray>().also { incomingWaiter = it }
            }
        return deferred.await()
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

    private companion object {
        const val INLINE_DATA_LIMIT = 64 * 1024
    }
}

private suspend fun <T> Task<T>.awaitResult(): T =
    suspendCancellableCoroutine { cont ->
        addOnSuccessListener { result ->
            if (cont.isActive) cont.resume(result)
        }
        addOnFailureListener { error ->
            if (cont.isActive) cont.resumeWithException(error)
        }
        addOnCanceledListener {
            if (cont.isActive) cont.cancel()
        }
    }

private const val ABSENT = -1

private class WireReader(
    payload: ByteBuffer,
) {
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

private fun encodeStatus(
    transport: Int,
    flags: Int,
    peerName: String?,
): ByteArray {
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

private fun encodeEnvelope(
    id: String,
    topic: String,
    receivedMs: Long,
    payload: ByteArray,
): ByteArray {
    val out = ByteArrayOutputStream()
    writeBytes(out, id.toByteArray(Charsets.UTF_8))
    writeBytes(out, topic.toByteArray(Charsets.UTF_8))
    writeI64(out, receivedMs)
    writeBytes(out, payload)
    return out.toByteArray()
}

private fun writeBytes(
    out: ByteArrayOutputStream,
    bytes: ByteArray,
) {
    writeU32(out, bytes.size)
    out.write(bytes)
}

private fun writeU32(
    out: ByteArrayOutputStream,
    value: Int,
) {
    out.write(value and 0xFF)
    out.write((value shr 8) and 0xFF)
    out.write((value shr 16) and 0xFF)
    out.write((value shr 24) and 0xFF)
}

private fun writeI64(
    out: ByteArrayOutputStream,
    value: Long,
) {
    for (shift in 0 until 64 step 8) {
        out.write(((value shr shift) and 0xFF).toInt())
    }
}
