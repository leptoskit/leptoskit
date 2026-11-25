package dev.leptoskit.plugins.liveactivities

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.graphics.Color
import android.os.Build
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Android implements ongoing progress Live Updates (methods 10 and 11),
 * upgrading to Android 16's `Notification.ProgressStyle` when running on
 * API 36+ and falling back to an ongoing progress notification on earlier
 * Android releases. iOS-only ActivityKit methods (0..=9) validate payloads
 * before returning Unavailable so malformed calls stay InvalidArgument on
 * every host.
 */
class LiveActivitiesPlugin(
    private val context: Context? = null,
) : NativePlugin {
    override val name = "live-activities"
    override val methods = intArrayOf(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11)
    private var closed = false

    override suspend fun invoke(
        method: Int,
        payload: ByteBuffer,
    ): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)

        if (method !in methods) throw PluginException(PluginException.UNAVAILABLE)

        when (method) {
            0, 1, 6, 7 -> {
                requireEmpty(payload)
            }

            2 -> {
                StartMessage.decode(payload)
            }

            3 -> {
                UpdateMessage.decode(payload)
            }

            4 -> {
                EndMessage.decode(payload)
            }

            5 -> {
                val reader = Reader(payload, 68)
                reader.identifier()
                reader.finish()
            }

            8 -> {
                ScheduledStartMessage.decode(payload)
            }

            9 -> {
                ScheduledUpdateMessage.decode(payload)
            }

            10 -> {
                val request = AndroidLiveUpdateMessage.decode(payload)
                postAndroidLiveUpdate(request)
                return encodeIdentifier(request.id)
            }

            11 -> {
                val reader = Reader(payload, 68)
                val id = reader.identifier()
                reader.finish()
                endAndroidLiveUpdate(id)
                return byteArrayOf()
            }

            else -> {
                throw PluginException(PluginException.UNAVAILABLE)
            }
        }

        throw PluginException(PluginException.UNAVAILABLE)
    }

    override fun close() {
        closed = true
    }

    private fun postAndroidLiveUpdate(request: AndroidLiveUpdateMessage) {
        val ctx = context?.applicationContext ?: throw PluginException(PluginException.UNAVAILABLE)
        val manager =
            ctx.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
                ?: throw PluginException(PluginException.UNAVAILABLE)

        if (Build.VERSION.SDK_INT >= 26) {
            val channel =
                NotificationChannel(
                    CHANNEL_ID,
                    "Live Updates",
                    NotificationManager.IMPORTANCE_DEFAULT,
                ).apply {
                    description = "Real-time ongoing progress updates"
                    setSound(null, null)
                    enableVibration(false)
                }
            manager.createNotificationChannel(channel)
        }

        val builder =
            if (Build.VERSION.SDK_INT >= 26) {
                Notification.Builder(ctx, CHANNEL_ID)
            } else {
                @Suppress("DEPRECATION")
                Notification.Builder(ctx)
            }

        val icon =
            if (ctx.applicationInfo.icon != 0) {
                ctx.applicationInfo.icon
            } else {
                android.R.drawable.ic_dialog_info
            }
        val scaledMax = 1000
        val scaledProgress = (request.progress * scaledMax).toInt().coerceIn(0, scaledMax)

        builder
            .setSmallIcon(icon)
            .setContentTitle(request.title)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setProgress(scaledMax, scaledProgress, false)

        if (request.body.isNotEmpty()) {
            builder.setContentText(request.body)
        }

        if (Build.VERSION.SDK_INT >= 36) {
            applyAndroid16ProgressStyle(builder, request)
        }

        manager.notify(LIVE_UPDATE_TAG, notificationId(request.id), builder.build())
    }

    private fun endAndroidLiveUpdate(id: String) {
        val ctx = context?.applicationContext ?: throw PluginException(PluginException.UNAVAILABLE)
        val manager =
            ctx.getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
                ?: throw PluginException(PluginException.UNAVAILABLE)
        manager.cancel(LIVE_UPDATE_TAG, notificationId(id))
    }

    /**
     * Best-effort Android 16 (API 36) `Notification.ProgressStyle` + promoted
     * ongoing status-bar chip wiring via reflection so this source compiles
     * cleanly against SDK 34..36 without losing native Android 16 behavior.
     */
    private fun applyAndroid16ProgressStyle(
        builder: Notification.Builder,
        request: AndroidLiveUpdateMessage,
    ) {
        try {
            val styleClass = Class.forName("android.app.Notification\$ProgressStyle")
            val style = styleClass.getDeclaredConstructor().newInstance()
            val segmentClass = Class.forName("android.app.Notification\$ProgressStyle\$Segment")
            val addSegment = styleClass.getMethod("addProgressSegment", segmentClass)
            var totalLength = 0

            if (request.segments.isEmpty()) {
                val segment = segmentClass.getConstructor(Int::class.javaPrimitiveType).newInstance(100)
                addSegment.invoke(style, segment)
                totalLength = 100
            } else {
                for (entry in request.segments) {
                    val segment =
                        segmentClass.getConstructor(Int::class.javaPrimitiveType).newInstance(entry.length)
                    if (entry.colorHex != null) {
                        segmentClass
                            .getMethod("setColor", Int::class.javaPrimitiveType)
                            .invoke(segment, Color.parseColor(entry.colorHex))
                    }
                    addSegment.invoke(style, segment)
                    totalLength += entry.length
                }
            }

            val progressUnits = (request.progress * totalLength).toInt().coerceIn(0, totalLength)
            styleClass.getMethod("setProgress", Int::class.javaPrimitiveType).invoke(style, progressUnits)
            builder.setStyle(style as Notification.Style)

            if (request.shortCriticalText.isNotEmpty()) {
                Notification.Builder::class.java
                    .getMethod("setShortCriticalText", CharSequence::class.java)
                    .invoke(builder, request.shortCriticalText)
            }
            Notification.Builder::class.java
                .getMethod("setRequestPromotedOngoing", Boolean::class.javaPrimitiveType)
                .invoke(builder, true)
        } catch (_: Throwable) {
            // Fall back to the standard ongoing progress notification already configured on `builder`.
        }
    }

    private fun notificationId(id: String): Int = id.hashCode() and 0x7FFF_FFFF

    private fun encodeIdentifier(id: String): ByteArray {
        val utf8 = id.toByteArray(StandardCharsets.UTF_8)
        return ByteBuffer
            .allocate(4 + utf8.size)
            .order(ByteOrder.LITTLE_ENDIAN)
            .putInt(utf8.size)
            .put(utf8)
            .array()
    }

    private companion object {
        const val CHANNEL_ID = "dev.leptoskit.live_updates"
        const val LIVE_UPDATE_TAG = "dev.leptoskit.live_updates"
    }
}

private fun requireEmpty(payload: ByteBuffer) {
    if (payload.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
}

private object StartMessage {
    fun decode(payload: ByteBuffer) {
        val reader = Reader(payload, 4096)
        decodeFrom(reader)
        reader.finish()
    }

    fun decodeFrom(reader: Reader) {
        reader.identifier()
        reader.identifier()
        reader.text(128, required = true)

        when (reader.u32()) {
            0, 1 -> Unit
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        reader.fields()
    }
}

private object UpdateMessage {
    fun decode(payload: ByteBuffer) {
        val reader = Reader(payload, 4096)
        decodeFrom(reader)
        reader.finish()
    }

    fun decodeFrom(reader: Reader) {
        reader.identifier()
        reader.fields()
    }
}

private object ScheduleMessage {
    fun decodeFrom(reader: Reader) {
        if (reader.u32() != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
        val hasStale = reader.u32()
        val seconds = reader.u32()
        when (hasStale) {
            0 -> if (seconds != 0) throw PluginException(PluginException.INVALID_ARGUMENT)
            1 -> if (seconds !in 1..86_400) throw PluginException(PluginException.INVALID_ARGUMENT)
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        val relevance = java.lang.Float.intBitsToFloat(reader.u32())
        if (!relevance.isFinite() || relevance < 0.0f || relevance > 1.0f) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }
        when (reader.u32()) {
            0, 1 -> Unit
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }
    }
}

private object ScheduledStartMessage {
    fun decode(payload: ByteBuffer) {
        val reader = Reader(payload, 4200)
        StartMessage.decodeFrom(reader)
        ScheduleMessage.decodeFrom(reader)
        reader.finish()
    }
}

private object ScheduledUpdateMessage {
    fun decode(payload: ByteBuffer) {
        val reader = Reader(payload, 4200)
        UpdateMessage.decodeFrom(reader)
        ScheduleMessage.decodeFrom(reader)
        reader.finish()
    }
}

private data class ProgressSegment(
    val length: Int,
    val colorHex: String?,
)

private data class AndroidLiveUpdateMessage(
    val id: String,
    val title: String,
    val body: String,
    val shortCriticalText: String,
    val progress: Float,
    val segments: List<ProgressSegment>,
) {
    companion object {
        fun decode(payload: ByteBuffer): AndroidLiveUpdateMessage {
            val reader = Reader(payload, 4096)
            val id = reader.identifier()
            if (reader.u32() != 1) throw PluginException(PluginException.INVALID_ARGUMENT)
            val title = reader.text(128, required = true)
            val body = reader.text(256, required = false)
            val shortCriticalText = reader.text(32, required = false)
            val progress = java.lang.Float.intBitsToFloat(reader.u32())
            if (!progress.isFinite() || progress < 0.0f || progress > 1.0f) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }
            val count = reader.u32()
            if (count < 0 || count > 8) throw PluginException(PluginException.TOO_LARGE)
            val segments = ArrayList<ProgressSegment>(count)
            repeat(count) {
                val length = reader.u32()
                if (length !in 1..10_000) throw PluginException(PluginException.INVALID_ARGUMENT)
                val colorHex =
                    when (reader.u32()) {
                        0 -> {
                            null
                        }

                        1 -> {
                            val hex = reader.text(7, required = true)
                            if (!isValidHexColor(hex)) throw PluginException(PluginException.INVALID_ARGUMENT)
                            hex
                        }

                        else -> {
                            throw PluginException(PluginException.INVALID_ARGUMENT)
                        }
                    }
                segments.add(ProgressSegment(length, colorHex))
            }
            reader.finish()
            return AndroidLiveUpdateMessage(id, title, body, shortCriticalText, progress, segments)
        }

        private fun isValidHexColor(value: String): Boolean {
            if (value.length != 7 || value[0] != '#') return false
            return value.substring(1).all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }
        }
    }
}

private object EndMessage {
    fun decode(payload: ByteBuffer) {
        val reader = Reader(payload, 4096)
        reader.identifier()
        val kind = reader.u32()
        val after = reader.u32()

        when (kind) {
            0, 1 -> if (after != 0) throw PluginException(PluginException.INVALID_ARGUMENT)
            2 -> if (after !in 1..86_400) throw PluginException(PluginException.INVALID_ARGUMENT)
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        when (reader.u32()) {
            0 -> Unit
            1 -> reader.fields()
            else -> throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        reader.finish()
    }
}

private class Reader(
    private val buffer: ByteBuffer,
    maximum: Int,
) {
    private val endExclusive = buffer.position() + buffer.remaining()

    init {
        if (buffer.remaining() > maximum) throw PluginException(PluginException.INVALID_ARGUMENT)
        buffer.order(ByteOrder.LITTLE_ENDIAN)
    }

    fun u32(): Int {
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)

        return buffer.int
    }

    fun identifier(): String {
        val value = text(64, required = true)

        if (!value.all { it.isLetterOrDigit() || it == '-' || it == '_' || it == '.' }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return value
    }

    fun text(
        maximum: Int,
        required: Boolean,
    ): String {
        if (buffer.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        val length = buffer.int

        if (length < 0 || length > maximum || length > buffer.remaining()) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        val bytes = ByteArray(length)
        buffer.get(bytes)
        val value =
            try {
                StandardCharsets.UTF_8
                    .newDecoder()
                    .onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT)
                    .decode(ByteBuffer.wrap(bytes))
                    .toString()
            } catch (_: CharacterCodingException) {
                throw PluginException(PluginException.INVALID_ARGUMENT)
            }

        if (required && value.trim().isEmpty()) throw PluginException(PluginException.INVALID_ARGUMENT)

        if (value.any { it.code <= 31 || it.code in 127..159 }) {
            throw PluginException(PluginException.INVALID_ARGUMENT)
        }

        return value
    }

    fun fields() {
        val count = u32()

        if (count !in 0..16) throw PluginException(PluginException.INVALID_ARGUMENT)
        val seen = HashSet<String>()

        repeat(count) {
            val key = identifier()

            if (!seen.add(key)) throw PluginException(PluginException.INVALID_ARGUMENT)
            text(256, required = false)
        }
    }

    fun finish() {
        if (buffer.position() != endExclusive) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}
